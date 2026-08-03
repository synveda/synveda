//! A context pack's document as a VedaFlow object (PRMT-2, ADR-0050).
//!
//! **The object is a document, not a pack.** ADR-0050 decision 3: one
//! entry per document, its name the path `pack/document`, its object the
//! document's content address. That is what makes the demotion rule work
//! — a chunk row carries the address of the document it was cut from, so a
//! chunk composes as published only while the tree names *that* document at
//! *that* address, and editing a published document demotes every chunk of
//! it rather than laundering the edit through chunks the tree still appears
//! to name (ADR-0031 decision 5, reaching chunks through their document).
//!
//! It is also the first *bundle* ADR-0032's curator glob has to glob over:
//! `payments/*` is a rule about one pack, which was not expressible while
//! prompts were the only paths.
//!
//! The encoding is [`crate::channels::MemoryAsset`]'s, for its reasons:
//! canonical JSON with bytewise-sorted keys, human-readable because FLOW-6
//! renders a diff of it and FLOW-8 exports it into a real git repository.
//!
//! # What is in the address, and what is not
//!
//! In: the pack, the document name, the scope, the tier, the title, and the
//! content. Those are the things a reviewer consents to.
//!
//! Out: **the author**, on [`crate::prompts`]'s reason — a handover is not
//! an edit. Out too: **the chunks**. They are a deterministic function of
//! the content ([`synveda_types::chunk`]), so hashing them would be hashing
//! the same bytes twice, and a chunker change must re-cut a document rather
//! than silently re-address one nobody edited.

use std::collections::HashMap;

use serde_json::json;
use sqlx::PgConnection;
use synveda_types::{
    AssetKind, Channel, ContextPackName, DocumentPath, Error, PackDocument, Result, ScopeId,
    Sensitivity, TenantId,
};

use crate::Written;
use crate::channels::{ChannelRef, read_members};
use crate::hash::{CommitHash, ObjectHash, object_hash};
use crate::objects::put_object;
use crate::policy::canonical_json;

/// One document of a context pack, as VedaFlow addresses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextPackAsset {
    /// The scope that stands behind it.
    pub scope_id: ScopeId,
    /// The bundle it belongs to.
    pub pack: ContextPackName,
    /// Its classification — declared per document rather than per pack
    /// (ADR-0050 decision 12), because a glossary of public terms and an
    /// internal runbook are plausibly the same bundle. Every chunk cut from
    /// it inherits this tier, which is what CTX-4 and ADR-0038's per-scope
    /// tier check then apply per entry.
    pub sensitivity: Sensitivity,
    /// The document: its name within the pack, its title, and its text.
    pub document: PackDocument,
}

impl ContextPackAsset {
    /// The object's bytes: canonical JSON, keys sorted bytewise.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let value = json!({
            "content": self.document.content,
            "document": self.document.name.as_str(),
            "pack": self.pack.as_str(),
            "scope": self.scope_id.as_uuid().to_string(),
            "sensitivity": self.sensitivity.as_str(),
            "title": self.document.title,
        });
        let mut out = String::with_capacity(self.document.content.len() + 512);
        // Every value is a string: the canonical form's float rejection
        // cannot fire, and the expect says so.
        canonical_json(&value, &mut out).expect("a context pack asset contains no numbers");
        out.into_bytes()
    }

    /// The content address — computable without touching the database, so
    /// composition can check a chunk's stored document address against the
    /// entry that admitted it for the cost of a hash.
    #[must_use]
    pub fn address(&self) -> ObjectHash {
        object_hash(AssetKind::ContextPack, &self.canonical_bytes())
    }

    /// The path this asset takes in a channel tree: `pack/document`.
    #[must_use]
    pub fn path(&self) -> DocumentPath {
        DocumentPath::new(self.pack.clone(), self.document.name.clone())
    }

    /// That path as the tree entry name.
    #[must_use]
    pub fn entry_name(&self) -> String {
        self.path().to_string()
    }

    /// Parses an asset back out of an object's bytes — what composition
    /// reads and what a review renders.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the bytes are not a document's canonical
    /// form. For a stored object that means the object and this code have
    /// drifted, and saying so beats guessing.
    pub fn from_bytes(bytes: &[u8]) -> Result<ContextPackAsset> {
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|err| Error::Invalid {
                message: format!("context pack object is not JSON: {err}"),
            })?;
        let field = |name: &str| -> Result<String> {
            value
                .get(name)
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .ok_or_else(|| Error::Invalid {
                    message: format!("context pack object has no string field {name:?}"),
                })
        };
        Ok(ContextPackAsset {
            scope_id: field("scope")?.parse().map_err(|err| Error::Invalid {
                message: format!("context pack object scope is not a scope id: {err}"),
            })?,
            pack: field("pack")?.parse()?,
            sensitivity: field("sensitivity")?.parse()?,
            document: PackDocument {
                name: field("document")?.parse()?,
                title: field("title")?,
                content: field("content")?,
            },
        })
    }
}

/// Writes a document's object, returning its address.
///
/// Dedups like every other object write, and here that is load-bearing
/// rather than merely tidy: re-authoring an unchanged document returns the
/// same address, so the chunk rows already cut from it still match and
/// nothing is re-embedded (ADR-0050 decision 4).
#[tracing::instrument(name = "vedaflow.put_context_pack", skip_all, fields(tenant.id = %tenant), err(Display))]
pub async fn put_context_pack(
    conn: &mut PgConnection,
    tenant: TenantId,
    asset: &ContextPackAsset,
) -> Result<Written<ObjectHash>> {
    put_object(
        conn,
        tenant,
        AssetKind::ContextPack,
        &asset.canonical_bytes(),
    )
    .await
}

/// One scope's context-pack channel, keyed the way composition reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextPackChannelState {
    /// The scope whose channel this is.
    pub scope_id: ScopeId,
    /// The commit the channel serves — what the block watermarks.
    pub commit: CommitHash,
    /// Whether a FLOW-7 pin chose that commit rather than the ref.
    pub pinned: bool,
    /// Document path → the address that scope admitted it at.
    pub members: HashMap<DocumentPath, ObjectHash>,
}

/// [`read_members`] for a context-pack channel, keyed by document path.
///
/// An entry name that is not a document path cannot occur — only this
/// crate writes pack channels and the surfaces above it parse paths before
/// they get here — so one is an internal error rather than a member to drop
/// quietly from a composition.
pub async fn read_context_pack_members(
    conn: &mut PgConnection,
    tenant: TenantId,
    scopes: &[ScopeId],
    channel: Channel,
) -> Result<Vec<ContextPackChannelState>> {
    read_members(conn, tenant, scopes, ChannelRef::context_pack(channel))
        .await?
        .into_iter()
        .map(|snapshot| {
            let members =
                snapshot
                    .members
                    .into_iter()
                    .map(|member| {
                        let path = member.name.parse::<DocumentPath>().map_err(|err| {
                            Error::Internal {
                                message: format!(
                                    "context pack channel entry {:?} is not a document path: {err}",
                                    member.name
                                ),
                            }
                        })?;
                        Ok((path, member.object))
                    })
                    .collect::<Result<HashMap<DocumentPath, ObjectHash>>>()?;
            Ok(ContextPackChannelState {
                scope_id: snapshot.scope_id,
                commit: snapshot.commit,
                pinned: snapshot.pinned,
                members,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn asset(content: &str) -> ContextPackAsset {
        ContextPackAsset {
            scope_id: ScopeId::from_uuid(Uuid::from_bytes([2; 16])),
            pack: "payments".parse().unwrap(),
            sensitivity: Sensitivity::Internal,
            document: PackDocument {
                name: "runbooks/refunds.md".parse().unwrap(),
                title: "Refunds runbook".to_owned(),
                content: content.to_owned(),
            },
        }
    }

    #[test]
    fn a_document_object_is_canonical_json_with_sorted_keys() {
        let text = String::from_utf8(asset("Escalate over £500.").canonical_bytes()).unwrap();
        assert_eq!(
            text,
            "{\"content\":\"Escalate over £500.\",\"document\":\"runbooks/refunds.md\",\
             \"pack\":\"payments\",\"scope\":\"02020202-0202-0202-0202-020202020202\",\
             \"sensitivity\":\"internal\",\"title\":\"Refunds runbook\"}"
        );
    }

    /// The property the whole demotion rule rests on (ADR-0050 decision 3):
    /// an edit moves the document's address, so every chunk cut from the
    /// old one stops matching the tree and falls off the published set.
    #[test]
    fn the_address_moves_with_every_governed_field() {
        let base = asset("Escalate over £500.");
        assert_eq!(base.address(), asset("Escalate over £500.").address());
        assert_ne!(
            base.address(),
            asset("Escalate over £5000.").address(),
            "content"
        );

        let mut renamed = base.clone();
        renamed.document.name = "runbooks/refunds-v2.md".parse().unwrap();
        assert_ne!(base.address(), renamed.address(), "document name");

        let mut rebundled = base.clone();
        rebundled.pack = "billing".parse().unwrap();
        assert_ne!(base.address(), rebundled.address(), "pack");

        let mut moved = base.clone();
        moved.scope_id = ScopeId::from_uuid(Uuid::from_bytes([9; 16]));
        assert_ne!(base.address(), moved.address(), "scope");

        let mut reclassified = base.clone();
        reclassified.sensitivity = Sensitivity::Confidential;
        assert_ne!(base.address(), reclassified.address(), "sensitivity");

        let mut retitled = base.clone();
        retitled.document.title = "Something else".to_owned();
        assert_ne!(base.address(), retitled.address(), "title");
    }

    #[test]
    fn the_object_names_no_author_and_no_chunks() {
        let text = String::from_utf8(asset("Escalate.").canonical_bytes()).unwrap();
        assert!(!text.contains("owner"), "{text}");
        assert!(!text.contains("author"), "{text}");
        // The chunks are a pure function of the content, so hashing them
        // would hash the same bytes twice.
        assert!(!text.contains("chunk"), "{text}");
    }

    #[test]
    fn the_entry_name_is_the_document_path() {
        let asset = asset("Escalate.");
        assert_eq!(asset.entry_name(), "payments/runbooks/refunds.md");
        assert_eq!(
            asset.entry_name().parse::<DocumentPath>().unwrap(),
            asset.path()
        );
    }

    #[test]
    fn the_bytes_round_trip_through_the_parser() {
        let original = asset("# Refunds\n\nEscalate over £500.\n");
        let parsed = ContextPackAsset::from_bytes(&original.canonical_bytes()).expect("parse");
        assert_eq!(parsed, original);
        assert_eq!(parsed.address(), original.address());
        parsed
            .document
            .validate()
            .expect("a stored document is valid");
    }

    #[test]
    fn a_malformed_object_is_named_rather_than_guessed_at() {
        assert!(ContextPackAsset::from_bytes(b"not json").is_err());
        assert!(ContextPackAsset::from_bytes(br#"{"pack":"a"}"#).is_err());
        // A name the vocabulary refuses cannot be resurrected by having
        // been stored once.
        assert!(
            ContextPackAsset::from_bytes(
                br#"{"content":"c","document":"Not A Name","pack":"payments","scope":
                     "02020202-0202-0202-0202-020202020202","sensitivity":"internal",
                     "title":"t"}"#
            )
            .is_err()
        );
    }

    /// Asset kind is part of the address (ADR-0030 decision 4), which is
    /// ADR-0050 option 8's second surviving reason for the pack being an
    /// asset at all: identical bytes governed differently are different
    /// objects.
    #[test]
    fn the_asset_kind_is_part_of_the_address() {
        let bytes = asset("Escalate.").canonical_bytes();
        assert_ne!(
            object_hash(AssetKind::ContextPack, &bytes),
            object_hash(AssetKind::Memory, &bytes)
        );
        assert_ne!(
            object_hash(AssetKind::ContextPack, &bytes),
            object_hash(AssetKind::Prompt, &bytes)
        );
    }
}
