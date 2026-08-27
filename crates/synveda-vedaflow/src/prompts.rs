//! A prompt template as a VedaFlow object (PRMT-1, ADR-0049).
//!
//! The first asset type whose channel entries are **paths** rather than
//! record ids — the shape ADR-0031's `ChannelMember::name` reserved ("a
//! record id for memories, a path for the authored asset types") and the
//! one ADR-0032's curator glob was written to accept before anything
//! produced it.
//!
//! The encoding is [`crate::channels::MemoryAsset`]'s, for its reasons:
//! canonical JSON with bytewise-sorted keys, human-readable because FLOW-6
//! renders a diff of it and FLOW-8 exports it into a real git repository.
//!
//! # What is in the address, and what is not
//!
//! In: the name, the scope, the tier, the description, the template, and
//! the variable schema. Those are the things a reviewer consents to, so
//! changing any of them must drop the prompt off any published set that
//! admitted the old version (ADR-0031 decision 5).
//!
//! Out: **the author**. A memory's owner is inside its address because a
//! record is one person's material at one person's scope; a prompt is the
//! scope's, and who typed it is on the audit chain and in the draft row.
//! Putting it in the address would mean a handover demoted a published
//! prompt to unreviewed without one character of the text changing.
//!
//! The variables are sorted by name in the canonical form, because a schema
//! is a set: reordering a declaration list is not an edit and must not cost
//! a re-review.

use std::collections::HashMap;

use serde_json::json;
use sqlx::PgConnection;
use synveda_types::{
    AssetKind, Channel, Error, PromptName, PromptTemplate, PromptVariable, Result, ScopeId,
    Sensitivity, TenantId,
};

use crate::Written;
use crate::channels::{ChannelRef, read_members};
use crate::hash::{CommitHash, ObjectHash, object_hash};
use crate::objects::put_object;
use crate::policy::canonical_json;

/// A prompt as VedaFlow addresses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptAsset {
    /// The scope that stands behind it.
    pub scope_id: ScopeId,
    /// Its classification — what the approval matrix prices the review at.
    pub sensitivity: Sensitivity,
    /// The template, its schema, its name and its description.
    pub template: PromptTemplate,
}

impl PromptAsset {
    /// The object's bytes: canonical JSON, keys sorted bytewise, variables
    /// sorted by name.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut variables: Vec<&PromptVariable> = self.template.variables.iter().collect();
        variables.sort_by(|left, right| left.name.cmp(&right.name));
        let value = json!({
            "description": self.template.description,
            "name": self.template.name.as_str(),
            "scope": self.scope_id.as_uuid().to_string(),
            "sensitivity": self.sensitivity.as_str(),
            "template": self.template.template,
            "variables": variables.iter().map(|variable| json!({
                "default": variable.default,
                "description": variable.description,
                "name": variable.name,
            })).collect::<Vec<_>>(),
        });
        let mut out = String::with_capacity(self.template.template.len() + 512);
        // Every value is a string, a null, an array, or an object of those:
        // the canonical form's float rejection cannot fire, and the expect
        // says so.
        canonical_json(&value, &mut out).expect("a prompt asset contains no numbers");
        out.into_bytes()
    }

    /// The content address — computable without touching the database, so a
    /// resolve can check a served version against the entry that admitted it
    /// for the cost of a hash.
    #[must_use]
    pub fn address(&self) -> ObjectHash {
        object_hash(AssetKind::Prompt, &self.canonical_bytes())
    }

    /// The name this asset takes in a channel tree: the prompt's own name.
    #[must_use]
    pub fn entry_name(&self) -> String {
        self.template.name.to_string()
    }

    /// Parses an asset back out of an object's bytes — what a resolve serves
    /// and what a review renders.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the bytes are not a prompt's canonical form.
    /// For a stored object that means the object and this code have drifted,
    /// and saying so beats guessing.
    pub fn from_bytes(bytes: &[u8]) -> Result<PromptAsset> {
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|err| Error::Invalid {
                message: format!("prompt object is not JSON: {err}"),
            })?;
        let field = |name: &str| -> Result<String> {
            value
                .get(name)
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .ok_or_else(|| Error::Invalid {
                    message: format!("prompt object has no string field {name:?}"),
                })
        };
        let optional = |value: &serde_json::Value, name: &str| -> Result<Option<String>> {
            match value.get(name) {
                None | Some(serde_json::Value::Null) => Ok(None),
                Some(serde_json::Value::String(text)) => Ok(Some(text.clone())),
                Some(_) => Err(Error::Invalid {
                    message: format!("prompt object field {name:?} is not a string"),
                }),
            }
        };
        let variables = value
            .get("variables")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| Error::Invalid {
                message: "prompt object has no variables array".to_owned(),
            })?
            .iter()
            .map(|variable| {
                Ok(PromptVariable {
                    name: variable
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| Error::Invalid {
                            message: "prompt object variable has no name".to_owned(),
                        })?
                        .to_owned(),
                    description: optional(variable, "description")?,
                    default: optional(variable, "default")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(PromptAsset {
            scope_id: field("scope")?.parse().map_err(|err| Error::Invalid {
                message: format!("prompt object scope is not a scope id: {err}"),
            })?,
            sensitivity: field("sensitivity")?.parse()?,
            template: PromptTemplate {
                name: field("name")?.parse()?,
                description: field("description")?,
                template: field("template")?,
                variables,
            },
        })
    }
}

/// Writes a prompt's object, returning its address.
///
/// Dedups like every other object write: re-saving unchanged content stores
/// nothing and returns the same address, which is what makes "the draft
/// moved" a comparison rather than a flag.
#[tracing::instrument(name = "vedaflow.put_prompt", skip_all, fields(tenant.id = %tenant), err(Display))]
pub async fn put_prompt(
    conn: &mut PgConnection,
    tenant: TenantId,
    asset: &PromptAsset,
) -> Result<Written<ObjectHash>> {
    put_object(conn, tenant, AssetKind::Prompt, &asset.canonical_bytes()).await
}

/// One scope's prompt channel, keyed the way resolution reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptChannelState {
    /// The scope whose channel this is.
    pub scope_id: ScopeId,
    /// The commit the channel serves — what a resolve cites and what a
    /// consumer may pin.
    pub commit: CommitHash,
    /// Whether a FLOW-7 pin chose that commit rather than the ref.
    pub pinned: bool,
    /// Prompt name → the address that scope admitted it at.
    pub members: HashMap<PromptName, ObjectHash>,
}

/// [`read_members`] for a prompt channel, keyed by name.
///
/// An entry name that is not a valid prompt name cannot occur — only this
/// crate writes prompt channels and the surfaces above it parse names
/// before they get here — so one is an internal error rather than a member
/// to drop quietly from a resolution.
pub async fn read_prompt_members(
    conn: &mut PgConnection,
    tenant: TenantId,
    scopes: &[ScopeId],
    channel: Channel,
) -> Result<Vec<PromptChannelState>> {
    read_members(conn, tenant, scopes, ChannelRef::prompt(channel))
        .await?
        .into_iter()
        .map(|snapshot| {
            let members = snapshot
                .members
                .into_iter()
                .map(|member| {
                    let name =
                        member
                            .name
                            .parse::<PromptName>()
                            .map_err(|err| Error::Internal {
                                message: format!(
                                    "prompt channel entry {:?} is not a prompt name: {err}",
                                    member.name
                                ),
                            })?;
                    Ok((name, member.object))
                })
                .collect::<Result<HashMap<PromptName, ObjectHash>>>()?;
            Ok(PromptChannelState {
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

    fn asset(template: &str) -> PromptAsset {
        PromptAsset {
            scope_id: ScopeId::from_uuid(Uuid::from_bytes([2; 16])),
            sensitivity: Sensitivity::Internal,
            template: PromptTemplate {
                name: "support/triage".parse().unwrap(),
                description: "triage reply".to_owned(),
                template: template.to_owned(),
                variables: vec![PromptVariable::required("subject")],
            },
        }
    }

    #[test]
    fn a_prompt_object_is_canonical_json_with_sorted_keys() {
        let text = String::from_utf8(asset("Re: {{ subject }}").canonical_bytes()).unwrap();
        assert_eq!(
            text,
            "{\"description\":\"triage reply\",\"name\":\"support/triage\",\
             \"scope\":\"02020202-0202-0202-0202-020202020202\",\"sensitivity\":\"internal\",\
             \"template\":\"Re: {{ subject }}\",\
             \"variables\":[{\"default\":null,\"description\":null,\"name\":\"subject\"}]}"
        );
    }

    /// The property publication rests on (ADR-0031 decision 5): an edit
    /// moves the address, so it falls off the published set rather than
    /// riding a published name.
    #[test]
    fn the_address_moves_with_every_governed_field() {
        let base = asset("Re: {{ subject }}");
        assert_eq!(base.address(), asset("Re: {{ subject }}").address());
        assert_ne!(
            base.address(),
            asset("Reply about {{ subject }}").address(),
            "template"
        );

        let mut renamed = base.clone();
        renamed.template.name = "support/triage-2".parse().unwrap();
        assert_ne!(base.address(), renamed.address(), "name");

        let mut moved = base.clone();
        moved.scope_id = ScopeId::from_uuid(Uuid::from_bytes([9; 16]));
        assert_ne!(base.address(), moved.address(), "scope");

        let mut reclassified = base.clone();
        reclassified.sensitivity = Sensitivity::Confidential;
        assert_ne!(base.address(), reclassified.address(), "sensitivity");

        let mut redescribed = base.clone();
        redescribed.template.description = "something else".to_owned();
        assert_ne!(base.address(), redescribed.address(), "description");

        // The schema is content: a variable gaining a default changes what
        // a consumer may omit, which is a change a reviewer consented to.
        let mut defaulted = base.clone();
        defaulted.template.variables[0].default = Some("the outage".to_owned());
        assert_ne!(base.address(), defaulted.address(), "variable default");

        let mut described = base.clone();
        described.template.variables[0].description = Some("what it is about".to_owned());
        assert_ne!(base.address(), described.address(), "variable description");
    }

    /// A schema is a set. Reordering the declarations is not an edit, and
    /// costing a re-review for one would teach authors to fear the file.
    #[test]
    fn the_variable_order_is_not_in_the_address() {
        let mut ordered = asset("{{ alpha }} {{ beta }}");
        ordered.template.variables = vec![
            PromptVariable::required("alpha"),
            PromptVariable::required("beta"),
        ];
        let mut reversed = ordered.clone();
        reversed.template.variables.reverse();
        assert_eq!(ordered.address(), reversed.address());
    }

    /// The author is deliberately outside the address: a handover is not an
    /// edit, and demoting a published prompt for one would be a surprise
    /// nobody could act on.
    #[test]
    fn the_object_names_no_author() {
        let text = String::from_utf8(asset("Re: {{ subject }}").canonical_bytes()).unwrap();
        assert!(!text.contains("owner"), "{text}");
        assert!(!text.contains("author"), "{text}");
    }

    #[test]
    fn the_bytes_round_trip_through_the_parser() {
        let mut original = asset("Re: {{ subject }} — {{ tone }}");
        original.template.variables.push(PromptVariable {
            name: "tone".to_owned(),
            description: Some("how to sound".to_owned()),
            default: Some("neutral".to_owned()),
        });
        let parsed = PromptAsset::from_bytes(&original.canonical_bytes()).expect("parse");
        assert_eq!(parsed, original);
        assert_eq!(parsed.address(), original.address());
        parsed
            .template
            .validate()
            .expect("a stored prompt is valid");
    }

    #[test]
    fn a_malformed_object_is_named_rather_than_guessed_at() {
        assert!(PromptAsset::from_bytes(b"not json").is_err());
        assert!(PromptAsset::from_bytes(br#"{"name":"a"}"#).is_err());
        // A name the vocabulary refuses cannot be resurrected by having
        // been stored once.
        assert!(
            PromptAsset::from_bytes(
                br#"{"description":"d","name":"Not A Name","scope":
                     "02020202-0202-0202-0202-020202020202","sensitivity":"internal",
                     "template":"t","variables":[]}"#
            )
            .is_err()
        );
    }

    /// Asset kind is part of the address (ADR-0030 decision 4): the same
    /// bytes registered as a memory are a different object, because a
    /// prompt and a memory are governed by different rules.
    #[test]
    fn the_asset_kind_is_part_of_the_address() {
        let bytes = asset("Re: {{ subject }}").canonical_bytes();
        assert_ne!(
            object_hash(AssetKind::Prompt, &bytes),
            object_hash(AssetKind::Knowledge, &bytes)
        );
    }
}
