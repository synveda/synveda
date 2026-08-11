//! Sealed per-tenant secrets (TEN-4, ADR-0064 decision 9).
//!
//! The custody ADR-0060 decision 7 deferred to this feature. Its reasoning
//! was that "shipping a plaintext outbound credential in tenant data now, to
//! be encrypted in a later feature, is the version of this that is hard to
//! walk back: the rows outlive the decision" — so the rows arrive now, with
//! the key, and never exist in the intermediate state.
//!
//! Every value here is an envelope. This module stores and returns those
//! bytes and never opens one; sealing and opening need a [`KeyRing`] and live
//! at the caller, which is what keeps a store module from being a place a
//! plaintext credential can be found.
//!
//! [`KeyRing`]: crate::keys::KeyRing

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use synveda_types::{Error, Result, TenantId};

fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        if db.code().as_deref() == Some("23503") {
            // 23503 foreign_key_violation: a secret for a tenant that does
            // not exist.
            return Error::NotFound {
                entity: db.to_string(),
            };
        }
        if db.code().as_deref() == Some("23514") {
            return Error::Invalid {
                message: db.to_string(),
            };
        }
        if db.code().as_deref() == Some("42501") {
            return crate::rls::backstop_error(db);
        }
    }
    Error::Storage {
        message: err.to_string(),
    }
}

/// A stored secret: the envelope, and when it was last written.
#[derive(Clone)]
pub struct SealedSecret {
    /// The envelope. Opening it needs this tenant's key at the generation the
    /// header names.
    pub sealed: Vec<u8>,
    /// When this value was last written — what an operator reads to answer
    /// "when was this credential last rotated".
    pub updated_at: DateTime<Utc>,
}

// Ciphertext is not secret, but a `Debug` that dumps a credential-shaped blob
// into a log is still how a credential ends up in a log.
impl std::fmt::Debug for SealedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SealedSecret")
            .field("bytes", &self.sealed.len())
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

/// Writes a secret, replacing any value already under `name`.
///
/// An upsert rather than an insert: rotating a credential is the common case
/// and a caller that had to delete first would have a window with no
/// credential at all.
#[tracing::instrument(
    name = "store.tenant_secrets.put",
    skip_all,
    fields(tenant.id = %tenant_id, secret.name = name),
    err(Display)
)]
pub async fn put(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    name: &str,
    sealed: &[u8],
) -> Result<()> {
    sqlx::query!(
        r#"
        insert into tenant_secrets (tenant_id, name, sealed)
        values ($1, $2, $3)
        on conflict (tenant_id, name) do update
            set sealed = excluded.sealed, updated_at = now()
        "#,
        tenant_id.as_uuid(),
        name,
        sealed,
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(())
}

/// Reads a secret's envelope, or `None` when the tenant has none under that
/// name.
#[tracing::instrument(
    name = "store.tenant_secrets.get",
    skip_all,
    fields(tenant.id = %tenant_id, secret.name = name),
    err(Display)
)]
pub async fn get(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    name: &str,
) -> Result<Option<SealedSecret>> {
    let row = sqlx::query!(
        r#"
        select sealed, updated_at
        from tenant_secrets
        where tenant_id = $1 and name = $2
        "#,
        tenant_id.as_uuid(),
        name,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    Ok(row.map(|row| SealedSecret {
        sealed: row.sealed,
        updated_at: row.updated_at,
    }))
}

/// Destroys a secret. Returns whether one was there, so a caller can tell a
/// real revocation from a replayed one without asking twice.
///
/// DELETE is granted here and withheld from `tenant_keys`, and the contrast
/// is deliberate: a credential that cannot be destroyed cannot be revoked,
/// while a key that can be dropped is data made unreadable one statement from
/// a bug.
#[tracing::instrument(
    name = "store.tenant_secrets.delete",
    skip_all,
    fields(tenant.id = %tenant_id, secret.name = name),
    err(Display)
)]
pub async fn delete(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    name: &str,
) -> Result<bool> {
    let result = sqlx::query!(
        "delete from tenant_secrets where tenant_id = $1 and name = $2",
        tenant_id.as_uuid(),
        name,
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(result.rows_affected() > 0)
}

/// The names a tenant holds secrets under, for an operator listing them.
/// Names only — a listing that returned ciphertext would be a listing
/// somebody eventually logs.
#[tracing::instrument(
    name = "store.tenant_secrets.names",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn names(executor: impl PgExecutor<'_>, tenant_id: TenantId) -> Result<Vec<String>> {
    let rows = sqlx::query_scalar!(
        "select name from tenant_secrets where tenant_id = $1 order by name",
        tenant_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    Ok(rows)
}
