//! Tenant-scoped, credential-free index of console sessions (ADR-0130).
//!
//! Custody (`console_sessions`) stays pre-tenant and keyed by the secret's
//! hash. This index names the same row by a random public id, under forced
//! RLS, with the verified subject and the session-family digest computed at
//! login. It never answers authentication: deleting custody cascades here, so
//! an index row without live custody cannot exist.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use synveda_types::{Error, Result, TenantId};
use uuid::Uuid;

/// Largest inventory page.
pub const MAX_PAGE: u32 = 100;

/// Largest revoke-all batch committed in one transaction.
pub const REVOKE_BATCH: u32 = 64;

/// A new index row, written in the same transaction as its custody row.
#[derive(Debug, Clone)]
pub struct NewEntry<'a> {
    /// The tenant the verified login resolved to.
    pub tenant_id: TenantId,
    /// Random public id.
    pub id: Uuid,
    /// Verified `sub`.
    pub subject: &'a str,
    /// Exact configured issuer.
    pub issuer: &'a str,
    /// SHA-256 of the console cookie secret, as custody stores it.
    pub custody_hash: &'a [u8; 32],
    /// Session-family digest, when the issuer has a revocation profile.
    pub family_digest: Option<&'a [u8; 32]>,
    /// Custody's hard cap.
    pub expires_at: DateTime<Utc>,
}

/// One inventory row as a caller may see it: no hash, digest or token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Random public id.
    pub id: Uuid,
    /// Exact configured issuer.
    pub issuer: String,
    /// When the login completed.
    pub created_at: DateTime<Utc>,
    /// Last use, truncated to the minute.
    pub last_seen_at: DateTime<Utc>,
    /// Hard expiry.
    pub expires_at: DateTime<Utc>,
}

/// Keyset position after the last row of a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    /// `created_at` of the last row returned.
    pub created_at: DateTime<Utc>,
    /// `id` of the last row returned.
    pub id: Uuid,
}

/// What one revocation removed. `family_digest` is absent for an issuer
/// without a revocation profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Removed {
    /// The removed session's family digest, when it had one.
    pub family_digest: Option<[u8; 32]>,
}

fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        match db.code().as_deref() {
            Some("23505" | "23503") => {
                return Error::Conflict {
                    message: db.to_string(),
                };
            }
            Some("23514") => {
                return Error::Invalid {
                    message: db.to_string(),
                };
            }
            Some("42501") => return crate::rls::backstop_error(db),
            _ => {}
        }
    }
    Error::Storage {
        message: err.to_string(),
    }
}

fn digest(bytes: Vec<u8>) -> Result<[u8; 32]> {
    bytes.try_into().map_err(|_| Error::Internal {
        message: "stored session-family digest is not 32 bytes".to_owned(),
    })
}

/// Writes the index row for a custody row created in the same transaction.
#[tracing::instrument(name = "store.session_index.insert", skip_all, err(Display))]
pub async fn insert(executor: impl PgExecutor<'_>, entry: &NewEntry<'_>) -> Result<()> {
    sqlx::query!(
        r#"
        insert into console_session_index
            (tenant_id, id, subject, issuer, custody_hash, family_digest, expires_at)
        values ($1, $2, $3, $4, $5, $6, $7)
        "#,
        entry.tenant_id.as_uuid(),
        entry.id,
        entry.subject,
        entry.issuer,
        &entry.custody_hash[..],
        entry.family_digest.map(|digest| &digest[..]),
        entry.expires_at,
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(())
}

/// One page of the subject's live sessions, newest first. Rows whose custody
/// has passed its hard cap are omitted even before the worker purges them.
/// Callers ask for one row beyond the page they serve to learn whether
/// another page exists, so the bound is [`MAX_PAGE`] plus one.
#[tracing::instrument(name = "store.session_index.page", skip_all, err(Display))]
pub async fn page(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    subject: &str,
    after: Option<Cursor>,
    limit: u32,
) -> Result<Vec<Entry>> {
    let limit = i64::from(limit.clamp(1, MAX_PAGE + 1));
    let rows = sqlx::query!(
        r#"
        select entry.id, entry.issuer, entry.created_at,
               date_trunc('minute', custody.last_seen_at) as "last_seen_at!",
               custody.absolute_expires_at as expires_at
          from console_session_index entry
          join console_sessions custody on custody.token_hash = entry.custody_hash
         where entry.tenant_id = $1
           and entry.subject = $2
           and custody.absolute_expires_at > now()
           and ($3::timestamptz is null
                or (entry.created_at, entry.id) < ($3::timestamptz, $4::uuid))
         order by entry.created_at desc, entry.id desc
         limit $5
        "#,
        tenant_id.as_uuid(),
        subject,
        after.map(|cursor| cursor.created_at),
        after.map(|cursor| cursor.id),
        limit,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    Ok(rows
        .into_iter()
        .map(|row| Entry {
            id: row.id,
            issuer: row.issuer,
            created_at: row.created_at,
            last_seen_at: row.last_seen_at,
            expires_at: row.expires_at,
        })
        .collect())
}

/// Deletes one of the subject's sessions by public id: custody first, which
/// cascades to the index row. Callers hold the tenant's exclusive admission
/// transaction, which serialises concurrent revocations without row locks the
/// runtime role is not granted. `None` for an unknown id, another subject's id
/// or one already revoked — the caller answers all three alike.
#[tracing::instrument(name = "store.session_index.remove", skip_all, err(Display))]
pub async fn remove(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    subject: &str,
    id: Uuid,
) -> Result<Option<Removed>> {
    let row = sqlx::query!(
        r#"
        with target as (
            select custody_hash, family_digest
              from console_session_index
             where tenant_id = $1 and subject = $2 and id = $3
        )
        delete from console_sessions custody
         using target
         where custody.token_hash = target.custody_hash
        returning target.family_digest
        "#,
        tenant_id.as_uuid(),
        subject,
        id,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(|row| {
        Ok(Removed {
            family_digest: row.family_digest.map(digest).transpose()?,
        })
    })
    .transpose()
}

/// Deletes up to [`REVOKE_BATCH`] of the subject's sessions, oldest first.
#[tracing::instrument(name = "store.session_index.remove_batch", skip_all, err(Display))]
pub async fn remove_batch(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    subject: &str,
) -> Result<Vec<Removed>> {
    let rows = sqlx::query!(
        r#"
        with target as (
            select custody_hash, family_digest
              from console_session_index
             where tenant_id = $1 and subject = $2
             order by created_at, id
             limit $3
        )
        delete from console_sessions custody
         using target
         where custody.token_hash = target.custody_hash
        returning target.family_digest
        "#,
        tenant_id.as_uuid(),
        subject,
        i64::from(REVOKE_BATCH),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter()
        .map(|row| {
            Ok(Removed {
                family_digest: row.family_digest.map(digest).transpose()?,
            })
        })
        .collect()
}

/// Whether the subject still has any indexed session, expired or not.
#[tracing::instrument(name = "store.session_index.any_remaining", skip_all, err(Display))]
pub async fn any_remaining(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    subject: &str,
) -> Result<bool> {
    sqlx::query_scalar!(
        r#"
        select exists(
            select 1 from console_session_index
             where tenant_id = $1 and subject = $2
        ) as "remaining!"
        "#,
        tenant_id.as_uuid(),
        subject,
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)
}
