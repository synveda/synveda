//! The governed scope chain as the offline pipelines take it (CPR-7,
//! ADR-0074 decision 1).
//!
//! The background sweeps replaced three call shapes that all lived on the
//! old hierarchy's read-through cache (HIER-2): the per-scope chain, the
//! owner's own chain, and the tenant root. One helper per shape, all built
//! on `scopes` + `scope_closure`, with no cache in front — the sweeps run
//! per tenant per cycle rather than per request, and ADR-0016's freshness
//! argument was about requests.
//!
//! The seal rides the chain head only: a `principal`-shaped scope is
//! somebody's own, and its sealed-ness is a property of the identity that
//! owns it (ADR-0059 decision 7) — one indexed read, and no other shape
//! ever carries one.

use sqlx::PgConnection;
use synveda_policy::ScopeNode;
use synveda_store::{identities, scopes};
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{Result, ScopeId, TenantId};

/// `scope_id`'s chain, node-first to the tenant root, as the PDP takes it.
///
/// Empty when the scope does not exist for this tenant — the same
/// fail-closed reading a deleted node's chain got: absent, not forbidden.
#[tracing::instrument(
    name = "ingest.chain.scope_chain",
    skip_all,
    fields(tenant.id = %tenant_id, scope.id = %scope_id),
    err(Display)
)]
pub async fn scope_chain(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    scope_id: ScopeId,
) -> Result<Vec<ScopeNode>> {
    let Some(scope) = scopes::get(&mut *conn, tenant_id, scope_id).await? else {
        return Ok(Vec::new());
    };
    let mut nodes = vec![ScopeNode::from_scope(
        &scope,
        sealed(conn, tenant_id, &scope).await?,
    )];
    for ancestor in scopes::ancestors(&mut *conn, tenant_id, scope_id).await? {
        nodes.push(ScopeNode::from_scope(&ancestor, false));
    }
    Ok(nodes)
}

/// Whether `scope` is sealed — the identity-owner derivation, on the one
/// shape that is ever somebody's own.
async fn sealed(conn: &mut PgConnection, tenant_id: TenantId, scope: &Scope) -> Result<bool> {
    if scope.kind != ScopeKind::Principal {
        return Ok(false);
    }
    Ok(identities::by_scope(&mut *conn, tenant_id, scope.id)
        .await?
        .is_some_and(|identity| identity.sealed()))
}
