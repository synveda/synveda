//! A caller's own console sessions and presented credential (ADR-0130).
//!
//! Inventory reads the tenant-scoped index, never custody's sealed tokens,
//! and returns only the public id, configured issuer and coarse times.
//! Revoking deletes custody, which removes the index row, and records the
//! session family in the revocation ledger in the same exclusive tenant
//! transaction. A ledger at capacity refuses the whole revocation.
//!
//! Absent, foreign and already revoked ids share one answer, so a guessed id
//! learns nothing. Administrator inventory is deliberately not offered.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_identity::revocation::{self, IdentifierKind, RevocationMode};
use synveda_policy::{Action, Resource};
use synveda_store::rls;
use synveda_store::session_index::{self, Cursor};
use synveda_store::token_revocations::{self, Kind, Recorded};
use synveda_types::{Error, Result, TenantId};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::app::AppState;
use crate::audit;
use crate::authz;
use crate::request::{body, commit, tenant_id};
use crate::telemetry::{LOGIN_SESSION_OPERATIONS_TOTAL, TOKEN_REVOCATIONS_TOTAL};

/// Default inventory page.
const DEFAULT_PAGE: u32 = 50;

/// Revoke-all batches committed by one request before it reports progress.
const REVOKE_ALL_BATCHES_PER_REQUEST: usize = 4;

/// One console session, as its owner sees it.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConsoleSessionView {
    /// Random public id; the only handle a revoke accepts.
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    /// The configured issuer that authenticated the login.
    pub issuer: String,
    /// When the login completed.
    pub created_at: DateTime<Utc>,
    /// Last use, to the minute.
    pub last_seen_at: DateTime<Utc>,
    /// When the session ends regardless of use.
    pub expires_at: DateTime<Utc>,
}

/// One page of the caller's console sessions, newest first.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConsoleSessionPage {
    /// Live sessions created by verified console logins.
    pub sessions: Vec<ConsoleSessionView>,
    /// Pass as `after` for the next page; absent on the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_after: Option<String>,
}

/// Inventory paging.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ConsoleSessionQuery {
    /// Page size, 1 to 100. Defaults to 50.
    pub limit: Option<u32>,
    /// The previous page's `next_after`.
    pub after: Option<String>,
}

/// Progress of a revoke-all request.
#[derive(Debug, Serialize, ToSchema)]
pub struct RevokeAllProgress {
    /// Sessions this request revoked.
    pub revoked: u32,
    /// False when sessions remain; call again to continue.
    pub complete: bool,
}

/// Which identifier of the presented bearer to revoke.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CredentialIdentifier {
    /// This bearer only, by its token id.
    Token,
    /// Every bearer of this bearer's issuer session.
    Family,
}

/// `POST /v1/me/credential/revoke` body.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RevokeCredentialBody {
    /// `token` or `family`.
    pub identifier: CredentialIdentifier,
}

/// What a credential revocation recorded.
#[derive(Debug, Serialize, ToSchema)]
pub struct CredentialRevocation {
    /// The identifier now in the ledger.
    pub identifier: CredentialIdentifier,
    /// Whether this issuer's profile refuses revoked bearers. False means
    /// the revocation is recorded and reported, but not yet enforced.
    pub enforced: bool,
}

async fn respond<T: IntoResponse>(
    state: &AppState,
    op: &'static str,
    result: Result<T>,
) -> Response {
    let outcome = crate::response::outcome(&result);
    metrics::counter!(LOGIN_SESSION_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome)
        .increment(1);
    crate::response::finish(state, op, result).await
}

fn caller() -> Result<synveda_identity::TenantContext> {
    synveda_identity::current_tenant().ok_or_else(|| Error::Internal {
        message: "login-session route ran outside a tenant scope".to_owned(),
    })
}

fn encode_cursor(cursor: Cursor) -> String {
    format!("{}.{}", cursor.created_at.timestamp_micros(), cursor.id)
}

fn decode_cursor(raw: &str) -> Result<Cursor> {
    let invalid = || Error::Invalid {
        message: "after is not a console-session page cursor".to_owned(),
    };
    let (micros, id) = raw.split_once('.').ok_or_else(invalid)?;
    let micros: i64 = micros.parse().map_err(|_| invalid())?;
    Ok(Cursor {
        created_at: DateTime::from_timestamp_micros(micros).ok_or_else(invalid)?,
        id: id.parse().map_err(|_| invalid())?,
    })
}

/// `GET /v1/me/console-sessions`.
#[utoipa::path(
    get,
    path = "/v1/me/console-sessions",
    operation_id = "list_my_console_sessions",
    tag = "me",
    params(ConsoleSessionQuery),
    responses(
        (status = 200, description = "The caller's live console sessions, newest first. Sessions opened before the inventory existed are not listed and end at their hard cap.", body = ConsoleSessionPage),
        (status = 400, description = "Malformed cursor", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Listing console sessions is not permitted", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "login_sessions.list", skip_all)]
pub(crate) async fn list(
    State(state): State<AppState>,
    Query(query): Query<ConsoleSessionQuery>,
) -> Response {
    let result = list_inner(&state, query).await.map(Json);
    respond(&state, "list", result).await
}

async fn list_inner(state: &AppState, query: ConsoleSessionQuery) -> Result<ConsoleSessionPage> {
    let context = caller()?;
    let tenant_id = tenant_id()?;
    let limit = query.limit.unwrap_or(DEFAULT_PAGE);
    if !(1..=session_index::MAX_PAGE).contains(&limit) {
        return Err(Error::Invalid {
            message: format!("limit must be between 1 and {}", session_index::MAX_PAGE),
        });
    }
    let after = query.after.as_deref().map(decode_cursor).transpose()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let authorized = authz::require(
        state,
        &mut tx,
        Action::LoginSessionRead,
        Resource::Tenant(tenant_id),
        None,
    )
    .await?;
    // One extra row tells whether another page exists without a count.
    let mut rows =
        session_index::page(&mut *tx, tenant_id, context.subject(), after, limit + 1).await?;
    let more = rows.len() > usize::try_from(limit).unwrap_or(usize::MAX);
    rows.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::AuthzDecision,
        Resource::Tenant(tenant_id).to_string(),
        Outcome::Allow,
        json!({
            "op": "login_sessions.list",
            "authz": audit::decision_context(Action::LoginSessionRead, &authorized),
            "sessions_disclosed": rows.len(),
        }),
    )
    .await?;
    commit(tx).await?;
    let next_after = more.then(|| rows.last()).flatten().map(|row| {
        encode_cursor(Cursor {
            created_at: row.created_at,
            id: row.id,
        })
    });
    Ok(ConsoleSessionPage {
        sessions: rows
            .into_iter()
            .map(|row| ConsoleSessionView {
                id: row.id,
                issuer: row.issuer,
                created_at: row.created_at,
                last_seen_at: row.last_seen_at,
                expires_at: row.expires_at,
            })
            .collect(),
        next_after,
    })
}

/// `DELETE /v1/me/console-sessions/{id}`.
#[utoipa::path(
    delete,
    path = "/v1/me/console-sessions/{id}",
    operation_id = "revoke_my_console_session",
    tag = "me",
    params(("id" = String, Path, format = "uuid")),
    responses(
        (status = 204, description = "The session is not live for the caller: revoked now, already revoked, expired or never the caller's. Its stored credential is deleted and its issuer session family is recorded as revoked."),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Revoking console sessions is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 429, description = "The revocation ledger is at capacity; nothing was revoked", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "login_sessions.revoke", skip_all)]
pub(crate) async fn revoke(State(state): State<AppState>, Path(id): Path<Uuid>) -> Response {
    let result = revoke_inner(&state, id)
        .await
        .map(|()| StatusCode::NO_CONTENT);
    respond(&state, "revoke", result).await
}

async fn revoke_inner(state: &AppState, id: Uuid) -> Result<()> {
    let context = caller()?;
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_write_tx(&state.pool, tenant_id).await?;
    let authorized = authz::require(
        state,
        &mut tx,
        Action::LoginSessionRevoke,
        Resource::Tenant(tenant_id),
        None,
    )
    .await?;
    let removed = session_index::remove(&mut *tx, tenant_id, context.subject(), id).await?;
    let family = match removed.and_then(|removed| removed.family_digest) {
        Some(digest) => Some(record(&mut tx, tenant_id, &digest, Kind::Family).await?),
        None => None,
    };
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::LoginSessionRevoked,
        Resource::Tenant(tenant_id).to_string(),
        Outcome::Success,
        json!({
            "op": "login_sessions.revoke",
            "authz": audit::decision_context(Action::LoginSessionRevoke, &authorized),
            "sessions_revoked": u8::from(removed.is_some()),
            "family": family.map(recorded_label),
        }),
    )
    .await?;
    commit(tx).await
}

/// `POST /v1/me/console-sessions/revoke-all`.
#[utoipa::path(
    post,
    path = "/v1/me/console-sessions/revoke-all",
    operation_id = "revoke_all_my_console_sessions",
    tag = "me",
    responses(
        (status = 200, description = "Bounded progress. Each committed batch deleted its sessions' stored credentials and recorded their families; repeat until `complete`. A cookie-authenticated caller revokes its own session too.", body = RevokeAllProgress),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Revoking console sessions is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 429, description = "The revocation ledger reached capacity; batches before it stay revoked and the rest were not", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "login_sessions.revoke_all", skip_all)]
pub(crate) async fn revoke_all(State(state): State<AppState>) -> Response {
    let result = revoke_all_inner(&state).await.map(Json);
    respond(&state, "revoke_all", result).await
}

async fn revoke_all_inner(state: &AppState) -> Result<RevokeAllProgress> {
    let mut revoked = 0_u32;
    for _ in 0..REVOKE_ALL_BATCHES_PER_REQUEST {
        let (count, remaining) = match revoke_batch(state).await {
            Ok(progress) => progress,
            Err(Error::RateLimited { message }) => {
                return Err(Error::RateLimited {
                    message: format!("{message}; {revoked} sessions were revoked before it"),
                });
            }
            Err(error) => return Err(error),
        };
        revoked = revoked.saturating_add(count);
        if !remaining {
            return Ok(RevokeAllProgress {
                revoked,
                complete: true,
            });
        }
    }
    Ok(RevokeAllProgress {
        revoked,
        complete: false,
    })
}

async fn revoke_batch(state: &AppState) -> Result<(u32, bool)> {
    let context = caller()?;
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_write_tx(&state.pool, tenant_id).await?;
    let authorized = authz::require(
        state,
        &mut tx,
        Action::LoginSessionRevoke,
        Resource::Tenant(tenant_id),
        None,
    )
    .await?;
    let removed = session_index::remove_batch(&mut *tx, tenant_id, context.subject()).await?;
    let mut families = 0_u32;
    for digest in removed.iter().filter_map(|removed| removed.family_digest) {
        record(&mut tx, tenant_id, &digest, Kind::Family).await?;
        families += 1;
    }
    let remaining = session_index::any_remaining(&mut *tx, tenant_id, context.subject()).await?;
    let count = u32::try_from(removed.len()).unwrap_or(u32::MAX);
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::LoginSessionRevoked,
        Resource::Tenant(tenant_id).to_string(),
        Outcome::Success,
        json!({
            "op": "login_sessions.revoke_all",
            "authz": audit::decision_context(Action::LoginSessionRevoke, &authorized),
            "sessions_revoked": count,
            "families_recorded": families,
            "complete": !remaining,
        }),
    )
    .await?;
    commit(tx).await?;
    Ok((count, remaining))
}

/// `POST /v1/me/credential/revoke`.
#[utoipa::path(
    post,
    path = "/v1/me/credential/revoke",
    operation_id = "revoke_my_credential",
    tag = "me",
    request_body = RevokeCredentialBody,
    responses(
        (status = 200, description = "The presented bearer's token or issuer-session family is recorded as revoked. Synveda refuses it on later requests only when `enforced`; the issuer itself is not told.", body = CredentialRevocation),
        (status = 400, description = "The bearer lacks the identifier, or its issuer has no revocation profile", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Revoking credentials is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 429, description = "The revocation ledger is at capacity; nothing was recorded", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "login_sessions.revoke_credential", skip_all)]
pub(crate) async fn revoke_credential(
    State(state): State<AppState>,
    payload: std::result::Result<Json<RevokeCredentialBody>, JsonRejection>,
) -> Response {
    let result = async {
        let request = body(payload)?;
        revoke_credential_inner(&state, request.identifier).await
    }
    .await
    .map(Json);
    respond(&state, "revoke_credential", result).await
}

async fn revoke_credential_inner(
    state: &AppState,
    identifier: CredentialIdentifier,
) -> Result<CredentialRevocation> {
    let context = caller()?;
    let tenant_id = tenant_id()?;
    let (digest, kind, mode) = presented_identifier(&context, identifier)?;
    let mut tx = rls::begin_tenant_write_tx(&state.pool, tenant_id).await?;
    let authorized = authz::require(
        state,
        &mut tx,
        Action::LoginSessionRevoke,
        Resource::Tenant(tenant_id),
        None,
    )
    .await?;
    let recorded = record(&mut tx, tenant_id, &digest, kind).await?;
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::CredentialRevoked,
        Resource::Tenant(tenant_id).to_string(),
        Outcome::Success,
        json!({
            "op": "login_sessions.revoke_credential",
            "authz": audit::decision_context(Action::LoginSessionRevoke, &authorized),
            "identifier": kind_label(kind),
            "recorded": recorded_label(recorded),
            "enforced": mode == RevocationMode::Enforce,
        }),
    )
    .await?;
    commit(tx).await?;
    Ok(CredentialRevocation {
        identifier,
        enforced: mode == RevocationMode::Enforce,
    })
}

fn presented_identifier(
    context: &synveda_identity::TenantContext,
    identifier: CredentialIdentifier,
) -> Result<([u8; 32], Kind, RevocationMode)> {
    let unsupported = || Error::Invalid {
        message: "this credential's issuer has no revocation profile".to_owned(),
    };
    let identity = context.claims.oidc_token.as_ref().ok_or_else(unsupported)?;
    let profile = identity.revocation.ok_or_else(unsupported)?;
    let keys =
        revocation::bearer_keys(context.tenant.id, identity, context.claims.credential_class)
            .map_err(|refusal| Error::Invalid {
                message: format!(
                    "the credential is outside its issuer's revocation profile ({})",
                    refusal.as_str()
                ),
            })?;
    match identifier {
        CredentialIdentifier::Token => Ok((
            *keys.token.as_bytes(),
            Kind::Token {
                expires_at: keys.expires_at,
            },
            profile.mode,
        )),
        CredentialIdentifier::Family => {
            let family = keys.family.ok_or_else(|| Error::Invalid {
                message: "a service credential has no interactive session family".to_owned(),
            })?;
            Ok((*family.as_bytes(), Kind::Family, profile.mode))
        }
    }
}

async fn record(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    digest: &[u8; 32],
    kind: Kind,
) -> Result<Recorded> {
    let recorded = token_revocations::record(tx, tenant_id, digest, kind).await?;
    metrics::counter!(
        TOKEN_REVOCATIONS_TOTAL,
        "kind" => kind_label(kind),
        "outcome" => recorded_label(recorded),
    )
    .increment(1);
    if recorded == Recorded::LedgerFull {
        return Err(Error::RateLimited {
            message: "the revocation ledger is at capacity".to_owned(),
        });
    }
    Ok(recorded)
}

const fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Token { .. } => IdentifierKind::Token.as_str(),
        Kind::Family => IdentifierKind::Family.as_str(),
    }
}

const fn recorded_label(recorded: Recorded) -> &'static str {
    match recorded {
        Recorded::New => "recorded",
        Recorded::AlreadyRevoked => "already_revoked",
        Recorded::LedgerFull => "ledger_full",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_round_trip_and_refuse_malformed_input() {
        let cursor = Cursor {
            created_at: DateTime::from_timestamp_micros(1_729_700_000_123_456).unwrap(),
            id: Uuid::from_u128(0x0198_0000_0000_7000_8000_0000_0000_0001),
        };
        assert_eq!(decode_cursor(&encode_cursor(cursor)).unwrap(), cursor);
        for malformed in ["", "1", "x.y", "1.not-a-uuid", "99999999999999999999.0"] {
            assert!(decode_cursor(malformed).is_err(), "{malformed:?}");
        }
    }

    #[test]
    fn session_views_carry_no_credential_material() {
        let view = ConsoleSessionView {
            id: Uuid::from_u128(0x0198_0000_0000_7000_8000_0000_0000_0001),
            issuer: "https://auth.example.test/realms/synveda".to_owned(),
            created_at: Utc::now(),
            last_seen_at: Utc::now(),
            expires_at: Utc::now(),
        };
        let rendered = serde_json::to_value(&view).unwrap();
        let mut keys: Vec<&str> = rendered
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["created_at", "expires_at", "id", "issuer", "last_seen_at"]
        );
    }
}
