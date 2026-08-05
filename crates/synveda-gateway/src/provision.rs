//! JIT user provisioning (AUTH-2, ADR-0013): at login completion, place a
//! first-time subject in the tenancy hierarchy from its IdP groups.
//!
//! Resolution order (ADR-0013 decision 3): the `group_mappings` override
//! table first, then the `synveda-{dept}-{team}` convention — groups in
//! lexicographic order, first resolution wins; a convention group whose
//! candidate splits match zero or several teams resolves nothing. Nothing
//! resolves → the user lands under the reserved `quarantine` scope, which
//! the PDP's base layer forbids everything to (ADR-0013 decisions 4–5;
//! the forbid moved from `bootstrap@2` into every pack's compiled base,
//! ADR-0014 decision 2).
//!
//! This is a system write path driven by verified IdP claims — the same
//! trust class as tenant admission and the future SCIM sync (AUTH-4) — so
//! no PDP check guards the placement itself; enforcement happens on every
//! subsequent action through the identity's quarantine status. It is NOT
//! a path to governed assets (seed §2.2).
//!
//! Audited since AUD-1 (ADR-0019): a created identity chains
//! `identity.provisioned` in the provisioning transaction, and the
//! admin-group binding chains `role.bound` when it is first established —
//! not on every login's no-op upsert (decision 6). The actor is the
//! provisioned subject itself: this plane runs at login completion,
//! outside the task-local tenant scope.

use serde_json::json;
use synveda_audit::{Actor, AuditAction, Outcome};
use synveda_identity::{
    ProvisioningClaims, contains_admin_group, convention_candidates, personal_slug,
};
use synveda_store::{directory, group_mappings, hierarchy, identities, rls, role_bindings};
use synveda_types::{
    Error, HierarchyNode, Identity, IdentityId, IdentityKind, Result, Role, ScopeId, ScopeKind,
    Tenant,
};

use crate::app::AppState;
use crate::audit;
use crate::telemetry::JIT_PROVISIONS_TOTAL;

/// A provisioned login: the identity and the scope its personal node sits
/// under (the mapped scope, or the quarantine scope).
pub struct Provisioned {
    /// The subject's identity — existing on repeat logins, fresh on first.
    pub identity: Identity,
    /// The identity's personal scope node.
    pub scope: HierarchyNode,
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
                // Another login for this subject (or the quarantine scope's
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

    // The admin convention group (AUTHZ-3, ADR-0015 decision 6): upserted
    // at *every* login completion — adding someone to `synveda-admins`
    // works on their next login. Additive only: leaving the group revokes
    // nothing until AUTH-4/5 bring mover/leaver sync; unbinding stays an
    // explicit, PDP-gated action. Only the binding's first establishment
    // chains an audit event (ADR-0019 decision 6) — repeat logins are
    // no-op upserts; a concurrent first-login race can double-record,
    // which over-records rather than under-records.
    let admin = contains_admin_group(&claims.groups);
    if admin {
        let established = !role_bindings::for_subject_on_scopes(&mut *tx, tenant.id, subject, &[])
            .await?
            .iter()
            .any(|binding| binding.scope_id.is_none() && binding.role == Role::OrgAdmin);
        role_bindings::bind(&mut *tx, tenant.id, subject, None, Role::OrgAdmin).await?;
        if established {
            audit::record_as(
                &mut tx,
                tenant.id,
                Actor::subject(subject),
                AuditAction::RoleBound,
                format!("tenant {}", tenant.id),
                Outcome::Success,
                json!({
                    "origin": "jit-admin-group",
                    "binding": {"subject": subject, "role": Role::OrgAdmin, "scope_id": null},
                }),
            )
            .await?;
        }
        tracing::info!(tenant.id = %tenant.id, "admin-group login: tenant-wide org-admin bound");
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
        let scope = hierarchy::node(&mut *tx, identity.scope_id)
            .await?
            .ok_or_else(|| Error::Internal {
                message: format!("identity {} lost its scope node", identity.id),
            })?;
        if admin {
            tx.commit().await.map_err(|err| Error::Storage {
                message: format!("commit admin binding: {err}"),
            })?;
        }
        return Ok((Provisioned { identity, scope }, "existing"));
    }

    // The other half of the correspondence rule (AUTH-4, ADR-0059
    // decision 4): a directory may have created this person before they
    // ever logged in, and binding the subject to *that* identity is what
    // stops one person from having two — each with its own personal scope
    // and half their memory in it.
    if let Some(adopted) = adopt_directory_identity(&mut tx, tenant, subject, claims).await? {
        let scope = hierarchy::node(&mut *tx, adopted.scope_id)
            .await?
            .ok_or_else(|| Error::Internal {
                message: format!("identity {} lost its scope node", adopted.id),
            })?;
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
                "parent": {"slug": scope.slug, "path": scope.path},
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
            scope.path = %scope.path,
            "first login bound its subject to a directory-created identity"
        );
        return Ok((
            Provisioned {
                identity: adopted,
                scope,
            },
            "bound",
        ));
    }

    let (parent, label) = match resolve_mapping(&mut tx, tenant.id, &claims.groups).await? {
        Some(scope) => (scope, "mapped"),
        // An admin-group subject with no team mapping is placed under the
        // org root, never quarantined — quarantine's base forbid would
        // nullify the very binding that makes the tenant governable
        // (ADR-0015 decision 6).
        None if admin => (ensure_root(&mut tx, tenant).await?, "admin"),
        None => (ensure_quarantine(&mut tx, tenant).await?, "quarantined"),
    };

    let identity_id = IdentityId::new();
    let display_name = claims.display_name.as_deref().unwrap_or(subject);
    let scope = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant.id,
        Some(parent.id),
        ScopeKind::User,
        &personal_slug(claims.email.as_deref(), subject, identity_id),
        display_name,
    )
    .await?;
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
            "placement": label,
            "identity": {"id": identity.id, "subject": identity.subject},
            "parent": {"slug": parent.slug, "path": parent.path},
            "groups": claims.groups,
        }),
    )
    .await?;
    tx.commit().await.map_err(|err| Error::Storage {
        message: format!("commit provisioning transaction: {err}"),
    })?;
    // Provisioning committed hierarchy nodes (the personal scope, maybe
    // the root/quarantine): flush the tenant's chains and entity
    // fragments (ADR-0016 decision 5, ADR-0017 decision 5). The
    // existing-identity path above mutates no hierarchy and does not
    // invalidate.
    state.invalidate_hierarchy(tenant.id);
    tracing::info!(
        identity.id = %identity.id,
        scope.path = %scope.path,
        quarantined = identity.quarantined,
        "identity provisioned"
    );
    Ok((Provisioned { identity, scope }, label))
}

/// Resolves the groups to a placement scope, or `None` for quarantine:
/// overrides before convention, groups in lexicographic order, first
/// resolution wins (ADR-0013 decision 3).
///
/// Shared with the SCIM reconciler (AUTH-4, ADR-0059 decision 6), which is
/// the whole of what "joining is AUTH-2's resolver, called from somewhere
/// else" means: the two doors differ in where the group names come from —
/// a token claim or the directory mirror — and in nothing else.
pub(crate) async fn resolve_mapping(
    tx: &mut sqlx::PgConnection,
    tenant_id: synveda_types::TenantId,
    groups: &[String],
) -> Result<Option<HierarchyNode>> {
    if groups.is_empty() {
        return Ok(None);
    }
    let mut sorted: Vec<String> = groups.to_vec();
    sorted.sort();
    sorted.dedup();

    // Overrides: `for_groups` returns group-name order, so the first row
    // is the deterministic winner.
    for mapping in group_mappings::for_groups(&mut *tx, tenant_id, &sorted).await? {
        match hierarchy::node(&mut *tx, mapping.scope_id).await? {
            // The rank rule forbids nodes under a user; a mapping pointed
            // at one cannot place anybody — skip it rather than fail login.
            Some(scope) if scope.kind != ScopeKind::User => return Ok(Some(scope)),
            _ => {
                tracing::warn!(
                    mapping.group = mapping.group_name,
                    scope.id = %mapping.scope_id,
                    "group mapping targets a user scope; ignoring it"
                );
            }
        }
    }

    // Convention: one hierarchy-validated query per convention-shaped
    // group; exactly one matching team is a mapping, anything else is
    // unresolved and the next group is tried (ADR-0013 decision 3).
    for group in &sorted {
        let candidates = convention_candidates(group);
        if candidates.is_empty() {
            continue;
        }
        let (departments, teams): (Vec<String>, Vec<String>) = candidates
            .into_iter()
            .map(|candidate| (candidate.department, candidate.team))
            .unzip();
        let mut matches =
            hierarchy::teams_matching(&mut *tx, tenant_id, &departments, &teams).await?;
        match matches.len() {
            1 => return Ok(matches.pop()),
            0 => {}
            several => {
                tracing::warn!(
                    mapping.group = group.as_str(),
                    matches = several,
                    "convention group is ambiguous in this hierarchy; skipping it"
                );
            }
        }
    }
    Ok(None)
}

/// The tenant's org root — created from the tenant's own slug and name on
/// first use (seed §2.1 zero-config: a fresh tenant needs no admin before
/// first login).
async fn ensure_root(tx: &mut sqlx::PgConnection, tenant: &Tenant) -> Result<HierarchyNode> {
    match hierarchy::root(&mut *tx, tenant.id).await? {
        Some(root) => Ok(root),
        None => {
            hierarchy::create(
                &mut *tx,
                ScopeId::new(),
                tenant.id,
                None,
                ScopeKind::Org,
                &tenant.slug,
                &tenant.name,
            )
            .await
        }
    }
}

/// The tenant's quarantine scope — the org root's child with the reserved
/// slug — creating the root and the quarantine node on first use.
pub(crate) async fn ensure_quarantine(
    tx: &mut sqlx::PgConnection,
    tenant: &Tenant,
) -> Result<HierarchyNode> {
    let root = ensure_root(&mut *tx, tenant).await?;
    match hierarchy::child_by_slug(&mut *tx, root.id, identities::QUARANTINE_SLUG).await? {
        Some(quarantine) => Ok(quarantine),
        None => {
            hierarchy::create(
                &mut *tx,
                ScopeId::new(),
                tenant.id,
                Some(root.id),
                ScopeKind::Team,
                identities::QUARANTINE_SLUG,
                "Quarantine",
            )
            .await
        }
    }
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
        candidate = directory::user_by_external_id(&mut **tx, tenant.id, anchor).await?;
    }
    if candidate.is_none()
        && let Some(email) = claims.email.as_deref()
    {
        candidate = directory::user_by_user_name(&mut **tx, tenant.id, email).await?;
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
