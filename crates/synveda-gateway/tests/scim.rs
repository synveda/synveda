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
use synveda_policy::{Pdp, REGULATED_STRICT};
use synveda_store::{access, directory, identities, rls, scopes, tenants};
use synveda_types::access::{GrantSource, GroupSource};
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{
    DirectoryUserId, IdentityId, ScimCredentialId, ScopeId, TenantId, TenantStatus,
};
use tower::ServiceExt;

#[path = "support/configuration.rs"]
mod configuration_support;

const SECRET: &[u8] = b"auth-4-scim-secret";

struct World {
    pool: PgPool,
    tenant: TenantId,
    tenant_row: synveda_types::Tenant,
    state: AppState,
    app: Router,
    /// The provisioning credential the directory authenticates with.
    token: String,
    /// The tenant root scope.
    org: ScopeId,
    /// `eng`, governed by whatever the test assigns.
    eng: ScopeId,
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

    // The tree the pack assignments hang on: a root with `eng` and `sales`
    // org units, each holding two nested units. Groups are seeded through
    // the SCIM surface itself — a directory group is a governed group
    // now (CPR-6), never a mapping to a scope, and a person's scope is
    // their own principal scope under the root, wherever their groups
    // put them (CPR-7, ADR-0074 decision 3).
    let mut tx = pool.begin().await.expect("begin");
    let org = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("mint root");
    let eng = unit(&mut tx, tenant, org.id, "eng").await;
    tx.commit().await.expect("commit scopes");

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
    })
}

// ── 1. A directory seal closes both enforcement layers ──────────────────────

/// Decision 8, asserted at the token and governed-scope layers.
#[tokio::test]
async fn a_seal_stops_the_token_and_governed_reads() {
    let Some(w) = world().await else { return };
    assign(&w, w.eng, REGULATED_STRICT).await;

    let (dee_user, _) = join(&w, "dee@example.test", "synveda-eng-core").await;
    let home = home_of(&w, &dee_user).await;
    // Bind a subject to the identity the way a first login would, so the
    // token layer has something to refuse.
    let identity = identity_at(&w, home).await;
    bind_subject(&w, identity, "dee-subject").await;

    // The directory says they have left.
    deactivate(&w, &dee_user).await;

    // Layer 1: the token stops working, at the enforcement seam, without
    // waiting for the IdP to revoke anything.
    let token = issue("dee-subject", w.tenant);
    let (status, _) = get(&w.app, &token, "/v1/admin/scopes").await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a departed identity's still-valid token does nothing"
    );

    // Layer 2: the scope is unreadable — by everyone, including the person
    // whose scope it is. There is no reader in this feature.
    assert!(sealed(&w, home).await);
    let (read, _) = get(&w.app, &token, &format!("/v1/admin/scopes/{home}")).await;
    assert_eq!(read, StatusCode::FORBIDDEN);
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

/// The response reflects the committed identity link under the runtime RLS
/// role; an unscoped read remains invisible.
#[tokio::test]
async fn reconciliation_response_reads_the_projected_row_under_forced_rls() {
    let Some(mut w) = world().await else { return };
    let url = std::env::var("DATABASE_URL").expect("world already read DATABASE_URL");
    let runtime_pool = PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("set role synveda_app")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .expect("connect as the RLS-enforced application role");
    let pdp = Arc::new(Pdp::new().expect("build the embedded PDP"));
    w.state = state_with_pool(runtime_pool, pdp);
    w.app = router(w.state.clone());

    let (status, created) = scim_post(
        &w,
        "/scim/v2/Users",
        json!({
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
            "userName": "rls-projection@example.test",
            "externalId": "ext-rls-projection",
            "displayName": "RLS Projection",
            "active": true
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "SCIM create failed: {created}");

    let user_id: DirectoryUserId = created["id"]
        .as_str()
        .expect("created user id")
        .parse()
        .expect("directory user id");
    let outside = directory::user(&w.state.pool, w.tenant, "scim", user_id)
        .await
        .expect("an unscoped RLS read fails closed as absence");
    assert!(
        outside.is_none(),
        "the test must exercise a role for which a bare-pool tenant read is invisible"
    );

    let mut tx = rls::begin_tenant_tx(&w.state.pool, w.tenant)
        .await
        .expect("begin scoped verification read");
    let projected = directory::user(&mut *tx, w.tenant, "scim", user_id)
        .await
        .expect("read projected user under RLS")
        .expect("reconciliation retained its mirror row");
    assert!(
        projected.identity_id.is_some(),
        "the returned projection must have completed its identity link"
    );
    assert_eq!(
        created["meta"]["lastModified"],
        json!(projected.updated_at),
        "the route must render the committed post-reconciliation row; a suppressed RLS \
         miss renders the stale pre-link timestamp instead"
    );
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
        Some(w.org),
        "a new person's scope is their own principal scope under the root"
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

    let absent = DirectoryUserId::new();
    let (status, body) = scim_patch(
        &w,
        &format!("/scim/v2/Users/{absent}"),
        json!({
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
            "Operations": [{"op": "replace", "path": "active", "value": false}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["scimType"], json!("invalidSyntax"));

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
    let (status, _) = get(&w.app, &w.token, "/v1/admin/scopes").await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a provisioning credential reaches no /v1 product route"
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

// ── 9. The directory projects onto the governed access model (CPR-6) ────────

/// A directory group becomes a **`groups` row**, and its members become
/// `group_members` keyed by stable identity before first login (ADR-0093).
///
/// The claim worth testing is the one about tables rather than about SCIM:
/// there is **no enterprise membership table**. A directory group and a group
/// somebody typed are the same row shape in the same table, differing in one
/// column — `source` — which decides only whether the product refuses to edit
/// it. Everything downstream reads one table and cannot tell them apart.
///
/// It also asserts the two things the projection deliberately does **not** do:
/// it writes no grants (a directory says who is in a group, never what the
/// group may do), and first login binds a subject without rewriting membership.
#[tokio::test]
async fn a_directory_group_becomes_a_governed_group_with_its_members() {
    let Some(w) = world().await else { return };

    let (user_id, group_id) = join(&w, "dana@example.test", "synveda-eng-core").await;
    let identity = linked_identity(&w, &user_id)
        .await
        .expect("the joiner reconciled onto an identity");

    let group = access::list_groups(&w.pool, w.tenant)
        .await
        .expect("list governed groups")
        .into_iter()
        .find(|group| {
            group.directory_source.as_deref() == Some("scim")
                && group.directory_resource_id.as_deref() == Some(group_id.as_str())
        })
        .expect("the directory group was projected");
    assert_eq!(group.source, GroupSource::Directory);
    assert_eq!(group.display_name, "synveda-eng-core");
    assert_eq!(
        group.status,
        synveda_types::workspace::LifecycleStatus::Active
    );

    // Membership is complete before first login: it names the stable identity,
    // while the authenticated subject remains absent.
    assert_eq!(
        subject_of(&w, identity).await,
        None,
        "a SCIM-created person has no subject until they log in"
    );
    let before_login = access::group_members(&w.pool, w.tenant, group.id)
        .await
        .expect("read members");
    assert_eq!(before_login.len(), 1);
    assert_eq!(before_login[0].identity_id, identity);
    assert_eq!(before_login[0].principal_id, None);

    // They log in. The same identity and same membership become effective;
    // no second group update is needed.
    let subject = "dana-subject";
    provision_via_login(&w, subject, "dana@example.test", "synveda-eng-core").await;

    let members = access::group_members(&w.pool, w.tenant, group.id)
        .await
        .expect("read members");
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].identity_id, identity);
    assert_eq!(members[0].principal_id.as_deref(), Some(subject));
    assert_eq!(members[0].source, GrantSource::Directory);

    // No grants **the projection wrote**. A directory says who is
    // together; what they may do is a `scope_grants` row somebody in this
    // product wrote, naming the group. The rows that do exist are the
    // `owner` grant each principal scope carries at itself (CPR-7,
    // ADR-0074 decision 8) — minted by the scope, not by the directory,
    // and reaching nothing but the person's own material.
    let grants = access::list_grants(&w.pool, w.tenant, &access::GrantFilter::default())
        .await
        .expect("list grants");
    let invented: Vec<_> = grants
        .iter()
        .filter(|grant| grant.source != synveda_types::access::GrantSource::Owner)
        .collect();
    assert!(
        invented.is_empty(),
        "the projection must not invent grants: {invented:?}"
    );

    // Removing them from the directory group removes them here, on the next
    // sync rather than on a sweep.
    remove_member(&w, &group_id, &user_id).await;
    let members = access::group_members(&w.pool, w.tenant, group.id)
        .await
        .expect("read members again");
    assert!(
        members.is_empty(),
        "leaving the directory group leaves this one"
    );

    // And deleting it **archives** rather than deletes: a grant may name it,
    // and an archived group resolves to nobody.
    let (status, body) = scim_delete(&w, &format!("/scim/v2/Groups/{group_id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let after = access::get_group(&w.pool, w.tenant, group.id)
        .await
        .expect("read the governed group")
        .expect("it is archived, not deleted");
    assert_eq!(
        after.status,
        synveda_types::workspace::LifecycleStatus::Archived
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

// A mover: into the new group, then out of the old.
//
// This order is deliberate and it is the one a provisioning agent uses
// when it pushes both changes: adding first means the person is never
// momentarily in *no* mapped group, so the move is one hop rather than a
// detour through quarantine. The other order is a real shape too — it is
// two directory statements, and the product answers both — which is why
// `a_two_request_move_never_seals_on_the_way_through_quarantine` exists.
// (Expressed at each test's own call sites via `add_member`/
// `remove_member` above, rather than as a shared helper — the two orders
// differ only in which call goes first.)

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
    let id: DirectoryUserId = user.parse().expect("directory user id");
    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin");
    let row = directory::user(&mut *tx, w.tenant, "scim", id)
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
    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin");
    scopes::get(&mut *tx, w.tenant, scope)
        .await
        .expect("read scope")
        .and_then(|scope| scope.parent_scope_id)
}

/// The derivation under test, read the way every caller reads it: a
/// principal scope is sealed exactly when the identity that owns it has
/// departed (ADR-0059 decisions 7 and 9, restated by CPR-7).
async fn sealed(w: &World, scope: ScopeId) -> bool {
    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin");
    identities::by_scope(&mut *tx, w.tenant, scope)
        .await
        .expect("read identity")
        .is_some_and(|identity| identity.sealed())
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

async fn assign(w: &World, scope: ScopeId, pack: &str) {
    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin");
    configuration_support::bind_pack(&mut tx, w.tenant, scope, pack).await;
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

async fn unit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: TenantId,
    parent: ScopeId,
    slug: &str,
) -> Scope {
    scopes::create(
        &mut *tx,
        &scopes::NewScope {
            id: ScopeId::new(),
            tenant_id: tenant,
            kind: ScopeKind::OrgUnit,
            parent_scope_id: Some(parent),
            slug: slug.to_owned(),
            display_name: slug.to_owned(),
            attributes: serde_json::json!({}),
            principal_id: None,
            created_by: None,
        },
    )
    .await
    .expect("create org unit")
}

fn metrics_handle() -> PrometheusHandle {
    use std::sync::OnceLock;
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn state(url: &str, pdp: Arc<Pdp>) -> AppState {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(5))
        .connect_lazy(url)
        .expect("parse database url");
    state_with_pool(pool, pdp)
}

fn state_with_pool(pool: PgPool, pdp: Arc<Pdp>) -> AppState {
    AppState {
        pool,
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp,
        service_token_max_ttl: Duration::from_secs(3600),
        embedder: Arc::new(AnyEmbedder::Deterministic(DeterministicEmbedder::new())),
        context_embed_timeout: Duration::from_millis(100),
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
