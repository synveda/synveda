//! FLOW-5's acceptance criteria (ADR-0034): **knowledge climbing two
//! levels with distinct approver sets**, and **denial at any level
//! audited with reason** — over the product's own HTTP surfaces, under
//! the PDP, against a live Postgres.
//!
//! The walk is the feature's own sentence made true: a team's procedure
//! reaches the org unit under the org unit's approvers, then the tenant
//! root, and at each level the level below's curator is refused by the
//! PDP because grants inherit down the subtree and never up. What makes
//! it a *promotion* rather than a row in a table is the reader-side
//! assertion of each hop: after the first, a reader granted at the unit
//! composes the procedure *as the unit's published material* — the entry
//! names the publishing scope, not the team the record lives at — and
//! after the second, a member of another department receives it in their
//! own `inject`, because the tenant root is on every member's chain and
//! no org unit is (CPR-7: placement is identity, grants decide).
//!
//! Around the AC walk: the direction rule (a climb goes up, never
//! sideways or down), the disclosure rule that lets a user climb their
//! own personal memory and stops anyone else from climbing it for them,
//! and the two senses in which a scope holds material — the second of
//! which is what lets the department propose onward what the team
//! climbed into it, with nothing stored to make the hop possible.
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
use synveda_audit::StoredEvent;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder};
use synveda_policy::Pdp;
use synveda_retrieval::index::SearchIndex;
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::{access, identities, scopes, tenants};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{
    GrantId, Identity, IdentityId, IdentityKind, RecordClass, RecordId, RecordKind, ScopeId,
    Sensitivity, TenantId, TenantStatus,
};
use tower::ServiceExt;

const SECRET: &[u8] = b"flow-5-test-secret";

/// The team knowledge that climbs.
const RUNBOOK: &str = "rotate the signing key every 90 days, on the first tuesday";

// ── Harness ──────────────────────────────────────────────────────────────────

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn state(url: &str) -> AppState {
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
        pdp: Arc::new(Pdp::new().expect("build the embedded PDP")),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index: Arc::new(
            SearchIndex::open(
                std::env::temp_dir()
                    .join("synveda-flow5-tests")
                    .join(TenantId::new().to_string()),
            )
            .expect("open sidecar"),
        ),
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

async fn admitted_tenant() -> Option<(PgPool, TenantId)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping FLOW-5 cross-scope test: DATABASE_URL is not set \
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
    let id = TenantId::new();
    let slug = format!("flow5-{}", id.as_uuid().simple());
    tenants::create(&pool, id, &slug, "FLOW-5 test tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((pool, id))
}

fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("checked by admitted_tenant")
}

/// The tree the climb happens in:
///
/// ```text
/// root (tenant)
/// ├── eng (org unit)
/// │   ├── platform (org unit)   ← the runbook lives here
/// │   └── payments (org unit)   ← reads it after the first hop, never before
/// └── sales (org unit)
///     └── field (org unit)      ← reads it after the second hop, never before
/// ```
///
/// Two branches, because "the tenant" has to mean something a branch
/// publication does not: the second hop is proven by a reader outside
/// `eng` entirely.
struct Org {
    root: Scope,
    eng: Scope,
    platform: Scope,
    payments: Scope,
    field: Scope,
}

async fn seed_hierarchy(pool: &PgPool, tenant: TenantId) -> Org {
    let mut tx = pool.begin().await.expect("begin");
    let unit = async |tx: &mut sqlx::PgConnection, parent: ScopeId, slug: &str, display: &str| {
        scopes::create(
            tx,
            &scopes::NewScope {
                id: ScopeId::new(),
                tenant_id: tenant,
                kind: ScopeKind::OrgUnit,
                parent_scope_id: Some(parent),
                slug: slug.to_owned(),
                display_name: display.to_owned(),
                attributes: serde_json::json!({}),
                principal_id: None,
                created_by: None,
            },
        )
        .await
        .expect("create org unit")
    };
    let root = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("mint root");
    let eng = unit(&mut tx, root.id, "eng", "Engineering").await;
    let platform = unit(&mut tx, eng.id, "platform", "Platform").await;
    let payments = unit(&mut tx, eng.id, "payments", "Payments").await;
    let sales = unit(&mut tx, root.id, "sales", "Sales").await;
    let field = unit(&mut tx, sales.id, "field", "Field").await;
    tx.commit().await.expect("commit scopes");
    Org {
        root,
        eng,
        platform,
        payments,
        field,
    }
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

async fn seed_record(
    pool: &PgPool,
    tenant: TenantId,
    scope: ScopeId,
    owner: IdentityId,
    content: &str,
) -> RecordId {
    let id = RecordId::new();
    records::insert(
        pool,
        id,
        tenant,
        &RecordState {
            scope_id: scope,
            owner_id: owner,
            kind: RecordKind::Derived,
            class: RecordClass::Procedure,
            content: content.to_owned(),
            sensitivity: Sensitivity::Internal,
            provenance: json!({"source": "flow-5 acceptance test"}),
            valid_from: chrono::Utc::now() - chrono::Duration::hours(1),
            valid_to: None,
        },
        &RecordEmbedding {
            model: DeterministicEmbedder::MODEL.to_owned(),
            vector: vec![0.25; 16],
        },
    )
    .await
    .expect("insert record");
    id
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

async fn post(app: &Router, token: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build request");
    call(app, request).await
}

#[path = "session_seed.rs"]
mod session_seed;

async fn inject(app: &Router, token: &str, run: synveda_types::SessionId) -> Value {
    let request = Request::builder()
        .method("POST")
        .uri(format!("/v1/sessions/{run}/context-runs"))
        .header("authorization", format!("Bearer {token}"))
        .header(
            "idempotency-key",
            synveda_types::ContextRunId::new().to_string(),
        )
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .expect("build context-run request");
    let (status, body) = call(app, request).await;
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "context run failed: {status} {body}"
    );
    body
}

/// Whether a caller's own context block carries the runbook.
async fn reads_runbook(app: &Router, token: &str, run: synveda_types::SessionId) -> bool {
    inject(app, token, run).await["rendered"]
        .as_str()
        .expect("block text")
        .contains(RUNBOOK)
}

/// Opens a proposal at `target`, optionally naming where the material is.
async fn open_climb(
    app: &Router,
    token: &str,
    target: ScopeId,
    source: Option<ScopeId>,
    records: &[RecordId],
    title: &str,
) -> (StatusCode, Value) {
    let mut body = json!({
        "scope_id": target,
        "record_ids": records,
        "title": title,
    });
    if let Some(source) = source {
        body["source_scope_id"] = json!(source);
    }
    post(app, token, "/v1/proposals", body).await
}

/// Opens a climb that must succeed, returning its id.
async fn climb(
    app: &Router,
    token: &str,
    target: ScopeId,
    source: ScopeId,
    records: &[RecordId],
    title: &str,
) -> String {
    let (status, body) = open_climb(app, token, target, Some(source), records, title).await;
    assert_eq!(status, StatusCode::OK, "opening the climb failed: {body}");
    assert_eq!(
        body["source_scope_id"].as_str(),
        Some(source.to_string().as_str()),
        "the proposal must record where the material came from: {body}"
    );
    body["id"].as_str().expect("proposal id").to_owned()
}

/// The human-readable half of an error body, whichever variant it is.
fn detail(body: &Value) -> String {
    ["message", "reason", "error"]
        .iter()
        .filter_map(|field| body.get(field).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

async fn events(pool: &PgPool, tenant: TenantId, action: &str) -> Vec<StoredEvent> {
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("tenant tx");
    let mut all = synveda_audit::tail(&mut tx, tenant, 500)
        .await
        .expect("read chain");
    all.reverse();
    all.into_iter()
        .filter(|event| event.action == action)
        .collect()
}

// ── AC: two levels, each level's approvers, denial audited with reason ───────

/// The headline walk, and the only test in this file that asserts on the
/// whole feature at once.
///
/// A platform-team runbook climbs to Engineering and then to the tenant
/// root. Each hop is refused for the level below's curator — grants
/// inherit down the subtree, so a team curator holds nothing at the org
/// unit and an org-unit curator holds nothing at the root — and carried
/// by that level's own. After the first hop a reader granted at
/// Engineering composes the runbook as Engineering's *published*
/// material; after the second, a member of a different branch receives
/// it in their own inject, because the root is the one scope on every
/// member's chain. That is what makes it a promotion rather than a row:
/// the audience widened, at each level, under that level's approvers.
///
/// The approver *sets* are the pack's, not the test's: under
/// `regulated-strict` a memory publication takes a curator **and** an
/// administrator, two distinct people, at an org unit *and* at the tenant
/// root — the root carries the same SHARED cell, because it is the widest
/// audience the product has. The two hops therefore need four principals,
/// two at each level, which is what "each level's approvers" means when
/// the levels differ in place but not in price.
#[tokio::test]
async fn knowledge_climbs_two_levels_under_each_levels_approvers() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let org = seed_hierarchy(&pool, tenant).await;
    let ravi = seed_user(&pool, tenant, "ravi@acme.test").await;
    seed_user(&pool, tenant, "tara@acme.test").await;
    seed_user(&pool, tenant, "dana@acme.test").await;
    seed_user(&pool, tenant, "evan@acme.test").await;
    seed_user(&pool, tenant, "olive@acme.test").await;
    seed_user(&pool, tenant, "owen@acme.test").await;
    seed_user(&pool, tenant, "pia@acme.test").await;
    seed_user(&pool, tenant, "sam@acme.test").await;
    // Each principal bound at exactly one level. Grants inherit down the
    // subtree, so granting the root's people at the root would let them
    // approve at the org unit too; the direction the AC cares about is
    // the other one, and it is proven by denial below: a grant never
    // climbs.
    bind(
        &pool,
        tenant,
        "tara@acme.test",
        org.platform.id,
        RoleKey::Curator,
    )
    .await;
    bind(
        &pool,
        tenant,
        "dana@acme.test",
        org.eng.id,
        RoleKey::Curator,
    )
    .await;
    bind(
        &pool,
        tenant,
        "evan@acme.test",
        org.eng.id,
        RoleKey::Administrator,
    )
    .await;
    bind(
        &pool,
        tenant,
        "olive@acme.test",
        org.root.id,
        RoleKey::Curator,
    )
    .await;
    bind(
        &pool,
        tenant,
        "owen@acme.test",
        org.root.id,
        RoleKey::Administrator,
    )
    .await;
    bind(
        &pool,
        tenant,
        "ravi@acme.test",
        org.platform.id,
        RoleKey::Member,
    )
    .await;
    // Opening hop 1 asks at Engineering, and asking is membership or a
    // grant at the scope asked (CPR-7: no subtree placement exists), so
    // the platform member who owns the runbook holds the org unit too —
    // which also reads him the platform record, eng being on its chain.
    bind(&pool, tenant, "ravi@acme.test", org.eng.id, RoleKey::Member).await;
    // The audience: a member of the sibling unit and a member of a unit in
    // the other branch — the grants that stand where placements used to.
    bind(
        &pool,
        tenant,
        "pia@acme.test",
        org.payments.id,
        RoleKey::Member,
    )
    .await;
    bind(
        &pool,
        tenant,
        "sam@acme.test",
        org.field.id,
        RoleKey::Member,
    )
    .await;

    let (ravi_t, tara, dana, evan, olive, owen, pia, sam) = (
        issue("ravi@acme.test", tenant),
        issue("tara@acme.test", tenant),
        issue("dana@acme.test", tenant),
        issue("evan@acme.test", tenant),
        issue("olive@acme.test", tenant),
        issue("owen@acme.test", tenant),
        issue("pia@acme.test", tenant),
        issue("sam@acme.test", tenant),
    );
    let app = router(state(&database_url()));
    // A run per reader that composes: a composition names the session it was
    // for since CPR-12, and these assertions are about what one reader
    // receives. Dana has none — the two facts she used to observe by recall
    // are now read at the channel (see below).
    let pia_run = session_seed::seed_run_for(&pool, tenant, "flow5-pia", "pia@acme.test")
        .await
        .session_id;
    let sam_run = session_seed::seed_run_for(&pool, tenant, "flow5-sam", "sam@acme.test")
        .await
        .session_id;
    let runbook = seed_record(&pool, tenant, org.platform.id, ravi.id, RUNBOOK).await;

    // Baseline: the runbook is the platform team's, and nobody outside it
    // reads it — not the sibling team, not the other department.
    assert!(
        !reads_runbook(&app, &pia, pia_run).await,
        "the runbook must not reach payments before it climbs"
    );
    assert!(
        !reads_runbook(&app, &sam, sam_run).await,
        "the runbook must not reach sales before it climbs"
    );
    // The reader-side baseline for the promotion proof below: Dana, who
    // curates Engineering, reads the runbook as platform's *derived*
    // material — her grant reaches the team because eng is on its chain —
    // and nothing more has happened to it yet.
    //
    // **Asserted at the channel rather than at a reader**, and the reason is a
    // capability this cut removed. `/v1/recall` widened a read to every scope
    // the caller may reach — which is how dana, whose own chain never runs
    // through the platform team, could observe the runbook where it lives. It
    // is deleted (CPR-12, ADR-0078 decision 5), and a context run composes
    // from the caller's own chain plus the *run's* scope chain — neither of
    // which reaches a team dana merely holds a grant over. So the fact is
    // asserted where it still lives: platform's derived channel holds the
    // runbook and its published channel does not.
    //
    // Prompt 18 re-cuts recall over the new model; the reader-side version of
    // this observation comes back with it.
    let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("tenant tx");
    let before = synveda_vedaflow::read_memory_members(
        &mut tx,
        tenant,
        &[org.eng.id],
        synveda_types::Channel::Published,
    )
    .await
    .expect("read engineering's published channel");
    drop(tx);
    assert!(
        before
            .first()
            .is_none_or(|channel| !channel.members.iter().any(|(id, _)| *id == runbook)),
        "before the climb the runbook is not Engineering's published material: {before:?}"
    );

    // ── Hop 1: platform → eng, under Engineering's curator ──────────────
    let hop1 = climb(
        &app,
        &ravi_t,
        org.eng.id,
        org.platform.id,
        &[runbook],
        "promote the signing-key runbook to Engineering",
    )
    .await;

    // Tara curates the platform team. She holds nothing at Engineering,
    // because grants inherit down the subtree and never up — so the level
    // below cannot approve the level above's publication.
    let (status, denied) = post(
        &app,
        &tara,
        &format!("/v1/proposals/{hop1}/approve"),
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a team curator must not review at the org unit: {denied}"
    );

    let (status, approved) = post(
        &app,
        &dana,
        &format!("/v1/proposals/{hop1}/approve"),
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the org unit's curator approval: {approved}"
    );
    assert_eq!(
        approved["state"].as_str(),
        Some("open"),
        "an org-unit publication is not one curator's to make: {approved}"
    );
    assert!(
        approved["outstanding"]
            .as_str()
            .is_some_and(|outstanding| outstanding.contains("administrator")),
        "and the response says what it still lacks: {approved}"
    );
    // The org unit's administrator is the second of the two distinct
    // approvers the matrix asks for at this scope shape.
    let (status, approved) = post(
        &app,
        &evan,
        &format!("/v1/proposals/{hop1}/approve"),
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the org unit administrator's approval: {approved}"
    );
    assert_eq!(
        approved["state"].as_str(),
        Some("approved"),
        "curator + administrator, two distinct people, satisfies an org unit: {approved}"
    );
    // Dana runs the effect: publishing takes `ChannelPublish` and the
    // working-tier read beside it, and she holds both at Engineering
    // (ADR-0031 decision 12 — nobody publishes what they cannot read).
    let (status, published) = post(
        &app,
        &dana,
        &format!("/v1/proposals/{hop1}/publish"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "publishing hop 1: {published}");
    assert_eq!(
        published["scope_id"].as_str(),
        Some(org.eng.id.to_string().as_str()),
        "the org unit's channel is what moved: {published}"
    );

    // The promotion, at the channel. The record still lives at the platform
    // team; what moved is **Engineering's** published channel, which is the
    // address the entry composes from after the climb (ADR-0034 decision 6).
    // That is the audience the first hop widens: everyone the org unit's
    // reviewed channel stands for, not only those whose grant already reached
    // the team.
    let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("tenant tx");
    let climbed = synveda_vedaflow::read_memory_members(
        &mut tx,
        tenant,
        &[org.eng.id],
        synveda_types::Channel::Published,
    )
    .await
    .expect("read engineering's published channel");
    drop(tx);
    assert!(
        climbed
            .first()
            .is_some_and(|channel| channel.members.iter().any(|(id, _)| *id == runbook)),
        "the climbed runbook is Engineering's published material: {climbed:?}"
    );
    // And an org-unit publication stops there: no org unit is on any
    // member's own chain (a principal scope hangs off the tenant root,
    // CPR-7), so a member of another branch still receives nothing in
    // their inject — only the root's publication reaches everyone.
    assert!(
        !reads_runbook(&app, &sam, sam_run).await,
        "an org-unit publication must not reach another department's inject"
    );

    // ── Hop 2: eng → org, on material eng holds only by publishing it ───
    // The record still lives at the platform team. Engineering can propose
    // it onward because its published tree names it at exactly its current
    // address, which is the second sense of holding material.
    let hop2 = climb(
        &app,
        &dana,
        org.root.id,
        org.eng.id,
        &[runbook],
        "promote the signing-key runbook to ACME",
    )
    .await;
    let (status, denied) = post(
        &app,
        &dana,
        &format!("/v1/proposals/{hop2}/approve"),
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a department curator must not review at the org: {denied}"
    );

    // Denial at this level, with a reason an auditor reads.
    let (status, rejected) = post(
        &app,
        &olive,
        &format!("/v1/proposals/{hop2}/reject"),
        json!({"reason": "org-wide runbooks need the platform team named as owner"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "rejecting hop 2: {rejected}");
    assert_eq!(rejected["state"].as_str(), Some("rejected"));
    assert!(
        !reads_runbook(&app, &sam, sam_run).await,
        "a rejected climb must change nothing"
    );

    // A revision is a new proposal (ADR-0032 decision 12). The tenant root
    // prices a memory at curator + administrator, two distinct people —
    // the same cell an org unit carries — so this hop needs both of the
    // root's people, and the first signature alone leaves it open.
    let hop2b = climb(
        &app,
        &dana,
        org.root.id,
        org.eng.id,
        &[runbook],
        "promote the signing-key runbook to ACME (owner named)",
    )
    .await;
    let (status, first) = post(
        &app,
        &olive,
        &format!("/v1/proposals/{hop2b}/approve"),
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the root's curator approves: {first}"
    );
    assert_eq!(
        first["state"].as_str(),
        Some("open"),
        "a tenant-root publication is not one curator's to make either: {first}"
    );
    let (status, second) = post(
        &app,
        &owen,
        &format!("/v1/proposals/{hop2b}/approve"),
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the root's administrator is the second: {second}"
    );
    assert_eq!(
        second["state"].as_str(),
        Some("approved"),
        "curator + administrator, two distinct people, satisfies the root too: {second}"
    );
    let (status, published) = post(
        &app,
        &olive,
        &format!("/v1/proposals/{hop2b}/publish"),
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "and then the root's curator publishes it: {published}"
    );

    // The audience widened, and here it is reader-visible: the tenant
    // root is the one scope on every member's own chain, so Sam — in a
    // branch that never held a grant over the runbook — now receives it
    // in his own inject, as published material (never `[unreviewed]`).
    assert!(
        reads_runbook(&app, &sam, sam_run).await,
        "after the second climb, another department must receive the runbook"
    );
    let block = inject(&app, &sam, sam_run).await;
    let text = block["rendered"].as_str().expect("rendered block");
    assert!(
        !text.contains("[unreviewed]"),
        "climbed material composes as published, not derived: {text}"
    );
    assert!(
        !text.contains("platform"),
        "the entry sits in the tenant root's section, not the team's: {text}"
    );
    // And Pia, who never could read the platform team, receives it too —
    // the tenant's publication is what reaches her, nothing narrower.
    assert!(
        reads_runbook(&app, &pia, pia_run).await,
        "after the second climb, the sibling team receives the runbook"
    );

    // ── The trail: two climbs, one denial, both levels named ────────────
    let opened = events(&pool, tenant, "vedaflow.proposal.opened").await;
    let climbs: Vec<&StoredEvent> = opened
        .iter()
        .filter(|event| event.payload.get("climb").is_some_and(|c| !c.is_null()))
        .collect();
    assert_eq!(
        climbs.len(),
        3,
        "three climbs were opened (one was rejected)"
    );
    assert_eq!(
        climbs[0].payload["source_scope_id"].as_str(),
        Some(org.platform.id.to_string().as_str())
    );
    assert_eq!(
        climbs[0].payload["target_scope_id"].as_str(),
        Some(org.eng.id.to_string().as_str())
    );
    assert_eq!(
        climbs[0].payload["climb"]["levels"].as_u64(),
        Some(1),
        "platform → eng is one level"
    );
    // The disclosure decision is on the event beside the open decision:
    // two governed decisions, two recorded contexts (ADR-0034 decision 9).
    assert!(
        climbs[0].payload["climb"]["source_read"]["pack"].is_string(),
        "the proposer's read at the source must be recorded: {}",
        climbs[0].payload
    );

    let rejections = events(&pool, tenant, "vedaflow.proposal.rejected").await;
    assert_eq!(rejections.len(), 1, "exactly one denial");
    let denial = &rejections[0];
    assert_eq!(
        denial.payload["target_scope_id"].as_str(),
        Some(org.root.id.to_string().as_str()),
        "the denial names the level it happened at"
    );
    assert_eq!(
        denial.payload["source_scope_id"].as_str(),
        Some(org.eng.id.to_string().as_str()),
        "and what it refused to take"
    );
    assert_eq!(
        denial.payload["reason"].as_str(),
        Some("org-wide runbooks need the platform team named as owner"),
        "a denial an auditor cannot read the reason for is not a review"
    );

    let publications = events(&pool, tenant, "vedaflow.channel.published").await;
    let climbed: Vec<(String, String)> = publications
        .iter()
        .filter_map(|event| {
            Some((
                event.payload.get("source_scope_id")?.as_str()?.to_owned(),
                event.payload.get("target_scope_id")?.as_str()?.to_owned(),
            ))
        })
        .collect();
    assert_eq!(
        climbed,
        [
            (org.platform.id.to_string(), org.eng.id.to_string()),
            (org.eng.id.to_string(), org.root.id.to_string()),
        ],
        "the chain shows the runbook climbing, hop by hop"
    );

    // The chain still verifies with all of it in.
    let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("tenant tx");
    let report = synveda_audit::verify(&mut tx, tenant)
        .await
        .expect("verify chain");
    assert!(
        matches!(report, synveda_audit::ChainVerification::Valid { .. }),
        "audit chain broke: {report}"
    );
}

// ── The direction rule ───────────────────────────────────────────────────────

/// A promotion climbs the hierarchy; it does not cross it and it does not
/// descend. Both refusals name the rule rather than collapsing into a
/// generic "invalid", because a caller that meant to climb needs to be
/// told which way is up.
#[tokio::test]
async fn a_climb_goes_up_never_sideways_and_never_down() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let org = seed_hierarchy(&pool, tenant).await;
    let ravi = seed_user(&pool, tenant, "ravi@acme.test").await;
    // Curator at both teams *and* the department, so every refusal below
    // is the direction rule talking and not the PDP.
    bind(
        &pool,
        tenant,
        "ravi@acme.test",
        org.platform.id,
        RoleKey::Curator,
    )
    .await;
    bind(
        &pool,
        tenant,
        "ravi@acme.test",
        org.payments.id,
        RoleKey::Curator,
    )
    .await;
    bind(
        &pool,
        tenant,
        "ravi@acme.test",
        org.eng.id,
        RoleKey::Curator,
    )
    .await;
    let token = issue("ravi@acme.test", tenant);
    let app = router(state(&database_url()));
    let runbook = seed_record(&pool, tenant, org.platform.id, ravi.id, RUNBOOK).await;

    // Sideways: payments is a sibling of platform, not an ancestor.
    let (status, sideways) = open_climb(
        &app,
        &token,
        org.payments.id,
        Some(org.platform.id),
        &[runbook],
        "climb sideways",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "sideways slipped through: {sideways}"
    );
    assert!(
        detail(&sideways).contains("not an ancestor"),
        "the refusal must name the direction rule: {sideways}"
    );

    // Downward: proposing the department's material into a team under it
    // would put content behind *fewer* approvers than it already has.
    let (status, down) = open_climb(
        &app,
        &token,
        org.platform.id,
        Some(org.eng.id),
        &[runbook],
        "climb down",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a descent slipped through: {down}"
    );
    assert!(
        detail(&down).contains("not an ancestor"),
        "the refusal must name the direction rule: {down}"
    );

    // And straight up is fine, from the same principal on the same record:
    // the three refusals above were about direction, nothing else.
    let (status, up) = open_climb(
        &app,
        &token,
        org.eng.id,
        Some(org.platform.id),
        &[runbook],
        "climb up",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the climb itself must work: {up}");
}

// ── The disclosure rule ──────────────────────────────────────────────────────

/// The rule that makes a climb safe is a read the proposer already holds
/// at the source (ADR-0034 decision 1), and the privacy floor is what
/// enforces it — with no clause about personal scopes anywhere in the
/// promotion code.
///
/// So a team's own curator cannot climb a teammate's personal memory (no
/// pack permits `MemoryRead` there), and the teammate can climb their own
/// (the self permit does). That second half is the sentence the feature
/// exists for: this is how tribal knowledge leaves one person's head.
#[tokio::test]
async fn only_a_principal_who_can_read_the_source_may_climb_it() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let org = seed_hierarchy(&pool, tenant).await;
    // Ravi's own leaf, nested under the team — the shape CPR-7 admits
    // (a principal may nest under an org unit, ADR-0074 decision 3), and
    // the shape this test needs: a climb to `platform` is structurally
    // legal only from a descendant of platform, and a login-minted leaf
    // at the tenant root is not one. The privacy floor is unimpressed by
    // either position — nothing above a `principal` scope reaches in
    // wherever it hangs.
    let ravi = {
        let mut tx = pool.begin().await.expect("begin");
        let own = scopes::create(
            &mut tx,
            &scopes::NewScope {
                id: ScopeId::new(),
                tenant_id: tenant,
                kind: ScopeKind::Principal,
                parent_scope_id: Some(org.platform.id),
                slug: scopes::principal_slug("ravi@acme.test"),
                display_name: "ravi@acme.test".to_owned(),
                attributes: serde_json::json!({}),
                principal_id: Some("ravi@acme.test".to_owned()),
                created_by: None,
            },
        )
        .await
        .expect("mint nested principal scope");
        let identity = identities::create(
            &mut tx,
            IdentityId::new(),
            tenant,
            Some("ravi@acme.test"),
            IdentityKind::User,
            None,
            None,
            own.id,
        )
        .await
        .expect("create identity");
        tx.commit().await.expect("commit user");
        identity
    };
    seed_user(&pool, tenant, "tara@acme.test").await;
    bind(
        &pool,
        tenant,
        "tara@acme.test",
        org.platform.id,
        RoleKey::Curator,
    )
    .await;
    bind(
        &pool,
        tenant,
        "ravi@acme.test",
        org.platform.id,
        RoleKey::Member,
    )
    .await;
    let (ravi_t, tara) = (
        issue("ravi@acme.test", tenant),
        issue("tara@acme.test", tenant),
    );
    let app = router(state(&database_url()));

    // Ravi's own note, at his own personal leaf — where every extracted
    // memory the write path produces lands (ADR-0033 decision 8).
    let note = seed_record(&pool, tenant, ravi.scope_id, ravi.id, RUNBOOK).await;

    // Tara curates the team and can publish there. She still cannot climb
    // Ravi's note into it, because she cannot read it: the structural
    // check passes (platform *is* an ancestor of the leaf) and the
    // disclosure decision is what refuses.
    let (status, denied) = open_climb(
        &app,
        &tara,
        org.platform.id,
        Some(ravi.scope_id),
        &[note],
        "promote a teammate's note",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a curator climbed a teammate's personal memory: {denied}"
    );

    // Ravi can, and that is the whole point.
    let (status, opened) = open_climb(
        &app,
        &ravi_t,
        org.platform.id,
        Some(ravi.scope_id),
        &[note],
        "promote my runbook to the team",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the owner's own climb: {opened}");
    assert_eq!(
        opened["source_scope_id"].as_str(),
        Some(ravi.scope_id.to_string().as_str())
    );

    // The disclosure is real and it is the point: Tara reviews content she
    // cannot read at its source. Requiring the source read of every
    // reviewer would leave personal material unclimbable forever — the
    // privacy floor admits nobody but the owner, so the review surface is
    // the only place a teammate's climb can be seen at all.
    let id = opened["id"].as_str().expect("proposal id");
    let request = Request::builder()
        .method("GET")
        .uri(format!("/v1/proposals/{id}"))
        .header("authorization", format!("Bearer {tara}"))
        .body(Body::empty())
        .expect("build request");
    let (status, detail_body) = call(&app, request).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the reviewer must see it: {detail_body}"
    );
    assert!(
        detail_body["members"][0]["content"]
            .as_str()
            .is_some_and(|content| content.contains(RUNBOOK)),
        "a reviewer who cannot read the source reviews what the proposal shows them: {detail_body}"
    );
}

// ── What a source holds ──────────────────────────────────────────────────────

/// A climb carries only material its source scope holds, in one of the two
/// senses ADR-0034 decision 3 names — and an edit takes material out of
/// both at once, because the address moves with the content.
#[tokio::test]
async fn a_climb_carries_only_what_its_source_holds() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let org = seed_hierarchy(&pool, tenant).await;
    let ravi = seed_user(&pool, tenant, "ravi@acme.test").await;
    seed_user(&pool, tenant, "dana@acme.test").await;
    seed_user(&pool, tenant, "evan@acme.test").await;
    bind(
        &pool,
        tenant,
        "ravi@acme.test",
        org.platform.id,
        RoleKey::Curator,
    )
    .await;
    // The climb's target is eng, and asking at a scope takes membership
    // or a grant there (CPR-7) — the same grant reads him the platform
    // record through the chain, which is what the disclosure check wants.
    bind(&pool, tenant, "ravi@acme.test", org.eng.id, RoleKey::Member).await;
    bind(
        &pool,
        tenant,
        "dana@acme.test",
        org.eng.id,
        RoleKey::Curator,
    )
    .await;
    // An org-unit publication takes a curator and an administrator, two
    // distinct people, whichever route it arrives by (the matrix golden).
    bind(
        &pool,
        tenant,
        "evan@acme.test",
        org.eng.id,
        RoleKey::Administrator,
    )
    .await;
    let (ravi_t, dana, evan) = (
        issue("ravi@acme.test", tenant),
        issue("dana@acme.test", tenant),
        issue("evan@acme.test", tenant),
    );
    let app = router(state(&database_url()));

    let runbook = seed_record(&pool, tenant, org.platform.id, ravi.id, RUNBOOK).await;
    let elsewhere = seed_record(
        &pool,
        tenant,
        org.payments.id,
        ravi.id,
        "payments closes the ledger on the last business day",
    )
    .await;

    // Not held at the named source: platform neither has the payments
    // record nor publishes it.
    let (status, refused) = open_climb(
        &app,
        &ravi_t,
        org.eng.id,
        Some(org.platform.id),
        &[runbook, elsewhere],
        "climb someone else's record along with mine",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "unheld material climbed: {refused}"
    );
    assert!(
        detail(&refused).contains(&elsewhere.to_string()),
        "the refusal must name the record it refused, not the request: {refused}"
    );

    // The department cannot propose the runbook onward yet either: it
    // neither holds nor publishes it until the first hop lands.
    let (status, early) = open_climb(
        &app,
        &dana,
        org.root.id,
        Some(org.eng.id),
        &[runbook],
        "climb from a scope that has not received it",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an empty source climbed: {early}"
    );

    // Land the first hop, then the second sense opens up.
    let hop1 = climb(
        &app,
        &ravi_t,
        org.eng.id,
        org.platform.id,
        &[runbook],
        "promote the runbook to Engineering",
    )
    .await;
    post(
        &app,
        &dana,
        &format!("/v1/proposals/{hop1}/approve"),
        Value::Null,
    )
    .await;
    post(
        &app,
        &evan,
        &format!("/v1/proposals/{hop1}/approve"),
        Value::Null,
    )
    .await;
    let (status, published) = post(
        &app,
        &dana,
        &format!("/v1/proposals/{hop1}/publish"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "publishing hop 1: {published}");

    let hop2 = climb(
        &app,
        &dana,
        org.root.id,
        org.eng.id,
        &[runbook],
        "the department proposes onward what the team climbed into it",
    )
    .await;

    // Now edit the record. It falls out of *both* senses at once — the
    // address it was approved at no longer matches, and the department's
    // tree no longer names its current content — so the publication is
    // refused by name rather than carried up on a stale review.
    let mut current = records::current(&pool, runbook)
        .await
        .expect("read record")
        .expect("record exists");
    current.state.content = format!("{RUNBOOK} (superseded)");
    records::update(
        &pool,
        runbook,
        &current.state,
        &RecordEmbedding {
            model: DeterministicEmbedder::MODEL.to_owned(),
            vector: vec![0.25; 16],
        },
    )
    .await
    .expect("edit record");

    seed_user(&pool, tenant, "olive@acme.test").await;
    bind(
        &pool,
        tenant,
        "olive@acme.test",
        org.root.id,
        RoleKey::Curator,
    )
    .await;
    seed_user(&pool, tenant, "owen@acme.test").await;
    bind(
        &pool,
        tenant,
        "owen@acme.test",
        org.root.id,
        RoleKey::Administrator,
    )
    .await;
    let olive = issue("olive@acme.test", tenant);
    let owen = issue("owen@acme.test", tenant);
    // Satisfy the root's matrix cell first — curator + administrator, two
    // distinct people — so the refusal below is unambiguously the *edit*
    // and not an unmet approval standing in front of it.
    for token in [&olive, &owen] {
        let (status, approved) = post(
            &app,
            token,
            &format!("/v1/proposals/{hop2}/approve"),
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "approving hop 2: {approved}");
    }
    let (status, conflict) = post(
        &app,
        &olive,
        &format!("/v1/proposals/{hop2}/publish"),
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "an edited record published on a stale review: {conflict}"
    );
    assert!(
        detail(&conflict).contains(&runbook.to_string()),
        "the refusal must name the record that moved: {conflict}"
    );
}
