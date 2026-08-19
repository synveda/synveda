//! The projection (AUTH-4, ADR-0059 decision 3): from what the directory
//! said onto what the product does about it.
//!
//! One function, and it is the only writer of this seam anywhere. AUTH-5's
//! scheduled pull sync writes the same mirror rows from a directory read
//! and calls [`reconcile`] — which is the whole reason that feature is an M
//! rather than a second implementation of joiner/mover/leaver.
//!
//! ## What it does, in the order it decides
//!
//! 1. **Find the person.** The correspondence rule (decision 4): the mirror
//!    row's own link, then a subject equal to the directory's anchor, then a
//!    case-folded email equal to `userName`. A SCIM create for somebody who
//!    already logged in **adopts** their JIT identity; it never makes a
//!    second one.
//! 2. **`active: false` seals** (decision 8) and stops. A seal is not a
//!    placement decision and never runs one.
//! 3. **No identity yet → join**: AUTH-2's mapping resolver, unchanged
//!    (ADR-0013 decision 3), against the group names the mirror holds
//!    instead of a token's claim. The identity is created with **no
//!    subject**; first login binds it (decision 5).
//! 4. **Placement changed → move** (decision 10). If the source and
//!    destination resolve the same effective pack the material follows and
//!    nothing is asked. If they differ, the **source** scope's pack decides
//!    — authority over material belongs where the material is — and a
//!    sealing pack leaves a former self behind on the old scope.
//!
//! Losing every group mapping is **quarantine, not departure** (decision
//! 11). Only `active: false` and `DELETE` seal, and that difference is the
//! difference between a misconfigured group and a person losing their
//! memory: one is reversible by fixing the mapping and the other is not.

use serde_json::json;
use synveda_audit::{Actor, AuditAction, Outcome};
use synveda_policy::{AuthzContext, Resource, ScopeNode};
use synveda_store::{directory, hierarchy, identities, policy_assignments, rls};
use synveda_types::{
    DirectoryUser, HierarchyNode, Identity, IdentityId, IdentityKind, Result, ScopeId, ScopeKind,
    Tenant,
};

use crate::app::AppState;
use crate::audit;
use crate::provision;
use crate::telemetry::SCIM_RECONCILES_TOTAL;

/// Which door a directory fact arrived through.
///
/// AUTH-4 threaded a `ScimCredentialId` here because there was one plane and
/// it always had one. AUTH-5's pull sync has no credential — it holds an
/// outbound one, which is a different thing and must never appear in an audit
/// payload (ADR-0060 decision 7) — so the parameter becomes the *source*.
///
/// This is ADR-0060 decision 2 made concrete: the two planes produce the same
/// lifecycle through the same reconciler, and the only thing that
/// distinguishes them on the chain is which one it was. A chain consumer that
/// asked "who deprovisioned this person" still gets an answer from either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectorySource {
    /// A provisioning agent pushed it, authenticated by this credential.
    Scim(synveda_types::ScimCredentialId),
    /// A scheduled pass read it from the directory itself.
    Pull {
        /// The connector that read it — `entra`, `okta`.
        connector: &'static str,
    },
}

impl DirectorySource {
    /// The audit payload fragment naming this source.
    ///
    /// `credential_id` keeps the key and the shape AUTH-4's consumers already
    /// read, and is `null` for a pull. Dropping the key on one plane would
    /// make a chain query that filters on it silently miss half the events.
    fn payload(self) -> serde_json::Value {
        match self {
            DirectorySource::Scim(id) => json!({"source": "scim", "credential_id": id}),
            DirectorySource::Pull { connector } => {
                json!({"source": "pull", "credential_id": null, "connector": connector})
            }
        }
    }
}

/// Folds a source's identifying fields into an event payload.
///
/// A function rather than a repeated literal so the two planes cannot drift
/// into describing themselves differently — which is the whole content of
/// "the chain cannot tell the two doors apart" (ADR-0060 decision 2).
fn with_source(mut payload: serde_json::Value, source: DirectorySource) -> serde_json::Value {
    let fragment = source.payload();
    if let (Some(payload), Some(fragment)) = (payload.as_object_mut(), fragment.as_object()) {
        for (key, value) in fragment {
            payload.insert(key.clone(), value.clone());
        }
    }
    payload
}

/// What a reconciliation did — the metric label, and what the caller logs.
///
/// Named for the result rather than for the verb so it does not read as a
/// second [`synveda_audit::Outcome`], which every audited path here also
/// carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reconciled {
    /// A first placement: a new identity and a new personal scope.
    Provisioned,
    /// An existing identity was linked to this mirror row rather than
    /// duplicated — the correspondence rule doing its job.
    Adopted,
    /// The person's placement changed and their scope moved with them.
    Moved,
    /// The person's placement changed across a policy boundary and the
    /// source pack said their material stays where it was written.
    MovedAndSealed,
    /// `active: false`: the personal scope is sealed.
    Sealed,
    /// Nothing the directory sent changes anything here.
    Unchanged,
}

impl Reconciled {
    const fn label(self) -> &'static str {
        match self {
            Reconciled::Provisioned => "provisioned",
            Reconciled::Adopted => "adopted",
            Reconciled::Moved => "moved",
            Reconciled::MovedAndSealed => "moved_and_sealed",
            Reconciled::Sealed => "sealed",
            Reconciled::Unchanged => "unchanged",
        }
    }
}

/// Projects one mirror row onto the product's own state.
///
/// Runs in one tenant transaction and flushes the hierarchy caches after
/// committing anything structural — through [`AppState::invalidate_hierarchy`]
/// and never the two caches individually, which is the seam ADR-0016
/// decision 5 and ADR-0017 decision 5 named this feature in.
#[tracing::instrument(
    name = "scim.reconcile",
    skip_all,
    fields(
        tenant.id = %tenant.id,
        directory.user = %user.id,
        scim.outcome = tracing::field::Empty,
    ),
    err(Display)
)]
pub async fn reconcile(
    state: &AppState,
    tenant: &Tenant,
    source: DirectorySource,
    user: &DirectoryUser,
) -> Result<Reconciled> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant.id).await?;
    let existing = find_identity(&mut tx, tenant, user).await?;

    let (outcome, structural) = if !user.active {
        (seal(&mut tx, tenant, source, existing).await?, true)
    } else {
        let groups = directory::group_names_for_user(&mut *tx, tenant.id, user.id).await?;
        // A departed identity is never resurrected (ADR-0059 decision 12).
        // `active: true` on somebody the directory previously deactivated is
        // a **rehire**, and a rehire is a new person: a new identity, a new
        // personal scope, and the sealed one left exactly as it was.
        //
        // Reached through the mirror row's own link, which is the one path
        // that can hand back a sealed identity — the other two matches
        // filter departed rows out. Without this the reconciler would carry
        // on and try to move somebody who has left, and a seal that a
        // reactivation could undo is not a retention hold.
        let existing = existing.filter(|identity| !identity.sealed());
        match existing {
            None => (
                place(state, &mut tx, tenant, source, user, &groups).await?,
                true,
            ),
            Some(identity) => {
                // The link is written even when nothing else changes: an
                // adopted JIT identity has to stop being findable only by
                // the fallback that found it once.
                if user.identity_id != Some(identity.id) {
                    directory::link_identity(&mut *tx, tenant.id, user.id, identity.id).await?;
                }
                let moved =
                    apply_placement(state, &mut tx, tenant, source, &identity, &groups).await?;
                match (moved, user.identity_id) {
                    (Some(outcome), _) => (outcome, true),
                    (None, None) => (Reconciled::Adopted, false),
                    (None, Some(_)) => (Reconciled::Unchanged, false),
                }
            }
        }
    };

    tx.commit()
        .await
        .map_err(|err| synveda_types::Error::Storage {
            message: format!("commit reconciliation: {err}"),
        })?;
    if structural {
        state.invalidate_hierarchy(tenant.id);
    }
    tracing::Span::current().record("scim.outcome", outcome.label());
    metrics::counter!(SCIM_RECONCILES_TOTAL, "outcome" => outcome.label()).increment(1);
    Ok(outcome)
}

/// The correspondence rule (ADR-0059 decision 4), in its one place.
///
/// Ordered, and the order is the point: the link is authoritative, the
/// directory's anchor is next because a token subject equal to it is the
/// case where `external_id_claim` is simply `sub`, and the case-folded
/// email is last because it is the weakest of the three and the only one a
/// customer's attribute mapping can make wrong.
///
/// A **departed** identity is never adopted: the seal does not lift, and a
/// rehire is a new person (decision 12).
///
/// The last match tries the **work address before the `userName`**, and
/// that ordering is a correction the acceptance demo produced. ADR-0059
/// decision 4 words this match as "`identities.email` = the mirror row's
/// `userName`", which assumes the two are the same string — usually true,
/// because a `userName` is normally a UPN. When they differ, the address is
/// the better key: an identity's `email` was taken from `work_email` at
/// placement, so comparing against `work_email` compares the same fact with
/// itself. The case that exposed it is a directory record **re-created**
/// with a new anchor and a new `userName` for somebody whose mailbox never
/// changed — and matching only the `userName` gave that person a second
/// identity and a second personal scope, silently.
async fn find_identity(
    tx: &mut sqlx::PgConnection,
    tenant: &Tenant,
    user: &DirectoryUser,
) -> Result<Option<Identity>> {
    if let Some(identity_id) = user.identity_id
        && let Some(identity) = identities::by_id(&mut *tx, tenant.id, identity_id).await?
    {
        return Ok(Some(identity));
    }
    if let Some(external_id) = &user.external_id
        && let Some(identity) = identities::by_subject(&mut *tx, tenant.id, external_id).await?
        && !identity.sealed()
    {
        return Ok(Some(identity));
    }
    for address in [user.work_email.as_deref(), Some(user.user_name.as_str())]
        .into_iter()
        .flatten()
    {
        if let Some(identity) = identities::by_email(&mut *tx, tenant.id, address).await?
            && !identity.sealed()
        {
            return Ok(Some(identity));
        }
    }
    Ok(None)
}

/// Whether creating this record would make a **second live directory record
/// for somebody the product already knows** — and if so, which record they
/// already have.
///
/// Asked before the mirror row is written, deliberately. The projection is
/// 1:1 in both directions (`scim_users_identity_unique`), so a second record
/// for one person would otherwise be caught by the constraint *after* the
/// create had committed: the client would get a `409` for a `POST` whose
/// resource now exists, which is both a wart and a lie.
///
/// Refusing rather than merging is the safe answer. Two live records for one
/// person is the directory being inconsistent with itself, and a product
/// that quietly merged them would make somebody's identity depend on which
/// record was touched last. `409 uniqueness` is the conformant way to say
/// so, and an administrator can then fix the directory.
pub async fn conflicting_record(
    state: &AppState,
    tenant: &Tenant,
    attributes: &synveda_store::directory::UserAttributes,
) -> Result<Option<synveda_types::DirectoryUserId>> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant.id).await?;
    let candidate = DirectoryUser {
        id: synveda_types::DirectoryUserId::new(),
        tenant_id: tenant.id,
        external_id: attributes.external_id.clone(),
        user_name: attributes.user_name.clone(),
        active: attributes.active,
        display_name: None,
        given_name: None,
        family_name: None,
        work_email: attributes.work_email.clone(),
        identity_id: None,
        version: 1,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let Some(identity) = find_identity(&mut tx, tenant, &candidate).await? else {
        return Ok(None);
    };
    let held = directory::user_for_identity(&mut *tx, tenant.id, identity.id).await?;
    Ok(held.map(|row| row.id))
}

/// `active: false` — the leaver (ADR-0059 decision 8).
async fn seal(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: &Tenant,
    source: DirectorySource,
    existing: Option<Identity>,
) -> Result<Reconciled> {
    // Somebody the directory deactivated before they ever reached us has
    // nothing to seal, and that is a success rather than an error: a
    // provisioning agent retries, and RFC 7644 has no vocabulary for
    // "already done".
    let Some(identity) = existing else {
        return Ok(Reconciled::Unchanged);
    };
    let Some(sealed) = identities::depart(tx, tenant.id, identity.id).await? else {
        return Ok(Reconciled::Unchanged);
    };
    audit::record_as(
        tx,
        tenant.id,
        Actor::system("scim"),
        AuditAction::IdentitySealed,
        format!("scope {}", sealed.scope_id),
        Outcome::Success,
        with_source(
            json!({
                "identity": {"id": sealed.id, "subject": sealed.subject},
                "scope_id": sealed.scope_id,
                "reason": "directory deactivation",
            }),
            source,
        ),
    )
    .await?;
    Ok(Reconciled::Sealed)
}

/// A first placement: AUTH-2's resolver, a personal scope, an identity with
/// no subject yet (ADR-0059 decisions 5 and 6).
async fn place(
    state: &AppState,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: &Tenant,
    source: DirectorySource,
    user: &DirectoryUser,
    groups: &[String],
) -> Result<Reconciled> {
    let (parent, label) = resolve_parent(tx, tenant, groups).await?;
    let identity_id = IdentityId::new();
    let display_name = user
        .display_name
        .clone()
        .unwrap_or_else(|| user.user_name.clone());
    let email = user
        .work_email
        .clone()
        .or_else(|| Some(user.user_name.clone()));
    let scope = hierarchy::create(
        tx,
        ScopeId::new(),
        tenant.id,
        Some(parent.id),
        ScopeKind::User,
        &synveda_identity::personal_slug(email.as_deref(), &user.user_name, identity_id),
        &display_name,
    )
    .await?;
    let identity = identities::create(
        tx,
        identity_id,
        tenant.id,
        // No subject: this person may never have logged in, and the row is
        // a placement waiting for one (ADR-0059 decision 5).
        None,
        IdentityKind::User,
        email.as_deref(),
        user.display_name.as_deref(),
        scope.id,
    )
    .await?;
    directory::link_identity(&mut **tx, tenant.id, user.id, identity.id).await?;
    // The same action and the same payload shape JIT provisioning chains
    // (ADR-0013), because the two doors produce the same thing and a chain
    // consumer should not have to know which one was used. `origin` is the
    // one field that differs, and it is additive.
    audit::record_as(
        tx,
        tenant.id,
        Actor::system("scim"),
        AuditAction::IdentityProvisioned,
        format!("scope {}", scope.id),
        Outcome::Success,
        with_source(
            json!({
                "placement": label,
                "identity": {"id": identity.id, "subject": identity.subject},
                "parent": {"slug": parent.slug, "path": parent.path},
                "groups": groups,
                "origin": "scim",
            }),
            source,
        ),
    )
    .await?;
    let _ = state;
    Ok(Reconciled::Provisioned)
}

/// Re-resolves placement for somebody who already exists, and moves them if
/// the directory says they belong somewhere else (ADR-0059 decision 10).
///
/// Returns `None` when nothing moved.
async fn apply_placement(
    state: &AppState,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: &Tenant,
    source: DirectorySource,
    identity: &Identity,
    groups: &[String],
) -> Result<Option<Reconciled>> {
    let home = hierarchy::node(&mut **tx, identity.scope_id)
        .await?
        .ok_or_else(|| synveda_types::Error::Internal {
            message: format!("identity {} lost its scope node", identity.id),
        })?;
    let (destination, label) = resolve_parent(tx, tenant, groups).await?;
    if home.parent_id == Some(destination.id) {
        return Ok(None);
    }
    let source_parent = match home.parent_id {
        Some(parent_id) => hierarchy::node(&mut **tx, parent_id).await?,
        None => None,
    };

    // **A move with quarantine at either end never seals**, whatever the
    // packs there say. This is not in ADR-0059 and it should be (amendment
    // 2 to decision 10).
    //
    // Quarantine is not a placement — it is where somebody waits for a
    // mapping to be fixed, or where they sit for the moment between being
    // created and being put in a group. Material is never *written under*
    // its pack, so a hop through it is not a policy boundary being crossed
    // and there is nothing for a pack to have an opinion about.
    //
    // Both ends matter and each has its own real shape. A directory that
    // removes somebody from one group before adding the next passes them
    // **into** quarantine; a directory that creates a person and *then*
    // puts them in a group — which is the ordinary joiner sequence both AC
    // clients use — takes them **out** of it. Without this rule a tenant
    // whose org root runs a different pack from its departments would seal
    // a scope on either of those, permanently, on a technicality of
    // request ordering: in the joiner's case sealing a scope that was
    // seconds old and empty.
    //
    // Decision 11 already says losing every group is quarantine rather
    // than departure. This is that sentence applied to the material as
    // well as to the person.
    let into_quarantine = label == "quarantined";
    let out_of_quarantine = source_parent.as_ref().is_some_and(|parent| {
        parent.slug == synveda_store::identities::QUARANTINE_SLUG && parent.depth == 1
    });
    let touches_quarantine = into_quarantine || out_of_quarantine;

    // Whose pack decides, and whether there is anything to decide: a move
    // inside one pack's governance re-prices nothing (decision 10).
    let source_pack = effective_pack_name(state, tx, tenant, source_parent.as_ref()).await?;
    let destination_pack = effective_pack_name(state, tx, tenant, Some(&destination)).await?;
    let crosses_boundary = source_pack.0 != destination_pack.0;
    let seals = crosses_boundary && source_pack.1 && !touches_quarantine;

    let outcome = if seals {
        // Seal and restart: the live row lets go of the old node first,
        // then a former self takes it — the one-personal-scope-per-node
        // constraint refuses the other order.
        let fresh = hierarchy::create(
            tx,
            ScopeId::new(),
            tenant.id,
            Some(destination.id),
            ScopeKind::User,
            &format!("{}-{}", home.slug, short(identity.id)),
            &home.name,
        )
        .await?;
        identities::rescope(tx, tenant.id, identity.id, fresh.id).await?;
        identities::seal_scope_as_former_self(
            tx,
            IdentityId::new(),
            tenant.id,
            identity.kind,
            identity.email.as_deref(),
            identity.display_name.as_deref(),
            home.id,
        )
        .await?;
        Reconciled::MovedAndSealed
    } else {
        hierarchy::move_node(tx, home.id, destination.id).await?;
        Reconciled::Moved
    };

    audit::record_as(
        tx,
        tenant.id,
        Actor::system("scim"),
        AuditAction::IdentityMoved,
        format!("scope {}", home.id),
        Outcome::Success,
        with_source(
            json!({
                "identity": {"id": identity.id, "subject": identity.subject},
                "placement": label,
                "from": {"scope_id": home.parent_id, "pack": source_pack.0},
                "to": {"scope_id": destination.id, "slug": destination.slug, "pack": destination_pack.0},
                // The two facts that make the effect reviewable: whether the
                // move crossed a policy boundary at all, and what the source
                // pack said about material when it did.
                "crossed_policy_boundary": crosses_boundary,
                "touches_quarantine": touches_quarantine,
                "personal_memory": if seals { "sealed_and_restarted" } else { "followed" },
            }),
            source,
        ),
    )
    .await?;
    Ok(Some(outcome))
}

/// The effective pack at a scope, and whether it seals a mover's material.
///
/// `None` (a person with no parent, which only a malformed hierarchy
/// produces) resolves at the tenant, whose default pack is the honest
/// answer for "governed by nothing more specific".
async fn effective_pack_name(
    state: &AppState,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: &Tenant,
    scope: Option<&HierarchyNode>,
) -> Result<(String, bool)> {
    let (resource, chain) = match scope {
        Some(node) => {
            let chain = hierarchy::chain(&mut **tx, tenant.id, node.id).await?;
            (Resource::Scope(node.id), chain)
        }
        None => (Resource::Tenant(tenant.id), Vec::new()),
    };
    let chain_ids: Vec<ScopeId> = chain.iter().map(|node| node.id).collect();
    let assignments = policy_assignments::for_scopes(&mut **tx, tenant.id, &chain_ids).await?;
    let default_pack = policy_assignments::default_pack(&mut **tx, tenant.id).await?;
    let chain_nodes = ScopeNode::from_hierarchy_chain(&chain);
    let context = AuthzContext {
        scopes: &chain_nodes,
        principal_scopes: &[],
        anchors: &[],
        groups: &[],
        resources: &[],
        assignments: &assignments,
        default_pack: default_pack.as_deref(),
        role_bindings: &[],
        grant: None,
        sensitivity: None,
        lapses: &[],
    };
    let effective = state.pdp.effective(tenant.id, resource, &context);
    Ok((effective.name, effective.mover.seals_on_move()))
}

/// Resolves the parent a person's personal scope belongs under, from their
/// group names — AUTH-2's resolver (ADR-0013 decision 3), unchanged.
async fn resolve_parent(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: &Tenant,
    groups: &[String],
) -> Result<(HierarchyNode, &'static str)> {
    match provision::resolve_mapping(tx, tenant.id, groups).await? {
        Some(scope) => Ok((scope, "mapped")),
        // Nothing resolved is quarantine, never departure (ADR-0059
        // decision 11): a group nobody mapped is a configuration mistake
        // somebody can fix, and a seal is not.
        None => Ok((
            provision::ensure_quarantine(tx, tenant).await?,
            "quarantined",
        )),
    }
}

/// The first segment of an identity id — enough to make a restarted
/// personal scope's slug unique among its new siblings without making it
/// unreadable.
fn short(id: IdentityId) -> String {
    id.to_string().chars().take(8).collect()
}
