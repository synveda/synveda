//! Tenant context propagation (TEN-1, ADR-0008).
//!
//! The gateway's tenant-resolution middleware wraps each request's remaining
//! stack in [`with_tenant`]; anything running inside that scope — handlers,
//! retrieval, composition — reads [`current_tenant`] without a parameter
//! threaded through every signature. The store tier deliberately sits below
//! this seam and keeps taking `TenantId` explicitly (ADR-0008).

use std::future::Future;

use synveda_types::Tenant;

use crate::token::Claims;

/// Who this request runs as: the resolved tenant plus the verified claims.
/// The full [`Claims`] ride along (not just the subject) so the PDP seam
/// can tell IdP-backed subjects from out-of-band ones — an IdP subject
/// with no provisioned identity is quarantined, a dev subject is not
/// (AUTH-2, ADR-0013 decision 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantContext {
    /// The resolved, active tenant.
    pub tenant: Tenant,
    /// The verified token claims (subject, tenant, provisioning).
    pub claims: Claims,
}

impl TenantContext {
    /// The verified token's `sub` claim.
    #[must_use]
    pub fn subject(&self) -> &str {
        &self.claims.subject
    }
}

tokio::task_local! {
    static TENANT_CONTEXT: TenantContext;
}

/// Runs `future` with `context` as the ambient tenant. Nested scopes shadow
/// (innermost wins), matching task-local semantics; the gateway sets exactly
/// one scope per request.
pub async fn with_tenant<F: Future>(context: TenantContext, future: F) -> F::Output {
    TENANT_CONTEXT.scope(context, future).await
}

/// The ambient tenant context, or `None` outside any [`with_tenant`] scope.
/// Callers on authenticated paths treat `None` as a broken invariant — the
/// middleware always establishes the scope before they run.
#[must_use]
pub fn current_tenant() -> Option<TenantContext> {
    TENANT_CONTEXT.try_with(Clone::clone).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use synveda_types::{TenantId, TenantStatus};

    fn context(subject: &str) -> TenantContext {
        let tenant = Tenant {
            id: TenantId::new(),
            slug: "acme".into(),
            name: "ACME".into(),
            status: TenantStatus::Active,
            created_at: "2026-07-18T12:00:00Z".parse().unwrap(),
        };
        TenantContext {
            claims: Claims {
                subject: subject.into(),
                tenant_id: tenant.id,
                provisioning: None,
                lifetime: None,
                credential_class: crate::token::CredentialClass::PrimaryBearer,
            },
            tenant,
        }
    }

    #[tokio::test]
    async fn context_is_visible_inside_the_scope_and_absent_outside() {
        assert_eq!(current_tenant(), None, "no ambient context outside scope");
        let ctx = context("alice");
        let seen = with_tenant(ctx.clone(), async { current_tenant() }).await;
        assert_eq!(seen, Some(ctx));
        assert_eq!(current_tenant(), None, "scope must not leak");
    }

    #[tokio::test]
    async fn context_survives_await_points_and_spawned_scope() {
        let ctx = context("alice");
        let seen = with_tenant(ctx.clone(), async {
            tokio::task::yield_now().await;
            current_tenant()
        })
        .await;
        assert_eq!(seen, Some(ctx));
    }
}
