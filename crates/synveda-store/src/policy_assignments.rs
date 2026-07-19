//! Per-node policy pack assignments and the tenant default (AUTHZ-2,
//! ADR-0014 decisions 3–4).
//!
//! Assignments are the *application* of packs: a node (and its subtree,
//! until a deeper assignment) runs the named pack. They are request-time
//! data — governed handlers read [`for_scopes`] for the resource's chain
//! inside their own transaction and hand the rows to the PDP, so a switch
//! is in force on the very next request. Both tables are tenant-scoped
//! (forced RLS, ADR-0009): reach them inside
//! [`crate::rls::begin_tenant_tx`].
//!
//! Whether the assigned name denotes a real pack (embedded product pack or
//! stored custom pack) is the gateway's check at assign time — the store
//! knows nothing of policy (seed §2.4).

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use synveda_types::{Error, PolicyAssignment, Result, ScopeId, TenantId};

/// Maps a sqlx error at the storage boundary into the shared taxonomy.
fn storage_error(err: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db) = &err {
        // 23503 foreign_key_violation: no such tenant or scope.
        if db.code().as_deref() == Some("23503") {
            return Error::NotFound {
                entity: "tenant or scope".to_owned(),
            };
        }
        // 23514 check_violation: malformed pack name.
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

/// Assigns `pack_name` at the node, replacing any previous assignment —
/// the subtree below runs it from the next request on (ADR-0014
/// decision 3).
#[tracing::instrument(
    name = "store.policy_assignments.assign",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %scope_id, policy.pack = pack_name),
    err(Display)
)]
pub async fn assign(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    scope_id: ScopeId,
    pack_name: &str,
) -> Result<PolicyAssignment> {
    let row = sqlx::query_as!(
        AssignmentRow,
        r#"
        insert into policy_pack_assignments (tenant_id, scope_id, pack_name)
        values ($1, $2, $3)
        on conflict (tenant_id, scope_id) do update
            set pack_name = excluded.pack_name,
                updated_at = now()
        returning tenant_id, scope_id, pack_name, updated_at
        "#,
        tenant_id.as_uuid(),
        scope_id.as_uuid(),
        pack_name,
    )
    .fetch_one(executor)
    .await
    .map_err(storage_error)?;
    Ok(row.into())
}

/// Removes the node's assignment; the node falls back to the inherited
/// pack (nearest assigned ancestor → tenant default → embedded default).
/// Returns whether an assignment was removed.
#[tracing::instrument(
    name = "store.policy_assignments.unassign",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %scope_id),
    err(Display)
)]
pub async fn unassign(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    scope_id: ScopeId,
) -> Result<bool> {
    let result = sqlx::query!(
        "delete from policy_pack_assignments where tenant_id = $1 and scope_id = $2",
        tenant_id.as_uuid(),
        scope_id.as_uuid(),
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(result.rows_affected() > 0)
}

/// The assignments carried by any of `scope_ids` — what a governed
/// handler reads for the resource's chain and hands to the PDP.
#[tracing::instrument(
    name = "store.policy_assignments.for_scopes",
    skip_all,
    fields(tenant.id = %tenant_id, scope.count = scope_ids.len()),
    err(Display)
)]
pub async fn for_scopes(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    scope_ids: &[ScopeId],
) -> Result<Vec<PolicyAssignment>> {
    let ids: Vec<uuid::Uuid> = scope_ids.iter().map(ScopeId::as_uuid).collect();
    let rows = sqlx::query_as!(
        AssignmentRow,
        r#"
        select tenant_id, scope_id, pack_name, updated_at
        from policy_pack_assignments
        where tenant_id = $1 and scope_id = any($2)
        "#,
        tenant_id.as_uuid(),
        &ids,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// Sets the tenant default pack (what AUTHZ-1's tenant-wide stored pack
/// became): in force wherever no node on the chain carries an assignment.
#[tracing::instrument(
    name = "store.policy_assignments.set_default",
    skip_all,
    fields(tenant.id = %tenant_id, policy.pack = pack_name),
    err(Display)
)]
pub async fn set_default(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    pack_name: &str,
) -> Result<()> {
    sqlx::query!(
        r#"
        insert into policy_pack_defaults (tenant_id, pack_name)
        values ($1, $2)
        on conflict (tenant_id) do update
            set pack_name = excluded.pack_name,
                updated_at = now()
        "#,
        tenant_id.as_uuid(),
        pack_name,
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(())
}

/// The tenant's default pack name, when one is stored.
#[tracing::instrument(
    name = "store.policy_assignments.default_pack",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn default_pack(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
) -> Result<Option<String>> {
    sqlx::query_scalar!(
        "select pack_name from policy_pack_defaults where tenant_id = $1",
        tenant_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)
}

/// Clears the tenant default; the embedded `regulated-strict` applies
/// wherever nothing is assigned. Returns whether a default was removed.
#[tracing::instrument(
    name = "store.policy_assignments.clear_default",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn clear_default(executor: impl PgExecutor<'_>, tenant_id: TenantId) -> Result<bool> {
    let result = sqlx::query!(
        "delete from policy_pack_defaults where tenant_id = $1",
        tenant_id.as_uuid(),
    )
    .execute(executor)
    .await
    .map_err(storage_error)?;
    Ok(result.rows_affected() > 0)
}

struct AssignmentRow {
    tenant_id: uuid::Uuid,
    scope_id: uuid::Uuid,
    pack_name: String,
    updated_at: DateTime<Utc>,
}

impl From<AssignmentRow> for PolicyAssignment {
    fn from(row: AssignmentRow) -> Self {
        PolicyAssignment {
            tenant_id: TenantId::from_uuid(row.tenant_id),
            scope_id: ScopeId::from_uuid(row.scope_id),
            pack_name: row.pack_name,
            updated_at: row.updated_at,
        }
    }
}
