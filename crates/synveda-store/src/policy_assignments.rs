//! Cedar's derived policy-selection projection (CPR-30, ADR-0089).
//!
//! There is deliberately no persisted `PolicyAssignment` model and no write
//! function in this module. Governed Configuration bindings are the sole
//! selector; these readers render only the compact shape the existing PDP
//! consumes while the Cedar entity model remains independent of storage.

use sqlx::PgConnection;
use synveda_types::{PolicyAssignment, Result, ScopeId, TenantId};

/// Render the policy-pack selectors carried by bindings at the supplied
/// scopes. Callers still pass the complete resource chain to the PDP, which
/// resolves nearest-first and falls back to the strict embedded pack.
#[tracing::instrument(
    name = "store.policy_assignments.for_scopes",
    skip_all,
    fields(tenant.id = %tenant_id, scope.count = scope_ids.len()),
    err(Display)
)]
pub async fn for_scopes(
    connection: &mut PgConnection,
    tenant_id: TenantId,
    scope_ids: &[ScopeId],
) -> Result<Vec<PolicyAssignment>> {
    crate::configuration::policy_assignments_for_scopes(connection, tenant_id, scope_ids).await
}

/// Render the tenant-root selector for the PDP's tenant-resource fallback.
#[tracing::instrument(
    name = "store.policy_assignments.default_pack",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn default_pack(
    connection: &mut PgConnection,
    tenant_id: TenantId,
) -> Result<Option<String>> {
    crate::configuration::tenant_policy_pack(connection, tenant_id).await
}
