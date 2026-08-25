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
use synveda_store::{directory, identities, rls, scopes};
use synveda_types::{DirectoryUser, Identity, IdentityId, IdentityKind, Result, Tenant};

use crate::app::AppState;
use crate::audit;
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
    /// Stable adapter key stored with directory-owned users and groups.
    pub const fn key(self) -> &'static str {
        match self {
            DirectorySource::Scim(_) => "scim",
            DirectorySource::Pull { connector } => connector,
        }
    }

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
            None => (place(state, &mut tx, tenant, source, user).await?, true),
            Some(identity) => {
                // The link is written even when nothing else changes: an
                // adopted JIT identity has to stop being findable only by
                // the fallback that found it once.
                if user.identity_id != Some(identity.id) {
                    directory::link_identity(
                        &mut *tx,
                        tenant.id,
                        user.directory_source.as_str(),
                        user.id,
                        identity.id,
                    )
                    .await?;
                }
                // Group-driven placement is gone with the hierarchy (CPR-7,
                // ADR-0074 decision 3): the identity stays at its own
                // principal scope, and belonging somewhere is expressed by
                // the directory groups the reconciler syncs onto
                // `groups`/`group_members` and the grants an administrator
                // writes — never by moving the person's scope.
                match user.identity_id {
                    None => (Reconciled::Adopted, false),
                    Some(_) => (Reconciled::Unchanged, false),
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
        state.invalidate_scopes(tenant.id);
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
        directory_source: attributes.directory_source.clone(),
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
    // **Live rows only.** A departed person's identity still matches on
    // address — that is what makes the correspondence rule work — but it
    // must not stand in front of a rehire. A sealed row holding an email
    // forever would 409 every returning employee, which is the one shape
    // this uniqueness exists to permit rather than refuse (ADR-0059
    // decision 8: the seal is permanent, the *address* is not).
    if identity.sealed() {
        return Ok(None);
    }
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
) -> Result<Reconciled> {
    let identity_id = IdentityId::new();
    let display_name = user
        .display_name
        .clone()
        .unwrap_or_else(|| user.user_name.clone());
    let email = user
        .work_email
        .clone()
        .or_else(|| Some(user.user_name.clone()));
    // The principal scope is keyed by the directory's own anchor — the
    // externalId when the customer's mapping sends one, else the mirror
    // row's stable resource id — because this person has no token subject
    // yet. First login adopts the identity through the correspondence rule
    // (ADR-0059 decision 4), and the anchor resolver reads the identity's
    // stable scope before attempting a token-subject lookup, so the
    // directory's key and the login's key are one scope, not two (CPR-7,
    // ADR-0074 decision 3).
    let mut directory_anchor = user
        .external_id
        .clone()
        .filter(|external| !external.trim().is_empty())
        .unwrap_or_else(|| user.id.to_string());
    // **A rehire's anchor collides with its former self's, on purpose —
    // and has to be broken on purpose too.** `principal_id` is unique per
    // tenant and immutable (`scopes_principal_id_check`, migration 0043):
    // one anchor names one scope for that scope's whole life. `place` is
    // reached only when no *live* identity matched (`reconcile`'s
    // sealed-filter above), which for the same directory resource
    // reactivated twice means exactly one thing — a departed identity
    // already holds the scope this anchor names, structurally
    // (`identities_scope_unique`: one identity per scope, ever, and
    // identity rows are never deleted). Reusing it here would not adopt
    // the old identity (the seal does not lift, decision 12) and would
    // not mint a new one either — `identities::create` would collide on
    // that same uniqueness and this rehire would 409. So a rehire mints
    // under a disambiguated anchor instead: the fresh identity id is
    // globally unique by construction, and appending it is what makes
    // "a rehire is a new person, with a new personal scope" true rather
    // than aspirational. The natural anchor is untouched for the
    // overwhelmingly common case — nobody's own scope shifts under them
    // just because someone else, somewhere, once departed.
    if scopes::principal_scope(&mut **tx, tenant.id, &directory_anchor)
        .await?
        .is_some()
    {
        directory_anchor = format!("{directory_anchor}#{identity_id}");
    }
    let scope =
        scopes::ensure_principal_scope(tx, tenant.id, &directory_anchor, &display_name).await?;
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
    directory::link_identity(
        &mut **tx,
        tenant.id,
        user.directory_source.as_str(),
        user.id,
        identity.id,
    )
    .await?;
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
                "placement": "own-scope",
                "identity": {"id": identity.id, "subject": identity.subject},
                "scope": {"id": scope.id, "slug": scope.slug, "kind": scope.kind},
                "origin": "scim",
            }),
            source,
        ),
    )
    .await?;
    let _ = state;
    Ok(Reconciled::Provisioned)
}
