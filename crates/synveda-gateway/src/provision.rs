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
//! 4, narrowed by ADR-0102): the first member of the IdP's
//! `synveda-admins` group may mint the tenant's initial `administrator`
//! grant. A durable insert-only marker closes that provider-controlled door
//! forever; every later administrator is a governed Synveda grant.
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
use synveda_store::directory::UniqueUserMatch;
use synveda_store::{access, directory, identities, rls, scopes};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::scope::Scope;
use synveda_types::{
    DirectoryUser, Error, GrantId, Identity, IdentityId, IdentityKind, Result, Tenant, TenantId,
};

use crate::app::AppState;
use crate::audit;
use crate::telemetry::{JIT_ADMIN_BOOTSTRAPS_TOTAL, JIT_PROVISIONS_TOTAL};

const MAX_REHIRE_PRINCIPAL_GRANTS: usize = 128;
const REHIRE_PRINCIPAL_GRANT_PROBE_LIMIT: i64 = 129;

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

enum DirectoryAdoption {
    NoMatch,
    NewlyBound(Identity),
    AlreadyBound(Identity),
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
    // Directory correspondence is the outer lock domain. It must precede
    // every principal-grant fence taken below so SCIM projection and login
    // cannot deadlock, and it keeps mirror activity stable through commit.
    directory::lock_correspondence(&mut tx, tenant.id).await?;

    let existing = identities::by_subject(&mut *tx, tenant.id, subject).await?;
    let departed_subject = existing.as_ref().is_some_and(Identity::sealed);
    let admin_group = synveda_identity::contains_admin_group(&claims.groups);
    if let Some(identity) = existing.filter(|identity| !identity.sealed()) {
        validate_existing_directory_identity(&mut tx, tenant, &identity, claims).await?;
        let scope = scopes::get(&mut *tx, tenant.id, identity.scope_id)
            .await?
            .ok_or_else(|| Error::Internal {
                message: format!("identity {} lost its scope", identity.id),
            })?;
        // Repairs directory identities bound by an earlier binary that did
        // not move the structural owner grant from provider anchor to token
        // subject. JIT identities are already aligned, so this is a no-op.
        transfer_principal_scope_owner(&mut tx, tenant, subject, &scope).await?;
        if admin_group {
            // Owner repair may need two sorted principal fences. Establish the
            // one-time subject-only administrator grant afterwards so lock
            // order is directory -> sorted principals -> subject, never its
            // inverse.
            seed_initial_admin_grant(&mut tx, tenant, subject).await?;
        }
        let scope_path = scope_path(&mut tx, tenant.id, &scope).await?;
        // **This branch commits.** It looks read-only and is not: the admin
        // convention above may have just won and written this tenant's first
        // `administrator` grant and its `access.granted` event, and a
        // directory-created identity whose subject is already bound reaches
        // the door down exactly this path. Returning without committing
        // would drop that first grant silently — the operator door of
        // ADR-0074 decision 4, failing for the population (directory-synced
        // initial administrators) it exists for.
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
    let adoption = adopt_directory_identity(&mut tx, tenant, subject, claims).await?;
    let adopted = match adoption {
        DirectoryAdoption::NoMatch if departed_subject => {
            // A departed subject can move only to a directory-anchored
            // successor. Without this fence the generic JIT path below
            // would silently lift the retention seal with a fresh scope.
            return Err(Error::Unauthenticated {
                message: "this identity has been deprovisioned".to_owned(),
            });
        }
        DirectoryAdoption::NoMatch => None,
        DirectoryAdoption::NewlyBound(identity) => Some((identity, true)),
        DirectoryAdoption::AlreadyBound(_) if departed_subject => {
            // A concurrent login completed the successor bind after the
            // departed-subject read. Retry from a fresh transaction so its
            // new structural owner grant cannot be mistaken for old
            // authority and revoked.
            return Err(Error::Conflict {
                message: "directory successor was bound concurrently".to_owned(),
            });
        }
        DirectoryAdoption::AlreadyBound(identity) => Some((identity, false)),
    };
    if let Some((adopted, newly_bound)) = adopted {
        let scope = scopes::get(&mut *tx, tenant.id, adopted.scope_id)
            .await?
            .ok_or_else(|| Error::Internal {
                message: format!("identity {} lost its scope", adopted.id),
            })?;
        if departed_subject {
            reestablish_rehire_authority(&mut tx, tenant, subject, &scope).await?;
        } else {
            transfer_principal_scope_owner(&mut tx, tenant, subject, &scope).await?;
        }
        if admin_group {
            // For a rehire, former authority was removed above. Current
            // verified claims may establish the administrator door for the
            // directory-anchored successor only if no earlier path has ever
            // consumed tenant bootstrap. Rehire and revocation do not reopen
            // it. This remains after the sorted owner fences for deadlock-free
            // order.
            seed_initial_admin_grant(&mut tx, tenant, subject).await?;
        }
        let scope_path = scope_path(&mut tx, tenant.id, &scope).await?;
        if newly_bound {
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
        }
        tx.commit().await.map_err(|err| Error::Storage {
            message: format!("commit subject binding: {err}"),
        })?;
        if newly_bound {
            tracing::info!(
                identity.id = %adopted.id,
                scope.slug = %scope.slug,
                "first login bound its subject to a directory-created identity"
            );
        }
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
    if admin_group {
        seed_initial_admin_grant(&mut tx, tenant, subject).await?;
    }
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

async fn validate_existing_directory_identity(
    tx: &mut sqlx::PgConnection,
    tenant: &Tenant,
    identity: &Identity,
    claims: &ProvisioningClaims,
) -> Result<()> {
    let linked = directory::user_for_identity(&mut *tx, tenant.id, identity.id).await?;
    if linked.as_ref().is_some_and(|row| !row.active) {
        // The mirror write is authoritative before asynchronous
        // reconciliation seals the identity row. Refuse this window so a
        // departing user cannot log in or re-establish admin authority.
        return Err(Error::Unauthenticated {
            message: "this identity has been deprovisioned".to_owned(),
        });
    }
    let candidate = matching_directory_user(&mut *tx, tenant.id, claims).await?;
    let Some(candidate) = candidate else {
        // A true correspondence absence is the normal external-OIDC/JIT path.
        // A linked mirror may also be keyed by a provider claim this token did
        // not carry; its active link and verified token subject remain the
        // stable binding in that case.
        return Ok(());
    };
    match candidate.identity_id {
        None => Err(Error::Dependency {
            service: "directory-projection".to_owned(),
            message: "identity projection is incomplete".to_owned(),
        }),
        Some(identity_id) if identity_id != identity.id => Err(Error::Unauthenticated {
            message: "identity claims could not be matched".to_owned(),
        }),
        Some(_) if linked.as_ref().is_some_and(|row| row.id == candidate.id) => Ok(()),
        Some(_) => Err(Error::Internal {
            message: format!(
                "directory user {} links identity {} but the reverse correspondence is absent",
                candidate.id, identity.id
            ),
        }),
    }
}

async fn matching_directory_user(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    claims: &ProvisioningClaims,
) -> Result<Option<DirectoryUser>> {
    let classify = |matched| match matched {
        UniqueUserMatch::NoMatch => Ok(None),
        UniqueUserMatch::Unique(row) => Ok(Some(*row)),
        UniqueUserMatch::Ambiguous => Err(Error::Unauthenticated {
            message: "identity claims could not be matched".to_owned(),
        }),
        UniqueUserMatch::Inactive => Err(Error::Unauthenticated {
            message: "this identity has been deprovisioned".to_owned(),
        }),
    };
    if let Some(anchor) = claims.external_id.as_deref()
        && let Some(row) =
            classify(directory::unique_user_by_external_id(&mut *tx, tenant_id, anchor).await?)?
    {
        return Ok(Some(row));
    }
    if let Some(email) = claims.email.as_deref() {
        return classify(directory::unique_user_by_user_name(&mut *tx, tenant_id, email).await?);
    }
    Ok(None)
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

/// Claims the one-time IdP administrator door and creates its governed noun.
///
/// The insert-only marker is the authority for whether bootstrap has ever
/// been consumed. A trigger records an earlier tenant-root administrator from
/// any other grant path, and revoking the first grant does not reopen this
/// provider-controlled path.
async fn seed_initial_admin_grant(
    tx: &mut sqlx::PgConnection,
    tenant: &Tenant,
    subject: &str,
) -> Result<()> {
    let root = scopes::ensure_tenant_root(&mut *tx, tenant.id).await?;
    let grant_id = synveda_types::GrantId::new();
    if access::claim_initial_administrator_bootstrap(&mut *tx, tenant.id, grant_id, subject).await?
    {
        let grant = access::create_grant(
            &mut *tx,
            &access::NewGrant {
                id: grant_id,
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
        metrics::counter!(JIT_ADMIN_BOOTSTRAPS_TOTAL, "outcome" => "claimed").increment(1);
        tracing::info!(tenant.id = %tenant.id, "admin-group login claimed initial administrator bootstrap");
    } else {
        metrics::counter!(JIT_ADMIN_BOOTSTRAPS_TOTAL, "outcome" => "closed").increment(1);
        tracing::info!(tenant.id = %tenant.id, "admin-group login observed closed administrator bootstrap");
    }
    Ok(())
}

/// Removes every direct authority row held by a departed token subject before
/// a directory-anchored successor may use that subject. Group authority is
/// identity-keyed and therefore already stays with the former identity; this
/// closes the principal-grant half of the rehire boundary.
async fn revoke_departed_subject_grants(
    tx: &mut sqlx::PgConnection,
    tenant: &Tenant,
    subject: &str,
    preserved_structural_owner: Option<GrantId>,
) -> Result<()> {
    // The caller already holds this fence together with the successor
    // anchor's fence. Re-acquiring it is safe and makes this helper robust if
    // its call shape changes: no grant insert may land between the bounded
    // snapshot and the last delete.
    access::lock_principal_grants(&mut *tx, tenant.id, subject).await?;
    let grants = access::principal_grants_bounded(
        &mut *tx,
        tenant.id,
        subject,
        preserved_structural_owner,
        REHIRE_PRINCIPAL_GRANT_PROBE_LIMIT,
    )
    .await?;
    ensure_rehire_grant_bound(grants.len())?;
    let mut revoked_count = 0usize;
    for grant in grants {
        let revoked = if grant.source.is_directory_managed() {
            access::revoke_directory_grant(&mut *tx, tenant.id, grant.id).await?
        } else {
            access::revoke_grant(&mut *tx, tenant.id, grant.id).await?
        };
        audit::record_as(
            &mut *tx,
            tenant.id,
            Actor::subject(subject),
            AuditAction::AccessRevoked,
            format!("scope {}", revoked.scope_id),
            Outcome::Success,
            json!({
                "origin": "directory-rehire",
                "grant": {
                    "id": revoked.id,
                    "scope_id": revoked.scope_id,
                    "role": revoked.role_key,
                    "source": revoked.source,
                    "subject": subject,
                },
            }),
        )
        .await?;
        revoked_count += 1;
    }
    tracing::info!(
        tenant.id = %tenant.id,
        authority.revoked = revoked_count,
        "former direct authority removed before directory rehire"
    );
    Ok(())
}

/// Re-establishes a returning subject's authority from the directory
/// successor and nothing else.
///
/// Both possible structural-owner principals are fenced before either the
/// owner or the departed subject's grant set is read. That gives the sequence
/// one serialisation point against generic grant creation and owner repair.
/// When the successor scope was already minted directly for `subject`, its
/// verified structural owner is the sole preserved row; every other old
/// subject-keyed grant is retired.
async fn reestablish_rehire_authority(
    tx: &mut sqlx::PgConnection,
    tenant: &Tenant,
    subject: &str,
    scope: &Scope,
) -> Result<()> {
    let anchor = principal_scope_anchor(scope)?;
    lock_owner_principals(&mut *tx, tenant.id, anchor, subject).await?;
    let owner = structural_owner(&mut *tx, tenant.id, scope).await?;
    let current_principal = structural_owner_principal(&owner, scope)?;
    if current_principal != anchor && current_principal != subject {
        return Err(Error::Internal {
            message: format!("principal scope {} owner differs from its anchor", scope.id),
        });
    }
    let preserved = (current_principal == subject).then_some(owner.id);
    revoke_departed_subject_grants(&mut *tx, tenant, subject, preserved).await?;
    if current_principal == anchor && anchor != subject {
        transfer_locked_structural_owner(&mut *tx, tenant, subject, scope, owner).await?;
    }
    Ok(())
}

fn ensure_rehire_grant_bound(grant_count: usize) -> Result<()> {
    if grant_count > MAX_REHIRE_PRINCIPAL_GRANTS {
        return Err(Error::Dependency {
            service: "directory-projection".to_owned(),
            message: "former authority exceeds the bounded rehire retirement limit".to_owned(),
        });
    }
    Ok(())
}

/// Moves the one structural `owner` grant minted with a pre-login principal
/// scope from its directory anchor to the verified token subject. The scope's
/// immutable structural anchor remains unchanged; only the authority-bearing
/// grant changes identity vocabulary.
async fn transfer_principal_scope_owner(
    tx: &mut sqlx::PgConnection,
    tenant: &Tenant,
    subject: &str,
    scope: &Scope,
) -> Result<()> {
    let anchor = principal_scope_anchor(scope)?;
    lock_owner_principals(&mut *tx, tenant.id, anchor, subject).await?;
    let owner = structural_owner(&mut *tx, tenant.id, scope).await?;
    let current_principal = structural_owner_principal(&owner, scope)?;
    if current_principal == subject {
        return Ok(());
    }
    if current_principal != anchor {
        return Err(Error::Internal {
            message: format!("principal scope {} owner differs from its anchor", scope.id),
        });
    }

    transfer_locked_structural_owner(&mut *tx, tenant, subject, scope, owner).await
}

fn principal_scope_anchor(scope: &Scope) -> Result<&str> {
    scope
        .principal_id
        .as_deref()
        .ok_or_else(|| Error::Internal {
            message: format!("adopted scope {} has no principal anchor", scope.id),
        })
}

async fn lock_owner_principals(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    anchor: &str,
    subject: &str,
) -> Result<()> {
    let mut principals = [anchor, subject];
    principals.sort_unstable();
    access::lock_principal_grants(&mut *tx, tenant_id, principals[0]).await?;
    if principals[1] != principals[0] {
        access::lock_principal_grants(&mut *tx, tenant_id, principals[1]).await?;
    }
    Ok(())
}

async fn structural_owner(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    scope: &Scope,
) -> Result<synveda_types::access::ScopeGrant> {
    let mut structural = access::structural_owner_grants(&mut *tx, tenant_id, scope.id)
        .await?
        .into_iter();
    let owner = structural.next().ok_or_else(|| Error::Internal {
        message: format!("principal scope {} has no structural owner grant", scope.id),
    })?;
    if structural.next().is_some() {
        return Err(Error::Internal {
            message: format!(
                "principal scope {} has multiple structural owner grants",
                scope.id
            ),
        });
    }
    Ok(owner)
}

fn structural_owner_principal<'a>(
    owner: &'a synveda_types::access::ScopeGrant,
    scope: &Scope,
) -> Result<&'a str> {
    owner
        .principal_id
        .as_deref()
        .ok_or_else(|| Error::Internal {
            message: format!(
                "principal scope {} has a non-principal owner grant",
                scope.id
            ),
        })
}

async fn transfer_locked_structural_owner(
    tx: &mut sqlx::PgConnection,
    tenant: &Tenant,
    subject: &str,
    scope: &Scope,
    owner: synveda_types::access::ScopeGrant,
) -> Result<()> {
    normalize_subject_owner_grant(&mut *tx, tenant, subject, scope, owner.id).await?;
    let revoked = access::revoke_grant(&mut *tx, tenant.id, owner.id).await?;
    audit::record_as(
        &mut *tx,
        tenant.id,
        Actor::subject(subject),
        AuditAction::AccessRevoked,
        format!("scope {}", scope.id),
        Outcome::Success,
        json!({
            "origin": "directory-login-owner-transfer",
            "grant": {"id": revoked.id, "scope_id": scope.id, "role": RoleKey::Owner},
        }),
    )
    .await?;
    let granted = access::create_grant(
        &mut *tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id: tenant.id,
            scope_id: scope.id,
            subject: GrantSubject::Principal {
                principal_id: subject.to_owned(),
            },
            role_key: RoleKey::Owner,
            source: GrantSource::Owner,
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
        format!("scope {}", scope.id),
        Outcome::Success,
        json!({
            "origin": "directory-login-owner-transfer",
            "grant": {"id": granted.id, "scope_id": scope.id, "role": RoleKey::Owner,
                      "subject": subject},
        }),
    )
    .await?;
    tracing::info!(
        tenant.id = %tenant.id,
        scope.id = %scope.id,
        "directory principal-scope ownership transferred to verified subject"
    );
    Ok(())
}

/// Removes a non-structural owner grant that would collide with the canonical
/// owner row during directory-anchor cutover.
///
/// The caller holds both the anchor and subject grant fences. PostgreSQL's
/// grant uniqueness deliberately ignores `source`, so an administrator may
/// already have given the verified subject `owner` directly (or through an
/// accepted invitation) while the pre-login anchor still holds the structural
/// owner. Deleting that redundant row before the structural replacement keeps
/// the cutover retryable and leaves exactly one authority-bearing owner row.
async fn normalize_subject_owner_grant(
    tx: &mut sqlx::PgConnection,
    tenant: &Tenant,
    subject: &str,
    scope: &Scope,
    structural_owner_id: GrantId,
) -> Result<()> {
    let existing = access::list_grants(
        &mut *tx,
        tenant.id,
        &access::GrantFilter {
            scope_id: Some(scope.id),
            principal_id: Some(subject.to_owned()),
        },
    )
    .await?
    .into_iter()
    .find(|grant| grant.role_key == RoleKey::Owner && grant.id != structural_owner_id);
    let Some(existing) = existing else {
        return Ok(());
    };
    if existing.source == GrantSource::Owner {
        return Err(Error::Internal {
            message: format!(
                "principal scope {} has multiple structural owner grants",
                scope.id
            ),
        });
    }

    let revoked = access::revoke_grant(&mut *tx, tenant.id, existing.id).await?;
    audit::record_as(
        &mut *tx,
        tenant.id,
        Actor::subject(subject),
        AuditAction::AccessRevoked,
        format!("scope {}", scope.id),
        Outcome::Success,
        json!({
            "origin": "directory-login-owner-normalization",
            "grant": {"id": revoked.id, "scope_id": scope.id, "role": RoleKey::Owner},
        }),
    )
    .await?;
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
) -> Result<DirectoryAdoption> {
    let candidate = matching_directory_user(tx, tenant.id, claims).await?;
    let Some(row) = candidate else {
        return Ok(DirectoryAdoption::NoMatch);
    };
    let Some(identity_id) = row.identity_id else {
        // A mirror row the reconciler has not projected yet holds no
        // placement to adopt. Falling through to JIT would create the
        // second identity this whole rule exists to prevent, so the login
        // provisions nothing and the next reconciliation resolves it.
        tracing::info!(
            directory.user = %row.id,
            "login is waiting for directory identity projection"
        );
        return Err(Error::Dependency {
            service: "directory-projection".to_owned(),
            message: "identity projection is incomplete".to_owned(),
        });
    };
    let Some(identity) = identities::by_id(&mut **tx, tenant.id, identity_id).await? else {
        return Err(Error::Internal {
            message: format!(
                "directory user {} links missing identity {identity_id}",
                row.id
            ),
        });
    };
    if identity.kind != IdentityKind::User {
        return Err(Error::Internal {
            message: format!(
                "directory user {} links non-user identity {identity_id}",
                row.id
            ),
        });
    }
    if identity.sealed() {
        tracing::info!(
            directory.user = %row.id,
            identity.id = %identity.id,
            "login is waiting for directory rehire projection"
        );
        return Err(Error::Dependency {
            service: "directory-projection".to_owned(),
            message: "identity projection is incomplete".to_owned(),
        });
    }
    match identity.subject.as_deref() {
        None => identities::bind_subject(tx, tenant.id, identity.id, subject)
            .await
            .map(DirectoryAdoption::NewlyBound),
        Some(bound) if bound == subject => Ok(DirectoryAdoption::AlreadyBound(identity)),
        Some(_) => Err(Error::Unauthenticated {
            message: "identity claims could not be matched".to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rehire_authority_retirement_has_a_closed_work_bound() {
        assert!(ensure_rehire_grant_bound(MAX_REHIRE_PRINCIPAL_GRANTS).is_ok());
        assert!(ensure_rehire_grant_bound(MAX_REHIRE_PRINCIPAL_GRANTS + 1).is_err());
        assert_eq!(
            REHIRE_PRINCIPAL_GRANT_PROBE_LIMIT,
            i64::try_from(MAX_REHIRE_PRINCIPAL_GRANTS + 1).expect("constant fits i64")
        );
    }
}
