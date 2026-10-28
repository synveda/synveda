//! Durable tenant usage on the request path and its authorised public view.
//!
//! Accepted Session event payload bytes and created ContextRuns are charged in
//! the transaction that accepts them, after its audit event, so a refusal
//! rolls back the work, its audit event and its idempotency record together.
//! Lock order is the tenant audit head, then the usage day, on every charging
//! path. Expensive work that precedes its durable completion checks the
//! standing budget first; that check is advisory and the charge decides.
//!
//! Overrides only narrow the deployment ceiling and always expire. Reading
//! usage needs `UsageRead` at the tenant and changing an override needs
//! `UsageBudgetManage`; both are content-free in the audit chain.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{Action, Resource};
use synveda_store::rls;
use synveda_store::usage::{self, Meter};
use synveda_types::{Error, Result, TenantId};

use crate::app::AppState;
use crate::audit;
use crate::authz;
use crate::limits::UsagePolicy;
use crate::request::{body, commit, tenant_id};
use crate::response::{finish, outcome};
use crate::telemetry::TENANT_USAGE_CHARGES_TOTAL;

const DEFAULT_DAYS: i32 = 7;

fn ceiling(policy: UsagePolicy, meter: Meter) -> i64 {
    match meter {
        Meter::SessionEventBytes => policy.ceilings.session_event_bytes,
        Meter::ContextRuns => policy.ceilings.context_runs,
    }
}

fn spent(meter: Meter) -> Error {
    Error::RateLimited {
        message: format!(
            "the tenant's daily {meter} budget is spent; a tenant administrator can review usage"
        ),
    }
}

fn charged(meter: Meter, outcome: &'static str) {
    metrics::counter!(
        TENANT_USAGE_CHARGES_TOTAL,
        "meter" => meter.as_str(),
        "outcome" => outcome,
    )
    .increment(1);
}

/// Refuses work the day's budget cannot admit before it starts.
pub(crate) async fn ensure_room(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    meter: Meter,
    units: i64,
    policy: UsagePolicy,
) -> Result<()> {
    if !policy.enforce {
        return Ok(());
    }
    let standing = usage::standing(tx, tenant_id, meter, ceiling(policy, meter)).await?;
    if standing.used.saturating_add(units) > standing.daily_limit {
        charged(meter, "precheck_refused");
        return Err(spent(meter));
    }
    Ok(())
}

/// Charges accepted work inside the transaction that accepts it.
pub(crate) async fn charge(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    meter: Meter,
    units: i64,
    policy: UsagePolicy,
) -> Result<()> {
    if units == 0 {
        return Ok(());
    }
    let landed = usage::charge(
        tx,
        tenant_id,
        meter,
        units,
        ceiling(policy, meter),
        policy.enforce,
    )
    .await?;
    match landed {
        None => {
            charged(meter, "refused");
            Err(spent(meter))
        }
        Some(charge) if charge.over_budget => {
            charged(meter, "observed_over_budget");
            Ok(())
        }
        Some(_) => {
            charged(meter, "charged");
            Ok(())
        }
    }
}

/// `GET /v1/usage` query.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct UsageParams {
    /// Days of history including today, 1 to 31; defaults to 7.
    days: Option<i32>,
}

/// One meter today.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct MeterView {
    /// `session_event_bytes` or `context_runs`.
    meter: String,
    /// Units charged today.
    used_today: i64,
    /// The deployment's daily ceiling.
    ceiling: i64,
    /// The limit in force: the ceiling, narrowed by any override.
    daily_limit: i64,
    /// The unexpired override, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    budget_override: Option<OverrideView>,
}

/// An expiring narrowing of one meter's daily limit.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct OverrideView {
    /// The narrowed daily limit.
    daily_limit: i64,
    /// Why it was set.
    reason: String,
    /// The subject that set it.
    set_by: String,
    /// When it was set.
    set_at: DateTime<Utc>,
    /// When it stops applying.
    expires_at: DateTime<Utc>,
}

/// One meter on one UTC day.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct UsageDayView {
    /// The UTC day.
    day: NaiveDate,
    /// `session_event_bytes` or `context_runs`.
    meter: String,
    /// Units charged that day.
    units: i64,
}

/// `GET /v1/usage`.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct UsageView {
    /// The database's current UTC day; meters reset when it changes.
    today: NaiveDate,
    /// Whether exhausted budgets refuse work or are only counted.
    enforced: bool,
    /// Every meter's standing today.
    meters: Vec<MeterView>,
    /// Charged days, newest first. A day without a row charged nothing.
    days: Vec<UsageDayView>,
}

/// `PUT /v1/usage/budgets/{meter}`.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BudgetOverrideBody {
    /// The narrowed daily limit. Zero stops the meter's work until expiry; a
    /// value above the deployment ceiling has no effect beyond the ceiling.
    daily_limit: i64,
    /// Why, without customer content. 1 to 500 characters.
    reason: String,
    /// Seconds until the override expires, 60 to 7,776,000 (90 days).
    expires_in_secs: i64,
}

fn meter(name: &str) -> Result<Meter> {
    name.parse().map_err(|_| Error::NotFound {
        entity: format!("usage meter {name:?}"),
    })
}

fn override_view(budget: usage::BudgetOverride) -> OverrideView {
    OverrideView {
        daily_limit: budget.daily_limit,
        reason: budget.reason,
        set_by: budget.set_by,
        set_at: budget.set_at,
        expires_at: budget.expires_at,
    }
}

fn subject() -> Result<String> {
    synveda_identity::current_tenant()
        .map(|context| context.claims.subject)
        .ok_or_else(|| Error::Internal {
            message: "usage route ran outside a tenant scope".to_owned(),
        })
}

/// The caller's tenant usage: today's standing per meter and recent days.
#[utoipa::path(
    get,
    path = "/v1/usage",
    operation_id = "get_tenant_usage",
    tag = "usage",
    params(UsageParams),
    responses(
        (status = 200, description = "The caller's tenant usage and budgets", body = UsageView),
        (status = 400, description = "The day window is out of range", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Tenant usage is not visible to the caller", body = crate::workspaces::ApiErrorBody),
        (status = 429, description = "A request limit was reached; retry after the indicated delay", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "usage.read", skip_all)]
pub(crate) async fn get(
    State(state): State<AppState>,
    Extension(policy): Extension<UsagePolicy>,
    Query(params): Query<UsageParams>,
) -> Response {
    let result = read(&state, policy, params.days.unwrap_or(DEFAULT_DAYS)).await;
    metrics::counter!(crate::telemetry::TENANT_USAGE_REQUESTS_TOTAL, "op" => "read", "outcome" => outcome(&result))
        .increment(1);
    finish(&state, "usage.read", result.map(Json)).await
}

async fn read(state: &AppState, policy: UsagePolicy, days: i32) -> Result<UsageView> {
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    authz::require(
        state,
        &mut tx,
        Action::UsageRead,
        Resource::Tenant(tenant_id),
        None,
    )
    .await?;
    let (today, recent) = usage::recent(&mut tx, tenant_id, days).await?;
    let overrides = usage::overrides(&mut tx, tenant_id).await?;
    commit(tx).await?;
    let meters = Meter::ALL
        .into_iter()
        .map(|meter| {
            let ceiling = ceiling(policy, meter);
            let budget = overrides
                .iter()
                .find(|budget| budget.meter == meter)
                .cloned();
            MeterView {
                meter: meter.as_str().to_owned(),
                used_today: recent
                    .iter()
                    .find(|day| day.meter == meter && day.day == today)
                    .map_or(0, |day| day.units),
                ceiling,
                daily_limit: budget
                    .as_ref()
                    .map_or(ceiling, |budget| budget.daily_limit.min(ceiling)),
                budget_override: budget.map(override_view),
            }
        })
        .collect();
    Ok(UsageView {
        today,
        enforced: policy.enforce,
        meters,
        days: recent
            .into_iter()
            .map(|day| UsageDayView {
                day: day.day,
                meter: day.meter.as_str().to_owned(),
                units: day.units,
            })
            .collect(),
    })
}

/// Sets an expiring override that narrows one meter's daily limit.
#[utoipa::path(
    put,
    path = "/v1/usage/budgets/{meter}",
    operation_id = "set_tenant_usage_budget",
    tag = "usage",
    params(("meter" = String, Path, description = "`session_event_bytes` or `context_runs`")),
    request_body = BudgetOverrideBody,
    responses(
        (status = 200, description = "The override in force", body = OverrideView),
        (status = 400, description = "The limit, reason or expiry is out of range", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Changing tenant budgets is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "No such meter", body = crate::workspaces::ApiErrorBody),
        (status = 429, description = "A request limit was reached; retry after the indicated delay", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "usage.budget.set", skip_all)]
pub(crate) async fn set_budget(
    State(state): State<AppState>,
    Path(name): Path<String>,
    payload: std::result::Result<Json<BudgetOverrideBody>, JsonRejection>,
) -> Response {
    let result = async {
        let meter = meter(&name)?;
        let request = body(payload)?;
        set(&state, meter, &request).await
    }
    .await;
    metrics::counter!(crate::telemetry::TENANT_USAGE_REQUESTS_TOTAL, "op" => "set", "outcome" => outcome(&result))
        .increment(1);
    finish(&state, "usage.budget.set", result.map(Json)).await
}

async fn set(state: &AppState, meter: Meter, request: &BudgetOverrideBody) -> Result<OverrideView> {
    let tenant_id = tenant_id()?;
    let set_by = subject()?;
    let reason = request.reason.trim();
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let authorized = authz::require(
        state,
        &mut tx,
        Action::UsageBudgetManage,
        Resource::Tenant(tenant_id),
        None,
    )
    .await?;
    let (current, previous) = usage::set_override(
        &mut tx,
        tenant_id,
        &usage::NewOverride {
            meter,
            daily_limit: request.daily_limit,
            reason,
            set_by: &set_by,
            ttl_secs: request.expires_in_secs,
        },
    )
    .await?;
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::TenantUsageBudgetChanged,
        format!("tenant {tenant_id}"),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::UsageBudgetManage, &authorized),
            "change": "set",
            "meter": meter.as_str(),
            "daily_limit": current.daily_limit,
            "expires_at": current.expires_at,
            "previous_daily_limit": previous.as_ref().map(|budget| budget.daily_limit),
            "previous_expires_at": previous.as_ref().map(|budget| budget.expires_at),
            "reason": reason,
        }),
    )
    .await?;
    commit(tx).await?;
    Ok(override_view(current))
}

/// Clears one meter's override, restoring the deployment ceiling.
#[utoipa::path(
    delete,
    path = "/v1/usage/budgets/{meter}",
    operation_id = "clear_tenant_usage_budget",
    tag = "usage",
    params(("meter" = String, Path, description = "`session_event_bytes` or `context_runs`")),
    responses(
        (status = 204, description = "No override is in force"),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Changing tenant budgets is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "No such meter", body = crate::workspaces::ApiErrorBody),
        (status = 429, description = "A request limit was reached; retry after the indicated delay", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "usage.budget.clear", skip_all)]
pub(crate) async fn clear_budget(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Response {
    let result = async {
        let meter = meter(&name)?;
        clear(&state, meter).await
    }
    .await;
    metrics::counter!(crate::telemetry::TENANT_USAGE_REQUESTS_TOTAL, "op" => "clear", "outcome" => outcome(&result))
        .increment(1);
    finish(
        &state,
        "usage.budget.clear",
        result.map(|()| StatusCode::NO_CONTENT),
    )
    .await
}

async fn clear(state: &AppState, meter: Meter) -> Result<()> {
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let authorized = authz::require(
        state,
        &mut tx,
        Action::UsageBudgetManage,
        Resource::Tenant(tenant_id),
        None,
    )
    .await?;
    let removed = usage::clear_override(&mut tx, tenant_id, meter).await?;
    if let Some(removed) = &removed {
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::TenantUsageBudgetChanged,
            format!("tenant {tenant_id}"),
            Outcome::Success,
            json!({
                "authz": audit::decision_context(Action::UsageBudgetManage, &authorized),
                "change": "cleared",
                "meter": meter.as_str(),
                "previous_daily_limit": removed.daily_limit,
                "previous_expires_at": removed.expires_at,
            }),
        )
        .await?;
    }
    commit(tx).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_meter_is_not_found_rather_than_invalid() {
        assert!(matches!(meter("tokens"), Err(Error::NotFound { .. })));
        assert_eq!(meter("context_runs").expect("meter"), Meter::ContextRuns);
    }

    #[test]
    fn a_spent_budget_is_a_rate_limit_naming_only_the_meter() {
        let error = spent(Meter::SessionEventBytes);
        assert!(matches!(error, Error::RateLimited { .. }));
        assert!(error.to_string().contains("session_event_bytes"));
    }
}
