//! PRMT-1 acceptance criteria (ADR-0049), over the real product surfaces.
//!
//! Two halves, and both are asserted **from the consumer's side** rather
//! than at the writing surface, because that is the only place either claim
//! can be false:
//!
//! - **a prompt change reaches a consumer only through review.** The direct
//!   publish route refuses under the default pack, naming the steward and
//!   the curator the `prompt` cell has priced at two distinct people since
//!   FLOW-3; the same two approvals through `POST /v1/proposals` carry it;
//!   and an edit under the published version moves the author's draft read
//!   while the consumer keeps being served the reviewed bytes at the
//!   reviewed commit.
//! - **a consumer pins a channel or a commit.** The floating read follows
//!   publications; the pinned one holds; and when a FLOW-7 rewind takes the
//!   pinned commit off the channel's first-parent line the pinned read is
//!   *refused naming both commits* rather than served or silently upgraded
//!   — because "<60s to fleet-wide effect" and a pin that outlives a
//!   withdrawal cannot both be true.
//!
//! Around them: the gradient walk (a nearer copy nobody may read does not
//! shadow the readable one further up), the schema refusals, the tier
//! nothing can mint, and the chain — where every act appears and no payload
//! carries template text.
//!
//! Tests need a live Postgres: they read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database), the house convention.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder};
use synveda_policy::Pdp;
use synveda_retrieval::index::SearchIndex;
use synveda_store::{access, identities, policy_assignments, scopes, tenants};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{
    GrantId, Identity, IdentityId, IdentityKind, PackConfig, ScopeId, TenantId, TenantStatus,
};
use tower::ServiceExt;

const SECRET: &[u8] = b"prmt-1-test-secret";

/// The prompt under review, version 1 and version 2. The edit is a
/// behaviour change a consumer would notice, which is the point: it must
/// not reach one until two people have said so.
const V1: &str = "Reply to {{ subject }} in two sentences. Be brief.";
const V2: &str = "Reply to {{ subject }} in two sentences. Offer a refund.";

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn index_root() -> std::path::PathBuf {
    std::env::temp_dir()
        .join("synveda-prmt1-tests")
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
    Hs256Verifier::new(SECRET).issue(subject, tenant_id, Duration::from_secs(300))
}

/// The fixture: one tenant, `acme → eng → {platform, payments}`, and the
/// five people a governed publication needs.
struct World {
    pool: PgPool,
    tenant: TenantId,
    app: Router,
    pdp: Arc<Pdp>,
    org: ScopeId,
    eng: ScopeId,
    platform: ScopeId,
    /// The author: a member grant at the platform team.
    alice: String,
    /// The curator who reviews and runs the effect.
    cora: String,
    /// The administrator whose approval the pack also asks for. Under the
    /// grant vocabulary the administrator *can* read content, so the
    /// separation this suite demonstrates is the one the model still
    /// draws: the member who proposed cannot run the effect — publishing
    /// is priced at curator and above.
    sam: String,
    /// The consumer: an agent anchored at the platform team, holding no
    /// grant at all — the membership floor is the whole of its reach.
    bea: String,
    /// An agent anchored at the other team, for the gradient.
    dave: String,
}

async fn world() -> Option<World> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping PRMT-1 prompt test: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
            return None;
        }
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
    let slug = format!("prmt1-{}", tenant.as_uuid().simple());
    tenants::create(
        &pool,
        tenant,
        &slug,
        "PRMT-1 test tenant",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");

    let mut tx = pool.begin().await.expect("begin");
    let root = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("mint root");
    let eng = unit(&mut tx, tenant, root.id, "eng").await;
    let platform = unit(&mut tx, tenant, eng.id, "platform").await;
    let payments = unit(&mut tx, tenant, eng.id, "payments").await;
    tx.commit().await.expect("commit hierarchy");

    for subject in ["alice", "cora", "sam"] {
        seed_user(&pool, tenant, subject).await;
    }
    // The consumers are headless agents anchored at their teams: placement
    // is identity since CPR-7 (ADR-0074 decision 3), so "a member of the
    // platform team" is an own scope under the platform org unit — which
    // is exactly the base layer's carve-out shape (ADR-0018 decision 4),
    // and the zero-config consumer the membership floor exists for.
    seed_agent(&pool, tenant, "bea", platform.id).await;
    seed_agent(&pool, tenant, "dave", payments.id).await;
    bind(&pool, tenant, "alice", platform.id, RoleKey::Member).await;
    bind(&pool, tenant, "cora", platform.id, RoleKey::Curator).await;
    bind(&pool, tenant, "sam", platform.id, RoleKey::Administrator).await;

    let pdp = Arc::new(Pdp::new().expect("build the embedded PDP"));
    let app = router(state(&url, pdp.clone()));
    Some(World {
        pool,
        tenant,
        app,
        pdp,
        org: root.id,
        eng: eng.id,
        platform: platform.id,
        alice: issue("alice", tenant),
        cora: issue("cora", tenant),
        sam: issue("sam", tenant),
        bea: issue("bea", tenant),
        dave: issue("dave", tenant),
    })
}

/// One org unit under a parent — the shape every grouping takes now that
/// rank is gone (ADR-0073 decision 4).
async fn unit(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
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

async fn seed_user(pool: &PgPool, tenant: TenantId, subject: &str) -> Identity {
    let mut tx = pool.begin().await.expect("begin");
    let own = scopes::ensure_principal_scope(&mut tx, tenant, subject, subject)
        .await
        .expect("mint principal scope");
    let identity = identities::create(
        &mut tx,
        IdentityId::new(),
        tenant,
        Some(subject),
        IdentityKind::User,
        None,
        None,
        own.id,
    )
    .await
    .expect("create identity");
    tx.commit().await.expect("commit user");
    identity
}

/// Seeds a headless consumer: a service identity whose own `principal`
/// scope nests under `anchor` — the shape the registration route mints
/// (`POST /v1/admin/service-identities`), and the only chain that runs
/// leaf → team → department → org since placement became identity.
async fn seed_agent(pool: &PgPool, tenant: TenantId, subject: &str, anchor: ScopeId) -> Identity {
    let mut tx = pool.begin().await.expect("begin");
    let leaf = scopes::create(
        &mut tx,
        &scopes::NewScope {
            id: ScopeId::new(),
            tenant_id: tenant,
            kind: ScopeKind::Principal,
            parent_scope_id: Some(anchor),
            slug: scopes::principal_slug(subject),
            display_name: subject.to_owned(),
            attributes: serde_json::json!({}),
            principal_id: Some(subject.to_owned()),
            created_by: None,
        },
    )
    .await
    .expect("mint agent scope");
    let identity = identities::create(
        &mut tx,
        IdentityId::new(),
        tenant,
        Some(subject),
        IdentityKind::Service,
        None,
        None,
        leaf.id,
    )
    .await
    .expect("create identity");
    tx.commit().await.expect("commit agent");
    identity
}

async fn bind(pool: &PgPool, tenant: TenantId, subject: &str, scope: ScopeId, role: RoleKey) {
    let mut tx = pool.begin().await.expect("begin");
    access::create_grant(
        &mut *tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id: tenant,
            scope_id: scope,
            subject: GrantSubject::Principal {
                principal_id: subject.to_owned(),
            },
            role_key: role,
            source: GrantSource::Automation,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .expect("create grant");
    tx.commit().await.expect("commit grant");
}

async fn call(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(request).await.expect("route responds");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, body)
}

/// The refusal a caller reads: `message` on the request-shaped errors,
/// `reason` on a policy denial. One helper, so a test asserts on what the
/// product actually says rather than on which arm of the taxonomy it took.
fn detail(body: &Value) -> String {
    body["message"]
        .as_str()
        .or_else(|| body["reason"].as_str())
        .unwrap_or_default()
        .to_owned()
}

async fn post(app: &Router, uri: &str, token: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build request");
    call(app, request).await
}

async fn get(app: &Router, uri: &str, token: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("build request");
    call(app, request).await
}

/// Authors a draft at a scope.
async fn author(
    w: &World,
    token: &str,
    scope: ScopeId,
    name: &str,
    template: &str,
) -> (StatusCode, Value) {
    // The schema has to agree with the template (ADR-0049 decision 12), so
    // the fixture declares `subject` exactly when the text uses it.
    let variables = if template.contains("{{ subject }}") {
        json!([{"name": "subject", "description": "what they wrote in about"}])
    } else {
        json!([])
    };
    let (status, body) = post(
        &w.app,
        "/v1/prompts",
        token,
        json!({
            "scope_id": scope,
            "name": name,
            "description": "how support replies",
            "template": template,
            "variables": variables,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "author {name} at {scope}: {body}");
    (status, body)
}

/// The consumer's call: resolve by name, walking the caller's own chain.
async fn resolve(w: &World, token: &str, name: &str) -> (StatusCode, Value) {
    get(&w.app, &format!("/v1/prompts/{name}"), token).await
}

/// Carries the whole prompt from a draft to a published version through the
/// review the default pack asks for: alice proposes, cora and sam approve,
/// cora runs the effect. Returns the commit the channel now serves.
async fn review_and_publish(w: &World, name: &str, title: &str) -> String {
    let (status, opened) = post(
        &w.app,
        "/v1/proposals",
        &w.alice,
        json!({
            "scope_id": w.platform,
            "prompt_names": [name],
            "title": title,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "open a prompt proposal: {opened}");
    let id = opened["id"].as_str().expect("proposal id").to_owned();

    for token in [&w.cora, &w.sam] {
        let (status, cast) = post(
            &w.app,
            &format!("/v1/proposals/{id}/approve"),
            token,
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "approve: {cast}");
    }
    let (status, published) = post(
        &w.app,
        &format!("/v1/proposals/{id}/publish"),
        &w.cora,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "publish: {published}");
    published["commit"].as_str().expect("commit").to_owned()
}

async fn events(pool: &PgPool, tenant: TenantId) -> Vec<synveda_audit::StoredEvent> {
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("tenant tx");
    let mut all = synveda_audit::tail(&mut tx, tenant, 500)
        .await
        .expect("read chain");
    all.reverse();
    all
}

// ── The acceptance criterion, first half ─────────────────────────────────────

/// **A prompt change reaches a consumer only through the review the pack in
/// force asks for**, measured from the consumer's side.
///
/// The direct route is not a hole to close here: it resolves the same matrix
/// (ADR-0032 decision 8), and under `regulated-strict` that matrix asks for
/// a steward *and* a curator, two distinct people — so the refusal is the
/// pack's arithmetic rather than a rule about prompts.
#[tokio::test]
async fn a_prompt_change_reaches_a_consumer_only_through_review() {
    let Some(w) = world().await else { return };

    // 1. Authored. Nothing a consumer reads has moved.
    let (status, draft) = author(&w, &w.alice, w.platform, "support/triage", V1).await;
    assert_eq!(status, StatusCode::OK, "author the draft: {draft}");
    assert_eq!(draft["name"], json!("support/triage"));
    assert!(
        draft["published"].is_null(),
        "nothing is published yet: {draft}"
    );

    let (status, absent) = resolve(&w, &w.bea, "support/triage").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a draft is not a version anyone is served: {absent}"
    );

    // 2. The direct route refuses, and the refusal is the matrix speaking.
    let (status, refused) = post(
        &w.app,
        &format!("/v1/channels/{}/publish", w.platform),
        &w.cora,
        json!({"prompt_names": ["support/triage"], "message": "ship it"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a curator alone cannot publish a prompt under regulated-strict: {refused}"
    );
    let refusal = detail(&refused);
    assert!(
        refusal.contains("administrator"),
        "the refusal must name what the pack is short of: {refusal}"
    );
    assert!(
        refusal.contains("proposal"),
        "and where to go with it: {refusal}"
    );
    let (status, still_absent) = resolve(&w, &w.bea, "support/triage").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{still_absent}");

    // 3. The review the pack asks for. Two distinct people, and the
    //    proposer cannot run the effect — publishing is priced at curator
    //    and above, beside the asset kind's read action (ADR-0031
    //    decision 12, ADR-0049 decision 4), so a member's approval is one
    //    thing and their authority to ship it is another.
    let (status, opened) = post(
        &w.app,
        "/v1/proposals",
        &w.alice,
        json!({
            "scope_id": w.platform,
            "prompt_names": ["support/triage"],
            "title": "support triage reply",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{opened}");
    let id = opened["id"].as_str().expect("proposal id").to_owned();
    assert_eq!(opened["asset"], json!("prompt"), "{opened}");
    assert_eq!(opened["state"], json!("open"), "{opened}");

    // What a reviewer is shown: the entry named by path, with the bytes
    // under review (FLOW-6, ADR-0035 decision 5, per-asset-kind since here).
    let (status, detail) = get(&w.app, &format!("/v1/proposals/{id}"), &w.cora).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    let member = &detail["members"][0];
    assert_eq!(member["member"], json!("support/triage"), "{member}");
    assert_eq!(member["asset"], json!("prompt"), "{member}");
    assert_eq!(member["effect"], json!("add"), "{member}");
    assert!(
        member["proposed"].as_str().expect("proposed").contains(V1),
        "the reviewer sees the bytes the approvals bind: {member}"
    );
    assert!(
        member["record_id"].is_null(),
        "an authored asset is named, not identified: {member}"
    );

    for token in [&w.cora, &w.sam] {
        let (status, cast) = post(
            &w.app,
            &format!("/v1/proposals/{id}/approve"),
            token,
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{cast}");
    }
    let (status, refused) = post(
        &w.app,
        &format!("/v1/proposals/{id}/publish"),
        &w.alice,
        json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the member who proposed it still cannot run the effect: publishing \
         takes `ChannelPublish` beside `PromptRead`, and a member grant holds \
         neither at the curator tier the pack prices this at: {refused}"
    );
    let (status, published) = post(
        &w.app,
        &format!("/v1/proposals/{id}/publish"),
        &w.cora,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{published}");
    let first_commit = published["commit"].as_str().expect("commit").to_owned();
    assert_eq!(
        published["channel"],
        json!("prompt/published"),
        "{published}"
    );

    // 4. The consumer is served it — by name, with no scope named, walking
    //    her own chain.
    let (status, served) = resolve(&w, &w.bea, "support/triage").await;
    assert_eq!(status, StatusCode::OK, "{served}");
    assert_eq!(served["template"], json!(V1), "{served}");
    assert_eq!(served["origin"], json!("head"), "{served}");
    assert_eq!(served["commit"], json!(first_commit), "{served}");
    assert_eq!(served["scope_id"], json!(w.platform), "{served}");
    assert_eq!(
        served["variables"][0]["name"],
        json!("subject"),
        "the schema travels with the template: {served}"
    );

    // 5. **The AC, from the reader's side.** The author edits the draft; the
    //    consumer keeps being served the reviewed bytes at the reviewed
    //    commit, and the author's own draft read shows the edit.
    let (status, edited) = author(&w, &w.alice, w.platform, "support/triage", V2).await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(
        edited["published"]["current"],
        json!(false),
        "the author is told the reviewed version is no longer their draft: {edited}"
    );
    assert_eq!(
        edited["published"]["commit"],
        json!(first_commit),
        "and which version consumers are still on: {edited}"
    );

    let (status, unchanged) = resolve(&w, &w.bea, "support/triage").await;
    assert_eq!(status, StatusCode::OK, "{unchanged}");
    assert_eq!(
        unchanged["template"],
        json!(V1),
        "the edit did not reach the consumer — this is the whole feature"
    );
    assert_eq!(unchanged["commit"], json!(first_commit));

    let (status, drafted) = get(
        &w.app,
        &format!(
            "/v1/prompts/support/triage?channel=draft&scope_id={}",
            w.platform
        ),
        &w.alice,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{drafted}");
    assert_eq!(
        drafted["template"],
        json!(V2),
        "the edit is real: {drafted}"
    );
    assert_eq!(drafted["origin"], json!("draft"), "{drafted}");

    // 6. The second review carries it, and the consumer moves.
    let second_commit = review_and_publish(&w, "support/triage", "offer a refund").await;
    assert_ne!(second_commit, first_commit);
    let (status, moved) = resolve(&w, &w.bea, "support/triage").await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["template"], json!(V2), "{moved}");
    assert_eq!(moved["commit"], json!(second_commit), "{moved}");

    // 7. The trail: every act on it, and no template text in any payload.
    let chain = events(&w.pool, w.tenant).await;
    let actions: Vec<&str> = chain.iter().map(|event| event.action.as_str()).collect();
    for action in [
        "prompt.authored",
        "prompt.resolved",
        "vedaflow.proposal.opened",
        "vedaflow.proposal.approved",
        "vedaflow.channel.published",
    ] {
        assert!(
            actions.contains(&action),
            "the chain is missing {action}: {actions:?}"
        );
    }
    for event in &chain {
        let payload = event.payload.to_string();
        for text in [V1, V2] {
            assert!(
                !payload.contains(text),
                "{} carries template text: {payload}",
                event.action
            );
        }
    }
    let published_prompt = chain
        .iter()
        .find(|event| {
            event.action == "vedaflow.channel.published"
                && event.payload["asset"] == json!("prompt")
        })
        .expect("a prompt publication on the chain");
    assert_eq!(
        published_prompt.payload["records"][0]["member"],
        json!("support/triage"),
        "the publication names the member by path: {}",
        published_prompt.payload
    );

    let mut tx = synveda_store::rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("tenant tx");
    let verification = synveda_audit::verify(&mut tx, w.tenant)
        .await
        .expect("verify the chain");
    assert!(
        matches!(verification, synveda_audit::ChainVerification::Valid { .. }),
        "{verification:?}"
    );
}

// ── The acceptance criterion, second half ────────────────────────────────────

/// **A consumer pins a channel or a commit** — and a rewind refuses the pin
/// rather than outliving it (ADR-0049 decision 10).
///
/// The two alternatives are both false statements. Serving the pinned bytes
/// after a withdrawal makes FLOW-7's "<60s to fleet-wide effect" a lie;
/// serving the head instead makes the pin one. The refusal names both
/// commits, and it reaches the consumer on its next call.
#[tokio::test]
async fn a_pinned_commit_holds_while_the_channel_moves_and_a_rewind_refuses_it() {
    let Some(w) = world().await else { return };
    author(&w, &w.alice, w.platform, "support/triage", V1).await;
    let first = review_and_publish(&w, "support/triage", "v1").await;
    author(&w, &w.alice, w.platform, "support/triage", V2).await;
    let second = review_and_publish(&w, "support/triage", "v2").await;

    // The floating consumer follows publications.
    let (_, floating) = resolve(&w, &w.bea, "support/triage").await;
    assert_eq!(floating["template"], json!(V2));

    // The pinned one holds the version it was built against — and the pin
    // is a parameter on a read, stored nowhere, governing nobody else
    // (ADR-0049 decision 9).
    let pinned_uri = format!(
        "/v1/prompts/support/triage?scope_id={}&commit={first}",
        w.platform
    );
    let (status, pinned) = get(&w.app, &pinned_uri, &w.bea).await;
    assert_eq!(status, StatusCode::OK, "{pinned}");
    assert_eq!(pinned["template"], json!(V1), "{pinned}");
    assert_eq!(pinned["origin"], json!("pinned-commit"), "{pinned}");
    assert_eq!(pinned["commit"], json!(first), "{pinned}");

    let (_, still_floating) = resolve(&w, &w.bea, "support/triage").await;
    assert_eq!(
        still_floating["template"],
        json!(V2),
        "one consumer's pin governs nobody else"
    );

    // A commit that scope's channel never held is refused the same way, so
    // the pin cannot reach across scopes or inventions.
    let (status, foreign) = get(
        &w.app,
        &format!(
            "/v1/prompts/support/triage?scope_id={}&commit={}",
            w.platform,
            "0".repeat(64)
        ),
        &w.bea,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{foreign}");

    // The rewind: a curator takes the channel back to the version before
    // the refund line.
    let (status, rolled) = post(
        &w.app,
        &format!("/v1/channels/{}/rollback", w.platform),
        &w.cora,
        json!({
            "asset": "prompt",
            "from_commit": second,
            "to_commit": first,
            "message": "the refund line was not policy",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rolled}");

    // The floating consumer heals on its next call.
    let (status, healed) = resolve(&w, &w.bea, "support/triage").await;
    assert_eq!(status, StatusCode::OK, "{healed}");
    assert_eq!(healed["template"], json!(V1), "{healed}");
    assert_eq!(healed["commit"], json!(first), "{healed}");

    // And the consumer pinned to the withdrawn commit is **refused**,
    // naming both commits — not served, and not silently upgraded.
    let (status, refused) = get(
        &w.app,
        &format!(
            "/v1/prompts/support/triage?scope_id={}&commit={second}",
            w.platform
        ),
        &w.bea,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    let message = detail(&refused);
    assert!(
        message.contains(&second),
        "names what it asked for: {message}"
    );
    assert!(message.contains(&first), "and what it would get: {message}");
    assert!(
        !message.contains("refund"),
        "and no content in a refusal: {message}"
    );

    // The pin that survives is the one still on the channel's line.
    let (status, older) = get(&w.app, &pinned_uri, &w.bea).await;
    assert_eq!(status, StatusCode::OK, "{older}");
    assert_eq!(older["template"], json!(V1), "{older}");
}

/// A standing FLOW-7 pin is the **ceiling** on what a consumer's pin can
/// reach (ADR-0049 decision 10, ADR-0036 decision 7).
///
/// "Exactly one thing decides what readers see" survives a request
/// parameter only if the scope's hold bounds it: a consumer may pin at or
/// below what the scope serves, never above. Otherwise a team holding its
/// fleet at an earlier version could be walked around by any caller that
/// names the newer commit — which is the pin's whole purpose, undone by a
/// query string.
#[tokio::test]
async fn a_scope_pin_is_the_ceiling_a_consumer_pin_cannot_reach_over() {
    let Some(w) = world().await else { return };
    author(&w, &w.alice, w.platform, "support/triage", V1).await;
    let first = review_and_publish(&w, "support/triage", "v1").await;
    author(&w, &w.alice, w.platform, "support/triage", V2).await;
    let second = review_and_publish(&w, "support/triage", "v2").await;

    // Both versions are reachable while the channel serves its head.
    let (status, newer) = get(
        &w.app,
        &format!(
            "/v1/prompts/support/triage?scope_id={}&commit={second}",
            w.platform
        ),
        &w.bea,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{newer}");

    // The team holds its readers at the earlier version.
    let (status, pinned) = post(
        &w.app,
        &format!("/v1/channels/{}/pin", w.platform),
        &w.cora,
        json!({
            "asset": "prompt",
            "commit": first,
            "reason": "the refund line is under review by legal",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{pinned}");

    // The floating consumer composes the held version, and says so.
    let (status, held) = resolve(&w, &w.bea, "support/triage").await;
    assert_eq!(status, StatusCode::OK, "{held}");
    assert_eq!(held["template"], json!(V1), "{held}");
    assert_eq!(
        held["origin"],
        json!("channel-pin"),
        "a response that cites a frozen commit says so: {held}"
    );

    // And the consumer naming the newer commit is refused — the hold is a
    // ceiling, not a default.
    let (status, over) = get(
        &w.app,
        &format!(
            "/v1/prompts/support/triage?scope_id={}&commit={second}",
            w.platform
        ),
        &w.bea,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a request parameter must not reach over the scope's own hold: {over}"
    );
    assert!(
        detail(&over).contains(&first),
        "and the refusal names what the scope does serve: {}",
        detail(&over)
    );

    // Pinning at the held commit itself still works.
    let (status, at_hold) = get(
        &w.app,
        &format!(
            "/v1/prompts/support/triage?scope_id={}&commit={first}",
            w.platform
        ),
        &w.bea,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{at_hold}");
    assert_eq!(at_hold["origin"], json!("pinned-commit"), "{at_hold}");
}

/// A pin freezes bytes and never authority (ADR-0049 decision 11) — CTX-4's
/// rule for handles, restated for commits.
///
/// The same pinned read, the same token, the same commit: it stops
/// resolving when the pack behind it is replaced, because the decision is
/// taken at request time and a commit hash is a name rather than a
/// capability.
#[tokio::test]
async fn a_pin_freezes_bytes_and_never_authority() {
    let Some(w) = world().await else { return };
    author(&w, &w.alice, w.platform, "support/triage", V1).await;
    let commit = review_and_publish(&w, "support/triage", "v1").await;
    let uri = format!(
        "/v1/prompts/support/triage?scope_id={}&commit={commit}",
        w.platform
    );
    let (status, served) = get(&w.app, &uri, &w.bea).await;
    assert_eq!(status, StatusCode::OK, "{served}");

    // The tenant's pack is replaced with one that permits nothing — the
    // ADR-0014 freshness path, thrown for real. No restart, no cache flush,
    // and nothing whatsoever touching the pin.
    let locked = format!("prmt1-locked-{}", w.tenant.as_uuid().simple());
    w.pdp
        .install_source(
            w.tenant,
            &locked,
            1,
            "permit (principal, action, resource) when { false };",
            PackConfig::default(),
        )
        .expect("install locked pack");
    policy_assignments::set_default(&w.pool, w.tenant, &locked)
        .await
        .expect("set locked pack as default");

    let (status, after) = get(&w.app, &uri, &w.bea).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a pinned commit carries no authority of its own: {after}"
    );
}

// ── The gradient ─────────────────────────────────────────────────────────────

/// Resolution walks the caller's own placement chain nearest-first, and
/// **skips the scopes the PDP refuses** (ADR-0049 decision 8).
///
/// Two properties in one arc: a team's version overrides the org's for that
/// team's people, and a nearer copy nobody may read does not shadow the
/// readable one further up — which is why a denied scope is skipped rather
/// than fatal.
#[tokio::test]
async fn resolution_walks_the_chain_nearest_first_and_skips_what_it_may_not_read() {
    let Some(w) = world().await else { return };
    // The org publishes the house version. Cora is bound curator at the
    // platform team only, so the org's publication needs its own
    // authority: bind her there for the two publications this test needs.
    bind(&w.pool, w.tenant, "cora", w.org, RoleKey::Curator).await;
    bind(&w.pool, w.tenant, "sam", w.org, RoleKey::Administrator).await;
    bind(&w.pool, w.tenant, "alice", w.org, RoleKey::Member).await;
    bind(&w.pool, w.tenant, "cora", w.eng, RoleKey::Curator).await;
    bind(&w.pool, w.tenant, "sam", w.eng, RoleKey::Administrator).await;
    bind(&w.pool, w.tenant, "alice", w.eng, RoleKey::Member).await;

    let org_text = "House style: plain words, no exclamation marks.";
    author(&w, &w.alice, w.org, "house-style", org_text).await;
    carry(&w, w.org, "house-style", "the house style").await;

    // Everyone gets it, from wherever they are.
    for token in [&w.bea, &w.dave] {
        let (status, served) = resolve(&w, token, "house-style").await;
        assert_eq!(status, StatusCode::OK, "{served}");
        assert_eq!(served["template"], json!(org_text), "{served}");
        assert_eq!(served["scope_id"], json!(w.org), "{served}");
    }

    // The platform team publishes its own version of the same name.
    let team_text = "House style, platform: plain words, and always a runbook link.";
    author(&w, &w.alice, w.platform, "house-style", team_text).await;
    review_and_publish(&w, "house-style", "the platform's own house style").await;

    let (_, nearer) = resolve(&w, &w.bea, "house-style").await;
    assert_eq!(
        nearer["template"],
        json!(team_text),
        "the nearer scope overrides — seed §4.4's gradient, applied to a fetch"
    );
    assert_eq!(nearer["scope_id"], json!(w.platform));
    let (_, other_team) = resolve(&w, &w.dave, "house-style").await;
    assert_eq!(
        other_team["template"],
        json!(org_text),
        "and the other team is unaffected: the walk is the caller's own chain"
    );

    // A department version bea may not read: `confidential` takes an
    // explicit content-role binding under every pack, and bea holds none.
    // The walk must skip it rather than shadow the org's readable one.
    let dept_text = "Engineering style, confidential: never quote the incident bridge.";
    let (status, authored) = post(
        &w.app,
        "/v1/prompts",
        &w.alice,
        json!({
            "scope_id": w.eng,
            "name": "eng-style",
            "description": "engineering style",
            "template": dept_text,
            "sensitivity": "confidential",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{authored}");
    carry(&w, w.eng, "eng-style", "the engineering style").await;

    let org_eng_text = "Engineering style: link the runbook.";
    let (status, authored) = post(
        &w.app,
        "/v1/prompts",
        &w.alice,
        json!({
            "scope_id": w.org,
            "name": "eng-style",
            "description": "engineering style",
            "template": org_eng_text,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{authored}");
    carry(&w, w.org, "eng-style", "the org's engineering style").await;

    let (status, skipped) = resolve(&w, &w.bea, "eng-style").await;
    assert_eq!(status, StatusCode::OK, "{skipped}");
    assert_eq!(
        skipped["template"],
        json!(org_eng_text),
        "the nearer copy she may not read is skipped, not fatal, and does \
         not shadow the one she may: {skipped}"
    );
    assert_eq!(skipped["scope_id"], json!(w.org), "{skipped}");

    // Cora holds curator at the department, which is the explicit grant
    // `confidential` is defined by. Her own chain no longer runs through
    // the department — placement is identity — so the nearer copy is
    // something she *names*, and the grant is what the named read turns
    // on.
    let (status, reached) = get(
        &w.app,
        &format!("/v1/prompts/eng-style?scope_id={}", w.eng),
        &w.cora,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reached}");
    assert_eq!(reached["template"], json!(dept_text), "{reached}");
    assert_eq!(reached["scope_id"], json!(w.eng), "{reached}");

    // A name nothing publishes is the uniform 404 — never an oracle.
    let (status, missing) = resolve(&w, &w.bea, "support/nothing-here").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
}

/// Carries a prompt authored at `scope` onto that scope's published channel
/// through the review the pack asks for. The team-level helper
/// [`review_and_publish`] targets the platform team; this one takes the
/// scope, for the org and department publications the gradient needs.
async fn carry(w: &World, scope: ScopeId, name: &str, title: &str) {
    let (status, opened) = post(
        &w.app,
        "/v1/proposals",
        &w.alice,
        json!({"scope_id": scope, "prompt_names": [name], "title": title}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{opened}");
    let id = opened["id"].as_str().expect("proposal id").to_owned();
    for token in [&w.cora, &w.sam] {
        let (status, cast) = post(
            &w.app,
            &format!("/v1/proposals/{id}/approve"),
            token,
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{cast}");
    }
    let (status, published) = post(
        &w.app,
        &format!("/v1/proposals/{id}/publish"),
        &w.cora,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{published}");
}

// ── The schema, and the tier nothing can mint ────────────────────────────────

/// The variable schema is enforced where it can fail (ADR-0049 decisions 12
/// and 13), and the tier no authored asset can carry is refused by name
/// (decision 5).
///
/// Each of these is a request a careful author makes by accident, and each
/// one names its offender rather than shipping a template a consumer cannot
/// fill in.
#[tokio::test]
async fn the_schema_is_enforced_at_authoring_and_the_top_tier_is_refused() {
    let Some(w) = world().await else { return };
    let refuse = async |body: Value, expect: &str| {
        let (status, refused) = post(&w.app, "/v1/prompts", &w.alice, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
        let message = detail(&refused);
        assert!(
            message.contains(expect),
            "the refusal must name the offender ({expect}): {message}"
        );
    };

    // A placeholder no variable declares: a consumer cannot supply it.
    refuse(
        json!({
            "scope_id": w.platform,
            "name": "support/a",
            "template": "Hi {{ name }}, about {{ topic }}",
            "variables": [{"name": "name"}],
        }),
        "topic",
    )
    .await;

    // A variable the template never uses: every consumer would fill it in
    // for nothing.
    refuse(
        json!({
            "scope_id": w.platform,
            "name": "support/b",
            "template": "Hi {{ name }}",
            "variables": [{"name": "name"}, {"name": "unused"}],
        }),
        "unused",
    )
    .await;

    // Prose inside braces: the strict reading, because the lenient one
    // ships a typo to a fleet as literal text (decision 13).
    refuse(
        json!({
            "scope_id": w.platform,
            "name": "support/c",
            "template": "Hi {{ user name }}",
            "variables": [{"name": "user"}],
        }),
        "not a placeholder",
    )
    .await;

    // The tier nothing in the product mints for an authored asset.
    refuse(
        json!({
            "scope_id": w.platform,
            "name": "support/d",
            "template": "nothing to see",
            "sensitivity": "restricted",
        }),
        "restricted",
    )
    .await;

    // A name that is not an identifier.
    refuse(
        json!({
            "scope_id": w.platform,
            "name": "Support Triage",
            "template": "nothing to see",
        }),
        "prompt name",
    )
    .await;

    // And the write seam is a decision, not a formality: bea holds no
    // content role and may author only at her own home.
    let (status, denied) = post(
        &w.app,
        "/v1/prompts",
        &w.bea,
        json!({
            "scope_id": w.platform,
            "name": "support/e",
            "template": "nothing to see",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "authoring at a shared scope takes PromptWrite: {denied}"
    );
}
