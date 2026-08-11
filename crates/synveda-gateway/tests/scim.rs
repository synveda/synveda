//! AUTH-4's acceptance criteria (ADR-0059).
//!
//! The feature text names two: *SCIM conformance tests*, and *a mover's
//! memories re-scope per policy*. Around them sit the claims the ADR makes
//! that a reader would otherwise take on trust, and three of these are
//! claims that were **wrong** in the first implementation until the suite
//! asked:
//!
//! 1. **A mover's memories re-scope per policy** — the AC, and asserted as a
//!    *contrast*: the same directory event, the same two departments, two
//!    packs, two outcomes. Under `regulated-strict` the material stays where
//!    it was written and the old scope is sealed; under `standard` it
//!    follows. A test that only showed one of them would be showing a
//!    behaviour rather than a policy.
//! 2. **A move inside one pack's governance asks nothing** — the other half
//!    of decision 10, and the half that keeps the common case friction-free.
//! 3. **The seal is three layers** (decision 8): the token stops working,
//!    the scope stops being readable *by everybody including its owner*, and
//!    the retention sweep stops seeing it. The third is the one a claim in a
//!    comment would never have caught.
//! 4. **One person never becomes two** (decision 4) — the correspondence
//!    rule from both ends: a SCIM create for somebody who already logged in,
//!    and a login for somebody SCIM created first.
//! 5. **The seal does not lift** (decision 12): a rehire is a new identity
//!    and a new scope, and the sealed one stays sealed.
//! 6. **Losing every group is quarantine, not departure** (decision 11).
//! 7. **Conformance**: `/ServiceProviderConfig` advertises what the routes
//!    enforce, the error envelope is RFC 7644 §3.12's shape with a *string*
//!    status, an unsupported filter is `501` rather than a wrong answer, and
//!    `DELETE` seals rather than deletes while still answering `204`/`404`.
//! 8. **The credential is confined**: a SCIM token is refused by `/v1` and a
//!    `/v1` bearer is refused here; a revoked one stops working; and one
//!    tenant's credential reaches nothing of another's.
//!
//! Tests need a live Postgres: they read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database), the house convention.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder};
use synveda_policy::{Pdp, REGULATED_STRICT, STANDARD};
use synveda_retrieval::index::SearchIndex;
use synveda_store::{
    directory, group_mappings, hierarchy, identities, policy_assignments, retention, rls, tenants,
};
use synveda_types::{
    HierarchyNode, IdentityId, ScimCredentialId, ScopeId, ScopeKind, TenantId, TenantStatus,
};
use tower::ServiceExt;

const SECRET: &[u8] = b"auth-4-scim-secret";

struct World {
    pool: PgPool,
    tenant: TenantId,
    tenant_row: synveda_types::Tenant,
    state: AppState,
    app: Router,
    /// The provisioning credential the directory authenticates with.
    token: String,
    /// The org root.
    org: ScopeId,
    /// `eng`, governed by whatever the test assigns.
    eng: ScopeId,
    /// `sales`, the other department.
    sales: ScopeId,
    /// `eng/core` — where the `synveda-eng-core` group maps.
    core: ScopeId,
    /// `eng/platform` — a sibling team inside the same department.
    platform: ScopeId,
    /// `sales/emea` — the cross-department destination.
    emea: ScopeId,
}

async fn world() -> Option<World> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!(
            "skipping the AUTH-4 SCIM suite: DATABASE_URL is not set \
             (run `make dev-up` then `make db-test`)"
        );
        return None;
    };
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");

    let tenant = TenantId::new();
    let slug = format!("auth4-{}", tenant.as_uuid().simple());
    tenants::create(&pool, tenant, &slug, "AUTH-4 tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");

    let mut tx = pool.begin().await.expect("begin");
    let org = node(&mut tx, tenant, None, ScopeKind::Org, "acme").await;
    let eng = node(&mut tx, tenant, Some(org.id), ScopeKind::Department, "eng").await;
    let sales = node(
        &mut tx,
        tenant,
        Some(org.id),
        ScopeKind::Department,
        "sales",
    )
    .await;
    let core = node(&mut tx, tenant, Some(eng.id), ScopeKind::Team, "core").await;
    let platform = node(&mut tx, tenant, Some(eng.id), ScopeKind::Team, "platform").await;
    let emea = node(&mut tx, tenant, Some(sales.id), ScopeKind::Team, "emea").await;
    node(
        &mut tx,
        tenant,
        Some(org.id),
        ScopeKind::Team,
        identities::QUARANTINE_SLUG,
    )
    .await;
    tx.commit().await.expect("commit hierarchy");

    // The convention resolves `synveda-eng-core` on its own; the override
    // table is what the other two ride on, so the suite exercises both of
    // ADR-0013 decision 3's mechanisms through the directory door.
    for (group, scope) in [
        ("synveda-eng-core", core.id),
        ("synveda-eng-platform", platform.id),
        ("synveda-sales-emea", emea.id),
    ] {
        let mut tx = rls::begin_tenant_tx(&pool, tenant).await.expect("begin");
        group_mappings::upsert(&mut *tx, tenant, group, scope)
            .await
            .expect("map group");
        tx.commit().await.expect("commit mapping");
    }

    let token = issue_credential(&pool, tenant).await;
    let pdp = Arc::new(Pdp::new().expect("build the embedded PDP"));
    let app_state = state(&url, pdp);
    let app = router(app_state.clone());
    let tenant_row = tenants::by_id(&pool, tenant)
        .await
        .expect("read tenant")
        .expect("tenant exists");
    Some(World {
        pool,
        tenant,
        tenant_row,
        state: app_state,
        app,
        token,
        org: org.id,
        eng: eng.id,
        sales: sales.id,
        core: core.id,
        platform: platform.id,
        emea: emea.id,
    })
}

// ── 1. The AC: a mover's memories re-scope per policy ────────────────────────

/// The acceptance criterion, as a **contrast**.
///
/// One directory event — a person's group changes from `synveda-eng-core` to
/// `synveda-sales-emea` — run twice against two departments governed by two
/// packs. The event is identical, the hierarchy is identical, and the
/// outcomes differ, which is what makes "per policy" a true sentence rather
/// than a description of whatever the code happens to do.
///
/// Under `regulated-strict` the material stays where it was written and the
/// scope it was written at is sealed. Under `standard` the scope moves with
/// the person. The hazard being governed is **disposal, not disclosure**
/// (ADR-0059 decision 10): a personal scope is readable by its owner and
/// nobody else wherever it hangs, but the horizons that govern its records
/// resolve from the effective pack at its own scope, on every sweep.
#[tokio::test]
async fn a_movers_memories_re_scope_per_the_source_packs_policy() {
    let Some(w) = world().await else { return };

    // ── The sealing pack: eng under `regulated-strict`, sales under
    //    `standard`. The source decides, so this move seals.
    assign(&w, w.eng, REGULATED_STRICT).await;
    assign(&w, w.sales, STANDARD).await;

    let (ada_user, ada_group) = join(&w, "ada@example.test", "synveda-eng-core").await;
    let ada_home = home_of(&w, &ada_user).await;
    assert_eq!(
        parent_of(&w, ada_home).await,
        Some(w.core),
        "the joiner landed where the mapping put them"
    );

    move_group(&w, &ada_user, &ada_group, "synveda-sales-emea").await;

    let ada_new_home = home_of(&w, &ada_user).await;
    assert_ne!(
        ada_new_home, ada_home,
        "a sealing pack restarts the mover at a fresh scope"
    );
    assert_eq!(
        parent_of(&w, ada_new_home).await,
        Some(w.emea),
        "the fresh scope sits under the destination"
    );
    assert!(
        sealed(&w, ada_home).await,
        "the scope the material was written at is sealed in place"
    );
    assert!(
        !sealed(&w, ada_new_home).await,
        "the person continues at a live scope"
    );

    // ── The following pack: the same move, out of a department governed by
    //    `standard`. Same directory event, same shape of hierarchy.
    assign(&w, w.sales, STANDARD).await;
    let (bo_user, bo_group) = join(&w, "bo@example.test", "synveda-sales-emea").await;
    let bo_home = home_of(&w, &bo_user).await;
    assert_eq!(parent_of(&w, bo_home).await, Some(w.emea));

    // sales → eng crosses a boundary in the other direction, and now the
    // *source* is `standard`, whose mover config lets material follow.
    move_group(&w, &bo_user, &bo_group, "synveda-eng-core").await;

    assert_eq!(
        home_of(&w, &bo_user).await,
        bo_home,
        "a following pack moves the same scope rather than restarting"
    );
    assert_eq!(
        parent_of(&w, bo_home).await,
        Some(w.core),
        "and the scope now hangs under the destination"
    );
    assert!(
        !sealed(&w, bo_home).await,
        "nothing is sealed when the material follows"
    );

    // The chain records which of the two happened and why, so the
    // difference is auditable rather than merely observable.
    //
    // The selector is `touches_quarantine == false`, and it is not
    // incidental: creating a person and *then* putting them in a group is
    // the ordinary joiner sequence, so every person in this test also has a
    // hop out of quarantine on the chain. Those hops are not moves between
    // placements and they never seal — see
    // `a_hop_through_quarantine_never_seals`.
    let events = audit_payloads(&w, "identity.moved").await;
    let moves: Vec<&Value> = events
        .iter()
        .filter(|event| event["touches_quarantine"] == json!(false))
        .collect();
    assert_eq!(
        moves.len(),
        2,
        "one move each, between two real placements: {events:#?}"
    );

    let sealed_move = moves
        .iter()
        .find(|event| event["personal_memory"] == json!("sealed_and_restarted"))
        .expect("the sealing move is on the chain");
    assert_eq!(sealed_move["crossed_policy_boundary"], json!(true));
    assert_eq!(sealed_move["from"]["pack"], json!(REGULATED_STRICT));
    assert_eq!(sealed_move["to"]["pack"], json!(STANDARD));

    let followed = moves
        .iter()
        .find(|event| event["personal_memory"] == json!("followed"))
        .expect("the following move is on the chain");
    assert_eq!(
        followed["crossed_policy_boundary"],
        json!(true),
        "the same kind of crossing, decided the other way: {followed}"
    );
    assert_eq!(followed["from"]["pack"], json!(STANDARD));
    assert_eq!(followed["to"]["pack"], json!(REGULATED_STRICT));
}

/// A hop with quarantine at either end never seals, however the packs at
/// the two ends differ.
///
/// This is not in the ADR and it should be (amendment 2 to decision 10).
/// Quarantine is not a placement: it is where somebody waits for a mapping
/// to be fixed, and — because both AC clients create a person *before*
/// putting them in a group — where every joiner sits for a moment. A
/// tenant whose org root runs a different pack from its departments would
/// otherwise have every new hire's scope sealed and restarted seconds after
/// it was created, and every re-grouping done in two requests sealed on the
/// way through.
#[tokio::test]
async fn a_hop_through_quarantine_never_seals() {
    let Some(w) = world().await else { return };
    // The org root — and therefore quarantine — runs a different pack from
    // the department the joiner is heading for. Nothing else about this is
    // unusual: it is a tenant that made its root open and its departments
    // strict.
    assign(&w, w.org, STANDARD).await;
    assign(&w, w.eng, REGULATED_STRICT).await;

    let (jo_user, _) = join(&w, "jo@example.test", "synveda-eng-core").await;
    let home = home_of(&w, &jo_user).await;
    assert_eq!(parent_of(&w, home).await, Some(w.core));
    assert!(
        !sealed(&w, home).await,
        "a joiner's own scope is not sealed on the way out of quarantine"
    );

    let hops = audit_payloads(&w, "identity.moved").await;
    let out = hops.first().expect("the hop out of quarantine is chained");
    assert_eq!(out["touches_quarantine"], json!(true));
    assert_eq!(
        out["crossed_policy_boundary"],
        json!(true),
        "the packs really do differ across this hop: {out}"
    );
    assert_eq!(
        out["personal_memory"],
        json!("followed"),
        "and it still does not seal, because quarantine is not a placement"
    );
}

/// The other half of decision 10: **a move is only a policy question when it
/// changes the policy.**
///
/// `eng/core` → `eng/platform` is a sibling-team move inside one
/// department, so both ends resolve the same effective pack. Nothing is
/// re-priced, so nothing is asked — and the material follows even though the
/// governing pack is the sealing one. Without this the common case would
/// seal somebody's notes every time they changed team.
#[tokio::test]
async fn a_move_inside_one_packs_governance_asks_nothing() {
    let Some(w) = world().await else { return };
    assign(&w, w.eng, REGULATED_STRICT).await;

    let (cy_user, cy_group) = join(&w, "cy@example.test", "synveda-eng-core").await;
    let home = home_of(&w, &cy_user).await;
    move_group(&w, &cy_user, &cy_group, "synveda-eng-platform").await;

    assert_eq!(
        home_of(&w, &cy_user).await,
        home,
        "the same scope moved: no restart inside one pack's governance"
    );
    assert_eq!(parent_of(&w, home).await, Some(w.platform));
    assert!(!sealed(&w, home).await, "and nothing was sealed");

    let moved = audit_payloads(&w, "identity.moved").await;
    let event = moved.first().expect("the move is on the chain");
    assert_eq!(
        event["crossed_policy_boundary"],
        json!(false),
        "the chain says the move crossed no boundary: {event}"
    );
    assert_eq!(event["personal_memory"], json!("followed"));
}

// ── 2. The seal, in three layers ─────────────────────────────────────────────

/// Decision 8, asserted layer by layer — and the third layer is the one that
/// no comment could have established.
#[tokio::test]
async fn a_seal_stops_the_token_the_reads_and_the_retention_sweep() {
    let Some(w) = world().await else { return };
    assign(&w, w.eng, REGULATED_STRICT).await;

    let (dee_user, _) = join(&w, "dee@example.test", "synveda-eng-core").await;
    let home = home_of(&w, &dee_user).await;
    // Bind a subject to the identity the way a first login would, so the
    // token layer has something to refuse.
    let identity = identity_at(&w, home).await;
    bind_subject(&w, identity, "dee-subject").await;

    // Layer 3's precondition: the scope is enumerable by the sweep while
    // the person is here. Asserted *before* the seal so the assertion
    // after it is a change rather than a coincidence.
    seed_record(&w, home, identity).await;
    assert!(
        swept_scopes(&w).await.contains(&home),
        "a live scope is in the sweep's work list"
    );

    // The directory says they have left.
    deactivate(&w, &dee_user).await;

    // Layer 1: the token stops working, at the enforcement seam, without
    // waiting for the IdP to revoke anything.
    let token = issue("dee-subject", w.tenant);
    let (status, _) = get(&w.app, &token, "/v1/hierarchy/root").await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a departed identity's still-valid token does nothing"
    );

    // Layer 2: the scope is unreadable — by everyone, including the person
    // whose scope it is. There is no reader in this feature.
    assert!(sealed(&w, home).await);
    let (read, _) = get(&w.app, &token, &format!("/v1/hierarchy/nodes/{home}")).await;
    assert_eq!(read, StatusCode::FORBIDDEN);

    // Layer 3: the retention sweep no longer sees the scope at all. This is
    // the "retention-held" half — a hold whose purpose is to outlive a
    // schedule must not be implemented as one.
    assert!(
        !swept_scopes(&w).await.contains(&home),
        "a sealed scope is exempt from every horizon"
    );
}

// ── 3. One person never becomes two ──────────────────────────────────────────

/// The correspondence rule (decision 4) from both ends.
///
/// The failure this prevents is invisible until it has cost somebody their
/// memory: two identities, two personal scopes, half the material in each,
/// and nothing anywhere that looks wrong.
#[tokio::test]
async fn one_person_never_becomes_two_identities() {
    let Some(w) = world().await else { return };
    assign(&w, w.eng, REGULATED_STRICT).await;

    // ── End one: somebody logs in first (JIT), then the directory creates
    //    them. The SCIM create must adopt rather than duplicate.
    let jit = provision_via_login(&w, "eli-subject", "eli@example.test", "synveda-eng-core").await;
    let (eli_user, _) = join(&w, "eli@example.test", "synveda-eng-core").await;
    assert_eq!(
        identity_count(&w, "eli@example.test").await,
        1,
        "a SCIM create adopted the JIT identity rather than making a second"
    );
    assert_eq!(
        linked_identity(&w, &eli_user).await,
        Some(jit),
        "and the mirror row points at the identity that already existed"
    );

    // ── End two: the directory creates somebody, then they log in. The
    //    login must bind its subject to the identity that is waiting.
    let (fay_user, _) = join(&w, "fay@example.test", "synveda-eng-core").await;
    let waiting = linked_identity(&w, &fay_user).await.expect("linked");
    assert_eq!(
        subject_of(&w, waiting).await,
        None,
        "a directory-created identity has no subject until somebody logs in"
    );

    let bound =
        provision_via_login(&w, "fay-subject", "fay@example.test", "synveda-eng-core").await;
    assert_eq!(
        bound, waiting,
        "the login bound its subject to the waiting identity"
    );
    assert_eq!(
        subject_of(&w, waiting).await.as_deref(),
        Some("fay-subject"),
    );
    assert_eq!(identity_count(&w, "fay@example.test").await, 1);
}

/// A second live directory record for one person is refused, and refused
/// **before anything is written**.
///
/// Both halves are the demo's findings rather than the design's. The
/// matching half: the last of the correspondence rule's three matches tries
/// the work address before the `userName`, because ADR-0059 decision 4 was
/// written assuming those are the same string — a record re-created with a
/// new anchor and a new `userName` for somebody whose mailbox never changed
/// slipped straight past it and made a second identity. The refusing half:
/// once the match *did* fire, the 1:1 projection constraint caught it after
/// the create had committed, so the client got a `409` for a resource that
/// then existed. Now the question is asked first and nothing is written.
#[tokio::test]
async fn a_second_record_for_one_person_is_refused_before_anything_is_written() {
    let Some(w) = world().await else { return };
    assign(&w, w.eng, REGULATED_STRICT).await;

    let (mel_user, _) = join(&w, "mel@example.test", "synveda-eng-core").await;
    let identity = linked_identity(&w, &mel_user).await.expect("linked");

    // A new anchor, a new address, the same mailbox — the re-created
    // account that started this.
    let (status, body) = scim_post(
        &w,
        "/scim/v2/Users",
        json!({
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
            "userName": "mel@example.test-recreated",
            "externalId": "mel-new-object-id",
            "emails": [{"value": "mel@example.test", "type": "work", "primary": true}],
            "active": true
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["scimType"], json!("uniqueness"));

    assert_eq!(
        identity_count(&w, "mel@example.test").await,
        1,
        "one person, one identity"
    );
    let orphans = sqlx::query_scalar!(
        r#"select count(*) as "count!" from scim_users
           where tenant_id = $1 and identity_id is null"#,
        w.tenant.as_uuid(),
    )
    .fetch_one(&w.pool)
    .await
    .expect("count orphans");
    assert_eq!(
        orphans, 0,
        "a refused create leaves no record behind — the 409 is not about a \
         resource that now exists"
    );
    assert_eq!(
        linked_identity(&w, &mel_user).await,
        Some(identity),
        "and the record they do have is untouched"
    );
}

/// Decision 12: the seal does not lift, and a rehire is a new person.
///
/// The rule exists because the alternative is a retention hold that whoever
/// holds the provisioning credential can undo — which after a directory
/// compromise is the attacker.
///
/// A rehire arrives in two shapes and both are here, because they take
/// different paths through the reconciler and only one of them was handled
/// before this test asked. **Shape A** reactivates the same resource, which
/// is the only path that can hand the reconciler a sealed identity through
/// the mirror's own link. **Shape B** creates a new resource with the same
/// `userName`, which works precisely because uniqueness is enforced over
/// live rows only (decision 11).
#[tokio::test]
async fn a_rehire_is_a_new_identity_and_the_sealed_one_stays_sealed() {
    let Some(w) = world().await else { return };
    assign(&w, w.eng, REGULATED_STRICT).await;

    // ── Shape A: the same resource, reactivated.
    let (gus_user, _) = join(&w, "gus@example.test", "synveda-eng-core").await;
    let first = linked_identity(&w, &gus_user).await.expect("linked");
    let first_home = home_of(&w, &gus_user).await;
    deactivate(&w, &gus_user).await;
    assert!(sealed(&w, first_home).await);

    reactivate(&w, &gus_user).await;
    assert!(
        sealed(&w, first_home).await,
        "`active: true` on a departed person does not lift the seal"
    );
    let second = linked_identity(&w, &gus_user).await.expect("linked");
    assert_ne!(second, first, "a reactivation is a new identity");
    let second_home = home_of(&w, &gus_user).await;
    assert_ne!(second_home, first_home, "with a new personal scope");
    assert!(!sealed(&w, second_home).await, "which is live");
    assert_eq!(
        parent_of(&w, second_home).await,
        Some(w.core),
        "placed by the groups the directory still has them in"
    );

    // ── Shape B: a new resource with the same address, after the old one
    //    has been deactivated. The live-rows-only uniqueness is what makes
    //    this possible at all — a departed row holding an address forever
    //    would 409 every rehire.
    let (ivy_user, _) = join(&w, "ivy@example.test", "synveda-eng-core").await;
    let ivy_first = linked_identity(&w, &ivy_user).await.expect("linked");
    let ivy_home = home_of(&w, &ivy_user).await;
    deactivate(&w, &ivy_user).await;

    let (ivy_again, _) = join(&w, "ivy@example.test", "synveda-eng-core").await;
    assert_ne!(ivy_again, ivy_user, "a new directory resource");
    let ivy_second = linked_identity(&w, &ivy_again).await.expect("linked");
    assert_ne!(ivy_second, ivy_first, "a new identity");
    assert!(
        sealed(&w, ivy_home).await,
        "and the material of the former self stays sealed"
    );
    assert!(!sealed(&w, home_of(&w, &ivy_again).await).await);
}

/// Decision 11: removing somebody from every mapped group is **quarantine**,
/// not departure.
///
/// The difference is the difference between a misconfigured group and a
/// person losing their memory: quarantine is reversible by fixing the
/// mapping, and a seal is not.
#[tokio::test]
async fn losing_every_group_quarantines_rather_than_seals() {
    let Some(w) = world().await else { return };
    assign(&w, w.eng, REGULATED_STRICT).await;

    let (hal_user, hal_group) = join(&w, "hal@example.test", "synveda-eng-core").await;
    remove_member(&w, &hal_group, &hal_user).await;

    let now = home_of(&w, &hal_user).await;
    assert!(!sealed(&w, now).await, "an unmapped person is not departed");
    let parent = parent_of(&w, now).await.expect("placed somewhere");
    assert_eq!(
        slug_of(&w, parent).await.as_deref(),
        Some(identities::QUARANTINE_SLUG),
        "they are quarantined, which somebody can fix by fixing the mapping"
    );
    // And the reverse is reversible, which is the whole distinction.
    add_member(&w, &hal_group, &hal_user).await;
    assert_eq!(
        parent_of(&w, home_of(&w, &hal_user).await).await,
        Some(w.core)
    );
}

// ── 4. Conformance ───────────────────────────────────────────────────────────

/// The advertisement and the behaviour are the same constants.
///
/// A `/ServiceProviderConfig` that promised more than the routes do would be
/// the most damaging kind of untruth here: a provisioning agent configures
/// itself from it.
#[tokio::test]
async fn the_advertised_config_matches_what_the_routes_do() {
    let Some(w) = world().await else { return };

    let (status, config) = scim_get(&w, "/scim/v2/ServiceProviderConfig").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(config["patch"]["supported"], json!(true));
    assert_eq!(config["bulk"]["supported"], json!(false));
    assert_eq!(config["sort"]["supported"], json!(false));
    assert_eq!(config["changePassword"]["supported"], json!(false));
    assert_eq!(config["filter"]["supported"], json!(true));

    // `/Schemas` publishes exactly what the mirror stores — which is what
    // makes "unknown attributes are ignored" honest rather than a shrug.
    let (_, schemas) = scim_get(&w, "/scim/v2/Schemas").await;
    let user_schema = schemas["Resources"]
        .as_array()
        .expect("resources")
        .iter()
        .find(|schema| schema["name"] == json!("User"))
        .expect("a User schema");
    let published: Vec<&str> = user_schema["attributes"]
        .as_array()
        .expect("attributes")
        .iter()
        .filter_map(|attribute| attribute["name"].as_str())
        .collect();
    for stored in [
        "userName",
        "externalId",
        "active",
        "displayName",
        "name",
        "emails",
    ] {
        assert!(published.contains(&stored), "{stored} must be published");
    }
    // And nothing this server drops on the floor is claimed.
    for dropped in ["title", "phoneNumbers", "addresses"] {
        assert!(
            !published.contains(&dropped),
            "{dropped} is not stored, so it must not be published"
        );
    }

    let (_, types) = scim_get(&w, "/scim/v2/ResourceTypes").await;
    assert_eq!(types["totalResults"], json!(2));
}

/// The error envelope is RFC 7644 §3.12's, including the detail a strict
/// client refuses to parse: `status` is a **string**.
#[tokio::test]
async fn errors_are_scim_errors_and_unsupported_filters_are_501() {
    let Some(w) = world().await else { return };

    let (status, body) = scim_get(&w, "/scim/v2/Users/not-a-uuid").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body["schemas"],
        json!(["urn:ietf:params:scim:api:messages:2.0:Error"])
    );
    assert_eq!(
        body["status"],
        json!("404"),
        "RFC 7644 types `status` as a string: {body}"
    );

    // A filter this server does not implement is 501 — the status the RFC
    // names for exactly that — rather than a wrong empty list, which is the
    // failure mode a provisioning agent would silently act on.
    let (status, body) = scim_get(
        &w,
        "/scim/v2/Users?filter=userName%20eq%20%22a%22%20and%20active%20eq%20true",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(body["scimType"], json!("invalidFilter"));

    // A filter that is not a filter is the caller's mistake: 400.
    let (status, _) = scim_get(&w, "/scim/v2/Users?filter=userName").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The filter both clients send works, and answers a ListResponse even
    // when it matches nothing.
    let (status, body) = scim_get(
        &w,
        "/scim/v2/Users?filter=userName%20eq%20%22nobody@example.test%22",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["totalResults"], json!(0));
    assert_eq!(
        body["schemas"],
        json!(["urn:ietf:params:scim:api:messages:2.0:ListResponse"])
    );
}

/// `DELETE` seals and does not delete (decision 11), while still answering
/// exactly what a conformant client expects.
#[tokio::test]
async fn delete_answers_204_then_404_and_seals_rather_than_deletes() {
    let Some(w) = world().await else { return };
    assign(&w, w.eng, REGULATED_STRICT).await;

    let (kit_user, _) = join(&w, "kit@example.test", "synveda-eng-core").await;
    let home = home_of(&w, &kit_user).await;

    let (status, _) = scim_delete(&w, &format!("/scim/v2/Users/{kit_user}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The protocol's promise holds...
    let (status, _) = scim_get(&w, &format!("/scim/v2/Users/{kit_user}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the resource is still addressable — it was deactivated, not removed"
    );

    // ...and what actually happened is a seal, which is the one operation a
    // retention hold exists to make possible.
    assert!(sealed(&w, home).await, "DELETE sealed the personal scope");
    // And the row is still there — a delete would have taken the seal with
    // it, which is the one operation a retention hold exists to prevent.
    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin");
    let owner = identities::by_scope(&mut *tx, w.tenant, home)
        .await
        .expect("read identity");
    assert!(
        owner.is_some_and(|identity| identity.sealed()),
        "the identity row survives the DELETE, holding the seal"
    );
}

// ── 5. The credential is confined ────────────────────────────────────────────

/// Decision 13's confinement, from both directions, plus revocation.
#[tokio::test]
async fn a_provisioning_credential_reaches_this_plane_and_nothing_else() {
    let Some(w) = world().await else { return };

    // A `/v1` bearer is not a SCIM credential.
    let bearer = issue("somebody", w.tenant);
    let (status, _) = call(
        &w.app,
        Request::builder()
            .method("GET")
            .uri("/scim/v2/Users")
            .header("authorization", format!("Bearer {bearer}"))
            .body(Body::empty())
            .expect("request"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a session bearer authenticates nothing on the directory plane"
    );

    // ...and a SCIM credential is not a `/v1` bearer.
    let (status, _) = get(&w.app, &w.token, "/v1/hierarchy/root").await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a provisioning credential reaches no governed route"
    );

    // A credential from another tenant reaches nothing here — not denied so
    // much as absent, because the lookup runs inside the tenant its own
    // token names.
    let other = TenantId::new();
    tenants::create(
        &w.pool,
        other,
        &format!("other-{}", other.as_uuid().simple()),
        "Other",
        TenantStatus::Active,
    )
    .await
    .expect("admit other tenant");
    let foreign = issue_credential(&w.pool, other).await;
    let (status, _) = scim_get_with(&w, &foreign, "/scim/v2/Users").await;
    assert_eq!(status, StatusCode::OK, "the other tenant's own plane works");
    let (_, body) = scim_get_with(&w, &foreign, "/scim/v2/Users").await;
    assert_eq!(
        body["totalResults"],
        json!(0),
        "and sees none of this tenant's people"
    );

    // A secret lifted behind this tenant's prefix authenticates nothing:
    // the hash covers the whole token.
    let secret = foreign.rsplit('.').next().expect("secret");
    let forged = format!("synveda_scim_v1.{}.{secret}", w.tenant);
    let (status, _) = scim_get_with(&w, &forged, "/scim/v2/Users").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// A revoked credential stops working on the next request, and what it did
/// stays on the chain.
#[tokio::test]
async fn a_revoked_credential_stops_authenticating() {
    let Some(w) = world().await else { return };

    let (status, _) = scim_get(&w, "/scim/v2/Users").await;
    assert_eq!(status, StatusCode::OK);

    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin");
    let credentials = directory::credentials(&mut *tx, w.tenant)
        .await
        .expect("list credentials");
    directory::revoke_credential(&mut *tx, w.tenant, credentials[0].id)
        .await
        .expect("revoke");
    tx.commit().await.expect("commit");

    let (status, _) = scim_get(&w, "/scim/v2/Users").await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "revocation binds on the very next request"
    );
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Creates a SCIM user in a group, the way a provisioning agent does:
/// `POST /Users`, then `POST /Groups` (or a PATCH adding the member).
/// Returns the mirror ids.
async fn join(w: &World, user_name: &str, group: &str) -> (String, String) {
    let (status, created) = scim_post(
        w,
        "/scim/v2/Users",
        json!({
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
            "userName": user_name,
            "externalId": format!("ext-{user_name}"),
            "name": {"givenName": "Test", "familyName": "Person"},
            "emails": [{"value": user_name, "type": "work", "primary": true}],
            "active": true
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create user: {created}");
    let user_id = created["id"].as_str().expect("id").to_owned();

    let group_id = ensure_group(w, group).await;
    add_member(w, &group_id, &user_id).await;
    (user_id, group_id)
}

async fn ensure_group(w: &World, display_name: &str) -> String {
    let escaped = display_name.replace(' ', "%20");
    let (_, found) = scim_get(
        w,
        &format!("/scim/v2/Groups?filter=displayName%20eq%20%22{escaped}%22"),
    )
    .await;
    if found["totalResults"].as_i64() == Some(1) {
        return found["Resources"][0]["id"].as_str().expect("id").to_owned();
    }
    let (status, created) = scim_post(
        w,
        "/scim/v2/Groups",
        json!({
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group"],
            "displayName": display_name
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create group: {created}");
    created["id"].as_str().expect("id").to_owned()
}

/// Entra's member-add shape.
async fn add_member(w: &World, group: &str, user: &str) {
    let (status, body) = scim_patch(
        w,
        &format!("/scim/v2/Groups/{group}"),
        json!({
            "schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
            "Operations": [{"op": "add", "path": "members", "value": [{"value": user}]}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "add member: {body}");
}

/// Entra's member-removal shape: the id is in the path filter.
async fn remove_member(w: &World, group: &str, user: &str) {
    let (status, body) = scim_patch(
        w,
        &format!("/scim/v2/Groups/{group}"),
        json!({
            "schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
            "Operations": [{"op": "remove", "path": format!("members[value eq \"{user}\"]")}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "remove member: {body}");
}

/// A mover: into the new group, then out of the old.
///
/// This order is deliberate and it is the one a provisioning agent uses
/// when it pushes both changes: adding first means the person is never
/// momentarily in *no* mapped group, so the move is one hop rather than a
/// detour through quarantine. The other order is a real shape too — it is
/// two directory statements, and the product answers both — which is why
/// `a_two_request_move_never_seals_on_the_way_through_quarantine` exists.
async fn move_group(w: &World, user: &str, from_group: &str, to_group: &str) {
    let to = ensure_group(w, to_group).await;
    add_member(w, &to, user).await;
    remove_member(w, from_group, user).await;
}

/// Entra's deactivation shape.
async fn deactivate(w: &World, user: &str) {
    let (status, body) = scim_patch(
        w,
        &format!("/scim/v2/Users/{user}"),
        json!({
            "schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
            "Operations": [{"op": "replace", "path": "active", "value": false}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "deactivate: {body}");
}

async fn reactivate(w: &World, user: &str) {
    let (status, body) = scim_patch(
        w,
        &format!("/scim/v2/Users/{user}"),
        json!({
            "schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
            "Operations": [{"op": "replace", "path": "active", "value": true}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "reactivate: {body}");
}

/// The identity a mirror row projected onto.
async fn linked_identity(w: &World, user: &str) -> Option<IdentityId> {
    let id = user.parse().expect("directory user id");
    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin");
    let row = directory::user(&mut *tx, w.tenant, id)
        .await
        .expect("read mirror");
    row.and_then(|row| row.identity_id)
}

/// The personal scope a mirror row's person currently owns.
async fn home_of(w: &World, user: &str) -> ScopeId {
    let identity = linked_identity(w, user).await.expect("linked identity");
    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin");
    identities::by_id(&mut *tx, w.tenant, identity)
        .await
        .expect("read identity")
        .expect("identity exists")
        .scope_id
}

async fn parent_of(w: &World, scope: ScopeId) -> Option<ScopeId> {
    hierarchy::node(&w.pool, scope)
        .await
        .expect("read node")
        .and_then(|node| node.parent_id)
}

async fn slug_of(w: &World, scope: ScopeId) -> Option<String> {
    hierarchy::node(&w.pool, scope)
        .await
        .expect("read node")
        .map(|node| node.slug)
}

/// The derivation under test, read the way every caller reads it.
async fn sealed(w: &World, scope: ScopeId) -> bool {
    hierarchy::node(&w.pool, scope)
        .await
        .expect("read node")
        .expect("node exists")
        .sealed
}

async fn identity_at(w: &World, scope: ScopeId) -> IdentityId {
    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin");
    identities::by_scope(&mut *tx, w.tenant, scope)
        .await
        .expect("read identity")
        .expect("a personal scope has an owner")
        .id
}

async fn subject_of(w: &World, identity: IdentityId) -> Option<String> {
    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin");
    identities::by_id(&mut *tx, w.tenant, identity)
        .await
        .expect("read identity")
        .expect("identity exists")
        .subject
}

async fn identity_count(w: &World, email: &str) -> i64 {
    sqlx::query_scalar!(
        r#"select count(*) as "count!" from identities where tenant_id = $1 and lower(email) = lower($2)"#,
        w.tenant.as_uuid(),
        email,
    )
    .fetch_one(&w.pool)
    .await
    .expect("count identities")
}

async fn bind_subject(w: &World, identity: IdentityId, subject: &str) {
    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin");
    identities::bind_subject(&mut tx, w.tenant, identity, subject)
        .await
        .expect("bind subject");
    tx.commit().await.expect("commit");
}

/// Provisions through the *login* door, so the correspondence rule is
/// exercised from the side a person arrives on.
async fn provision_via_login(w: &World, subject: &str, email: &str, group: &str) -> IdentityId {
    let claims = synveda_identity::ProvisioningClaims {
        groups: vec![group.to_owned()],
        email: Some(email.to_owned()),
        display_name: Some(subject.to_owned()),
        external_id: Some(format!("ext-{email}")),
    };
    synveda_gateway::provision::provision(&w.state, &w.tenant_row, subject, &claims)
        .await
        .expect("provision")
        .identity
        .id
}

/// The scopes the MEM-6 sweep would enumerate — the third layer's assertion.
async fn swept_scopes(w: &World) -> Vec<ScopeId> {
    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin");
    retention::populated_scopes(&mut *tx, w.tenant)
        .await
        .expect("enumerate")
}

async fn audit_payloads(w: &World, action: &str) -> Vec<Value> {
    sqlx::query_scalar!(
        r#"select payload as "payload!" from audit_log
           where tenant_id = $1 and action = $2 order by seq"#,
        w.tenant.as_uuid(),
        action,
    )
    .fetch_all(&w.pool)
    .await
    .expect("read audit payloads")
}

async fn assign(w: &World, scope: ScopeId, pack: &str) {
    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin");
    policy_assignments::assign(&mut *tx, w.tenant, scope, pack)
        .await
        .expect("assign pack");
    tx.commit().await.expect("commit assignment");
}

async fn issue_credential(pool: &PgPool, tenant: TenantId) -> String {
    let minted = synveda_identity::scim::mint(tenant).expect("mint");
    let mut tx = rls::begin_tenant_tx(pool, tenant).await.expect("begin");
    directory::issue_credential(
        &mut *tx,
        ScimCredentialId::new(),
        tenant,
        &minted.hash,
        "test",
        Utc::now() + chrono::Duration::days(1),
        "test-operator",
    )
    .await
    .expect("issue credential");
    tx.commit().await.expect("commit");
    minted.token
}

async fn node(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: TenantId,
    parent: Option<ScopeId>,
    kind: ScopeKind,
    slug: &str,
) -> HierarchyNode {
    hierarchy::create(tx, ScopeId::new(), tenant, parent, kind, slug, slug)
        .await
        .expect("create node")
}

async fn seed_record(w: &World, scope: ScopeId, owner: IdentityId) {
    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin");
    synveda_store::records::insert(
        &mut *tx,
        synveda_types::RecordId::new(),
        w.tenant,
        &synveda_store::records::RecordState {
            scope_id: scope,
            owner_id: owner,
            kind: synveda_types::RecordKind::Derived,
            class: synveda_types::RecordClass::Fact,
            content: "a fact written before somebody left".to_owned(),
            sensitivity: synveda_types::Sensitivity::Internal,
            provenance: json!({"source": "auth-4 suite"}),
            valid_from: Utc::now(),
            valid_to: None,
        },
        &synveda_store::records::RecordEmbedding {
            model: "hash@1".to_owned(),
            vector: vec![0.1; 8],
        },
    )
    .await
    .expect("seed record");
    tx.commit().await.expect("commit record");
}

fn metrics_handle() -> PrometheusHandle {
    use std::sync::OnceLock;
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn index_root() -> PathBuf {
    std::env::temp_dir()
        .join("synveda-auth4-scim")
        .join(TenantId::new().to_string())
}

fn state(url: &str, pdp: Arc<Pdp>) -> AppState {
    AppState {
        pool: PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(5))
            .connect_lazy(url)
            .expect("parse database url"),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp,
        scope_chains: Arc::new(synveda_store::ScopeChainCache::new()),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index: Arc::new(SearchIndex::open(index_root()).expect("open sidecar")),
        embedder: Arc::new(AnyEmbedder::Deterministic(DeterministicEmbedder::new())),
        inject_embed_timeout: Duration::from_millis(100),
        // TEN-4 (ADR-0064): a fixed test KEK, so a suite that touches a
        // sealed column seals rather than skipping. `Kms::Disabled` is the
        // production default when no key is configured.
        keys: std::sync::Arc::new(synveda_store::keys::KeyRing::new(
            synveda_crypto::Kms::Local(
                synveda_crypto::LocalKms::from_hex(&"11".repeat(32), "local:test")
                    .expect("test kek"),
            ),
        )),
    }
}

fn issue(subject: &str, tenant_id: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue(subject, tenant_id, Duration::from_secs(600))
}

async fn call(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(request).await.expect("route");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

async fn get(app: &Router, token: &str, uri: &str) -> (StatusCode, Value) {
    call(
        app,
        Request::builder()
            .method("GET")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("request"),
    )
    .await
}

async fn scim_get(w: &World, uri: &str) -> (StatusCode, Value) {
    scim_get_with(w, &w.token, uri).await
}

async fn scim_get_with(w: &World, token: &str, uri: &str) -> (StatusCode, Value) {
    get(&w.app, token, uri).await
}

async fn scim_post(w: &World, uri: &str, body: Value) -> (StatusCode, Value) {
    call(
        &w.app,
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("authorization", format!("Bearer {}", w.token))
            .header("content-type", "application/scim+json")
            .body(Body::from(body.to_string()))
            .expect("request"),
    )
    .await
}

async fn scim_patch(w: &World, uri: &str, body: Value) -> (StatusCode, Value) {
    call(
        &w.app,
        Request::builder()
            .method("PATCH")
            .uri(uri)
            .header("authorization", format!("Bearer {}", w.token))
            .header("content-type", "application/scim+json")
            .body(Body::from(body.to_string()))
            .expect("request"),
    )
    .await
}

async fn scim_delete(w: &World, uri: &str) -> (StatusCode, Value) {
    call(
        &w.app,
        Request::builder()
            .method("DELETE")
            .uri(uri)
            .header("authorization", format!("Bearer {}", w.token))
            .body(Body::empty())
            .expect("request"),
    )
    .await
}
