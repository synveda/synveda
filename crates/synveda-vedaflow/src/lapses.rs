//! A lapse's reviewed terms, as a VedaFlow object (AUTHZ-4, ADR-0037).
//!
//! A lapse proposal names exactly one member: an [`AssetKind::Policy`]
//! object holding the terms in canonical form. That is what the approvals
//! bind, and it is why ADR-0032 decision 6 — approvals bind bytes — holds
//! here *structurally* rather than by a recheck: the object is the only
//! copy of the terms, so there is no row to edit under an approval and
//! nothing for a publish-time address comparison to catch.
//!
//! The encoding is [`crate::channels::MemoryAsset`]'s, for its reasons:
//! canonical JSON with bytewise-sorted keys, human-readable because FLOW-6
//! renders a diff of it and FLOW-8 exports it into a real git repository,
//! where a length-prefixed binary blob would be worthless.

use serde_json::json;
use synveda_types::{AssetKind, Error, LapseTerms, Result, TenantId};

use crate::Written;
use crate::hash::{ObjectHash, object_hash};
use crate::objects::{put_object, read_object};
use crate::policy::canonical_json;

/// A lapse's terms at a content address.
///
/// Not a newtype over [`LapseTerms`] with a blanket `Serialize`: the
/// address is hashed into a review, so the encoding has to be stated here
/// and changed deliberately, never inherited from whatever serde does with
/// a field that gets added later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LapseAsset {
    /// The terms two approvers would be consenting to.
    pub terms: LapseTerms,
}

impl LapseAsset {
    /// Wraps terms for addressing.
    #[must_use]
    pub fn new(terms: LapseTerms) -> Self {
        LapseAsset { terms }
    }

    /// The object's bytes: canonical JSON, keys sorted bytewise.
    ///
    /// `duration_secs` is rendered as a JSON number, which the canonical
    /// rule admits because it is an integer — the one case
    /// [`canonical_json`] accepts, and the reason it rejects floats.
    ///
    /// `max_sensitivity` is always written, including at the working tier
    /// (AUTHZ-5, ADR-0038 decision 6). It is the term that decides what the
    /// approval matrix asks for, so it belongs in the address unconditionally
    /// — omitting it when it happens to be the default would make two grants
    /// with different requirements address identically the day the default
    /// changes.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let value = json!({
            "action": self.terms.action.as_str(),
            "duration_secs": self.terms.duration_secs,
            "grantee_scope": self.terms.grantee_scope_id.as_uuid().to_string(),
            "max_sensitivity": self.terms.max_sensitivity.as_str(),
            "reason": self.terms.reason,
            "target_scope": self.terms.target_scope_id.as_uuid().to_string(),
        });
        let mut out = String::with_capacity(self.terms.reason.len() + 256);
        canonical_json(&value, &mut out)
            .expect("a lapse's only number is an integer duration in seconds");
        out.into_bytes()
    }

    /// The content address — what the proposal's tree names and what every
    /// approval binds.
    #[must_use]
    pub fn address(&self) -> ObjectHash {
        object_hash(AssetKind::Policy, &self.canonical_bytes())
    }

    /// The name these terms take in the proposal's tree.
    ///
    /// The target scope rather than a fresh id, so two proposals opening
    /// the *same* grant over the same scope address identically and the
    /// object store dedups them — the FLOW-1 property, applied to the one
    /// asset whose whole content is five fields.
    #[must_use]
    pub fn entry_name(&self) -> String {
        self.terms.target_scope_id.as_uuid().to_string()
    }

    /// Parses terms back out of an object's bytes.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the bytes are not a lapse's canonical form —
    /// which for a stored object means the object and this code have
    /// drifted, and saying so beats guessing.
    pub fn from_bytes(bytes: &[u8]) -> Result<LapseTerms> {
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|err| Error::Invalid {
                message: format!("lapse object is not JSON: {err}"),
            })?;
        let field = |name: &str| -> Result<String> {
            value
                .get(name)
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
                .ok_or_else(|| Error::Invalid {
                    message: format!("lapse object has no string field {name:?}"),
                })
        };
        let duration_secs = value
            .get("duration_secs")
            .and_then(serde_json::Value::as_u64)
            .and_then(|secs| u32::try_from(secs).ok())
            .ok_or_else(|| Error::Invalid {
                message: "lapse object has no integer duration_secs".to_owned(),
            })?;
        let scope = |name: &str| -> Result<synveda_types::ScopeId> {
            field(name)?.parse().map_err(|err| Error::Invalid {
                message: format!("lapse object field {name:?} is not a scope id: {err}"),
            })
        };
        // An object written before AUTHZ-5 has no tier, and it means the
        // working one: the read path composed nothing above `internal` when
        // it was approved, so that is what its approvers consented to
        // (ADR-0038 decision 6). Absent is a known shape; present-and-wrong
        // is still a drift, and still says so.
        let max_sensitivity = match value.get("max_sensitivity") {
            None => synveda_types::Sensitivity::WORKING,
            Some(_) => field("max_sensitivity")?.parse()?,
        };
        Ok(LapseTerms {
            grantee_scope_id: scope("grantee_scope")?,
            target_scope_id: scope("target_scope")?,
            action: field("action")?.parse()?,
            max_sensitivity,
            duration_secs,
            reason: field("reason")?,
        })
    }
}

/// Writes a lapse's terms, returning the address.
///
/// Dedups like every other object write.
#[tracing::instrument(name = "vedaflow.put_lapse", skip_all, fields(tenant.id = %tenant), err(Display))]
pub async fn put_lapse(
    conn: &mut sqlx::PgConnection,
    tenant: TenantId,
    asset: &LapseAsset,
) -> Result<Written<ObjectHash>> {
    put_object(conn, tenant, AssetKind::Policy, &asset.canonical_bytes()).await
}

/// Reads a lapse's terms back from an address — what the grant surface uses
/// to run the effect of exactly what was approved, rather than of whatever
/// a request body says now.
#[tracing::instrument(name = "vedaflow.read_lapse", skip_all, fields(tenant.id = %tenant), err(Display))]
pub async fn read_lapse(
    conn: &mut sqlx::PgConnection,
    tenant: TenantId,
    address: ObjectHash,
) -> Result<LapseTerms> {
    let object = read_object(conn, tenant, address)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: "lapse object".to_owned(),
        })?;
    LapseAsset::from_bytes(&object.content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use synveda_types::{LapseAction, ScopeId, Sensitivity};
    use uuid::Uuid;

    fn terms() -> LapseTerms {
        LapseTerms {
            grantee_scope_id: ScopeId::from_uuid(Uuid::from_bytes([1; 16])),
            target_scope_id: ScopeId::from_uuid(Uuid::from_bytes([2; 16])),
            action: LapseAction::MemoryRead,
            max_sensitivity: Sensitivity::Internal,
            duration_secs: 3_600,
            reason: "joint incident review".to_owned(),
        }
    }

    #[test]
    fn the_object_is_canonical_json_with_sorted_keys() {
        let bytes = LapseAsset::new(terms()).canonical_bytes();
        let text = String::from_utf8(bytes).expect("utf-8");
        let keys: Vec<&str> = [
            "action",
            "duration_secs",
            "grantee_scope",
            "max_sensitivity",
            "reason",
            "target_scope",
        ]
        .into_iter()
        .collect();
        let mut last = 0;
        for key in keys {
            let at = text
                .find(&format!("\"{key}\""))
                .unwrap_or_else(|| panic!("{key} missing from {text}"));
            assert!(at > last || last == 0, "keys must be sorted: {text}");
            last = at;
        }
        assert!(text.starts_with('{') && text.ends_with('}'), "{text}");
    }

    /// The property the whole review rests on: the address moves when any
    /// term does, so an approval can never carry to different terms.
    #[test]
    fn every_term_is_in_the_address() {
        let base = LapseAsset::new(terms()).address();
        let variants = [
            LapseTerms {
                duration_secs: 3_601,
                ..terms()
            },
            LapseTerms {
                reason: "something else".to_owned(),
                ..terms()
            },
            LapseTerms {
                grantee_scope_id: ScopeId::from_uuid(Uuid::from_bytes([9; 16])),
                ..terms()
            },
            LapseTerms {
                target_scope_id: ScopeId::from_uuid(Uuid::from_bytes([9; 16])),
                ..terms()
            },
            // The tier is a term like any other, and the one that decides
            // what the matrix asks for: an approval given for a working-tier
            // grant can never carry to a restricted one (ADR-0038
            // decision 6).
            LapseTerms {
                max_sensitivity: Sensitivity::Restricted,
                ..terms()
            },
        ];
        for variant in variants {
            assert_ne!(
                LapseAsset::new(variant).address(),
                base,
                "a changed term must change the address"
            );
        }
        // And identical terms address identically, which is what makes the
        // object store dedup them.
        assert_eq!(LapseAsset::new(terms()).address(), base);
    }

    #[test]
    fn the_bytes_round_trip_through_the_parser() {
        let bytes = LapseAsset::new(terms()).canonical_bytes();
        assert_eq!(LapseAsset::from_bytes(&bytes).expect("parse"), terms());
    }

    #[test]
    fn a_malformed_object_is_named_rather_than_guessed_at() {
        assert!(LapseAsset::from_bytes(b"not json").is_err());
        assert!(LapseAsset::from_bytes(br#"{"action":"memory.read"}"#).is_err());
        // A duration that is not an integer is refused rather than
        // truncated: the canonical rule has no place for a float.
        assert!(
            LapseAsset::from_bytes(
                br#"{"action":"memory.read","duration_secs":1.5,"grantee_scope":"x",
                     "reason":"r","target_scope":"y"}"#
            )
            .is_err()
        );
    }

    /// A lapse is addressed as a policy asset, so identical bytes
    /// registered as a memory are a different object — the FLOW-1 rule that
    /// asset kind is part of the address (ADR-0030 decision 4).
    #[test]
    fn the_asset_kind_is_part_of_the_address() {
        let bytes = LapseAsset::new(terms()).canonical_bytes();
        assert_ne!(
            object_hash(AssetKind::Policy, &bytes),
            object_hash(AssetKind::Memory, &bytes)
        );
    }
}
