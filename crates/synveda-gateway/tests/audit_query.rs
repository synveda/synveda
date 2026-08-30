//! AUD-2 acceptance criteria (ADR-0045): "both questions answerable via
//! one API call each (uses bitemporal + refs)."
//!
//! Both questions use the real product surfaces and — this is the point —
//! **never a seeded audit row**. Disclosures exist here because people were
//! actually served Knowledge: alice and bob each create a session context run
//! and the chain records the exact immutable revisions delivered. The audit
//! surface then answers from those events and nothing else.
//!
//! The suite runs under the **real embedded packs** — a governed test
//! Configuration binds `regulated-strict` while retaining full trace evidence.
//! That is load-bearing: a blanket pack would grant `AuditRead` to everyone
//! and make every refusal in this file vacuous, while the zero-binding
//! enterprise fail-safe intentionally retains hashes rather than addresses.
//!
//! Tests need a live Postgres: they read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database), the house convention.

#[path = "../../synveda-store/tests/support/tenant_fixture.rs"]
mod tenant_fixture;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, behavior_test_router as router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder};
use synveda_policy::Pdp;
use synveda_store::{access, identities, knowledge as stored, rls, scopes};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::knowledge::{
    KnowledgeOrigin, KnowledgeRevisionContent, KnowledgeSourceType, KnowledgeType,
};
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{
    GrantId, Identity, IdentityId, IdentityKind, KnowledgeItemId, KnowledgeRevisionId,
    KnowledgeSourceId, ScopeId, Sensitivity, TenantId, TenantStatus, TraceRetentionMode,
};
use tower::ServiceExt;

const SECRET: &[u8] = b"aud-2-test-secret";

/// The material the disclosure question is asked about. Distinctive enough
/// that a leak sweep over a response body cannot miss it.
const RUNBOOK: &str = "Platform incident runbook: freeze the reconciliation job, compare the \
     ledger tail against the acquirer statement, then page the on-call and the controller \
     together.";

/// alice's own note — a second Knowledge item, so "what did alice know" has more
/// than one row and the fold has something to fold.
const NOTE: &str = "Alice prefers rebase-and-merge for her feature branches.";

/// Material only the payments team holds. Never disclosed to anyone in
/// this suite, which is what makes it the honest half of the
/// existence-oracle test: an item that exists and was never served must
/// answer exactly like one that does not exist at all.
const PAYMENTS_ONLY: &str = "Payments settlement cutover checklist, revision four.";

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn state_with(url: &str, pdp: Arc<Pdp>) -> AppState {
    AppState {
        pool: PgPoolOptions::new()
            // Each test owns one app and issues its requests sequentially. A
            // larger pool only multiplies the suite's potential connection
            // footprint by the tests Rust runs concurrently.
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(5))
            .connect_lazy(url)
            .expect("parse database url"),
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
    Hs256Verifier::new(SECRET).issue(subject, tenant_id, Duration::from_secs(300))
}

async fn admitted_tenant() -> Option<(PgPool, TenantId)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping AUD-2 test: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
            return None;
        }
    };
    let pool = PgPoolOptions::new()
        // Fixture writes are sequential within one world. Keep the 16 worlds
        // concurrent without reserving another four connections apiece.
        .max_connections(1)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::epoch::verify(&pool)
        .await
        .expect("apply migrations");
    let id = TenantId::new();
    let slug = format!("aud2-{}", id.as_uuid().simple());
    tenant_fixture::create(&pool, id, &slug, "AUD-2 test tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((pool, id))
}

fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("checked by admitted_tenant")
}

/// One org unit under a parent — the shape every grouping takes now that
/// rank is gone (ADR-0073 decision 4).
async fn unit(tx: &mut sqlx::PgConnection, tenant: TenantId, parent: ScopeId, slug: &str) -> Scope {
    scopes::create(
        tx,
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

async fn seed_scopes(pool: &PgPool, tenant: TenantId) -> (Scope, Scope) {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant tx");
    let root = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("mint root");
    let platform = unit(&mut tx, tenant, root.id, "platform").await;
    let payments = unit(&mut tx, tenant, root.id, "payments").await;
    tx.commit().await.expect("commit scopes");
    (platform, payments)
}

/// A person: their own principal scope under the tenant root, carrying the
/// identity row (CPR-7, ADR-0074 decision 3).
async fn seed_user(pool: &PgPool, tenant: TenantId, subject: &str) -> Identity {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant tx");
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

async fn seed_knowledge(
    pool: &PgPool,
    tenant: TenantId,
    scope: ScopeId,
    owner_principal_id: Option<&str>,
    knowledge_type: KnowledgeType,
    content: &str,
) -> (KnowledgeItemId, KnowledgeRevisionId) {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin Knowledge fixture");
    let source = stored::create_source(
        &mut tx,
        &stored::NewKnowledgeSource {
            id: KnowledgeSourceId::new(),
            tenant_id: tenant,
            scope_id: scope,
            source_type: KnowledgeSourceType::Manual,
            session_event_id: None,
            locator: None,
            source_revision: None,
            content_hash: None,
            metadata: json!({"fixture": "AUD-2/CPR-20"}),
            created_by: Some("audit-fixture".to_owned()),
        },
    )
    .await
    .expect("create Knowledge source");
    let item_id = KnowledgeItemId::new();
    let revision_id = KnowledgeRevisionId::new();
    stored::create_item(
        &mut tx,
        &stored::NewKnowledgeItem {
            id: item_id,
            tenant_id: tenant,
            scope_id: scope,
            project_id: None,
            owner_principal_id: owner_principal_id.map(str::to_owned),
            knowledge_type,
            origin: KnowledgeOrigin::Authored,
            created_by: Some("audit-fixture".to_owned()),
        },
        &stored::NewKnowledgeRevision {
            id: revision_id,
            content: KnowledgeRevisionContent {
                title: content.lines().next().unwrap_or("Knowledge").to_owned(),
                body_markdown: content.to_owned(),
                summary: content.to_owned(),
                tags: vec!["audit-fixture".to_owned()],
                sensitivity: Sensitivity::Internal,
                confidence_permille: 900,
                valid_from: Utc::now(),
                valid_to: None,
                stale_after: None,
                verification_metadata: json!({}),
                metadata: json!({"fixture": "AUD-2/CPR-20"}),
            },
            created_by: Some("audit-fixture".to_owned()),
        },
        &[source.id],
    )
    .await
    .expect("insert Knowledge");
    tx.commit().await.expect("commit Knowledge fixture");
    (item_id, revision_id)
}

/// A direct grant write — the bootstrap path, silent in the chain.
async fn grant(pool: &PgPool, tenant: TenantId, subject: &str, scope: ScopeId, role: RoleKey) {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant tx");
    access::create_grant(
        &mut tx,
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
    let response = app.clone().oneshot(request).await.expect("send request");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("json body")
    };
    (status, value)
}

async fn get(app: &Router, path: &str, token: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("build get request");
    call(app, request).await
}

/// Grant a role through the product surface, so the act lands on the chain
/// as `access.granted` — which is what makes the authority half of a
/// disclosure answer non-empty. A grant written straight to the store
/// (the AUTHZ-3 bootstrap, and what most fixtures do) chains nothing, and
/// an audit answer can only ever report what was recorded.
async fn grant_over_http(
    app: &Router,
    token: &str,
    scope: ScopeId,
    subject: &str,
    role: RoleKey,
) -> StatusCode {
    let request = Request::builder()
        .method("POST")
        .uri("/v1/admin/grants")
        .header("authorization", format!("Bearer {token}"))
        .header("idempotency-key", format!("aud2-grant-{subject}-{scope}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"principal_id": subject, "role": role, "scope_id": scope}).to_string(),
        ))
        .expect("build grant request");
    call(app, request).await.0
}

#[path = "support/configuration.rs"]
mod configuration_support;
#[path = "session_seed.rs"]
mod session_seed;

async fn inject(
    app: &Router,
    token: &str,
    session: synveda_types::SessionId,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(format!("/v1/sessions/{session}/context-runs"))
        .header(
            "idempotency-key",
            synveda_types::ContextRunId::new().to_string(),
        )
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .expect("build inject request");
    call(app, request).await
}

/// RFC 3339 with a `Z` offset — no `+` to survive a query string.
fn stamp(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

/// The world every test starts in: a runbook the whole tenant shares and
/// alice's own note, both served to alice; the runbook also served to
/// bob; a payments item nobody is ever served; and two auditors — dana
/// tenant-wide, erin at platform only.
///
/// The shared material lives at the tenant root because that is where a
/// session actually receives it (CPR-7): a member's own chain is their
/// principal scope and the root, and an org unit composes for nobody
/// without a grant — so the runbook sits at the root and the
/// payments-only item sits where no ungranted reader reaches it.
struct World {
    pool: PgPool,
    tenant: TenantId,
    app: Router,
    alice: String,
    dana: String,
    erin: String,
    runbook: KnowledgeItemId,
    runbook_revision: KnowledgeRevisionId,
    note: KnowledgeItemId,
    payments_only: KnowledgeItemId,
    /// An instant before any inject ran — the "what did alice know
    /// *then*" end of the bitemporal pair.
    before: DateTime<Utc>,
}

async fn world() -> Option<World> {
    let (pool, tenant) = admitted_tenant().await?;
    let (platform, payments) = seed_scopes(&pool, tenant).await;
    let alice = seed_user(&pool, tenant, "alice").await;
    let bob = seed_user(&pool, tenant, "bob").await;
    let _carol = seed_user(&pool, tenant, "carol").await;
    seed_user(&pool, tenant, "dana").await;
    seed_user(&pool, tenant, "erin").await;
    seed_user(&pool, tenant, "olive").await;
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("begin root read");
    let root = scopes::tenant_root(&mut *tx, tenant)
        .await
        .expect("read root")
        .expect("the world minted one");
    tx.commit().await.expect("commit root read");
    // The bootstrap is a direct write, exactly as AUTHZ-3 describes it:
    // the CLI break-glass grant is how a tenant gets its first
    // administrator, and it chains as break-glass rather than as a
    // governed act.
    grant(&pool, tenant, "olive", root.id, RoleKey::Administrator).await;
    // AUD-2 asks address-bearing historical questions, so its runtime
    // Configuration must retain full trace evidence. Bind that choice through
    // a typed VedaFlow change; the policy remains regulated-strict.
    let mut configuration_tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("begin governed Configuration fixture");
    configuration_support::bind_tenant_pack(&mut configuration_tx, tenant, "regulated-strict")
        .await;
    configuration_tx
        .commit()
        .await
        .expect("commit governed Configuration fixture");
    // Membership is a grant now, not a placement: alice and bob hold
    // `member` at platform — the ordinary shape of a person in a unit,
    // kept so the world is not a tenant of strangers, and inert on the
    // inject chain, which is the root and one's own scope and nothing
    // between (CPR-7).
    grant(&pool, tenant, "alice", platform.id, RoleKey::Member).await;
    grant(&pool, tenant, "bob", platform.id, RoleKey::Member).await;

    let (runbook, runbook_revision) = seed_knowledge(
        &pool,
        tenant,
        root.id,
        None,
        KnowledgeType::Procedure,
        RUNBOOK,
    )
    .await;
    let (note, _) = seed_knowledge(
        &pool,
        tenant,
        alice.scope_id,
        Some("alice"),
        KnowledgeType::Preference,
        NOTE,
    )
    .await;
    let (payments_only, _) = seed_knowledge(
        &pool,
        tenant,
        payments.id,
        None,
        KnowledgeType::Procedure,
        PAYMENTS_ONLY,
    )
    .await;

    let pdp = Arc::new(Pdp::new().expect("build the embedded PDP"));
    let app = router(state_with(&database_url(), pdp));

    // The two audit readers, granted through the product surface by the
    // administrator: dana at the tenant root, erin at platform only —
    // `AuditRead` reaches only the tenant resource (ADR-0045 decision 2),
    // so dana's grant reaches the chain and erin's does not. Going
    // through the route is what puts `access.granted` on the chain, which
    // is where historical authority lives — `scope_grants` is a
    // current-state table and a revoked grant leaves no row behind.
    let olive = issue("olive", tenant);
    assert_eq!(
        grant_over_http(&app, &olive, root.id, "dana", RoleKey::Administrator).await,
        StatusCode::CREATED,
        "dana holds administrator at the root, where the chain lives"
    );
    assert_eq!(
        grant_over_http(&app, &olive, platform.id, "erin", RoleKey::Administrator).await,
        StatusCode::CREATED,
        "erin holds the same role at platform only"
    );

    let before = Utc::now();
    let alice_token = issue("alice", tenant);
    let bob_token = issue("bob", tenant);

    // The disclosures this suite answers about are made here, by the product,
    // through the surface a real session uses — three runs, because a
    // composition names the run it was for since CPR-12.
    let alice_run = session_seed::seed_run_for(&pool, tenant, "aud2-alice", "alice").await;
    let bob_run = session_seed::seed_run_for(&pool, tenant, "aud2-bob", "bob").await;
    let alice_second = session_seed::open_run(
        &pool,
        tenant,
        alice_run.workspace_id,
        "aud2-alice-2",
        "alice",
    )
    .await;
    for (who, token, run) in [
        ("alice's session start", &alice_token, alice_run.session_id),
        ("bob's session start", &bob_token, bob_run.session_id),
        ("alice's second session", &alice_token, alice_second),
    ] {
        let (status, body) = inject(&app, token, run).await;
        assert_eq!(status, StatusCode::CREATED, "{who}: {body}");
    }

    let _ = bob;
    Some(World {
        pool,
        tenant,
        app,
        alice: alice_token,
        dana: issue("dana", tenant),
        erin: issue("erin", tenant),
        runbook,
        runbook_revision,
        note,
        payments_only,
        before,
    })
}

/// Today's window, as the route takes it: `from` and an explicit `until`.
fn day_window() -> (String, String) {
    let now = Utc::now();
    (
        stamp(now - ChronoDuration::days(1)),
        stamp(now + ChronoDuration::days(1)),
    )
}

fn subjects(disclosed: &Value) -> Vec<String> {
    disclosed
        .as_array()
        .expect("disclosed is an array")
        .iter()
        .map(|row| row["actor_subject"].as_str().expect("subject").to_owned())
        .collect()
}

// ── The acceptance criterion, first question ─────────────────────────

/// **"Who could see X on date D" — one API call.**
///
/// alice and bob were each served the platform runbook by a context run; carol
/// never was. One `GET /v1/audit/disclosures` names exactly those two, with
/// what each of them actually got, and the answer is stamped with the chain
/// it was taken against so it can be re-derived (ADR-0045 decisions 4
/// and 9).
#[tokio::test]
async fn who_could_see_this_knowledge_is_one_call_answered_from_the_chain() {
    let Some(w) = world().await else { return };
    let (from, until) = day_window();

    let (status, body) = get(
        &w.app,
        &format!(
            "/v1/audit/disclosures?knowledge_item={}&from={from}&until={until}",
            w.runbook
        ),
        &w.dana,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "the auditor's one call: {body}");

    let mut who = subjects(&body["disclosed"]);
    who.sort();
    who.dedup();
    assert_eq!(
        who,
        vec!["alice".to_owned(), "bob".to_owned()],
        "exactly the two the chain records being served the runbook: {body}"
    );
    assert!(
        !who.contains(&"carol".to_owned()),
        "carol started no session, and the chain records nobody was served \
         material they never asked for"
    );

    // Each disclosure carries what that reader got, not merely that they
    // got something (ADR-0041 decision 9).
    let first = &body["disclosed"][0];
    assert_eq!(
        first["knowledge_item_id"].as_str(),
        Some(w.runbook.to_string()).as_deref()
    );
    assert_eq!(
        first["action"].as_str(),
        Some("session.context.composed"),
        "being *given* material is its own act, kept apart from asking for it — \
         and since CPR-12 the act names the run it was for"
    );
    assert_eq!(
        first["knowledge_revision_id"].as_str(),
        Some(w.runbook_revision.to_string()).as_deref(),
        "a disclosure names the exact immutable revision served: {first}"
    );
    assert!(first["content_hash"].is_string(), "and its content hash");
    assert!(first["seq"].is_i64(), "and the chain row that proves it");

    // The completeness stamp (decision 9).
    assert!(body["head_seq"].as_i64().expect("head_seq") > 0);
    assert_eq!(
        body["head_hash"].as_str().map(str::len),
        Some(64),
        "a BLAKE3 head hash in hex"
    );
    assert_eq!(body["truncated"].as_bool(), Some(false));
}

/// **The two lists are never merged** (ADR-0045 decision 4).
///
/// `disclosed` is evidence; `authority` is the state that governed the
/// window. Collapsing them into one "could see" set would mean deciding
/// over reconstructed inputs, which is the replay ADR-0042 option 5
/// refused — so the response keeps them apart and says why in itself,
/// not only in the ADR.
#[tokio::test]
async fn disclosure_and_authority_arrive_as_two_lists_with_the_reason_in_the_response() {
    let Some(w) = world().await else { return };
    let (from, until) = day_window();

    let (status, body) = get(
        &w.app,
        &format!(
            "/v1/audit/disclosures?knowledge_item={}&from={from}&until={until}",
            w.runbook
        ),
        &w.dana,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert!(body["disclosed"].is_array(), "evidence");
    assert!(body["authority"].is_array(), "inputs");
    assert!(
        body.get("could_see").is_none(),
        "there is deliberately no merged set: {body}"
    );

    // The authority half is the events that opened and closed authority —
    // here, the two grants the world established. They exist nowhere
    // else: `scope_grants` is a current-state table, so the chain is the
    // only record that dana held `administrator` today.
    let actions: Vec<&str> = body["authority"]
        .as_array()
        .expect("authority array")
        .iter()
        .map(|event| event["action"].as_str().expect("action"))
        .collect();
    assert!(
        actions.contains(&"access.granted"),
        "the grants that opened authority are on the chain: {actions:?}"
    );

    let note = body["note"]
        .as_str()
        .expect("the note is part of the answer");
    assert!(
        note.contains("not merged") || note.contains("not merged:"),
        "the response states why the two lists are separate: {note}"
    );
}

// ── The acceptance criterion, second question ────────────────────────

/// **"What did agent A know at time T" — one API call, over bitemporal +
/// refs.**
///
/// One `GET /v1/audit/knowledge` returns what alice was *served*, folded
/// to one row per Knowledge item with the revision last delivered. The AC's
/// "uses bitemporal + refs" is asserted rather than asserted-about: every
/// id in the answer resolves in the bitemporal pair at the instant asked
/// at, and each row names the version by hash.
#[tokio::test]
async fn what_did_this_agent_know_is_one_call_and_its_ids_resolve_bitemporally() {
    let Some(w) = world().await else { return };
    let at = Utc::now();

    let (status, body) = get(
        &w.app,
        &format!(
            "/v1/audit/knowledge?subject=alice&valid_at={0}&as_known_at={0}",
            stamp(at)
        ),
        &w.dana,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the auditor's one call: {body}");

    let known = body["known"].as_array().expect("known array");
    let ids: Vec<String> = known
        .iter()
        .map(|row| {
            row["knowledge_item_id"]
                .as_str()
                .expect("knowledge_item_id")
                .to_owned()
        })
        .collect();
    assert!(
        ids.contains(&w.runbook.to_string()),
        "alice was served the runbook: {body}"
    );
    assert!(
        ids.contains(&w.note.to_string()),
        "and her own note: {body}"
    );
    assert!(
        !ids.contains(&w.payments_only.to_string()),
        "and never payments material"
    );

    // One row per item, not one per delivery: alice composed twice, so
    // the runbook was served twice and folds to a single row that says so.
    assert_eq!(
        ids.len(),
        known
            .iter()
            .map(|row| row["knowledge_item_id"].as_str().expect("id"))
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        "the fold is one row per Knowledge item: {body}"
    );
    let runbook_row = known
        .iter()
        .find(|row| row["knowledge_item_id"].as_str() == Some(&w.runbook.to_string()))
        .expect("the runbook row");
    assert_eq!(
        runbook_row["occasions"].as_u64(),
        Some(2),
        "alice was served the runbook in both her sessions: {runbook_row}"
    );

    // **refs**: the row names the version it delivered.
    assert_eq!(
        runbook_row["knowledge_revision_id"].as_str(),
        Some(w.runbook_revision.to_string()).as_deref(),
        "the answer names the exact revision, which makes it checkable: {runbook_row}"
    );
    assert!(runbook_row["content_hash"].is_string());
    assert_eq!(runbook_row["temporal_status"].as_str(), Some("valid"));
    assert!(runbook_row["valid_from"].is_string());
    assert!(runbook_row["transaction_time"].is_string());

    // **bitemporal**: every id the answer names resolves to the version
    // that was current at the instant asked at. The audit answer and the
    // corpus agree, which is the whole "uses bitemporal + refs" clause.
    let mut tx = synveda_store::rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("tenant tx");
    for id in &ids {
        let item_id: KnowledgeItemId = id.parse().expect("Knowledge item id");
        let revision_id: KnowledgeRevisionId = known
            .iter()
            .find(|row| row["knowledge_item_id"].as_str() == Some(id.as_str()))
            .and_then(|row| row["knowledge_revision_id"].as_str())
            .expect("disclosure carries a revision")
            .parse()
            .expect("Knowledge revision id");
        let version = stored::revision(&mut *tx, w.tenant, item_id, revision_id)
            .await
            .expect("exact revision read");
        let version = version.unwrap_or_else(|| {
            panic!("{item_id}/{revision_id} was disclosed at {at} and must remain immutable")
        });
        assert_eq!(
            version.tenant_id, w.tenant,
            "and it resolves inside the asking tenant"
        );
    }
    drop(tx);

    // And the answer says what it is, in itself.
    assert!(
        body["note"]
            .as_str()
            .expect("note")
            .contains("not what they"),
        "the response states that this is what A was served, not what A \
         was permitted to ask for: {body}"
    );
}

/// The instant is load-bearing: asked *before* any session ran, alice knew
/// nothing. Same call, same subject, same corpus — only both explicit time
/// axes differ.
#[tokio::test]
async fn the_instant_decides_what_the_answer_contains() {
    let Some(w) = world().await else { return };

    let (status, before) = get(
        &w.app,
        &format!(
            "/v1/audit/knowledge?subject=alice&valid_at={0}&as_known_at={0}",
            stamp(w.before)
        ),
        &w.dana,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        before["known"].as_array().expect("known").is_empty(),
        "before her first session alice had been served nothing: {before}"
    );

    let (status, now) = get(&w.app, "/v1/audit/knowledge?subject=alice", &w.dana).await;
    assert_eq!(status, StatusCode::OK, "both time axes default to now");
    assert!(
        !now["known"].as_array().expect("known").is_empty(),
        "and by now she has been: {now}"
    );
}

/// A hashes-only runtime deliberately drops aggregate and revision addresses.
/// The historical question must retain the hash as unresolved evidence rather
/// than silently dropping the disclosure or fabricating an address.
#[tokio::test]
async fn hashes_only_disclosures_remain_visible_as_unresolved_hash_evidence() {
    let Some(w) = world().await else { return };
    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin hashes-only Configuration change");
    let root = scopes::tenant_root(&mut *tx, w.tenant)
        .await
        .expect("read tenant root")
        .expect("tenant root exists");
    configuration_support::set_trace_retention(
        &mut tx,
        w.tenant,
        root.id,
        TraceRetentionMode::HashesOnly,
    )
    .await;
    tx.commit()
        .await
        .expect("commit hashes-only Configuration change");

    let run = session_seed::seed_run_for(&w.pool, w.tenant, "aud2-hashes", "alice").await;
    let (status, body) = inject(&w.app, &w.alice, run.session_id).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "hashes-only composition: {body}"
    );

    let (status, body) = get(&w.app, "/v1/audit/knowledge?subject=alice&limit=1", &w.dana).await;
    assert_eq!(status, StatusCode::OK, "hashes-only audit answer: {body}");
    assert_eq!(body["known"], json!([]));
    assert_eq!(body["outside_time"], json!([]));
    let unresolved = body["unresolved"].as_array().expect("unresolved array");
    assert!(
        !unresolved.is_empty(),
        "hash evidence must not disappear: {body}"
    );
    for row in unresolved {
        assert!(
            row.get("knowledge_item_id").is_none(),
            "no invented item ID: {row}"
        );
        assert!(
            row.get("knowledge_revision_id").is_none(),
            "no invented revision ID: {row}"
        );
        assert!(
            row["content_hash"].is_string(),
            "retained hash evidence: {row}"
        );
        assert_eq!(row["temporal_status"], "unresolved");
    }
}

// ── The refusals ─────────────────────────────────────────────────────

/// **A subtree-bound audit reader is refused rather than served a subset**
/// (ADR-0045 decision 2).
///
/// erin holds exactly the role dana holds — `administrator` — granted at
/// platform instead of at the tenant root. There is one chain per tenant
/// and no way to answer for part of it without silently omitting the
/// events whose `resource` string does not name a scope, so the answer is
/// a refusal. `AuditRead` reaches only the tenant resource, so this holds
/// for every route without any of them checking.
#[tokio::test]
async fn a_subtree_bound_auditor_is_refused_rather_than_served_a_subset() {
    let Some(w) = world().await else { return };
    let (from, until) = day_window();

    for path in [
        "/v1/audit/events".to_owned(),
        format!(
            "/v1/audit/disclosures?knowledge_item={}&from={from}&until={until}",
            w.runbook
        ),
        "/v1/audit/knowledge?subject=alice".to_owned(),
        "/v1/audit/export?limit=2".to_owned(),
        "/v1/audit/verify".to_owned(),
    ] {
        let (status, body) = get(&w.app, &path, &w.erin).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "erin's platform grant must not reach the tenant's chain at {path}: {body}"
        );
    }

    // And the same five are open to the same role held at the root, so
    // the refusal is about *where the grant is written* and not about the
    // role.
    let (status, _) = get(&w.app, "/v1/audit/verify", &w.dana).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "dana's root grant reaches the chain"
    );
}

/// An ordinary member holds nothing on the audit plane: alice can be
/// *asked about* and cannot ask (ADR-0015 decision 4 — no role, no
/// administrative power, under every pack).
#[tokio::test]
async fn the_subject_of_an_audit_answer_cannot_read_the_audit_plane() {
    let Some(w) = world().await else { return };

    for path in [
        "/v1/audit/events",
        "/v1/audit/knowledge?subject=alice",
        "/v1/audit/export",
    ] {
        let (status, body) = get(&w.app, path, &w.alice).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "an unbound member reads no trail at {path}: {body}"
        );
    }
}

// ── The properties that hold across every route ──────────────────────

/// **No content reaches an audit answer** (ADR-0045 decision 6).
///
/// Swept over every route's full response body: an auditor holds
/// `AuditRead` and no `KnowledgeRead`, and the surface has no content path to
/// forget to gate. Knowledge bodies are what an auditor would otherwise
/// acquire here, and this is the last route by which they could.
#[tokio::test]
async fn no_knowledge_content_reaches_any_audit_answer() {
    let Some(w) = world().await else { return };
    let (from, until) = day_window();

    for path in [
        "/v1/audit/events?limit=500".to_owned(),
        format!(
            "/v1/audit/disclosures?knowledge_item={}&from={from}&until={until}",
            w.runbook
        ),
        "/v1/audit/knowledge?subject=alice".to_owned(),
        "/v1/audit/export?limit=500".to_owned(),
        "/v1/audit/verify".to_owned(),
    ] {
        let (status, body) = get(&w.app, &path, &w.dana).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        let rendered = body.to_string();
        for content in [RUNBOOK, NOTE, PAYMENTS_ONLY] {
            assert!(
                !rendered.contains(content),
                "Knowledge content reached {path} — an auditor reads no content (seed §5)"
            );
        }
        // The distinctive middles too, in case a renderer ever truncates.
        for fragment in [
            "acquirer statement",
            "rebase-and-merge",
            "cutover checklist",
        ] {
            assert!(
                !rendered.contains(fragment),
                "a fragment of Knowledge content reached {path}: {fragment}"
            );
        }
    }
}

/// **The surface is not an existence oracle** (ADR-0045 compliance notes).
///
/// A Knowledge item that does not exist and one that exists but was never served
/// answer identically — same status, same shape, same empty list. An
/// auditor who could tell them apart would have a membership oracle over
/// the corpus, which is exactly what `recall`'s uniform refusal exists to
/// prevent (ADR-0041 decision 6).
#[tokio::test]
async fn a_disclosure_answer_is_not_an_existence_oracle() {
    let Some(w) = world().await else { return };
    let (from, until) = day_window();
    let nonexistent = KnowledgeItemId::new();

    let mut answers = Vec::new();
    for knowledge_item in [nonexistent, w.payments_only] {
        let (status, body) = get(
            &w.app,
            &format!(
                "/v1/audit/disclosures?knowledge_item={knowledge_item}&from={from}&until={until}"
            ),
            &w.dana,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "both answer, neither errors");
        assert!(
            body["disclosed"].as_array().expect("disclosed").is_empty(),
            "neither was ever served: {body}"
        );
        answers.push(body);
    }

    assert_eq!(
        answers[0]["disclosed"], answers[1]["disclosed"],
        "a Knowledge item that never existed and one that was never served are \
         indistinguishable in the answer"
    );
    assert_eq!(
        answers[0]["truncated"], answers[1]["truncated"],
        "including in the completeness stamp"
    );
}

/// **A truncated page says so, and its cursor advances** (ADR-0045
/// decision 9).
///
/// Walked at `limit=1` over the runbook's three-plus disclosures: each
/// page reports itself truncated with a cursor, and the walk collects
/// every disclosure the single-page answer contains. This is the property
/// the page arithmetic exists for — a page whose rows filter out must
/// still advance rather than report itself complete.
#[tokio::test]
async fn a_truncated_page_reports_itself_and_its_cursor_walks_the_whole_answer() {
    let Some(w) = world().await else { return };
    let (from, until) = day_window();

    let (_, whole) = get(
        &w.app,
        &format!(
            "/v1/audit/disclosures?knowledge_item={}&from={from}&until={until}",
            w.runbook
        ),
        &w.dana,
    )
    .await;
    let total = whole["disclosed"].as_array().expect("disclosed").len();
    assert!(total >= 2, "the world serves the runbook more than once");

    let mut walked = Vec::new();
    let mut after = 0_i64;
    for _ in 0..(total + 2) {
        let (status, page) = get(
            &w.app,
            &format!(
                "/v1/audit/disclosures?knowledge_item={}&from={from}&until={until}&limit=1&after={after}",
                w.runbook
            ),
            &w.dana,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        walked.extend(subjects(&page["disclosed"]));
        match page["next_cursor"].as_i64() {
            Some(next) => {
                assert_eq!(
                    page["truncated"].as_bool(),
                    Some(true),
                    "a page with a cursor is a truncated page: {page}"
                );
                assert!(
                    next > after,
                    "the cursor must advance: {next} after {after}"
                );
                after = next;
            }
            None => {
                assert_eq!(
                    page["truncated"].as_bool(),
                    Some(false),
                    "the last page is not truncated: {page}"
                );
                break;
            }
        }
    }

    assert_eq!(
        walked.len(),
        total,
        "walking the cursor collects exactly what one page contained"
    );
}

/// **Reading the trail is itself on the trail** (ADR-0045 decision 8).
///
/// An allowed admin-plane read chains its decision, so an audit query
/// appears in the next audit query's results. That is a property rather
/// than an accident: "who has been reading the trail" is a question a
/// regulator asks.
#[tokio::test]
async fn reading_the_trail_appears_in_the_next_reading_of_it() {
    let Some(w) = world().await else { return };

    let (status, first) = get(&w.app, "/v1/audit/events?limit=5", &w.dana).await;
    assert_eq!(status, StatusCode::OK);
    let head_after_first = first["head_seq"].as_i64().expect("head_seq");

    let (status, second) = get(
        &w.app,
        "/v1/audit/events?action=authz.decision&outcome=allow&limit=200",
        &w.dana,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let read_by_dana = second["events"]
        .as_array()
        .expect("events")
        .iter()
        .any(|event| {
            event["actor_subject"].as_str() == Some("dana")
                && event["payload"]["authz"]["action"].as_str() == Some("audit.read")
        });
    assert!(
        read_by_dana,
        "dana's first query is on the chain the second one reads: {second}"
    );

    assert!(
        second["head_seq"].as_i64().expect("head_seq") > head_after_first,
        "the chain grew because it was read — which is why the pages are \
         cursor-paginated and not offset-paginated"
    );
}

/// The chain check, reachable by an auditor holding no `DATABASE_URL`
/// (ADR-0045 decision 1) — the surface `synveda audit verify` was standing
/// in for since AUD-1.
#[tokio::test]
async fn the_chain_verifies_over_everything_this_suite_wrote() {
    let Some(w) = world().await else { return };

    let (status, body) = get(&w.app, "/v1/audit/verify", &w.dana).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["valid"].as_bool(),
        Some(true),
        "the chain this suite built verifies: {body}"
    );
    let events = body["events"].as_i64().expect("events");
    assert!(events > 0);
    assert_eq!(body["head_seq"].as_i64(), Some(events));
    assert_eq!(
        body["head_hash"].as_str().map(str::len),
        Some(64),
        "the response carries the exact verified BLAKE3 head: {body}"
    );
    assert!(body["broken_at"].is_null(), "nothing is broken: {body}");
}

/// A search filters by the whole vocabulary the AC names — actor, action,
/// time — and **denials are an ordinary filter value**, which is the AC's
/// "auditor role read-only incl. denials".
#[tokio::test]
async fn the_search_filters_by_actor_action_and_outcome_including_denials() {
    let Some(w) = world().await else { return };

    // Produce a denial to find: alice asking for the audit plane.
    let (status, _) = get(&w.app, "/v1/audit/events", &w.alice).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, denials) = get(
        &w.app,
        "/v1/audit/events?outcome=deny&action=authz.decision&limit=200",
        &w.dana,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let found = denials["events"]
        .as_array()
        .expect("events")
        .iter()
        .any(|event| event["actor_subject"].as_str() == Some("alice"));
    assert!(found, "alice's refusal is readable as a denial: {denials}");

    // By actor: every row is the actor asked for.
    let (status, by_actor) = get(&w.app, "/v1/audit/events?actor=bob&limit=200", &w.dana).await;
    assert_eq!(status, StatusCode::OK);
    for event in by_actor["events"].as_array().expect("events") {
        assert_eq!(event["actor_subject"].as_str(), Some("bob"));
    }
    assert!(
        !by_actor["events"].as_array().expect("events").is_empty(),
        "bob composed context, so bob is on the chain"
    );

    // By action: every row is the action asked for.
    let (status, compositions) = get(
        &w.app,
        "/v1/audit/events?action=session.context.composed&limit=200",
        &w.dana,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    for event in compositions["events"].as_array().expect("events") {
        assert_eq!(event["action"].as_str(), Some("session.context.composed"));
    }
}

/// Typed payload filters address exact governed nouns rather than searching
/// display strings or arbitrary JSON text. The context event is the strongest
/// fixture because it cites the selected immutable Knowledge revision and the
/// session/run that received it in one recorded decision.
#[tokio::test]
async fn typed_artifact_session_and_context_filters_select_exact_evidence() {
    let Some(w) = world().await else { return };

    let (status, all) = get(
        &w.app,
        "/v1/audit/events?action=session.context.composed&limit=200",
        &w.dana,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "composition evidence: {all}");
    let event = all["events"]
        .as_array()
        .expect("events")
        .iter()
        .find(|event| {
            event["payload"]["artifact_references"]
                .as_array()
                .is_some_and(|references| {
                    references.iter().any(|reference| {
                        reference["family"] == "knowledge"
                            && reference["artifact_id"] == w.runbook.to_string()
                            && reference["version"] == w.runbook_revision.to_string()
                    })
                })
        })
        .expect("one composition selected the runbook");
    let session_id = event["payload"]["session_id"].as_str().expect("session id");
    let context_run_id = event["payload"]["context_run_id"]
        .as_str()
        .expect("context run id");
    let configuration = &event["payload"]["configuration"];
    for field in [
        "binding_id",
        "binding_scope_id",
        "artifact_id",
        "version_id",
        "content_hash",
        "policy_pack",
    ] {
        assert!(
            configuration[field].is_string(),
            "composition evidence must freeze effective Configuration {field}: {event}"
        );
    }
    assert!(
        event["payload"]["relaxations"].is_array(),
        "the exact active relaxation-version set is recorded even when empty: {event}"
    );

    for path in [
        format!(
            "/v1/audit/events?artifact_family=knowledge&artifact_id={}&artifact_version={}&limit=200",
            w.runbook, w.runbook_revision
        ),
        format!("/v1/audit/events?session_id={session_id}&limit=200"),
        format!("/v1/audit/events?context_run_id={context_run_id}&limit=200"),
    ] {
        let (status, filtered) = get(&w.app, &path, &w.dana).await;
        assert_eq!(status, StatusCode::OK, "typed filter {path}: {filtered}");
        assert!(
            !filtered["events"].as_array().expect("events").is_empty(),
            "the exact evidence is addressable through {path}: {filtered}"
        );
    }
}

/// Export pages are one frozen prefix even though every page read appends a
/// later audit event. The public JSON alone is sufficient to recompute every
/// link and the tenant-bound genesis without querying Postgres.
#[tokio::test]
async fn deterministic_export_freezes_before_its_own_reads_and_verifies_offline() {
    let Some(w) = world().await else { return };
    let (status, first) = get(&w.app, "/v1/audit/export?after=0&limit=3", &w.dana).await;
    assert_eq!(status, StatusCode::OK, "first export page: {first}");
    let snapshot = first["snapshot_seq"].as_i64().expect("snapshot seq");
    let mut after = 0_i64;
    let mut page = first.clone();
    let mut events = Vec::new();
    loop {
        for field in [
            "format",
            "hash_algorithm",
            "canonicalization",
            "tenant_id",
            "genesis_hash",
            "snapshot_seq",
            "snapshot_hash",
        ] {
            assert_eq!(page[field], first[field], "frozen {field}: {page}");
        }
        events.extend(page["events"].as_array().expect("events").iter().cloned());
        match page["next_cursor"].as_i64() {
            Some(next) => {
                assert!(next > after, "cursor advances: {page}");
                after = next;
                let (status, next_page) = get(
                    &w.app,
                    &format!("/v1/audit/export?after={after}&through={snapshot}&limit=3"),
                    &w.dana,
                )
                .await;
                assert_eq!(status, StatusCode::OK, "next export page: {next_page}");
                page = next_page;
            }
            None => break,
        }
    }

    let assembled = json!({
        "format": first["format"],
        "hash_algorithm": first["hash_algorithm"],
        "canonicalization": first["canonicalization"],
        "tenant_id": first["tenant_id"],
        "genesis_hash": first["genesis_hash"],
        "snapshot_seq": first["snapshot_seq"],
        "snapshot_hash": first["snapshot_hash"],
        "events": events,
    });
    let verified = synveda_audit::verify_export(&assembled).expect("offline verification");
    assert_eq!(verified.tenant_id, w.tenant);
    assert_eq!(verified.head_seq, snapshot);
    assert_eq!(verified.events, snapshot);
    assert!(
        assembled["events"]
            .as_array()
            .expect("events")
            .iter()
            .all(|event| event["payload"]["op"] != "export"),
        "the prefix froze before this export's own audited reads: {assembled}"
    );
}

/// A misspelled action is a 400, not an empty answer: "no events" and "you
/// spelled it wrong" are different facts, and only one of them is an audit
/// finding. Same for a limit over the cap, which is refused rather than
/// silently trimmed — an audit surface must not quietly return less than
/// it was asked for.
#[tokio::test]
async fn a_typo_is_refused_rather_than_answered_with_nothing() {
    let Some(w) = world().await else { return };

    for path in [
        "/v1/audit/events?action=session.context.compozed",
        "/v1/audit/events?outcome=denied",
        "/v1/audit/events?limit=1001",
        "/v1/audit/events?limit=0",
        "/v1/audit/events?after=-1",
        "/v1/audit/events?artifact_family=unknown",
        "/v1/audit/events?artifact_id=orphan",
        "/v1/audit/events?artifact_version=orphan",
        "/v1/audit/export?after=-1",
        "/v1/audit/export?through=-1",
    ] {
        let (status, body) = get(&w.app, path, &w.dana).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{path} must be refused, never answered emptily: {body}"
        );
    }
}
