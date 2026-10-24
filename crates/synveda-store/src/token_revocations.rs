//! Bounded ledger of revoked token and session-family digests (ADR-0130).
//!
//! Rows hold only domain-separated digests, use database time and expire at
//! most twelve hours after revocation. A tenant's live rows are capped: a
//! revocation the ledger cannot hold is refused, never acknowledged.

use sqlx::PgExecutor;
use synveda_types::{Error, Result, TenantId};

/// Live rows one tenant may hold.
pub const MAX_LIVE_ROWS_PER_TENANT: i64 = 65_536;

/// Maximum expired rows removed by one purge statement.
pub const PURGE_BATCH_SIZE: u64 = 256;

/// The two identifier kinds the ledger stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// One bearer; kept until its own expiry plus verifier leeway.
    Token {
        /// The bearer's `exp`, in Unix seconds.
        expires_at: u64,
    },
    /// One interactive session family; kept twelve hours.
    Family,
}

impl Kind {
    const fn label(self) -> &'static str {
        match self {
            Self::Token { .. } => "token",
            Self::Family => "family",
        }
    }

    fn token_expiry(self) -> Option<i64> {
        match self {
            Self::Token { expires_at } => Some(i64::try_from(expires_at).unwrap_or(i64::MAX)),
            Self::Family => None,
        }
    }
}

/// What a record request did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    /// A new row was committed with this transaction.
    New,
    /// A live row already names this digest.
    AlreadyRevoked,
    /// The tenant's live-row cap is reached; nothing was written.
    LedgerFull,
}

/// A tenant's ledger occupancy, for the capacity and lag signals.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Occupancy {
    /// Live rows, counted up to the cap plus one.
    pub live_rows: i64,
    /// Age of the oldest expired row still retained.
    pub oldest_expired_age_seconds: Option<f64>,
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

/// Records one revocation. Callers hold the tenant's exclusive admission
/// transaction, which serialises the capacity check with the insert.
#[tracing::instrument(name = "store.token_revocations.record", skip_all, err(Display))]
pub async fn record(
    connection: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    digest: &[u8; 32],
    kind: Kind,
) -> Result<Recorded> {
    let state = sqlx::query!(
        r#"
        select exists(
                 select 1 from token_revocations
                  where tenant_id = $1 and digest = $2 and expires_at > now()
               ) as "live!",
               (select count(*) from (
                  select 1 from token_revocations
                   where tenant_id = $1 and expires_at > now()
                   limit $3
               ) live) as "live_rows!"
        "#,
        tenant_id.as_uuid(),
        &digest[..],
        MAX_LIVE_ROWS_PER_TENANT,
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(storage_error)?;
    if state.live {
        return Ok(Recorded::AlreadyRevoked);
    }
    if state.live_rows >= MAX_LIVE_ROWS_PER_TENANT {
        return Ok(Recorded::LedgerFull);
    }
    sqlx::query!(
        r#"
        with stale as (
            delete from token_revocations
             where tenant_id = $1 and digest = $2 and expires_at <= now()
        )
        insert into token_revocations (tenant_id, digest, kind, revoked_at, expires_at)
        values ($1, $2, $3, now(),
                case when $4::bigint is null then now() + interval '12 hours'
                     else least(now() + interval '12 hours',
                                greatest(to_timestamp($4::bigint), now()) + interval '60 seconds')
                end)
        "#,
        tenant_id.as_uuid(),
        &digest[..],
        kind.label(),
        kind.token_expiry(),
    )
    .execute(&mut *connection)
    .await
    .map_err(storage_error)?;
    Ok(Recorded::New)
}

/// Whether any of `digests` names a live revocation. Expired rows awaiting
/// purge never count.
#[tracing::instrument(name = "store.token_revocations.any_live", skip_all, err(Display))]
pub async fn any_live(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    digests: &[[u8; 32]],
) -> Result<bool> {
    if digests.is_empty() || digests.len() > 2 {
        return Err(Error::Internal {
            message: "a bearer has one token key and at most one family key".to_owned(),
        });
    }
    let digests: Vec<&[u8]> = digests.iter().map(|digest| &digest[..]).collect();
    sqlx::query_scalar!(
        r#"
        select exists(
            select 1 from token_revocations
             where tenant_id = $1 and digest = any($2::bytea[]) and expires_at > now()
        ) as "revoked!"
        "#,
        tenant_id.as_uuid(),
        &digests as &[&[u8]],
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)
}

/// Removes at most [`PURGE_BATCH_SIZE`] of one tenant's expired rows. The
/// singleton maintenance worker is the only purger, and an expired row never
/// counts, so no row lock is needed.
#[tracing::instrument(name = "store.token_revocations.purge_expired", skip_all, err(Display))]
pub async fn purge_expired(executor: impl PgExecutor<'_>, tenant_id: TenantId) -> Result<u64> {
    let result = sqlx::query!(
        r#"
        with expired as (
            select digest from token_revocations
             where tenant_id = $1 and expires_at <= now()
             order by expires_at, digest
             limit 256
        )
        delete from token_revocations revocation
         using expired
         where revocation.tenant_id = $1 and revocation.digest = expired.digest
        "#,
        tenant_id.as_uuid(),
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(result.rows_affected())
}

/// Live-row count (bounded by the cap) and oldest retained expiry age.
#[tracing::instrument(name = "store.token_revocations.occupancy", skip_all, err(Display))]
pub async fn occupancy(executor: impl PgExecutor<'_>, tenant_id: TenantId) -> Result<Occupancy> {
    let row = sqlx::query!(
        r#"
        select (select count(*) from (
                  select 1 from token_revocations
                   where tenant_id = $1 and expires_at > now()
                   limit $2
               ) live) as "live_rows!",
               (select extract(epoch from now() - expires_at)::double precision
                  from token_revocations
                 where tenant_id = $1 and expires_at <= now()
                 order by expires_at
                 limit 1) as oldest_expired_age_seconds
        "#,
        tenant_id.as_uuid(),
        MAX_LIVE_ROWS_PER_TENANT + 1,
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    Ok(Occupancy {
        live_rows: row.live_rows,
        oldest_expired_age_seconds: row.oldest_expired_age_seconds,
    })
}
