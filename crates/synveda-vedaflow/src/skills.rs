//! A skill's bundled file as a VedaFlow object (SKIL-1, ADR-0051).
//!
//! **The object is a file, not a bundle** — [`crate::packs`]'s decision,
//! taken again for its reasons: one entry per file, its name the path
//! `skill/path`, its object the file's content address. So editing one line
//! of `SKILL.md` re-stores one object, FLOW-6's diff renders per file, and
//! ADR-0032's curator glob gets `code-review/*` to glob over.
//!
//! The encoding is [`crate::channels::MemoryAsset`]'s, for its reasons:
//! canonical JSON with bytewise-sorted keys, human-readable because FLOW-6
//! renders a diff of it and FLOW-8 exports it into a real git repository.
//!
//! # "Unmodified" is a property of materialisation, not of storage
//!
//! ADR-0051 force 1 says a skill's bytes must leave Synveda untouched, and
//! this module wraps them in an envelope — which looks like a contradiction
//! and is not. What a client reads is the `content` field written verbatim
//! to `<root>/<skill>/<path>`; the envelope never reaches a disk. The
//! governed fields have to be inside the address for ADR-0030 decision 4's
//! reason (identical bytes governed differently are different objects), and
//! a tier that lived only on a mutable draft row could be raised or lowered
//! after review without moving anything a reviewer signed.
//!
//! # What is in the address, and what is not
//!
//! In: the skill, the path, the scope, the tier, and the content. Those are
//! the things a reviewer consents to.
//!
//! Out: **the author**, on [`crate::prompts`]'s reason — a handover is not
//! an edit. Out too: **the frontmatter**, which is a deterministic parse of
//! `SKILL.md`'s own bytes, so hashing it would be hashing the same bytes
//! twice.

use serde_json::json;
use sqlx::PgConnection;
use synveda_types::{
    AssetKind, Error, Result, ScopeId, Sensitivity, SkillFile, SkillName, TenantId,
};

use crate::Written;
use crate::hash::{ObjectHash, object_hash};
use crate::objects::put_object;
use crate::policy::canonical_json;

/// One file of a skill bundle, as VedaFlow addresses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillAsset {
    /// The scope that stands behind it.
    pub scope_id: ScopeId,
    /// The bundle it belongs to.
    pub skill: SkillName,
    /// The bundle's classification — declared per *skill* rather than per
    /// file (ADR-0051 decision 11), because a client loads a bundle whole,
    /// and carried on every file's envelope so that reclassifying
    /// re-addresses all of them.
    pub sensitivity: Sensitivity,
    /// The file: its path within the bundle, and its bytes.
    pub file: SkillFile,
}

impl SkillAsset {
    /// The object's bytes: canonical JSON, keys sorted bytewise.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let value = json!({
            "content": self.file.content,
            "path": self.file.path.as_str(),
            "scope": self.scope_id.as_uuid().to_string(),
            "sensitivity": self.sensitivity.as_str(),
            "skill": self.skill.as_str(),
        });
        let mut out = String::with_capacity(self.file.content.len() + 512);
        // Every value is a string: the canonical form's float rejection
        // cannot fire, and the expect says so.
        canonical_json(&value, &mut out).expect("a skill asset contains no numbers");
        out.into_bytes()
    }

    /// The content address — computable without touching the database, so an
    /// install can check what it wrote against the entry the commit named for
    /// the cost of a hash. That check is what makes "installs unmodified" a
    /// measurement rather than a claim (ADR-0051 force 2).
    #[must_use]
    pub fn address(&self) -> ObjectHash {
        object_hash(AssetKind::Skill, &self.canonical_bytes())
    }

    /// Parses an asset back out of an object's bytes — what an install writes
    /// and what a review renders.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the bytes are not a file's canonical form. For
    /// a stored object that means the object and this code have drifted, and
    /// saying so beats guessing.
    pub fn from_bytes(bytes: &[u8]) -> Result<SkillAsset> {
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|err| Error::Invalid {
                message: format!("skill object is not JSON: {err}"),
            })?;
        let field = |name: &str| -> Result<String> {
            value
                .get(name)
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .ok_or_else(|| Error::Invalid {
                    message: format!("skill object has no string field {name:?}"),
                })
        };
        Ok(SkillAsset {
            scope_id: field("scope")?.parse().map_err(|err| Error::Invalid {
                message: format!("skill object scope is not a scope id: {err}"),
            })?,
            skill: field("skill")?.parse()?,
            sensitivity: field("sensitivity")?.parse()?,
            file: SkillFile {
                path: field("path")?.parse()?,
                content: field("content")?,
            },
        })
    }
}

/// Writes a bundled file's object, returning its address.
///
/// Dedups like every other object write, and here that is load-bearing
/// rather than merely tidy: re-authoring an unchanged file returns the same
/// address, so a bundle whose `SKILL.md` moved and whose scripts did not
/// re-stores one object.
#[tracing::instrument(name = "vedaflow.put_skill", skip_all, fields(tenant.id = %tenant), err(Display))]
pub async fn put_skill(
    conn: &mut PgConnection,
    tenant: TenantId,
    asset: &SkillAsset,
) -> Result<Written<ObjectHash>> {
    put_object(conn, tenant, AssetKind::Skill, &asset.canonical_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn asset(content: &str) -> SkillAsset {
        SkillAsset {
            scope_id: ScopeId::from_uuid(Uuid::from_bytes([2; 16])),
            skill: "code-review".parse().unwrap(),
            sensitivity: Sensitivity::Internal,
            file: SkillFile {
                path: "scripts/check.py".parse().unwrap(),
                content: content.to_owned(),
            },
        }
    }

    #[test]
    fn a_file_object_is_canonical_json_with_sorted_keys() {
        let text = String::from_utf8(asset("print('ok')").canonical_bytes()).unwrap();
        assert_eq!(
            text,
            "{\"content\":\"print('ok')\",\"path\":\"scripts/check.py\",\
             \"scope\":\"02020202-0202-0202-0202-020202020202\",\
             \"sensitivity\":\"internal\",\"skill\":\"code-review\"}"
        );
    }

    /// The property "installs unmodified" is measured against (ADR-0051
    /// force 2): the address moves with everything a reviewer consented to,
    /// so a tree entry pins exactly those bytes at exactly that tier.
    #[test]
    fn the_address_moves_with_every_governed_field() {
        let base = asset("print('ok')");
        assert_eq!(base.address(), asset("print('ok')").address());
        assert_ne!(base.address(), asset("print('no')").address(), "content");

        let mut repathed = base.clone();
        repathed.file.path = "scripts/check2.py".parse().unwrap();
        assert_ne!(base.address(), repathed.address(), "path");

        let mut rebundled = base.clone();
        rebundled.skill = "code-audit".parse().unwrap();
        assert_ne!(base.address(), rebundled.address(), "skill");

        let mut moved = base.clone();
        moved.scope_id = ScopeId::from_uuid(Uuid::from_bytes([9; 16]));
        assert_ne!(base.address(), moved.address(), "scope");

        // The one a per-file tier would have made impossible to notice: a
        // reclassification re-addresses every file of the bundle, so a
        // published skill cannot change tier without a second review.
        let mut reclassified = base.clone();
        reclassified.sensitivity = Sensitivity::Confidential;
        assert_ne!(base.address(), reclassified.address(), "sensitivity");
    }

    #[test]
    fn the_object_names_no_author_and_no_frontmatter() {
        let text = String::from_utf8(asset("print('ok')").canonical_bytes()).unwrap();
        assert!(!text.contains("owner"), "{text}");
        assert!(!text.contains("author"), "{text}");
        // The frontmatter is a deterministic parse of SKILL.md's own bytes,
        // so hashing it would hash the same bytes twice.
        assert!(!text.contains("description"), "{text}");
    }

    #[test]
    fn the_bytes_round_trip_through_the_parser() {
        let original = asset("---\nname: code-review\n---\n");
        let parsed = SkillAsset::from_bytes(&original.canonical_bytes()).expect("parse");
        assert_eq!(parsed, original);
        assert_eq!(parsed.address(), original.address());
    }

    #[test]
    fn a_malformed_object_is_named_rather_than_guessed_at() {
        assert!(SkillAsset::from_bytes(b"not json").is_err());
        assert!(SkillAsset::from_bytes(br#"{"skill":"a"}"#).is_err());
        // A path the vocabulary refuses cannot be resurrected by having been
        // stored once — which for skills is a traversal defence rather than
        // a formatting one.
        assert!(
            SkillAsset::from_bytes(
                br#"{"content":"c","path":"../escape.py","scope":
                     "02020202-0202-0202-0202-020202020202","sensitivity":"internal",
                     "skill":"code-review"}"#
            )
            .is_err()
        );
    }

    /// Asset kind is part of the address (ADR-0030 decision 4), and for
    /// skills it is the sharpest instance in the product: the same bytes as
    /// a context-pack document are governed by a matrix with no security
    /// reviewer in it.
    #[test]
    fn the_asset_kind_is_part_of_the_address() {
        let bytes = asset("print('ok')").canonical_bytes();
        for other in [
            AssetKind::Memory,
            AssetKind::Prompt,
            AssetKind::ContextPack,
            AssetKind::Policy,
        ] {
            assert_ne!(
                object_hash(AssetKind::Skill, &bytes),
                object_hash(other, &bytes)
            );
        }
    }
}
