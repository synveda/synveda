//! Small request-shaped helpers shared by every `/v1` handler (re-homed
//! from the hierarchy routes CPR-7 deleted, ADR-0074): the task-local
//! tenant read, the JSON-body taxonomy map, the uniform-404 ownership
//! check, and the commit wrapper.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use synveda_types::scope::Scope;
use synveda_types::{Error, Result, ScopeId, TenantId};

/// The resolved tenant, from the request's task-local context.
pub(crate) fn tenant_id() -> Result<TenantId> {
    synveda_identity::current_tenant()
        .map(|context| context.tenant.id)
        .ok_or_else(|| Error::Internal {
            message: "route ran outside a tenant scope".to_owned(),
        })
}

/// Maps a malformed JSON body onto the taxonomy instead of axum's default
/// plain-text rejection.
pub(crate) fn body<T>(payload: std::result::Result<Json<T>, JsonRejection>) -> Result<T> {
    payload
        .map(|Json(inner)| inner)
        .map_err(|rejection| Error::Invalid {
            message: format!("invalid request body: {rejection}"),
        })
}

pub(crate) fn not_found(id: ScopeId) -> Error {
    Error::NotFound {
        entity: format!("scope {id}"),
    }
}

/// The uniform 404 for missing *and* foreign scopes. Under the production
/// `synveda_app` role RLS already hides other tenants' rows; this explicit
/// tenant check keeps the API correct even on connections that bypass RLS
/// (the dev-compose superuser — ADR-0009's accepted trade-off), and it is
/// why every mutation starts by fetching the scope.
pub(crate) fn found(scope: Option<Scope>, tenant_id: TenantId, id: ScopeId) -> Result<Scope> {
    scope
        .filter(|scope| scope.tenant_id == tenant_id)
        .ok_or_else(|| not_found(id))
}

pub(crate) async fn commit(tx: sqlx::Transaction<'static, sqlx::Postgres>) -> Result<()> {
    tx.commit().await.map_err(|err| Error::Storage {
        message: format!("commit transaction: {err}"),
    })
}
