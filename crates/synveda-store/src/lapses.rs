//! Standing lapse grants (AUTHZ-4, ADR-0037).
//!
//! A lapse's *terms* are a reviewed VedaFlow object named by a proposal
//! commit; this module keeps the *grant* — the row every authorization
//! context reads, in typed columns, because parsing an object per decision
//! is not a read path (ADR-0037 decision 16).
//!
//! **[`active_for_scopes`] is where expiry happens.** Nothing runs to end a
//! lapse: this query's predicate does, so a sweep that is down, wedged, or
//! never deployed cannot leave a cross-team read standing (decision 4).
//! [`due_for_expiry_event`] and [`record_expiry`] are the audit chain's
//! bookkeeping and nothing consults them to decide access.
//!
//! Tenant-scoped (forced RLS, ADR-0009): reach this table inside
//! [`crate::rls::begin_tenant_tx`].

use std::str::FromStr;

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use synveda_types::{
    Error, IdentityId, Lapse, LapseAction, LapseId, ProposalId, Result, ScopeId, Sensitivity,
    TenantId,
};

/// Maps a sqlx error at the storage boundary into the shared taxonomy.
fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        // 23503 foreign_key_violation: no such tenant, or no such proposal.
        if db.code().as_deref() == Some("23503") {
            return Error::NotFound {
                entity: "tenant or proposal".to_owned(),
            };
        }
        // 23505 unique_violation: this proposal's effect already ran. A
        // conflict rather than an error — the world moved under a
        // well-formed request (the ADR-0034 rule for publish-time races).
        if db.code().as_deref() == Some("23505") {
            return Error::Conflict {
                message: "this proposal's effect has already run; a lapse proposal \
                          grants at most once"
                    .to_owned(),
            };
        }
        // 23514 check_violation: an unknown action, a blank reason, a
        // window that ends before it starts.
        if db.code().as_deref() == Some("23514") {
            return Error::Invalid {
                message: db.to_string(),
            };
        }
        // 42501 insufficient_privilege: the RLS backstop (ADR-0009).
        if db.code().as_deref() == Some("42501") {
            return crate::rls::backstop_error(db);
        }
    }
    Error::Storage {
        message: err.to_string(),
    }
}

/// The stored shape, mapped into [`Lapse`] on the way out.
struct LapseRow {
    tenant_id: uuid::Uuid,
    id: uuid::Uuid,
    proposal_id: uuid::Uuid,
    grantee_scope_id: uuid::Uuid,
    target_scope_id: uuid::Uuid,
    action: String,
    max_sensitivity: String,
    reason: String,
    granted_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    granted_by: uuid::Uuid,
    revoked_at: Option<DateTime<Utc>>,
    revoked_by: Option<uuid::Uuid>,
    revoke_reason: Option<String>,
    expiry_recorded_at: Option<DateTime<Utc>>,
}

impl TryFrom<LapseRow> for Lapse {
    type Error = Error;

    fn try_from(row: LapseRow) -> Result<Self> {
        // The column's CHECK mirrors the vocabulary, so a value outside it
        // means code and schema drifted. Say so rather than shrug — the
        // role_bindings discipline (ADR-0015).
        let action = LapseAction::from_str(&row.action)?;
        let max_sensitivity = Sensitivity::from_str(&row.max_sensitivity)?;
        Ok(Lapse {
            id: LapseId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            proposal_id: ProposalId::from_uuid(row.proposal_id),
            grantee_scope_id: ScopeId::from_uuid(row.grantee_scope_id),
            target_scope_id: ScopeId::from_uuid(row.target_scope_id),
            action,
            max_sensitivity,
            reason: row.reason,
            granted_at: row.granted_at,
            expires_at: row.expires_at,
            granted_by: IdentityId::from_uuid(row.granted_by),
            revoked_at: row.revoked_at,
            revoked_by: row.revoked_by.map(IdentityId::from_uuid),
            revoke_reason: row.revoke_reason,
            expiry_recorded_at: row.expiry_recorded_at,
        })
    }
}

fn collect(rows: Vec<LapseRow>) -> Result<Vec<Lapse>> {
    rows.into_iter().map(Lapse::try_from).collect()
}

/// The grants standing over a caller: rows whose **grantee** scope is one of
/// `scope_ids` — the caller's placement chain — that are neither revoked nor
/// past their window at the database's own `now()`.
///
/// One indexed scan per governed request, on the partial index migration
/// 0022 creates for exactly this predicate. The chain is passed rather than
/// the subject because a lapse grants to a *scope*: everyone placed at or
/// under the grantee gets it, and membership changes need no second act
/// (ADR-0037 decision 2).
///
/// **This is the expiry mechanism.** `now()` is the database's, so every
/// decision in one request judges the same clock, and the window ends
/// whether or not anything is running.
#[tracing::instrument(
    name = "store.lapses.active_for_scopes",
    skip_all,
    fields(tenant.id = %tenant_id, scopes = scope_ids.len(), lapses = tracing::field::Empty),
    err(Display)
)]
pub async fn active_for_scopes(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    scope_ids: &[ScopeId],
) -> Result<Vec<Lapse>> {
    if scope_ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<uuid::Uuid> = scope_ids.iter().map(ScopeId::as_uuid).collect();
    let rows = sqlx::query_as!(
        LapseRow,
        r#"
        select tenant_id, id, proposal_id, grantee_scope_id, target_scope_id,
               action, max_sensitivity, reason, granted_at, expires_at, granted_by,
               revoked_at, revoked_by, revoke_reason, expiry_recorded_at
        from policy_lapses
        where tenant_id = $1
          and grantee_scope_id = any($2)
          and revoked_at is null
          and expires_at > now()
        order by granted_at, id
        "#,
        tenant_id.as_uuid(),
        &ids,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    tracing::Span::current().record("lapses", rows.len());
    collect(rows)
}

/// Runs an approved lapse proposal's effect: opens the grant.
///
/// The window starts **now**, never when the proposal opened — a proposal
/// that sat in a queue for a week must not spend the window it was approved
/// for (ADR-0037 decision 4). `duration_secs` is the reviewed terms' own,
/// already bounded by the target pack's ceiling at the surface that called
/// this, and both ends of the window come from the database's clock so a
/// grant can never be born already expired by a skewed one.
///
/// The unique constraint on `(tenant_id, proposal_id)` makes a replayed
/// effect a [`Error::Conflict`] rather than a second standing window.
#[tracing::instrument(
    name = "store.lapses.grant",
    skip_all,
    fields(tenant.id = %tenant_id, proposal.id = %proposal_id),
    err(Display)
)]
#[allow(clippy::too_many_arguments)]
pub async fn grant(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    proposal_id: ProposalId,
    grantee_scope_id: ScopeId,
    target_scope_id: ScopeId,
    action: LapseAction,
    max_sensitivity: Sensitivity,
    reason: &str,
    duration_secs: u32,
    granted_by: IdentityId,
) -> Result<Lapse> {
    let row = sqlx::query_as!(
        LapseRow,
        r#"
        insert into policy_lapses
            (tenant_id, id, proposal_id, grantee_scope_id, target_scope_id,
             action, max_sensitivity, reason, granted_at, expires_at, granted_by)
        values ($1, $2, $3, $4, $5, $6, $7, $8, now(),
                now() + make_interval(secs => $9::double precision), $10)
        returning tenant_id, id, proposal_id, grantee_scope_id, target_scope_id,
                  action, max_sensitivity, reason, granted_at, expires_at, granted_by,
                  revoked_at, revoked_by, revoke_reason, expiry_recorded_at
        "#,
        tenant_id.as_uuid(),
        LapseId::new().as_uuid(),
        proposal_id.as_uuid(),
        grantee_scope_id.as_uuid(),
        target_scope_id.as_uuid(),
        action.as_str(),
        max_sensitivity.as_str(),
        reason,
        f64::from(duration_secs),
        granted_by.as_uuid(),
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    row.try_into()
}

/// Ends a standing grant early, with a mandatory reason.
///
/// Returns the revoked row, or [`Error::NotFound`] when no *standing* grant
/// has that id — an already-revoked or already-expired grant is not
/// revocable, and saying so beats reporting success for a no-op.
#[tracing::instrument(
    name = "store.lapses.revoke",
    skip_all,
    fields(tenant.id = %tenant_id, lapse.id = %id),
    err(Display)
)]
pub async fn revoke(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: LapseId,
    revoked_by: IdentityId,
    reason: &str,
) -> Result<Lapse> {
    let row = sqlx::query_as!(
        LapseRow,
        r#"
        update policy_lapses
           set revoked_at = now(), revoked_by = $3, revoke_reason = $4
         where tenant_id = $1 and id = $2
           and revoked_at is null
           and expires_at > now()
        returning tenant_id, id, proposal_id, grantee_scope_id, target_scope_id,
                  action, max_sensitivity, reason, granted_at, expires_at, granted_by,
                  revoked_at, revoked_by, revoke_reason, expiry_recorded_at
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
        revoked_by.as_uuid(),
        reason,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?
    .ok_or_else(|| Error::NotFound {
        entity: "standing lapse".to_owned(),
    })?;
    row.try_into()
}

/// One grant by id, standing or not — the detail read and the audit's.
#[tracing::instrument(
    name = "store.lapses.by_id",
    skip_all,
    fields(tenant.id = %tenant_id, lapse.id = %id),
    err(Display)
)]
pub async fn by_id(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: LapseId,
) -> Result<Option<Lapse>> {
    let row = sqlx::query_as!(
        LapseRow,
        r#"
        select tenant_id, id, proposal_id, grantee_scope_id, target_scope_id,
               action, max_sensitivity, reason, granted_at, expires_at, granted_by,
               revoked_at, revoked_by, revoke_reason, expiry_recorded_at
        from policy_lapses where tenant_id = $1 and id = $2
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(Lapse::try_from).transpose()
}

/// Every grant ever made over `target_scope_id`, newest first — the admin
/// listing behind `PolicyRead`.
///
/// Expired and revoked rows are included deliberately: "who could read this
/// scope's material in March" is the question the surface exists for, and a
/// listing of only standing grants cannot answer it.
#[tracing::instrument(
    name = "store.lapses.at_target",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %target_scope_id),
    err(Display)
)]
pub async fn at_target(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    target_scope_id: ScopeId,
) -> Result<Vec<Lapse>> {
    let rows = sqlx::query_as!(
        LapseRow,
        r#"
        select tenant_id, id, proposal_id, grantee_scope_id, target_scope_id,
               action, max_sensitivity, reason, granted_at, expires_at, granted_by,
               revoked_at, revoked_by, revoke_reason, expiry_recorded_at
        from policy_lapses
        where tenant_id = $1 and target_scope_id = $2
        order by granted_at desc, id
        "#,
        tenant_id.as_uuid(),
        target_scope_id.as_uuid(),
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    collect(rows)
}

/// Grants whose window has closed and whose expiry has not been chained
/// yet — the sweep's input (ADR-0037 decision 4).
///
/// Bookkeeping, not enforcement: these grants stopped deciding anything the
/// moment `expires_at` passed, whether or not this ever runs. Revoked rows
/// are excluded — their ending is already on the chain as
/// `policy.lapse.revoked`, and a second event asserting the same fact is
/// something an auditor would have to reconcile (ADR-0019 decision 4).
///
/// `limit` bounds one pass so a tenant that accumulated a backlog does not
/// hold a transaction open across all of it.
#[tracing::instrument(
    name = "store.lapses.due_for_expiry_event",
    skip_all,
    fields(tenant.id = %tenant_id, due = tracing::field::Empty),
    err(Display)
)]
pub async fn due_for_expiry_event(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    limit: i64,
) -> Result<Vec<Lapse>> {
    let rows = sqlx::query_as!(
        LapseRow,
        r#"
        select tenant_id, id, proposal_id, grantee_scope_id, target_scope_id,
               action, max_sensitivity, reason, granted_at, expires_at, granted_by,
               revoked_at, revoked_by, revoke_reason, expiry_recorded_at
        from policy_lapses
        where tenant_id = $1
          and expiry_recorded_at is null
          and revoked_at is null
          and expires_at <= now()
        order by expires_at, id
        limit $2
        "#,
        tenant_id.as_uuid(),
        limit,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    tracing::Span::current().record("due", rows.len());
    collect(rows)
}

/// Stamps a grant as having had its expiry chained.
///
/// `where expiry_recorded_at is null` makes the stamp the idempotency key:
/// two overlapping sweeps cannot chain one expiry twice, and the loser
/// simply finds nothing to update — the FLOW-4 lesson, which cost a
/// double-counted projection to learn.
#[tracing::instrument(
    name = "store.lapses.record_expiry",
    skip_all,
    fields(tenant.id = %tenant_id, lapse.id = %id),
    err(Display)
)]
pub async fn record_expiry(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: LapseId,
) -> Result<bool> {
    let result = sqlx::query!(
        r#"
        update policy_lapses set expiry_recorded_at = now()
        where tenant_id = $1 and id = $2 and expiry_recorded_at is null
        "#,
        tenant_id.as_uuid(),
        id.as_uuid(),
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(result.rows_affected() == 1)
}
