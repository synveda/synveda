//! CPR-27 acceptance evidence for bounded OKF v0.2 exchange.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_store::{access, identities, rls, scopes, tenants};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::{GrantId, IdentityId, IdentityKind, TenantId, TenantStatus};
use tower::ServiceExt;

#[path = "support/configuration.rs"]
mod configuration_support;

const SECRET: &[u8] = b"cpr-27-okf-api";
const ADMIN: &str = "cpr27-admin";

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

fn state(url: &str) -> AppState {
    AppState {
        pool: PgPoolOptions::new()
            .max_connections(6)
            .acquire_timeout(Duration::from_secs(5))
            .connect_lazy(url)
            .expect("parse database url"),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp: Arc::new(synveda_policy::Pdp::new().expect("build embedded PDP")),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index: Arc::new(
            synveda_retrieval::SearchIndex::open(
                std::env::temp_dir()
                    .join("synveda-cpr27-tests")
                    .join(TenantId::new().to_string()),
            )
            .expect("open search sidecar"),
        ),
        embedder: Arc::new(synveda_ingest::embedding::AnyEmbedder::Deterministic(
            synveda_ingest::embedding::DeterministicEmbedder::new(),
        )),
        inject_embed_timeout: Duration::from_millis(100),
        keys: Arc::new(synveda_store::keys::KeyRing::new(
            synveda_crypto::Kms::Disabled,
        )),
    }
}

fn issue(subject: &str, tenant_id: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue(subject, tenant_id, Duration::from_secs(300))
}

async fn admitted_tenant() -> Option<(AppState, TenantId)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("skipping CPR-27 OKF API test: DATABASE_URL is not set");
            return None;
        }
    };
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");
    let tenant_id = TenantId::new();
    tenants::create(
        &pool,
        tenant_id,
        &format!("cpr27-{}", tenant_id.as_uuid().simple()),
        "CPR-27 OKF acceptance",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    let mut tx = rls::begin_tenant_tx(&pool, tenant_id)
        .await
        .expect("begin bootstrap transaction");
    let root = scopes::ensure_tenant_root(&mut tx, tenant_id)
        .await
        .expect("mint tenant root");
    let own = scopes::ensure_principal_scope(&mut tx, tenant_id, ADMIN, ADMIN)
        .await
        .expect("mint principal scope");
    identities::create(
        &mut tx,
        IdentityId::new(),
        tenant_id,
        Some(ADMIN),
        IdentityKind::User,
        None,
        Some(ADMIN),
        own.id,
    )
    .await
    .expect("create identity");
    access::create_grant(
        &mut *tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id,
            scope_id: root.id,
            subject: GrantSubject::Principal {
                principal_id: ADMIN.to_owned(),
            },
            role_key: RoleKey::Administrator,
            source: GrantSource::Automation,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .expect("seed administrator grant");
    configuration_support::bind_tenant_pack(&mut tx, tenant_id, synveda_policy::STANDARD).await;
    tx.commit().await.expect("commit bootstrap");
    Some((state(&url), tenant_id))
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    key: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
    if let Some(key) = key {
        request = request.header("idempotency-key", key);
    }
    let request = if let Some(body) = body {
        request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("build JSON request")
    } else {
        request.body(Body::empty()).expect("build request")
    };
    let response = app.clone().oneshot(request).await.expect("router responds");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect response")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| {
            panic!(
                "response should be JSON: {}",
                String::from_utf8_lossy(&bytes)
            )
        })
    };
    (status, value)
}

async fn workspace_and_project(app: &Router, token: &str) -> (String, String) {
    let (status, workspace) = call(
        app,
        "POST",
        "/v1/workspaces",
        token,
        Some("okf-workspace"),
        Some(json!({"slug": "okf-pulse", "display_name": "OKF Pulse"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workspace}");
    let workspace_id = workspace["id"].as_str().expect("workspace id");
    let (status, project) = call(
        app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/projects"),
        token,
        Some("okf-project"),
        Some(json!({"slug": "pulseboard", "display_name": "PulseBoard"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    (
        workspace_id.to_owned(),
        project["id"].as_str().expect("project id").to_owned(),
    )
}

fn entry(path: &str, content: &str) -> Value {
    json!({
        "logical_path": path,
        "kind": "file",
        "content_base64": STANDARD.encode(content),
    })
}

fn bundle() -> Value {
    json!({
        "source_kind": "git",
        "source_locator": "https://example.test/pulseboard.git",
        "source_revision": "c0ffee42",
        "encoding": "entries",
        "entries": [
            entry("index.md", "---\nokf_version: \"0.2\"\n---\n\n# PulseBoard\n"),
            entry("decisions/webhooks.md", r#"---
type: Decision
title: Webhook identity
description: Provider event IDs deduplicate webhook delivery.
tags: [Webhooks, Delivery]
generated: { by: human:alice, at: 2026-08-20T10:00:00Z }
status: stable
sources:
  - id: provider-contract
    resource: https://docs.example.test/events
vendor_extension: { owner: platform, level: 7 }
---

Use the provider event ID. See [request tracing](../conventions/tracing.md).
"#),
            entry("conventions/tracing.md", r#"---
type: Convention
title: Request tracing
description: Public requests use traceparent.
verified: { by: human:bob, at: 2026-08-24T09:00:00Z }
---

Use traceparent on public requests.
"#),
        ],
    })
}

#[tokio::test]
async fn okf_plan_materialization_vedaflow_provenance_export_and_isolation() {
    let _guard = serial().await;
    let Some((state, tenant)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant);
    let (_, project_id) = workspace_and_project(&app, &token).await;

    let (status, plan) = call(
        &app,
        "POST",
        &format!("/v1/projects/{project_id}/okf/imports"),
        &token,
        Some("okf-plan-one"),
        Some(bundle()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{plan}");
    assert_eq!(plan["job"]["format_version"], "0.2");
    assert_eq!(
        plan["job"]["specification_commit"],
        synveda_okf::OKF_SPEC_COMMIT
    );
    assert_eq!(plan["artifacts"].as_array().map(Vec::len), Some(3));
    assert_eq!(plan["mappings"].as_array().map(Vec::len), Some(2));
    let webhook_mapping = plan["mappings"]
        .as_array()
        .expect("mappings")
        .iter()
        .find(|mapping| mapping["content"]["title"] == "Webhook identity")
        .expect("webhook mapping");
    assert_eq!(webhook_mapping["classification"], "addition");
    assert_eq!(
        webhook_mapping["content"]["metadata"]["okf"]["frontmatter"]["vendor_extension"]["owner"],
        "platform"
    );
    let job_id = plan["job"]["id"].as_str().expect("job id").to_owned();
    let before: i64 =
        sqlx::query_scalar("select count(*) from knowledge_items where tenant_id = $1")
            .bind(tenant.as_uuid())
            .fetch_one(&state.pool)
            .await
            .expect("count Knowledge after planning");
    assert_eq!(before, 0, "a dry-run plan must not publish Knowledge");

    let (status, replay) = call(
        &app,
        "POST",
        &format!("/v1/projects/{project_id}/okf/imports"),
        &token,
        Some("okf-plan-one"),
        Some(bundle()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["job"]["id"], job_id);
    let (status, unchanged) = call(
        &app,
        "POST",
        &format!("/v1/projects/{project_id}/okf/imports"),
        &token,
        Some("okf-plan-same-content"),
        Some(bundle()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{unchanged}");
    assert_eq!(unchanged["job"]["id"], job_id);

    let artifact_id = plan["artifacts"][0]["id"].as_str().expect("artifact id");
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant)
        .await
        .expect("begin immutability probe");
    let error = sqlx::query(
        "update import_artifacts set body_markdown = 'tampered' where tenant_id = $1 and id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(uuid::Uuid::parse_str(artifact_id).expect("artifact uuid"))
    .execute(&mut *tx)
    .await
    .expect_err("artifact must be immutable");
    assert!(error.to_string().contains("append-only"), "{error}");
    tx.rollback().await.expect("rollback immutability probe");

    let (status, materialized) = call(
        &app,
        "POST",
        &format!("/v1/okf/imports/{job_id}/materialize"),
        &token,
        Some("okf-materialize"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{materialized}");
    assert_eq!(materialized["batch"]["source_kind"], "okf_import");
    assert_eq!(materialized["batch"]["event_count"], 0);
    assert_eq!(materialized["candidates"].as_array().map(Vec::len), Some(2));
    let candidate = materialized["candidates"]
        .as_array()
        .expect("candidates")
        .iter()
        .find(|candidate| candidate["content"]["title"] == "Webhook identity")
        .expect("webhook candidate");
    assert_eq!(candidate["state"], "pending");
    assert_eq!(candidate["source_kind"], "okf_import");
    assert_eq!(candidate["source_event_ids"], json!([]));
    assert_eq!(
        candidate["source_artifact_ids"].as_array().map(Vec::len),
        Some(1)
    );
    let still_none: i64 =
        sqlx::query_scalar("select count(*) from knowledge_items where tenant_id = $1")
            .bind(tenant.as_uuid())
            .fetch_one(&state.pool)
            .await
            .expect("count Knowledge after materialisation");
    assert_eq!(still_none, 0, "materialisation may only create candidates");

    let candidate_id = candidate["id"].as_str().expect("candidate id");
    let (status, accepted) = call(
        &app,
        "POST",
        &format!("/v1/capture-candidates/{candidate_id}/accept"),
        &token,
        Some("okf-accept"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{accepted}");
    assert_eq!(accepted["candidate"]["resulting_outcome"], "applied");
    let item_id = accepted["candidate"]["resulting_knowledge_item_id"]
        .as_str()
        .expect("resulting item");

    let (status, sources) = call(
        &app,
        "GET",
        &format!("/v1/knowledge/{item_id}/sources"),
        &token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sources}");
    let published_sources = sources["sources"].as_array().expect("published sources");
    assert_eq!(published_sources.len(), 2);
    assert!(published_sources.iter().any(|source| {
        source["source_type"] == "okf"
            && source["locator"]
                .as_str()
                .is_some_and(|value| value.contains("decisions/webhooks.md"))
    }));
    assert!(published_sources.iter().any(|source| {
        source["source_type"] == "url" && source["locator"] == "https://docs.example.test/events"
    }));

    let export_body = json!({"item_ids": [item_id]});
    let (status, first_export) = call(
        &app,
        "POST",
        &format!("/v1/projects/{project_id}/okf/exports"),
        &token,
        None,
        Some(export_body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first_export}");
    let (status, second_export) = call(
        &app,
        "POST",
        &format!("/v1/projects/{project_id}/okf/exports"),
        &token,
        None,
        Some(export_body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second_export}");
    assert_eq!(first_export, second_export, "export must be deterministic");
    let rendered = first_export["files"]
        .as_array()
        .expect("export files")
        .iter()
        .map(|file| file["content"].as_str().unwrap_or_default())
        .collect::<String>();
    assert!(rendered.contains("vendor_extension"));
    assert!(rendered.contains("provider-contract"));
    assert!(rendered.contains("https://docs.example.test/events"));
    assert!(!rendered.contains("timestamp:"));

    let (status, unsafe_plan) = call(
        &app,
        "POST",
        &format!("/v1/projects/{project_id}/okf/imports"),
        &token,
        Some("okf-unsafe"),
        Some(json!({
            "source_kind": "directory",
            "source_locator": "fixture:unsafe",
            "encoding": "entries",
            "entries": [entry("../escape.md", "---\ntype: Fact\n---\nEscape\n")],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{unsafe_plan}");

    let Some((foreign_state, foreign_tenant)) = admitted_tenant().await else {
        return;
    };
    let foreign_app = router(foreign_state);
    let foreign_token = issue(ADMIN, foreign_tenant);
    let (status, hidden) = call(
        &foreign_app,
        "GET",
        &format!("/v1/okf/imports/{job_id}"),
        &foreign_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{hidden}");

    let audit_rows: Vec<(String, Value)> = sqlx::query_as(
        "select action, payload from audit_log where tenant_id = $1 and action like 'okf.%' order by seq",
    )
    .bind(tenant.as_uuid())
    .fetch_all(&state.pool)
    .await
    .expect("load OKF audit rows");
    assert!(
        audit_rows
            .iter()
            .any(|(action, _)| action == "okf.import.planned")
    );
    assert!(
        audit_rows
            .iter()
            .any(|(action, _)| action == "okf.import.materialized")
    );
    assert!(
        audit_rows
            .iter()
            .any(|(action, _)| action == "okf.exported")
    );
    for (_, payload) in audit_rows {
        let text = payload.to_string();
        assert!(!text.contains("provider event ID"));
        assert!(!text.contains("traceparent on public"));
        assert!(!text.contains("vendor_extension"));
    }
}
