//! Serving the console bundle (CNSL-1, ADR-0056 decision 1).
//!
//! The bundle itself is Vite's problem. What is asserted here is what the
//! *gateway* adds around it, all of which is security-shaped:
//!
//!   * the SPA fallback is scoped to the console's own prefix, so a typo
//!     under `/v1` stays a JSON 404 rather than becoming an HTML page;
//!   * the Content-Security-Policy reaches the browser on every console
//!     response, because it is what makes ADR-0056 decision 8's "no
//!     third-party fetch" an enforcement rather than a claim;
//!   * `ServeDir` does not serve outside the directory it was given.
//!
//! These need no database: the console plane is unauthenticated by nature
//! (it is the page a signed-out operator lands on), so the state's pool is
//! deliberately pointed at an unreachable URL — a test that passed only
//! because a query succeeded would be testing the wrong thing.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_types::TenantId;
use tower::ServiceExt;

/// A URL that parses but connects nowhere: nothing here may need a database.
const UNREACHABLE_URL: &str = "postgres://nobody:nothing@127.0.0.1:1/void";

const INDEX_HTML: &str = "<!doctype html><title>Synveda console</title><div id=root></div>";

/// Serialises tests: the Prometheus recorder is process-global, and these
/// tests set a process-wide environment variable (same rationale as
/// tests/tenant_resolution.rs, plus `SYNVEDA_CONSOLE_DIR`).
async fn serial() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

/// Writes a bundle-shaped directory and points the gateway at it.
fn bundle(name: &str) -> tempdir::TempDir {
    let dir = tempdir::TempDir::new(name);
    std::fs::create_dir_all(dir.path().join("assets")).expect("create assets dir");
    std::fs::write(dir.path().join("index.html"), INDEX_HTML).expect("write index.html");
    std::fs::write(
        dir.path().join("assets").join("index-abc123.js"),
        "export const ok = 1;\n",
    )
    .expect("write asset");
    // The gateway reads this once, when the router is built.
    unsafe { std::env::set_var("SYNVEDA_CONSOLE_DIR", dir.path()) };
    dir
}

fn state() -> AppState {
    AppState {
        pool: PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_millis(50))
            .connect_lazy(UNREACHABLE_URL)
            .expect("parse database url"),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(b"console-serving-test")),
        login: None,
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp: Arc::new(synveda_policy::Pdp::new().expect("build the embedded PDP")),
        scope_chains: Arc::new(synveda_store::ScopeChainCache::new()),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index: Arc::new(
            synveda_retrieval::SearchIndex::open(
                std::env::temp_dir()
                    .join("synveda-gateway-tests")
                    .join(TenantId::new().to_string()),
            )
            .expect("open search index"),
        ),
        embedder: Arc::new(synveda_ingest::embedding::AnyEmbedder::Deterministic(
            synveda_ingest::embedding::DeterministicEmbedder::new(),
        )),
        inject_embed_timeout: Duration::from_millis(100),
    }
}

async fn body_string(response: axum::response::Response) -> String {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read response body")
        .to_bytes();
    String::from_utf8(bytes.to_vec()).expect("utf-8 body")
}

async fn get(uri: &str) -> axum::response::Response {
    router(state())
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .expect("route")
}

// ── Serving ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_bundle_is_served_from_the_gateways_own_origin() {
    let _guard = serial().await;
    let _dir = bundle("console-serves");

    let response = get("/console/").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_string(response).await.contains("Synveda console"));

    let asset = get("/console/assets/index-abc123.js").await;
    assert_eq!(asset.status(), StatusCode::OK);
}

/// Client-side routing means `/console/proposals/42` is a real page the
/// filesystem has never heard of.
#[tokio::test]
async fn an_unknown_console_path_falls_back_to_the_app() {
    let _guard = serial().await;
    let _dir = bundle("console-fallback");

    let response = get("/console/proposals/0198abcd").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_string(response).await.contains("Synveda console"));
}

/// The fallback is scoped to the prefix, and this is the assertion that
/// keeps it there. A fallback that reached the API would turn every `/v1`
/// typo into an HTML page and every client's error handling into a parse
/// failure — the failure mode is a client reporting "invalid JSON" for
/// what was really a 404.
#[tokio::test]
async fn the_fallback_does_not_reach_the_api_or_the_ops_plane() {
    let _guard = serial().await;
    let _dir = bundle("console-scope");

    for uri in ["/v1/nonexistent", "/auth/nonexistent", "/nonexistent"] {
        let response = get(uri).await;
        let status = response.status();
        let body = body_string(response).await;
        assert!(
            !body.contains("<!doctype html>"),
            "{uri} was answered with the console's HTML"
        );
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::UNAUTHORIZED,
            "{uri} got {status}"
        );
    }
}

/// `/v1` keeps its own 401 even with a console mounted: the console plane
/// is unauthenticated, and merging it must not have made anything else so.
#[tokio::test]
async fn mounting_the_console_authenticates_nothing_differently() {
    let _guard = serial().await;
    let _dir = bundle("console-authn");

    let response = get("/v1/whoami").await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ── The headers around it ────────────────────────────────────────────────────

/// ADR-0056 decision 8, enforced rather than promised. A bundle that grew a
/// CDN reference fails visibly in a browser instead of quietly working
/// everywhere except the air-gapped deployments this product is sold into.
#[tokio::test]
async fn every_console_response_carries_the_policy_that_forbids_a_third_party() {
    let _guard = serial().await;
    let _dir = bundle("console-csp");

    for uri in [
        "/console/",
        "/console/assets/index-abc123.js",
        "/console/deep/link",
    ] {
        let response = get(uri).await;
        let headers = response.headers();
        let csp = headers
            .get(header::CONTENT_SECURITY_POLICY)
            .unwrap_or_else(|| panic!("{uri} carried no Content-Security-Policy"))
            .to_str()
            .expect("ascii policy");

        assert!(csp.contains("default-src 'none'"), "{uri}: {csp}");
        assert!(csp.contains("connect-src 'self'"), "{uri}: {csp}");
        assert!(!csp.contains("unsafe-inline"), "{uri}: {csp}");
        assert!(!csp.contains("unsafe-eval"), "{uri}: {csp}");

        assert_eq!(
            headers.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(),
            "nosniff",
            "{uri}"
        );
        assert_eq!(
            headers.get(header::REFERRER_POLICY).unwrap(),
            "no-referrer",
            "{uri}"
        );
        assert_eq!(
            headers.get(header::CACHE_CONTROL).unwrap(),
            "no-cache",
            "{uri}"
        );
    }
}

/// The API must not inherit the console's headers. A JSON client has no use
/// for a script policy, and a `no-cache` bolted onto every `/v1` response
/// would be this feature quietly changing the product's caching contract.
#[tokio::test]
async fn the_api_does_not_inherit_the_consoles_headers() {
    let _guard = serial().await;
    let _dir = bundle("console-header-scope");

    let response = get("/v1/whoami").await;
    assert!(
        response
            .headers()
            .get(header::CONTENT_SECURITY_POLICY)
            .is_none()
    );
    assert!(response.headers().get(header::REFERRER_POLICY).is_none());
}

// ── Path traversal ───────────────────────────────────────────────────────────

/// `ServeDir` refuses to escape its root, and this pins it. The gateway
/// runs beside a database URL and a private key; a static file server that
/// could be walked upwards would be the worst bug in the product.
#[tokio::test]
async fn the_bundle_directory_cannot_be_escaped() {
    let _guard = serial().await;
    let dir = bundle("console-traversal");
    // A file next to the bundle, of the shape a real deployment has.
    let secret = dir.path().parent().unwrap().join("outside.txt");
    std::fs::write(&secret, "not for you").expect("write the file outside");

    for uri in [
        "/console/../outside.txt",
        "/console/..%2foutside.txt",
        "/console/%2e%2e/outside.txt",
        "/console/assets/../../outside.txt",
    ] {
        let response = get(uri).await;
        let status = response.status();
        let body = body_string(response).await;
        assert!(
            !body.contains("not for you"),
            "{uri} escaped the bundle directory"
        );
        assert!(
            status != StatusCode::OK || body.contains("Synveda console"),
            "{uri}: {status}"
        );
    }

    std::fs::remove_file(&secret).ok();
}

// ── No bundle ────────────────────────────────────────────────────────────────

/// A deployment that ships no console, or a developer who has not built
/// one, gets a 404 under the prefix and a working product everywhere else.
/// Refusing to boot would make a static asset a dependency of the audit log.
#[tokio::test]
async fn a_missing_bundle_is_a_404_and_not_a_broken_gateway() {
    let _guard = serial().await;
    let empty = tempdir::TempDir::new("console-absent");
    unsafe { std::env::set_var("SYNVEDA_CONSOLE_DIR", empty.path()) };

    assert_eq!(get("/console/").await.status(), StatusCode::NOT_FOUND);
    // The product still works.
    assert_eq!(get("/healthz").await.status(), StatusCode::OK);
    assert_eq!(get("/v1/whoami").await.status(), StatusCode::UNAUTHORIZED);
}

/// A minimal self-cleaning temporary directory. Written here rather than
/// taken as a dependency: three functions do not justify a crate, and the
/// gateway's dev-dependency list is part of what `cargo deny` audits.
mod tempdir {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new(name: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join("synveda-console-tests")
                .join(format!("{name}-{}-{unique}", std::process::id()));
            std::fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }
}
