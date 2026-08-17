//! CPR-4: the OpenAPI contract is authoritative (ADR-0071 decision 7).
//!
//! Three things have to be true for that sentence to mean anything, and this
//! suite is each of them:
//!
//! 1. **The committed document is the tree's document.** `docs/api/openapi.json`
//!    is derived from the handlers, so a DTO change that nobody regenerated is
//!    a failing test rather than a stale file somebody finds later.
//! 2. **Every documented path is actually mounted.** A contract that describes
//!    a route the router does not serve is worse than no contract: a client
//!    generated from it fails at runtime with a 404 it cannot explain.
//! 3. **The document does not quietly grow or shrink.** The set of paths is
//!    asserted here explicitly, so adding a route to this plane without
//!    documenting it — or documenting one and forgetting to mount it — fails.
//!
//! Needs no database: every assertion is either pure or an unauthenticated
//! request that the tenant middleware refuses before anything opens a
//! connection. That is deliberate — the contract check must run in CI, and CI
//! has no Postgres.
//!
//! To refresh the document after changing a DTO or a handler annotation:
//!
//! ```sh
//! SYNVEDA_WRITE_OPENAPI=1 cargo test -p synveda-gateway --test openapi
//! node scripts/generate-api-types.mjs
//! ```

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::{openapi, telemetry};
use synveda_identity::Hs256Verifier;
use tower::ServiceExt;

/// The committed document, relative to the workspace root.
const DOCUMENT: &str = "../../docs/api/openapi.json";

/// Every path CPR-4 puts on the contract. Written out rather than derived from
/// the document, because a check that read the document to decide what the
/// document should contain would pass for any document at all.
const DECLARED_PATHS: &[&str] = &[
    "/v1/me",
    "/v1/projects/{project_id}",
    "/v1/projects/{project_id}/repositories",
    "/v1/projects/{project_id}/repositories/{repository_id}",
    "/v1/workspaces",
    "/v1/workspaces/{workspace_id}",
    "/v1/workspaces/{workspace_id}/projects",
];

const SECRET: &[u8] = b"cpr-4-openapi-test-secret";

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

/// A router over a pool that never connects. Every request this suite makes is
/// refused by the tenant middleware first, so the pool is never used — which
/// is what makes the whole suite runnable without a database.
fn app() -> Router {
    let state = AppState {
        pool: PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_millis(50))
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .expect("parse database url"),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp: Arc::new(synveda_policy::Pdp::new().expect("build the embedded PDP")),
        scope_chains: Arc::new(synveda_store::ScopeChainCache::new()),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index: Arc::new(
            synveda_retrieval::SearchIndex::open(
                std::env::temp_dir()
                    .join("synveda-openapi-tests")
                    .join(synveda_types::TenantId::new().to_string()),
            )
            .expect("open search index"),
        ),
        embedder: Arc::new(synveda_ingest::embedding::AnyEmbedder::Deterministic(
            synveda_ingest::embedding::DeterministicEmbedder::new(),
        )),
        inject_embed_timeout: Duration::from_millis(100),
        keys: Arc::new(synveda_store::keys::KeyRing::new(
            synveda_crypto::Kms::Disabled,
        )),
    };
    router(state)
}

/// The committed document must be the one this tree produces.
#[test]
fn the_committed_document_is_the_trees_document() {
    let derived = openapi::document();
    if std::env::var("SYNVEDA_WRITE_OPENAPI").is_ok() {
        std::fs::write(DOCUMENT, &derived).expect("write the OpenAPI document");
        eprintln!("wrote {DOCUMENT}");
        return;
    }
    let committed = std::fs::read_to_string(DOCUMENT).unwrap_or_else(|err| {
        panic!(
            "{DOCUMENT} is missing ({err}). Generate it with \
             `SYNVEDA_WRITE_OPENAPI=1 cargo test -p synveda-gateway --test openapi`"
        )
    });
    assert_eq!(
        committed, derived,
        "{DOCUMENT} is out of date with the handlers it describes. Refresh it with \
         `SYNVEDA_WRITE_OPENAPI=1 cargo test -p synveda-gateway --test openapi`, then \
         regenerate the frontend types with `node scripts/generate-api-types.mjs`."
    );
}

/// The document declares exactly this plane — no more (a path nobody mounted)
/// and no fewer (a route nobody documented).
#[test]
fn the_document_declares_exactly_this_plane() {
    let mut declared = openapi::declared_paths();
    declared.sort();
    let mut expected: Vec<String> = DECLARED_PATHS
        .iter()
        .map(|path| (*path).to_owned())
        .collect();
    expected.sort();
    assert_eq!(
        declared, expected,
        "the OpenAPI document and this suite's list of CPR-4 paths disagree. \
         A route added to this plane belongs in both."
    );
}

/// Every documented path is mounted on the router.
///
/// The evidence is the *status*: an unmounted path 404s from axum's own
/// matcher, and a mounted one behind the tenant middleware answers 401 for a
/// request with no credential. So 401 proves the route exists and 404 proves
/// the document is describing something the gateway does not serve.
#[tokio::test]
async fn every_documented_path_is_mounted() {
    let app = app();
    let id = uuid_placeholder();
    for (path, method) in documented_operations() {
        let concrete = path
            .replace("{workspace_id}", &id)
            .replace("{project_id}", &id)
            .replace("{repository_id}", &id);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method.as_str())
                    .uri(&concrete)
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router responds");
        assert_ne!(
            response.status(),
            StatusCode::NOT_FOUND,
            "{method} {path} is in the contract but not mounted on the router"
        );
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {path} answered {} to an unauthenticated request; every \
             documented route sits behind the tenant middleware",
            response.status()
        );
    }
}

/// A path the document does not declare must not be reachable on this plane —
/// the same check from the other side, so a mounted-but-undocumented sibling
/// route is caught rather than assumed absent.
#[tokio::test]
async fn a_path_this_plane_does_not_declare_is_not_mounted() {
    let app = app();
    let id = uuid_placeholder();
    for path in [
        "/v1/workspaces/{workspace_id}/repositories",
        "/v1/projects",
        "/v1/workspaces/{workspace_id}/projects/{project_id}",
    ] {
        let concrete = path
            .replace("{workspace_id}", &id)
            .replace("{project_id}", &id);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&concrete)
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router responds");
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "{path} is mounted but not on the contract"
        );
    }
}

/// The document is well formed in the ways the frontend generator depends on:
/// unique operation ids, a resolvable `$ref` everywhere, a declared security
/// scheme, and an error body on every 4xx.
#[test]
fn the_document_is_generatable() {
    let document: Value = serde_json::from_str(&openapi::document()).expect("valid JSON");
    let schemas = document["components"]["schemas"]
        .as_object()
        .expect("components.schemas");
    assert!(
        document["components"]["securitySchemes"]["bearer"].is_object(),
        "the bearer scheme must be declared once, for every path"
    );

    let mut operation_ids: Vec<String> = Vec::new();
    let mut refs: Vec<String> = Vec::new();
    collect_refs(&document, &mut refs);
    for reference in &refs {
        let name = reference
            .strip_prefix("#/components/schemas/")
            .unwrap_or_else(|| panic!("unexpected $ref target {reference}"));
        assert!(
            schemas.contains_key(name),
            "{reference} resolves to nothing; the generator would emit `unknown`"
        );
    }

    for (path, item) in document["paths"].as_object().expect("paths") {
        for (method, operation) in item.as_object().expect("path item") {
            let id = operation["operationId"]
                .as_str()
                .unwrap_or_else(|| panic!("{method} {path} has no operationId"));
            assert!(
                !operation_ids.contains(&id.to_owned()),
                "duplicate operationId {id:?}: the generated operation map would lose one"
            );
            operation_ids.push(id.to_owned());
            assert!(
                operation["security"].is_array(),
                "{method} {path} declares no security; every route here needs a bearer"
            );
            for (code, response) in operation["responses"].as_object().expect("responses") {
                if code.starts_with('4') {
                    assert_eq!(
                        response["content"]["application/json"]["schema"]["$ref"]
                            .as_str()
                            .unwrap_or_default(),
                        "#/components/schemas/ApiErrorBody",
                        "{method} {path} {code} must document the taxonomy body"
                    );
                }
            }
        }
    }
    assert_eq!(
        operation_ids.len(),
        12,
        "CPR-4 declares twelve operations: {operation_ids:?}"
    );
}

/// Every (path, method) pair the document declares.
fn documented_operations() -> Vec<(String, String)> {
    let document: Value = serde_json::from_str(&openapi::document()).expect("valid JSON");
    let mut pairs = Vec::new();
    for (path, item) in document["paths"].as_object().expect("paths") {
        for method in item.as_object().expect("path item").keys() {
            pairs.push((path.clone(), method.to_uppercase()));
        }
    }
    pairs
}

fn collect_refs(value: &Value, into: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, nested) in map {
                if key == "$ref"
                    && let Some(reference) = nested.as_str()
                {
                    into.push(reference.to_owned());
                }
                collect_refs(nested, into);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| collect_refs(item, into)),
        _ => {}
    }
}

fn uuid_placeholder() -> String {
    synveda_types::TenantId::new().to_string()
}
