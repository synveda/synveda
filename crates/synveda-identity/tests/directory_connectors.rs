//! AUTH-5 (ADR-0060): the Entra and Okta connectors against in-process mock
//! servers — the vendors' paging, their attribute shapes, and what happens
//! when one of them stops answering half way through.
//!
//! The sharpest assertions here are the two negatives. A pass that fails on
//! page two must still return page one, because ADR-0060 decision 3.1 says
//! presence survives an incomplete pass and the code only honours that if it
//! does not discard what it read on the way out. And a failure message must
//! never carry the credential, because decision 7 makes the outbound secret
//! the first one in this product that has to be recoverable — so it is the
//! one thing that must not reach a log, a span or an error string.
//!
//! No network: everything binds `127.0.0.1:0`, which is the MockIdp
//! discipline this crate already holds itself to.

use std::sync::{Arc, Mutex};

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Json};
use axum::routing::{get, post};
use axum::{Router, http::StatusCode};
use serde_json::{Value, json};
use synveda_identity::directory::entra::EntraConnector;
use synveda_identity::directory::okta::OktaConnector;
use synveda_identity::directory::{DirectoryConnector, Enumeration, Secret};

const CLIENT_SECRET: &str = "entra-client-secret-do-not-log";
const OKTA_TOKEN: &str = "00OktaApiTokenDoNotLog";

async fn spawn(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock");
    let addr = listener.local_addr().expect("mock addr");
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("mock serve");
    });
    format!("http://{addr}")
}

// ── Entra ───────────────────────────────────────────────────────────────────

/// Records the token request so the grant's shape can be asserted.
type Seen = Arc<Mutex<Vec<String>>>;

fn entra_router(base: Arc<Mutex<String>>, seen: Seen, fail_second_page: bool) -> Router {
    #[allow(clippy::needless_pass_by_value)]
    async fn token(
        State((_, seen)): State<(Arc<Mutex<String>>, Seen)>,
        body: String,
    ) -> Json<Value> {
        seen.lock().expect("lock").push(body);
        Json(json!({"access_token": "graph-access-token", "expires_in": 3599}))
    }

    async fn users(
        State((base, _)): State<(Arc<Mutex<String>>, Seen)>,
        Query(query): Query<std::collections::HashMap<String, String>>,
    ) -> Json<Value> {
        let base = base.lock().expect("lock").clone();
        if query.contains_key("$skiptoken") {
            return Json(json!({"value": [{
                "id": "u2", "userPrincipalName": "bob@example.test",
                "displayName": "Bob", "givenName": "Bob", "surname": "Stone",
                "mail": "bob.stone@example.test", "accountEnabled": false
            }]}));
        }
        Json(json!({
            "value": [{
                "id": "u1", "userPrincipalName": "alice@example.test",
                "displayName": "Alice", "givenName": "Alice", "surname": "Ng",
                "mail": "alice.ng@example.test", "accountEnabled": true
            }],
            "@odata.nextLink": format!("{base}/v1.0/users?$skiptoken=page2")
        }))
    }

    async fn users_failing_second_page(
        State((base, _)): State<(Arc<Mutex<String>>, Seen)>,
        Query(query): Query<std::collections::HashMap<String, String>>,
    ) -> axum::response::Response {
        if query.contains_key("$skiptoken") {
            return (StatusCode::TOO_MANY_REQUESTS, "throttled").into_response();
        }
        let base = base.lock().expect("lock").clone();
        Json(json!({
            "value": [{
                "id": "u1", "userPrincipalName": "alice@example.test",
                "displayName": "Alice", "givenName": null, "surname": null,
                "mail": null, "accountEnabled": true
            }],
            "@odata.nextLink": format!("{base}/v1.0/users?$skiptoken=page2")
        }))
        .into_response()
    }

    async fn groups() -> Json<Value> {
        Json(json!({"value": [{"id": "g1", "displayName": "synveda-eng-core"}]}))
    }

    async fn members(Path(group): Path<String>) -> Json<Value> {
        assert_eq!(group, "g1");
        Json(json!({"value": [{"id": "u1"}, {"id": "u-unknown"}]}))
    }

    let users_route = if fail_second_page {
        get(users_failing_second_page)
    } else {
        get(users)
    };
    Router::new()
        .route("/{tenant}/oauth2/v2.0/token", post(token))
        .route("/v1.0/users", users_route)
        .route("/v1.0/groups", get(groups))
        .route("/v1.0/groups/{group}/members", get(members))
        .with_state((base, seen))
}

#[tokio::test]
async fn entra_pages_users_and_preserves_stable_group_membership() {
    let base = Arc::new(Mutex::new(String::new()));
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let url = spawn(entra_router(Arc::clone(&base), Arc::clone(&seen), false)).await;
    *base.lock().expect("lock") = url.clone();

    let connector = EntraConnector::new(
        "tenant-1".to_owned(),
        "client-1".to_owned(),
        Secret::new(CLIENT_SECRET),
        Some(url.clone()),
        Some(url),
    )
    .expect("build connector");
    assert_eq!(connector.name(), "entra");

    let enumeration = connector.enumerate().await;
    assert!(
        enumeration.is_complete(),
        "every page answered, so the pass may speak about absence"
    );
    let users = &enumeration.snapshot().users;
    assert_eq!(users.len(), 2, "the nextLink was followed");

    let alice = &users[0];
    assert_eq!(alice.external_id, "u1");
    assert_eq!(alice.user_name, "alice@example.test");
    assert!(alice.active);
    assert_eq!(alice.work_email.as_deref(), Some("alice.ng@example.test"));
    let groups = &enumeration.snapshot().groups;
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].external_id, "g1");
    assert_eq!(groups[0].display_name, "synveda-eng-core");
    assert_eq!(groups[0].member_external_ids, vec!["u1", "u-unknown"]);

    // `accountEnabled: false` is an act and survives as one. Membership is
    // retained by stable external id even when a referenced user was not in
    // this enumeration; projection decides whether the identity exists.
    let bob = &users[1];
    assert!(!bob.active);

    let grant = seen
        .lock()
        .expect("lock")
        .first()
        .cloned()
        .expect("a token request");
    assert!(grant.contains("grant_type=client_credentials"));
    assert!(
        grant.contains("scope=https"),
        "the .default scope is asked for"
    );
}

#[tokio::test]
async fn an_entra_pass_that_fails_half_way_keeps_what_it_read() {
    let base = Arc::new(Mutex::new(String::new()));
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let url = spawn(entra_router(Arc::clone(&base), seen, true)).await;
    *base.lock().expect("lock") = url.clone();

    let connector = EntraConnector::new(
        "tenant-1".to_owned(),
        "client-1".to_owned(),
        Secret::new(CLIENT_SECRET),
        Some(url.clone()),
        Some(url),
    )
    .expect("build connector");

    let enumeration = connector.enumerate().await;
    let Enumeration::Partial { snapshot, failure } = &enumeration else {
        panic!("a throttled second page is not a complete pass");
    };
    assert_eq!(
        snapshot.users.len(),
        1,
        "page one's user was listed and is present; discarding them would \
         make the next pass believe they had gone missing"
    );
    assert_eq!(snapshot.users[0].user_name, "alice@example.test");
    assert!(failure.contains("429"), "the status is reported: {failure}");
    assert!(
        !enumeration.is_complete(),
        "and nothing may conclude absence from it"
    );
}

#[tokio::test]
async fn a_refused_entra_token_never_echoes_the_client_secret() {
    // The token request is the one call in a pass whose *body* carries the
    // secret, and a vendor error body routinely echoes the request it
    // refused. This is the failure path most likely to write a credential
    // into a log line.
    async fn refuse(body: String) -> impl IntoResponse {
        (
            StatusCode::UNAUTHORIZED,
            // A deliberately hostile mock: it reflects the request body,
            // secret and all, exactly as a careless IdP might.
            format!("invalid_client, request was: {body}"),
        )
    }
    let url = spawn(Router::new().route("/{tenant}/oauth2/v2.0/token", post(refuse))).await;

    let connector = EntraConnector::new(
        "tenant-1".to_owned(),
        "client-1".to_owned(),
        Secret::new(CLIENT_SECRET),
        Some(url.clone()),
        Some(url),
    )
    .expect("build connector");

    let enumeration = connector.enumerate().await;
    let Enumeration::Partial { snapshot, failure } = &enumeration else {
        panic!("a refused token is not a complete pass");
    };
    assert!(snapshot.users.is_empty());
    assert!(
        !failure.contains(CLIENT_SECRET),
        "the failure must not carry the credential: {failure}"
    );
    assert!(failure.contains("401"), "but it must say what happened");
}

// ── Okta ────────────────────────────────────────────────────────────────────

fn okta_router(base: Arc<Mutex<String>>, seen: Seen) -> Router {
    async fn users(
        State((base, seen)): State<(Arc<Mutex<String>>, Seen)>,
        headers: axum::http::HeaderMap,
        Query(query): Query<std::collections::HashMap<String, String>>,
    ) -> axum::response::Response {
        seen.lock().expect("lock").push(
            headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned(),
        );
        if query.contains_key("after") {
            return Json(json!([{
                "id": "u2", "status": "DEPROVISIONED",
                "profile": {"login": "bob@example.test", "email": "bob@example.test",
                            "firstName": "Bob", "lastName": "Stone", "displayName": "Bob"}
            }]))
            .into_response();
        }
        let base = base.lock().expect("lock").clone();
        // `self` first, deliberately: a client that takes the first URL
        // rather than the one whose rel says `next` loops on page one.
        let link = format!(
            "<{base}/api/v1/users>; rel=\"self\", <{base}/api/v1/users?after=u1>; rel=\"next\""
        );
        (
            [(axum::http::header::LINK, link)],
            Json(json!([{
                "id": "u1", "status": "ACTIVE",
                "profile": {"login": "alice@example.test", "email": "alice@example.test",
                            "firstName": "Alice", "lastName": "Ng", "displayName": "Alice"}
            }])),
        )
            .into_response()
    }

    async fn groups() -> Json<Value> {
        Json(json!([{"id": "g1", "profile": {"name": "synveda-eng-core"}}]))
    }

    async fn group_users(Path(group): Path<String>) -> Json<Value> {
        assert_eq!(group, "g1");
        Json(json!([{
            "id": "u1", "status": "ACTIVE",
            "profile": {"login": "alice@example.test"}
        }]))
    }

    Router::new()
        .route("/api/v1/users", get(users))
        .route("/api/v1/groups", get(groups))
        .route("/api/v1/groups/{group}/users", get(group_users))
        .with_state((base, seen))
}

#[tokio::test]
async fn okta_follows_the_link_header_and_maps_its_statuses() {
    let base = Arc::new(Mutex::new(String::new()));
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let url = spawn(okta_router(Arc::clone(&base), Arc::clone(&seen))).await;
    *base.lock().expect("lock") = url.clone();

    let connector = OktaConnector::new(url, Secret::new(OKTA_TOKEN)).expect("build connector");
    assert_eq!(connector.name(), "okta");

    let enumeration = connector.enumerate().await;
    assert!(enumeration.is_complete());
    let users = &enumeration.snapshot().users;
    assert_eq!(
        users.len(),
        2,
        "rel=\"next\" was followed, rel=\"self\" was not"
    );

    assert_eq!(users[0].user_name, "alice@example.test");
    assert!(users[0].active, "ACTIVE is here");
    assert_eq!(enumeration.snapshot().groups.len(), 1);
    assert_eq!(enumeration.snapshot().groups[0].external_id, "g1");
    assert_eq!(
        enumeration.snapshot().groups[0].member_external_ids,
        vec!["u1"]
    );
    assert!(!users[1].active, "DEPROVISIONED is Okta's leaver");
    assert_eq!(users[1].family_name.as_deref(), Some("Stone"));

    let auth = seen
        .lock()
        .expect("lock")
        .first()
        .cloned()
        .expect("a request");
    assert!(auth.starts_with("SSWS "), "Okta's own scheme: {auth}");
}

#[tokio::test]
async fn a_refused_okta_read_never_echoes_the_api_token() {
    async fn refuse(headers: axum::http::HeaderMap) -> impl IntoResponse {
        let echoed = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        (StatusCode::FORBIDDEN, format!("denied for {echoed}"))
    }
    let url = spawn(Router::new().route("/api/v1/users", get(refuse))).await;

    let connector = OktaConnector::new(url, Secret::new(OKTA_TOKEN)).expect("build connector");
    let enumeration = connector.enumerate().await;
    let Enumeration::Partial { snapshot, failure } = &enumeration else {
        panic!("a 403 is not a complete pass");
    };
    assert!(snapshot.users.is_empty());
    assert!(
        !failure.contains(OKTA_TOKEN),
        "the failure must not carry the credential: {failure}"
    );
    assert!(failure.contains("403"));
}
