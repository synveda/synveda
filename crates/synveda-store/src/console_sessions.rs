//! Server-side custody of a browser's credential (CNSL-1, ADR-0056).
//!
//! The console's cookie carries an opaque secret; this module stores what
//! that secret names — an issuer, an access token, and the refresh token a
//! browser may not hold (ADR-0027 decision 6). Every function here is keyed
//! by the **hash** of the secret, never the secret itself, and the caller is
//! responsible for hashing before it arrives (`synveda_identity::console`).
//!
//! Deliberately not tenant-scoped: see migration 0034's header. A session
//! row carries no tenant because the tenant comes from verifying the access
//! token it holds, which is what makes ADR-0056 decision 2's invariant —
//! *the session's authority is the token's authority* — a property of the
//! schema rather than of the code that reads it.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use synveda_types::{Error, Result};

/// A stored console session. Carries no tenant and no subject on purpose
/// (ADR-0056 decision 2): both come from verifying [`Self::access_token`].
#[derive(Debug, Clone)]
pub struct ConsoleSession {
    /// The issuer to refresh against.
    pub issuer: String,
    /// The `/v1` bearer this session names.
    pub access_token: String,
    /// When that bearer expires, when the IdP reported a lifetime. `None`
    /// means "use it until the gateway rejects it" — the ADPT-1 rule.
    pub access_expires_at: Option<DateTime<Utc>>,
    /// The refresh token, when the issuer granted one. Absence is what
    /// makes a session eventually need a fresh login.
    pub refresh_token: Option<String>,
    /// The hard cap, past which no refresh is attempted.
    pub absolute_expires_at: DateTime<Utc>,
    /// When this session was last used, on the coarse cadence [`touch`]
    /// writes it.
    pub last_seen_at: DateTime<Utc>,
}

fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        // 23505 unique_violation: a token hash collision is a bug or a
        // replayed insert, not a storage fault.
        if db.code().as_deref() == Some("23505") {
            return Error::Conflict {
                message: db.to_string(),
            };
        }
        // 23514 check_violation: a caller handed us something outside the
        // column vocabulary (an over-long token, a cap before creation).
        if db.code().as_deref() == Some("23514") {
            return Error::Invalid {
                message: db.to_string(),
            };
        }
    }
    Error::Storage {
        message: err.to_string(),
    }
}

/// Opens a session. `token_hash` is SHA-256 of the secret the browser will
/// hold; the secret never reaches this crate.
#[tracing::instrument(name = "store.console_sessions.create", skip_all, err(Display))]
#[allow(clippy::too_many_arguments)]
pub async fn create(
    executor: impl PgExecutor<'_>,
    token_hash: &[u8; 32],
    issuer: &str,
    access_token: &str,
    access_expires_at: Option<DateTime<Utc>>,
    refresh_token: Option<&str>,
    absolute_expires_at: DateTime<Utc>,
) -> Result<()> {
    sqlx::query!(
        r#"
        insert into console_sessions
            (token_hash, issuer, access_token, access_expires_at,
             refresh_token, absolute_expires_at)
        values ($1, $2, $3, $4, $5, $6)
        "#,
        &token_hash[..],
        issuer,
        access_token,
        access_expires_at,
        refresh_token,
        absolute_expires_at,
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(())
}

/// Resolves a session by the hash of its secret. Returns `None` for an
/// unknown hash **and** for one whose hard cap has passed: an expired
/// session is not a session, and making the caller re-check an invariant the
/// query can express is how one caller eventually forgets.
#[tracing::instrument(name = "store.console_sessions.by_hash", skip_all, err(Display))]
pub async fn by_hash(
    executor: impl PgExecutor<'_>,
    token_hash: &[u8; 32],
) -> Result<Option<ConsoleSession>> {
    let row = sqlx::query!(
        r#"
        select issuer, access_token, access_expires_at,
               refresh_token, absolute_expires_at, last_seen_at
        from console_sessions
        where token_hash = $1 and absolute_expires_at > now()
        "#,
        &token_hash[..],
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    Ok(row.map(|row| ConsoleSession {
        issuer: row.issuer,
        access_token: row.access_token,
        access_expires_at: row.access_expires_at,
        refresh_token: row.refresh_token,
        absolute_expires_at: row.absolute_expires_at,
        last_seen_at: row.last_seen_at,
    }))
}

/// Writes a renewed access token back, after the gateway refreshed it
/// against the issuer. `refresh_token` is `None` for an issuer that does not
/// rotate — the stored one is then left alone rather than nulled, which is
/// the bug this signature exists to make hard to write.
#[tracing::instrument(name = "store.console_sessions.renew", skip_all, err(Display))]
pub async fn renew(
    executor: impl PgExecutor<'_>,
    token_hash: &[u8; 32],
    access_token: &str,
    access_expires_at: Option<DateTime<Utc>>,
    refresh_token: Option<&str>,
) -> Result<()> {
    sqlx::query!(
        r#"
        update console_sessions
        set access_token = $2,
            access_expires_at = $3,
            refresh_token = coalesce($4, refresh_token),
            last_seen_at = now()
        where token_hash = $1
        "#,
        &token_hash[..],
        access_token,
        access_expires_at,
        refresh_token,
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(())
}

/// Advances `last_seen_at`, but only if it is already older than
/// `staleness`. A review screen polls, and a row rewritten on every poll
/// would turn the read path into a write path for no gain.
#[tracing::instrument(name = "store.console_sessions.touch", skip_all, err(Display))]
pub async fn touch(
    executor: impl PgExecutor<'_>,
    token_hash: &[u8; 32],
    staleness: chrono::Duration,
) -> Result<()> {
    let staleness =
        sqlx::postgres::types::PgInterval::try_from(staleness).map_err(|err| Error::Invalid {
            message: format!("staleness is not a representable interval: {err}"),
        })?;
    sqlx::query!(
        r#"
        update console_sessions
        set last_seen_at = now()
        where token_hash = $1 and last_seen_at < now() - $2::interval
        "#,
        &token_hash[..],
        staleness,
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(())
}

/// Destroys a session — sign-out. Returns whether a row was there to
/// destroy, so the caller can tell a real sign-out from a replayed one
/// without asking a second question.
#[tracing::instrument(name = "store.console_sessions.delete", skip_all, err(Display))]
pub async fn delete(executor: impl PgExecutor<'_>, token_hash: &[u8; 32]) -> Result<bool> {
    let result = sqlx::query!(
        "delete from console_sessions where token_hash = $1",
        &token_hash[..],
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(result.rows_affected() > 0)
}

/// Reaps sessions past their hard cap. Returns how many went, for the
/// caller's metric.
#[tracing::instrument(name = "store.console_sessions.purge_expired", skip_all, err(Display))]
pub async fn purge_expired(executor: impl PgExecutor<'_>) -> Result<u64> {
    let result = sqlx::query!("delete from console_sessions where absolute_expires_at <= now()")
        .execute(executor)
        .await
        .map_err(storage_error)?;
    Ok(result.rows_affected())
}
