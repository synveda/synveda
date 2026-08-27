//! Idempotency records (CPR-4, ADR-0071 decision 6): what makes retrying a
//! creation safe.
//!
//! Creation is the one verb HTTP gives no retry story for. A client whose
//! request times out cannot tell "it never arrived" from "it arrived, and the
//! answer went into a dead socket", and retrying the second makes two
//! workspaces. The key is the client's claim that *this is that request
//! again*; this table is what makes the claim answerable.
//!
//! ## The digest is the half that is easy to leave out
//!
//! Storing only (key → resource) would make a key reused for a **different**
//! request answer with the resource from the first one — the client asked to
//! create `payments` and was handed `ledger`, with a 200. So the record
//! carries a digest of the canonical request, and a key whose digest does not
//! match is a conflict rather than a replay.
//!
//! ## Keyed by subject
//!
//! An idempotency key is a token a client mints for itself, with no
//! coordination; two clients minting `1` must not collide. The primary key
//! therefore carries the subject as well as the tenant, which also means one
//! caller cannot probe another's keys for existence.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use synveda_types::{Error, Result, TenantId};
use uuid::Uuid;

/// The digest width: BLAKE3-256, the hash the audit chain and VedaFlow's
/// content addressing already use (ADR-0019, ADR-0030).
pub const DIGEST_BYTES: usize = 32;

/// Longest idempotency key, in characters. Long enough for a UUID, a ULID or
/// a short opaque token; short enough that the column is not a payload.
pub const MAX_KEY_CHARS: usize = 255;

/// One remembered creation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdempotencyRecord {
    /// Which operation the key was claimed for.
    pub operation: String,
    /// The key the client minted.
    pub key: String,
    /// BLAKE3-256 of the canonical request that first used it.
    pub request_digest: Vec<u8>,
    /// The resource that request produced.
    pub resource_id: Uuid,
    /// When it was first used.
    pub created_at: DateTime<Utc>,
}

/// Checks an idempotency key's shape: non-blank, bounded, and printable ASCII.
///
/// Printable ASCII because the key arrives in an HTTP header and lands in a
/// database column, a log line and an error message — a control character in
/// any of those is somebody else's bug wearing this feature's clothes.
///
/// # Errors
///
/// [`Error::Invalid`] when the key is blank, too long, or holds anything
/// outside printable ASCII.
pub fn validate_key(key: &str) -> Result<()> {
    if key.trim().is_empty() {
        return Err(Error::Invalid {
            message: "an idempotency key cannot be blank".to_owned(),
        });
    }
    let len = key.chars().count();
    if len > MAX_KEY_CHARS {
        return Err(Error::Invalid {
            message: format!("an idempotency key is at most {MAX_KEY_CHARS} characters, got {len}"),
        });
    }
    if !key.chars().all(|c| c.is_ascii_graphic()) {
        return Err(Error::Invalid {
            message: "an idempotency key holds printable ASCII only".to_owned(),
        });
    }
    Ok(())
}

/// Looks up what a key already produced, if anything.
#[tracing::instrument(
    name = "store.idempotency.find",
    skip_all,
    fields(tenant.id = %tenant_id, idempotency.operation = operation),
    err(Display)
)]
pub async fn find(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    subject: &str,
    operation: &str,
    key: &str,
) -> Result<Option<IdempotencyRecord>> {
    sqlx::query_as!(
        IdempotencyRecord,
        r#"
        select operation, idempotency_key as "key", request_digest, resource_id, created_at
        from idempotency_records
        where tenant_id = $1 and subject = $2 and operation = $3 and idempotency_key = $4
        "#,
        tenant_id.as_uuid(),
        subject,
        operation,
        key,
    )
    .fetch_optional(executor)
    .await
    .map_err(crate::workspaces::storage_error)
}

/// Remembers what this key produced, in the same transaction as the creation
/// itself.
///
/// Same transaction is the whole point: a record written afterwards could be
/// lost between the two commits, and the client's retry would then create a
/// second resource — which is the failure this table exists to prevent,
/// arriving through the door built to prevent it.
///
/// A second writer for the same key blocks on the primary key until the first
/// commits and then sees [`Error::Conflict`]; the caller re-reads and replays
/// (see the gateway's idempotency seam).
#[tracing::instrument(
    name = "store.idempotency.remember",
    skip_all,
    fields(tenant.id = %tenant_id, idempotency.operation = operation),
    err(Display)
)]
pub async fn remember(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    subject: &str,
    operation: &str,
    key: &str,
    request_digest: &[u8],
    resource_id: Uuid,
) -> Result<()> {
    if request_digest.len() != DIGEST_BYTES {
        return Err(Error::Internal {
            message: format!(
                "an idempotency digest is {DIGEST_BYTES} bytes, got {}",
                request_digest.len()
            ),
        });
    }
    sqlx::query!(
        r#"
        insert into idempotency_records
            (tenant_id, subject, operation, idempotency_key, request_digest, resource_id)
        values ($1, $2, $3, $4, $5, $6)
        "#,
        tenant_id.as_uuid(),
        subject,
        operation,
        key,
        request_digest,
        resource_id,
    )
    .execute(executor)
    .await
    .map_err(crate::workspaces::storage_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_bounded_printable_and_non_blank() {
        validate_key("018f2c1a-0000-7000-8000-000000000000").unwrap();
        validate_key(&"k".repeat(MAX_KEY_CHARS)).unwrap();
        for bad in ["", "   ", "with space", "tab\there", "control\u{7f}"] {
            assert!(validate_key(bad).is_err(), "{bad:?}");
        }
        assert!(validate_key(&"k".repeat(MAX_KEY_CHARS + 1)).is_err());
    }
}
