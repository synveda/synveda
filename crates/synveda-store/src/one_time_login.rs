//! Deployment-scoped, pre-tenant one-time login rows (OPS-7, ADR-0126).
//!
//! Selectors are SHA-256 hashes of random state/code values. Callers seal
//! payloads before insertion and open them after retrieval. No tenant has
//! been identified yet, so these tables deliberately have no tenant RLS.

use sqlx::PgPool;
use synveda_types::{Error, Result};

const CAP: i64 = 10_000;
const PENDING_LOCK: i64 = 0x5359_4E56_4544_4101;
const HANDOFF_LOCK: i64 = 0x5359_4E56_4544_4102;

/// An atomically consumed pending row and its database-clock expiry verdict.
pub struct PendingRow {
    /// Deployment-key envelope bound to `state_hash`.
    pub payload_sealed: Vec<u8>,
    /// True when the row's deadline had passed at consumption.
    pub expired: bool,
}

/// An atomically consumed handoff row and its database-clock expiry verdict.
pub struct HandoffRow {
    /// Hash of the CLI's separately minted state.
    pub state_hash: [u8; 32],
    /// Deployment-key envelope bound to `code_hash`.
    pub payload_sealed: Vec<u8>,
    /// True when the row's deadline had passed at consumption.
    pub expired: bool,
}

fn storage(error: sqlx::Error) -> Error {
    Error::Storage {
        message: error.to_string(),
    }
}

fn full() -> Error {
    Error::RateLimited {
        message: "too many logins in flight; retry shortly".to_owned(),
    }
}

/// Parks a pending flow under the database's ten-minute clock and shared cap.
/// A transaction-wide advisory lock makes cap enforcement exact across pods.
pub async fn park_pending(
    pool: &PgPool,
    state_hash: &[u8; 32],
    correlation_hash: Option<&[u8; 32]>,
    payload_sealed: &[u8],
) -> Result<()> {
    let mut tx = pool.begin().await.map_err(storage)?;
    sqlx::query!("select pg_advisory_xact_lock($1)", PENDING_LOCK)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    sqlx::query!("delete from pending_logins where expires_at <= now()")
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    let count = sqlx::query_scalar!("select count(*) from pending_logins")
        .fetch_one(&mut *tx)
        .await
        .map_err(storage)?
        .unwrap_or(CAP);
    if count >= CAP {
        return Err(full());
    }
    sqlx::query!(
        "insert into pending_logins (state_hash, correlation_hash, payload_sealed) values ($1, $2, $3)",
        &state_hash[..],
        correlation_hash.map(|hash| &hash[..]),
        payload_sealed,
    )
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    tx.commit().await.map_err(storage)
}

/// Reads an exact browser-bound pending flow without consuming it. An expired
/// row remains visible for callback routing; only consumption refuses expiry.
pub async fn peek_pending(
    pool: &PgPool,
    state_hash: &[u8; 32],
    correlation_hash: Option<&[u8; 32]>,
) -> Result<Option<Vec<u8>>> {
    let row = sqlx::query_scalar!(
        "select payload_sealed from pending_logins where state_hash = $1 and (correlation_hash is null or correlation_hash = $2)",
        &state_hash[..],
        correlation_hash.map(|hash| &hash[..]),
    )
    .fetch_optional(pool)
    .await
    .map_err(storage)?;
    Ok(row)
}

/// Atomically takes an exact browser-bound pending flow. A mismatched cookie
/// cannot burn the state; expiry is returned from the database's clock.
pub async fn consume_pending(
    pool: &PgPool,
    state_hash: &[u8; 32],
    correlation_hash: Option<&[u8; 32]>,
) -> Result<Option<PendingRow>> {
    let row = sqlx::query!(
        r#"delete from pending_logins
            where state_hash = $1 and (correlation_hash is null or correlation_hash = $2)
            returning payload_sealed, (expires_at <= now()) as "expired!""#,
        &state_hash[..],
        correlation_hash.map(|hash| &hash[..]),
    )
    .fetch_optional(pool)
    .await
    .map_err(storage)?;
    Ok(row.map(|row| PendingRow {
        payload_sealed: row.payload_sealed,
        expired: row.expired,
    }))
}

/// Drops a failed callback only if its browser correlation matches.
pub async fn abandon_pending(
    pool: &PgPool,
    state_hash: &[u8; 32],
    correlation_hash: Option<&[u8; 32]>,
) -> Result<bool> {
    let removed = sqlx::query!(
        "delete from pending_logins where state_hash = $1 and (correlation_hash is null or correlation_hash = $2)",
        &state_hash[..],
        correlation_hash.map(|hash| &hash[..]),
    )
    .execute(pool)
    .await
    .map_err(storage)?;
    Ok(removed.rows_affected() == 1)
}

/// Parks a completed CLI session under the database's one-minute clock and
/// shared cap. The session payload is already deployment-key sealed.
pub async fn park_handoff(
    pool: &PgPool,
    code_hash: &[u8; 32],
    state_hash: &[u8; 32],
    payload_sealed: &[u8],
) -> Result<()> {
    let mut tx = pool.begin().await.map_err(storage)?;
    sqlx::query!("select pg_advisory_xact_lock($1)", HANDOFF_LOCK)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    sqlx::query!("delete from cli_handoffs where expires_at <= now()")
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    let count = sqlx::query_scalar!("select count(*) from cli_handoffs")
        .fetch_one(&mut *tx)
        .await
        .map_err(storage)?
        .unwrap_or(CAP);
    if count >= CAP {
        return Err(full());
    }
    sqlx::query!(
        "insert into cli_handoffs (code_hash, state_hash, payload_sealed) values ($1, $2, $3)",
        &code_hash[..],
        &state_hash[..],
        payload_sealed,
    )
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    tx.commit().await.map_err(storage)
}

/// Deletes a CLI code on its first redemption attempt, before checking the
/// independent state. This prevents a wrong-state probe from being retried.
pub async fn consume_handoff(pool: &PgPool, code_hash: &[u8; 32]) -> Result<Option<HandoffRow>> {
    let row = sqlx::query!(
        r#"delete from cli_handoffs where code_hash = $1
            returning state_hash, payload_sealed, (expires_at <= now()) as "expired!""#,
        &code_hash[..],
    )
    .fetch_optional(pool)
    .await
    .map_err(storage)?;
    row.map(|row| {
        let state_hash = row.state_hash.try_into().map_err(|_| Error::Storage {
            message: "stored CLI state hash has the wrong length".to_owned(),
        })?;
        Ok(HandoffRow {
            state_hash,
            payload_sealed: row.payload_sealed,
            expired: row.expired,
        })
    })
    .transpose()
}
