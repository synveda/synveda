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
//! AUD-1 wiring point: `identity.provisioned` becomes an audit event when
//! the hash-chained log lands; until then provisioning is visible in the
//! `identity.provision` span and `synveda_jit_provisions_total`.

use synveda_identity::{ProvisioningClaims, contains_admin_group, convention_candidates};
use synveda_store::{group_mappings, hierarchy, identities, rls, role_bindings};
use synveda_types::{
    Error, HierarchyNode, Identity, IdentityId, Result, Role, ScopeId, ScopeKind, Tenant,
};

use crate::app::AppState;
use crate::telemetry::JIT_PROVISIONS_TOTAL;

/// A provisioned login: the identity and the scope its personal node sits
/// under (the mapped scope, or the quarantine scope).
pub(crate) struct Provisioned {
    /// The subject's identity — existing on repeat logins, fresh on first.
    pub identity: Identity,
    /// The identity's personal scope node.
    pub scope: HierarchyNode,
}

/// Provisions `subject` into `tenant`'s hierarchy if this is its first
/// login, and returns the identity either way. Runs in one tenant
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
pub(crate) async fn provision(
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
    // explicit, PDP-gated action. An AUD-1 emission point.
    let admin = contains_admin_group(&claims.groups);
    if admin {
        role_bindings::bind(&mut *tx, tenant.id, subject, None, Role::OrgAdmin).await?;
        tracing::info!(tenant.id = %tenant.id, "admin-group login: tenant-wide org-admin bound");
    }

    if let Some(identity) = identities::by_subject(&mut *tx, tenant.id, subject).await? {
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
        subject,
        claims.email.as_deref(),
        claims.display_name.as_deref(),
        scope.id,
    )
    .await?;
    tx.commit().await.map_err(|err| Error::Storage {
        message: format!("commit provisioning transaction: {err}"),
    })?;
    // Provisioning committed hierarchy nodes (the personal scope, maybe
    // the root/quarantine): bump the tenant's scope-chain generation
    // (ADR-0016 decision 5). The existing-identity path above mutates no
    // hierarchy and does not invalidate.
    state.scope_chains.invalidate(tenant.id);
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
async fn resolve_mapping(
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
async fn ensure_quarantine(tx: &mut sqlx::PgConnection, tenant: &Tenant) -> Result<HierarchyNode> {
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

/// A slug for the personal scope node: a readable base (email local part,
/// else the subject) sanitised into the slug grammar, plus an identity-id
/// suffix so siblings never collide. Paths are display-only (ADR-0011).
fn personal_slug(email: Option<&str>, subject: &str, id: IdentityId) -> String {
    let base = email
        .and_then(|address| address.split('@').next())
        .filter(|local| !local.is_empty())
        .unwrap_or(subject);
    let mut readable: String = base
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    readable.truncate(40);
    let readable = readable.trim_matches('-');
    let suffix = &id.as_uuid().simple().to_string()[..8];
    if readable.is_empty() {
        format!("u-{suffix}")
    } else {
        format!("{readable}-{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn personal_slugs_fit_the_grammar() {
        let id = IdentityId::new();
        let suffix = &id.as_uuid().simple().to_string()[..8];
        let cases = [
            (
                Some("alice@example.test"),
                "sub-1",
                format!("alice-{suffix}"),
            ),
            (None, "Alice Q. User", format!("alice-q--user-{suffix}")),
            (Some("@nolocal"), "--", format!("u-{suffix}")),
            (None, "ûñïçøðé", format!("u-{suffix}")),
        ];
        for (email, subject, want) in cases {
            let slug = personal_slug(email, subject, id);
            assert_eq!(slug, want);
            assert!(
                slug.len() <= 63
                    && slug.chars().next().unwrap().is_ascii_alphanumeric()
                    && slug
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "slug {slug:?} breaks the grammar"
            );
        }
    }
}
