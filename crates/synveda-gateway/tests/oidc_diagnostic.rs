//! CPR-45 production-seam acceptance for the provider-neutral OIDC one-shot.
//! The real shipped binary consumes its mounted issuer file, fetches bounded
//! discovery/JWKS metadata and reports only closed stages and outcomes.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Json, Redirect, Response};
use axum::routing::get;
use serde_json::{Value, json};

const KEY_JWK: &str = include_str!("fixtures/idp_key_a.jwk.json");
const CLIENT_SENTINEL: &str = "never-log-diagnostic-client-sentinel";
const PROVIDER_SENTINEL: &str = "http://never-log-provider-sentinel.invalid";

#[derive(Clone)]
struct DiagnosticProvider {
    issuer: String,
    mode: Arc<AtomicU8>,
}

async fn discovery(State(provider): State<DiagnosticProvider>) -> Response {
    match provider.mode.load(Ordering::SeqCst) {
        0 => Json(discovery_document(&provider.issuer)).into_response(),
        1 => Json(discovery_document(PROVIDER_SENTINEL)).into_response(),
        2 => Redirect::temporary("http://127.0.0.1:1/redirects-are-refused").into_response(),
        3 => {
            let body = vec![b'x'; 1_048_577];
            (
                [(header::CONTENT_TYPE, "application/json")],
                Body::from(body),
            )
                .into_response()
        }
        4 => StatusCode::BAD_REQUEST.into_response(),
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn jwks() -> Json<Value> {
    Json(json!({
        "keys": [serde_json::from_str::<Value>(KEY_JWK).expect("public JWK fixture")]
    }))
}

fn discovery_document(issuer: &str) -> Value {
    json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/authorize"),
        "token_endpoint": format!("{issuer}/token"),
        "jwks_uri": format!("{issuer}/jwks"),
        "scopes_supported": ["openid", "profile", "email"],
        "code_challenge_methods_supported": ["S256"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code"],
    })
}

async fn run_diagnostic(issuer_file: &std::path::Path, issuer: &str) -> std::process::Output {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_synveda-oidc-diagnostic"));
    command
        .env_remove("SYNVEDA_OIDC_ISSUERS")
        .env_remove("SYNVEDA_PUBLIC_URL_FILE")
        .env_remove("SYNVEDA_INSECURE_DEVELOPMENT_HTTP_FILE")
        .env_remove("SYNVEDA_OIDC_EXPECTED_ISSUER_FILE")
        .env("SYNVEDA_PUBLIC_URL", "http://app.example.test")
        .env("SYNVEDA_INSECURE_DEVELOPMENT_HTTP", "true")
        .env("SYNVEDA_OIDC_ISSUERS_FILE", issuer_file)
        .env("SYNVEDA_OIDC_EXPECTED_ISSUER", issuer)
        .kill_on_drop(true)
        .output()
        .await
        .expect("run shipped OIDC diagnostic binary")
}

#[tokio::test]
async fn shipped_diagnostic_accepts_exact_contract_and_refuses_hostile_metadata_content_free() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind diagnostic provider");
    let issuer = format!("http://{}/realms/synveda", listener.local_addr().unwrap());
    let provider = DiagnosticProvider {
        issuer: issuer.clone(),
        mode: Arc::new(AtomicU8::new(0)),
    };
    let app = Router::new()
        .route(
            "/realms/synveda/.well-known/openid-configuration",
            get(discovery),
        )
        .route("/realms/synveda/jwks", get(jwks))
        .with_state(provider.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve diagnostic provider");
    });

    let issuer_file = std::env::temp_dir().join(format!(
        "synveda-oidc-diagnostic-{}-{}.json",
        std::process::id(),
        synveda_types::TenantId::new()
    ));
    let config = json!([{
        "issuer": issuer,
        "client_id": CLIENT_SENTINEL,
        "audience": "synveda-test-api",
        "tenant": {"static": {"tenant_id": synveda_types::TenantId::new()}},
    }]);
    std::fs::write(
        &issuer_file,
        serde_json::to_vec(&config).expect("serialize issuer fixture"),
    )
    .expect("write issuer fixture");

    let passed = run_diagnostic(&issuer_file, &provider.issuer).await;
    assert!(passed.status.success(), "{:?}", passed.status);
    assert!(passed.stdout.is_empty());
    assert_eq!(
        String::from_utf8(passed.stderr).expect("UTF-8 stderr"),
        "OIDC diagnostic passed for 1 configured issuer(s)\n"
    );

    for mode in 1..=4 {
        provider.mode.store(mode, Ordering::SeqCst);
        let refused = run_diagnostic(&issuer_file, &provider.issuer).await;
        assert_eq!(refused.status.code(), Some(78), "mode {mode}");
        assert!(refused.stdout.is_empty(), "mode {mode}");
        let stderr = String::from_utf8(refused.stderr).expect("UTF-8 stderr");
        assert!(
            stderr.contains("OIDC discovery diagnostic was refused"),
            "{stderr}"
        );
        assert!(!stderr.contains(CLIENT_SENTINEL), "{stderr}");
        assert!(!stderr.contains("never-log"), "{stderr}");
        assert!(!stderr.contains(&provider.issuer), "{stderr}");
        assert!(
            !stderr.contains(issuer_file.to_string_lossy().as_ref()),
            "{stderr}"
        );
    }

    server.abort();
    let _ = server.await;
    std::fs::remove_file(&issuer_file).expect("remove issuer fixture");
}
