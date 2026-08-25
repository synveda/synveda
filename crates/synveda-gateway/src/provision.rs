//! JIT user provisioning (AUTH-2, ADR-0013), re-cut onto governed scopes
//! by CPR-7 (ADR-0074 decision 3): at login completion, a first-time
//! subject is bound to its own `principal`-shaped scope, minted in the
//! provisioning transaction.
//!
//! There is no placement convention any more. The
//! `synveda-{dept}-{team}` convention, the `group_mappings` override table
//! and the reserved `quarantine` scope are deleted with the hierarchy:
//! "unmapped" now means *ungranted* — a principal with no grants reaches
//! nothing beyond their own scope because the anchor model and the
//! base-layer privacy floor say so, decided per action rather than per
//! person. Directory adapters rebuild placement as enterprise-profile
//! configuration in a later prompt of the programme.
//!
//! The one convention that survives is the admin door (ADR-0074 decision
//! 4): a member of the IdP's `synveda-admins` group is upserted an
//! `administrator` grant at the tenant root scope at every login — the
//! ADR-0015 decision 6 shape on the new noun, and the operator door
//! ADR-0073 recorded as missing: a tenant's first admin-group login mints
//! its first grant.
//!
//! This is a system write path driven by verified IdP claims — the same
//! trust class as tenant admission and the SCIM sync (AUTH-4) — so no PDP
//! check guards the minting itself; enforcement happens on every
//! subsequent action through the anchor model. It is NOT a path to
//! governed assets (seed §2.2).
//!
//! Audited since AUD-1 (ADR-0019): a created identity chains
//! `identity.provisioned` in the provisioning transaction, and the admin
//! grant's first establishment chains `access.granted` — not every
//! login's no-op upsert (decision 6). The actor is the provisioned
//! subject itself: this plane runs at login completion, outside the
//! task-local tenant scope.

use serde_json::json;
use synveda_audit::{Actor, AuditAction, Outcome};
use synveda_identity::ProvisioningClaims;
use synveda_store::{access, directory, identities, rls, scopes};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::scope::Scope;
use synveda_types::{Error, Identity, IdentityId, IdentityKind, Result, Tenant, TenantId};

use crate::app::AppState;
use crate::audit;
use crate::telemetry::JIT_PROVISIONS_TOTAL;

/// A provisioned login: the identity and the principal scope that is its
/// own.
pub struct Provisioned {
    /// The subject's identity — existing on repeat logins, fresh on first.
    pub identity: Identity,
    /// The identity's own principal-shaped scope.
    pub scope: Scope,
    /// That scope's slug chain from the tenant root — display only, and
    /// read here because the transaction that minted the scope is the only
    /// place its ancestry is already open (CPR-7: the login response
    /// promises a *path*, and a bare slug is not one).
    pub scope_path: String,
}

/// Provisions `subject` into `tenant`'s hierarchy if this is its first
/// login, and returns the identity either way.
///
/// Public because it is the login path's whole entry point and AUTH-4's
/// acceptance suite drives it directly: the correspondence rule (ADR-0059
/// decision 4) is a claim about what happens when a person arrives, and a
/// test that could only reach it through a mock IdP's redirect chain would
/// be testing the redirect chain. Runs in one tenant
/// transaction; the first-login race (two concurrent callbacks) surfaces
/// as a unique-constraint conflict, retried once to adopt the winner's
/// identity (ADR-0013 decision 2).
#[tracing::instrument(
    name = "identity.provision",
    skip_all,
    fields(
        tenant.id = %tenant.id,
        identity.outcome = tracing::field::Empty,
        scope.id = tracing::field::Empty,
    ),
    err(Display)
)]
pub async fn provision(
    state: &AppState,
    tenant: &Tenant,
    subject: &str,
    claims: &ProvisioningClaims,
) -> Result<Provisioned> {
    let mut conflict_retried = false;
    let outcome = loop {
        match provision_once(state, tenant, subject, claims).await {
            Err(Error::Conflict { .. }) if !conflict_retried => {
                // Another login for this subject (or the tenant root's
                // creation) won the race; the re-read adopts its rows.
                conflict_retried = true;
            }
            outcome => break outcome,
        }
    };
    let (outcome, label) = match outcome {
        Ok((provisioned, label)) => (Ok(provisioned), label),
        Err(error) => (Err(error), "error"),
    };
    metrics::counter!(JIT_PROVISIONS_TOTAL, "outcome" => label).increment(1);
    let provisioned = outcome?;
    let span = tracing::Span::current();
    span.record("identity.outcome", label);
    span.record("scope.id", tracing::field::display(provisioned.scope.id));
    Ok(provisioned)
}

async fn provision_once(
    state: &AppState,
    tenant: &Tenant,
    subject: &str,
    claims: &ProvisioningClaims,
) -> Result<(Provisioned, &'static str)> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant.id).await?;

    // The admin convention group (AUTHZ-3, ADR-0015 decision 6; CPR-7,
    // ADR-0074 decision 4): upserted at *every* login completion — adding
    // someone to `synveda-admins` works on their next login. Additive only:
    // leaving the group revokes nothing until mover/leaver sync; revoking
    // stays an explicit, PDP-gated action. Only the grant's first
    // establishment chains an audit event (ADR-0019 decision 6) — repeat
    // logins are no-op upserts.
    if synveda_identity::contains_admin_group(&claims.groups) {
        ensure_admin_grant(&mut tx, tenant, subject).await?;
    }

    if let Some(identity) = identities::by_subject(&mut *tx, tenant.id, subject).await? {
        // A departed identity does not log in (AUTH-4, ADR-0059
        // decision 8). The enforcement seam would refuse every subsequent
        // action anyway, but refusing here means the person is told at the
        // door rather than handed a session that can do nothing — and the
        // subject stays bound, so nothing re-provisions them through the
        // JIT path below.
        if identity.sealed() {
            return Err(Error::Unauthenticated {
                message: "this identity has been deprovisioned".to_owned(),
            });
        }
        let scope = scopes::get(&mut *tx, tenant.id, identity.scope_id)
            .await?
            .ok_or_else(|| Error::Internal {
                message: format!("identity {} lost its scope", identity.id),
            })?;
        let scope_path = scope_path(&mut tx, tenant.id, &scope).await?;
        // **This branch commits.** It looks read-only and is not: the admin
        // convention above may have just written this tenant's first
        // `administrator` grant and its `access.granted` event, and a
        // directory-created identity whose subject is already bound reaches
        // the door down exactly this path. Returning without committing
        // dropped that grant silently on every such login — the operator
        // door of ADR-0074 decision 4, failing for the one population
        // (directory-synced admins) it exists for.
        tx.commit().await.map_err(|err| Error::Storage {
            message: format!("commit login provisioning: {err}"),
        })?;
        // `bound`, not "existing": the metric's outcome vocabulary since the
        // scope cutover is `own-scope` (a first login minted the scope) and
        // `bound` (this login adopted rows that were already there) — the
        // directory-adoption branch below and this one are the two ways a
        // login binds to something it did not mint, and one word covers
        // both (CPR-7, ADR-0074 decision 3).
        return Ok((
            Provisioned {
                identity,
                scope,
                scope_path,
            },
            "bound",
        ));
    }

    // The other half of the correspondence rule (AUTH-4, ADR-0059
    // decision 4): a directory may have created this person before they
    // ever logged in, and binding the subject to *that* identity is what
    // stops one person from having two — each with its own personal scope
    // and half their memory in it.
    if let Some(adopted) = adopt_directory_identity(&mut tx, tenant, subject, claims).await? {
        let scope = scopes::get(&mut *tx, tenant.id, adopted.scope_id)
            .await?
            .ok_or_else(|| Error::Internal {
                message: format!("identity {} lost its scope", adopted.id),
            })?;
        let scope_path = scope_path(&mut tx, tenant.id, &scope).await?;
        audit::record_as(
            &mut tx,
            tenant.id,
            Actor::subject(subject),
            AuditAction::IdentityProvisioned,
            format!("scope {}", scope.id),
            Outcome::Success,
            json!({
                "placement": "directory",
                "identity": {"id": adopted.id, "subject": adopted.subject},
                "scope": {"slug": scope.slug},
                "groups": claims.groups,
                "origin": "login-bind",
            }),
        )
        .await?;
        tx.commit().await.map_err(|err| Error::Storage {
            message: format!("commit subject binding: {err}"),
        })?;
        tracing::info!(
            identity.id = %adopted.id,
            scope.slug = %scope.slug,
            "first login bound its subject to a directory-created identity"
        );
        return Ok((
            Provisioned {
                identity: adopted,
                scope,
                scope_path,
            },
            "bound",
        ));
    }

    // First login: the subject's own principal scope, minted in the same
    // transaction as the identity row that binds it (CPR-7, ADR-0074
    // decision 3). No mapping, no parent convention — the scope hangs at
    // the tenant root, and everything beyond it is a grant.
    let identity_id = IdentityId::new();
    let display_name = claims.display_name.as_deref().unwrap_or(subject);
    let scope = scopes::ensure_principal_scope(&mut tx, tenant.id, subject, display_name).await?;
    let identity = identities::create(
        &mut tx,
        identity_id,
        tenant.id,
        Some(subject),
        IdentityKind::User,
        claims.email.as_deref(),
        claims.display_name.as_deref(),
        scope.id,
    )
    .await?;
    audit::record_as(
        &mut tx,
        tenant.id,
        Actor::subject(subject),
        AuditAction::IdentityProvisioned,
        format!("scope {}", scope.id),
        Outcome::Success,
        json!({
            "placement": "own-scope",
            "identity": {"id": identity.id, "subject": identity.subject},
            "scope": {"id": scope.id, "slug": scope.slug, "kind": scope.kind},
            "groups": claims.groups,
        }),
    )
    .await?;
    let scope_path = scope_path(&mut tx, tenant.id, &scope).await?;
    tx.commit().await.map_err(|err| Error::Storage {
        message: format!("commit provisioning transaction: {err}"),
    })?;
    // Provisioning committed a scope: flush the tenant's entity
    // fragments (ADR-0017 decision 5). The existing-identity path above
    // mutates no scope and does not invalidate.
    state.invalidate_scopes(tenant.id);
    tracing::info!(
        identity.id = %identity.id,
        scope.slug = %scope.slug,
        "identity provisioned"
    );
    Ok((
        Provisioned {
            identity,
            scope,
            scope_path,
        },
        "own-scope",
    ))
}

/// A scope's slug chain from the tenant root, for the login response's
/// display-only `scope_path`. Falls back to the bare slug rather than
/// failing a login: a path is a nicety, and a closure row that has not
/// landed is not a reason to refuse somebody their session.
async fn scope_path(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    scope: &Scope,
) -> Result<String> {
    Ok(scopes::path(&mut *tx, tenant_id, scope.id)
        .await?
        .unwrap_or_else(|| scope.slug.clone()))
}

/// The admin door (ADR-0074 decision 4): an `administrator` grant at the
/// tenant root scope, upserted for every `synveda-admins` login.
///
/// Additive only, and first-establishment-audited — the ADR-0015 decision
/// 6 discipline on the grant noun. This is also the operator door
/// ADR-0073 recorded as missing: on a tenant's first admin-group login it
/// mints the first grant any caller holds, which is what makes the tenant
/// governable without break-glass access to the database.
async fn ensure_admin_grant(
    tx: &mut sqlx::PgConnection,
    tenant: &Tenant,
    subject: &str,
) -> Result<()> {
    let root = scopes::ensure_tenant_root(&mut *tx, tenant.id).await?;
    let existing = access::list_grants(
        &mut *tx,
        tenant.id,
        &access::GrantFilter {
            principal_id: Some(subject.to_owned()),
            ..access::GrantFilter::default()
        },
    )
    .await?;
    let established = !existing
        .iter()
        .any(|grant| grant.scope_id == root.id && grant.role_key == RoleKey::Administrator);
    if established {
        let grant = access::create_grant(
            &mut *tx,
            &access::NewGrant {
                id: synveda_types::GrantId::new(),
                tenant_id: tenant.id,
                scope_id: root.id,
                subject: GrantSubject::Principal {
                    principal_id: subject.to_owned(),
                },
                role_key: RoleKey::Administrator,
                source: GrantSource::Automation,
                invite_id: None,
                granted_by: None,
            },
        )
        .await?;
        audit::record_as(
            &mut *tx,
            tenant.id,
            Actor::subject(subject),
            AuditAction::AccessGranted,
            format!("scope {}", root.id),
            Outcome::Success,
            json!({
                "origin": "jit-admin-group",
                "grant": {"subject": subject, "role": RoleKey::Administrator,
                          "scope_id": root.id, "id": grant.id},
            }),
        )
        .await?;
    }
    tracing::info!(tenant.id = %tenant.id, "admin-group login: administrator grant at the tenant root ensured");
    Ok(())
}

/// Finds the identity a directory created for this subject, if any, and
/// binds the subject to it (AUTH-4, ADR-0059 decisions 4 and 5).
///
/// The lookup is the mirror's, in the ADR's order: the issuer's configured
/// anchor claim against `externalId`, then the verified email against
/// `userName`, case-folded. Both are conservative — a mirror row that
/// already projected onto somebody is never re-bound, and a departed
/// identity is never adopted (decision 12).
///
/// **Why the anchor is a per-issuer claim.** Entra issues a pairwise `sub`
/// — unique per (application, user) — so it will never equal the directory
/// object id its provisioning agent sends. A server that joined on `sub`
/// alone would give every Entra user two identities and half their memory
/// in each. `IssuerConfig::external_id_claim` is set to `oid` for Entra,
/// beside `groups_claim`, which is the per-issuer seam ADR-0010 built for
/// exactly this class of vendor difference.
async fn adopt_directory_identity(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: &Tenant,
    subject: &str,
    claims: &ProvisioningClaims,
) -> Result<Option<Identity>> {
    let mut candidate = None;
    if let Some(anchor) = claims.external_id.as_deref() {
        candidate = directory::unique_user_by_external_id(&mut **tx, tenant.id, anchor).await?;
    }
    if candidate.is_none()
        && let Some(email) = claims.email.as_deref()
    {
        candidate = directory::unique_user_by_user_name(&mut **tx, tenant.id, email).await?;
    }
    let Some(row) = candidate else {
        return Ok(None);
    };
    let Some(identity_id) = row.identity_id else {
        // A mirror row the reconciler has not projected yet holds no
        // placement to adopt. Falling through to JIT would create the
        // second identity this whole rule exists to prevent, so the login
        // provisions nothing and the next reconciliation resolves it.
        return Ok(None);
    };
    let Some(identity) = identities::by_id(&mut **tx, tenant.id, identity_id).await? else {
        return Ok(None);
    };
    if identity.sealed() || identity.subject.is_some() {
        return Ok(None);
    }
    identities::bind_subject(tx, tenant.id, identity.id, subject)
        .await
        .map(Some)
}
