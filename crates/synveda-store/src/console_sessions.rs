//! Server-side custody of a browser's credential (CNSL-1, ADR-0056).
//!
//! The console's cookie carries an opaque secret; this module stores what
//! that secret names — an issuer, an access token, and the refresh token a
//! browser may not hold (ADR-0027 decision 6). Every function here is keyed
//! by the **hash** of the secret, never the secret itself, and the caller is
//! responsible for hashing before it arrives (`synveda_identity::console`).
//!
//! Since TEN-4 the two token columns are **sealed** (ADR-0064 decision 5),
//! and this module handles only the envelopes: it neither seals nor opens.
//! The key is the *deployment's*, not a tenant's, and the reason is the
//! reason this table has no `tenant_id` — a session is read before the
//! tenant exists, so there is no tenant to select a key by. Sealing lives in
//! the gateway, which is the one crate that may depend on both the key ring
//! and this module; `synveda-identity` is this crate's sibling and cannot
//! reach a `KeyRing` at all.
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
/// (ADR-0056 decision 2): both come from verifying the sealed access token.
#[derive(Clone)]
pub struct ConsoleSession {
    /// The issuer to refresh against.
    pub issuer: String,
    /// The `/v1` bearer this session names, sealed under the deployment key.
    pub access_token_sealed: Vec<u8>,
    /// When that bearer expires, when the IdP reported a lifetime. `None`
    /// means "use it until the gateway rejects it" — the ADPT-1 rule.
    pub access_expires_at: Option<DateTime<Utc>>,
    /// The refresh token, when the issuer granted one, sealed. Absence is
    /// what makes a session eventually need a fresh login.
    pub refresh_token_sealed: Option<Vec<u8>>,
    /// The hard cap, past which no refresh is attempted.
    pub absolute_expires_at: DateTime<Utc>,
    /// When this session was last used, on the coarse cadence [`touch`]
    /// writes it.
    pub last_seen_at: DateTime<Utc>,
}

// Hand-written rather than derived: the derive would render two
// credential-shaped byte vectors, and a ciphertext in a log is still a step
// on the path to a plaintext in a log. Sizes and times only.
impl std::fmt::Debug for ConsoleSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConsoleSession")
            .field("issuer", &self.issuer)
            .field("access_token_bytes", &self.access_token_sealed.len())
            .field("access_expires_at", &self.access_expires_at)
            .field(
                "refresh_token_bytes",
                &self.refresh_token_sealed.as_ref().map(Vec::len),
            )
            .field("absolute_expires_at", &self.absolute_expires_at)
            .field("last_seen_at", &self.last_seen_at)
            .finish()
    }
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
/// hold; the secret never reaches this crate, and neither do the tokens —
/// the caller seals them first.
#[tracing::instrument(name = "store.console_sessions.create", skip_all, err(Display))]
#[allow(clippy::too_many_arguments)]
pub async fn create(
    executor: impl PgExecutor<'_>,
    token_hash: &[u8; 32],
    issuer: &str,
    access_token_sealed: &[u8],
    access_expires_at: Option<DateTime<Utc>>,
    refresh_token_sealed: Option<&[u8]>,
    absolute_expires_at: DateTime<Utc>,
) -> Result<()> {
    sqlx::query!(
        r#"
        insert into console_sessions
            (token_hash, issuer, access_token_sealed, access_expires_at,
             refresh_token_sealed, absolute_expires_at)
        values ($1, $2, $3, $4, $5, $6)
        "#,
        &token_hash[..],
        issuer,
        access_token_sealed,
        access_expires_at,
        refresh_token_sealed,
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
        select issuer, access_token_sealed, access_expires_at,
               refresh_token_sealed, absolute_expires_at, last_seen_at
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
        access_token_sealed: row.access_token_sealed,
        access_expires_at: row.access_expires_at,
        refresh_token_sealed: row.refresh_token_sealed,
        absolute_expires_at: row.absolute_expires_at,
        last_seen_at: row.last_seen_at,
    }))
}

/// Writes a renewed access token back, after the gateway refreshed it
/// against the issuer and re-sealed it. `refresh_token_sealed` is `None` for
/// an issuer that does not rotate — the stored one is then left alone rather
/// than nulled, which is the bug this signature exists to make hard to write.
#[tracing::instrument(name = "store.console_sessions.renew", skip_all, err(Display))]
pub async fn renew(
    executor: impl PgExecutor<'_>,
    token_hash: &[u8; 32],
    access_token_sealed: &[u8],
    access_expires_at: Option<DateTime<Utc>>,
    refresh_token_sealed: Option<&[u8]>,
) -> Result<()> {
    sqlx::query!(
        r#"
        update console_sessions
        set access_token_sealed = $2,
            access_expires_at = $3,
            refresh_token_sealed = coalesce($4, refresh_token_sealed),
            last_seen_at = now()
        where token_hash = $1
        "#,
        &token_hash[..],
        access_token_sealed,
        access_expires_at,
        refresh_token_sealed,
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
