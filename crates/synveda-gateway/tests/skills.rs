//! SKIL-1 acceptance criteria (ADR-0051), over the real product surfaces.
//!
//! The criterion is "a skill authored in Synveda installs and runs
//! unmodified in Claude Code and one other client". Its three verbs are
//! measured in three different places, and only the first two are here:
//!
//! - **authored … only through review** is asserted from the *consumer's*
//!   side, exactly as PRMT-1 and PRMT-2 assert theirs — with one difference
//!   that is decision 18's whole point: the direct publish route refuses
//!   under **every** pack, not only under `regulated-strict`, because the
//!   invariant floor asks for a security reviewer and two distinct
//!   approvers and no pack may opt out of either.
//! - **installs unmodified** is a hash comparison. This suite asserts the
//!   half the server owns: a resolve returns the reviewed bytes and every
//!   file's content address recomputes from them. The other half — two
//!   clients' roots holding byte-identical trees — is `demos/skil-1-skills.sh`,
//!   because it is a claim about a filesystem.
//! - **runs** is the demo's top layer, and its behavioural half is deferred
//!   with a recorded trigger (ADR-0051): whether a *model* reaches for a
//!   skill is a property of the model.
//!
//! Around them: the spec's own rules enforced where they can still be
//! fixed, the gradient walk, the pin a rewind refuses, the scanner that
//! keeps a credential off a laptop, and the chain — where every act appears
//! and no payload carries file content.
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
use synveda_identity::{Hs256Verifier, personal_slug};
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder};
use synveda_policy::Pdp;
use synveda_retrieval::index::SearchIndex;
use synveda_store::{hierarchy, identities, policy_assignments, role_bindings, tenants};
use synveda_types::{
    HierarchyNode, Identity, IdentityId, IdentityKind, Role, ScopeId, ScopeKind, Sensitivity,
    SkillFile, TenantId, TenantStatus,
};
use synveda_vedaflow::SkillAsset;
use tower::ServiceExt;

const SECRET: &[u8] = b"skil-1-test-secret";

/// The skill under review, version 1 and version 2. The edit is a behaviour
/// change a client would run, which is the point: it must not reach one
/// until two people have said so, one of them a security reviewer.
fn manifest(body: &str) -> String {
    format!(
        "---\n\
         name: code-review\n\
         description: Review a diff and report defects. Use when asked to review changes.\n\
         allowed-tools:\n\
         \x20 - Read\n\
         \x20 - Bash(git diff *)\n\
         ---\n\
         \n\
         # Code Review\n\
         \n\
         {body}\n"
    )
}

const V1_BODY: &str = "Read the diff and report every defect you would fix.";
const V2_BODY: &str = "Read the diff, report every defect, and rewrite the tests.";
const SCRIPT: &str = "import sys\nprint('checked', sys.argv[1:])\n";

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn index_root() -> std::path::PathBuf {
    std::env::temp_dir()
        .join("synveda-skil1-tests")
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
        pdp,
        scope_chains: Arc::new(synveda_store::ScopeChainCache::new()),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index: Arc::new(SearchIndex::open(index_root()).expect("open sidecar")),
        embedder: Arc::new(AnyEmbedder::Deterministic(DeterministicEmbedder::new())),
        inject_embed_timeout: Duration::from_millis(100),
    }
}

fn issue(subject: &str, tenant_id: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue(subject, tenant_id, Duration::from_secs(300))
}

/// The fixture: one tenant, `acme → eng → {platform, payments}`, and the
/// people a *skill* publication needs — which is one more than a prompt's,
/// because the invariant floor asks for a security reviewer on every one.
struct World {
    pool: PgPool,
    tenant: TenantId,
    app: Router,
    org: ScopeId,
    eng: ScopeId,
    platform: ScopeId,
    /// The author: a contributor at the platform team.
    alice: String,
    /// The curator who runs the effect. Placed at the platform team, so the
    /// membership floor supplies the `SkillRead` publishing also takes.
    cora: String,
    /// The steward the pack asks for — placed in the *other* team, so his
    /// authority here is a role binding and nothing else (ADR-0049's
    /// finding, inherited).
    sam: String,
    /// The security reviewer the **floor** asks for, on every skill, under
    /// every pack. Bound rather than placed, for `sam`'s reason.
    sec: String,
    /// The consumer: placed at the platform team, holding no role at all.
    bea: String,
    /// Someone in the other team, for the gradient.
    dave: String,
}

async fn world() -> Option<World> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping SKIL-1 skills test: DATABASE_URL is not set \
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
    let slug = format!("skil1-{}", tenant.as_uuid().simple());
    tenants::create(
        &pool,
        tenant,
        &slug,
        "SKIL-1 test tenant",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");

    let mut tx = pool.begin().await.expect("begin");
    let org = node(&mut tx, tenant, None, ScopeKind::Org, "acme").await;
    let eng = node(&mut tx, tenant, Some(org.id), ScopeKind::Department, "eng").await;
    let platform = node(&mut tx, tenant, Some(eng.id), ScopeKind::Team, "platform").await;
    let payments = node(&mut tx, tenant, Some(eng.id), ScopeKind::Team, "payments").await;
    node(
        &mut tx,
        tenant,
        Some(org.id),
        ScopeKind::Team,
        identities::QUARANTINE_SLUG,
    )
    .await;
    tx.commit().await.expect("commit hierarchy");

    for (subject, parent) in [
        ("alice", platform.id),
        ("cora", platform.id),
        ("sam", payments.id),
        ("sec", payments.id),
        ("bea", platform.id),
        ("dave", payments.id),
    ] {
        seed_user(&pool, tenant, subject, parent).await;
    }
    bind(&pool, tenant, "alice", platform.id, Role::Contributor).await;
    bind(&pool, tenant, "cora", platform.id, Role::Curator).await;
    bind(&pool, tenant, "sam", platform.id, Role::Steward).await;
    bind(&pool, tenant, "sec", platform.id, Role::SecurityReviewer).await;
    // The org too, so a climb has approvers waiting where it lands.
    bind(&pool, tenant, "cora", org.id, Role::Curator).await;
    bind(&pool, tenant, "sam", org.id, Role::Steward).await;
    bind(&pool, tenant, "sec", org.id, Role::SecurityReviewer).await;

    let pdp = Arc::new(Pdp::new().expect("build the embedded PDP"));
    let app = router(state(&url, pdp.clone()));
    Some(World {
        pool,
        tenant,
        app,
        org: org.id,
        eng: eng.id,
        platform: platform.id,
        alice: issue("alice", tenant),
        cora: issue("cora", tenant),
        sam: issue("sam", tenant),
        sec: issue("sec", tenant),
        bea: issue("bea", tenant),
        dave: issue("dave", tenant),
    })
}

async fn node(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    tenant: TenantId,
    parent: Option<ScopeId>,
    kind: ScopeKind,
    slug: &str,
) -> HierarchyNode {
    hierarchy::create(tx, ScopeId::new(), tenant, parent, kind, slug, slug)
        .await
        .expect("create node")
}

async fn seed_user(pool: &PgPool, tenant: TenantId, subject: &str, parent: ScopeId) -> Identity {
    let mut tx = pool.begin().await.expect("begin");
    let id = IdentityId::new();
    let leaf = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(parent),
        ScopeKind::User,
        &personal_slug(None, subject, id),
        subject,
    )
    .await
    .expect("create personal scope");
    let identity = identities::create(
        &mut tx,
        id,
        tenant,
        subject,
        IdentityKind::User,
        None,
        None,
        leaf.id,
    )
    .await
    .expect("create identity");
    tx.commit().await.expect("commit user");
    identity
}

async fn bind(pool: &PgPool, tenant: TenantId, subject: &str, scope: ScopeId, role: Role) {
    let mut tx = pool.begin().await.expect("begin");
    role_bindings::bind(&mut *tx, tenant, subject, Some(scope), role)
        .await
        .expect("bind role");
    tx.commit().await.expect("commit binding");
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
/// `reason` on a policy denial.
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

/// Authors the two-file bundle at a scope.
async fn author(w: &World, token: &str, scope: ScopeId, body: &str) -> (StatusCode, Value) {
    author_files(
        w,
        token,
        scope,
        "code-review",
        &[
            ("SKILL.md", manifest(body)),
            ("scripts/check.py", SCRIPT.to_owned()),
        ],
    )
    .await
}

async fn author_files(
    w: &World,
    token: &str,
    scope: ScopeId,
    name: &str,
    files: &[(&str, String)],
) -> (StatusCode, Value) {
    post(
        &w.app,
        "/v1/skills",
        token,
        json!({
            "scope_id": scope,
            "name": name,
            "files": files.iter().map(|(path, content)| json!({
                "path": path,
                "content": content,
            })).collect::<Vec<_>>(),
        }),
    )
    .await
}

/// The client's call: resolve by name, walking the caller's own chain.
async fn resolve(w: &World, token: &str, name: &str) -> (StatusCode, Value) {
    get(&w.app, &format!("/v1/skills/{name}"), token).await
}

/// Carries a bundle from a draft to a published version through the review
/// **every** pack asks for: alice proposes, sam (steward) and sec (security
/// reviewer) approve, cora runs the effect. Returns the commit.
async fn review_and_publish(w: &World, scope: ScopeId, name: &str, title: &str) -> String {
    let (status, opened) = post(
        &w.app,
        "/v1/proposals",
        &w.alice,
        json!({"scope_id": scope, "skill_names": [name], "title": title}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "open a skill proposal: {opened}");
    let id = opened["id"].as_str().expect("proposal id").to_owned();

    for token in [&w.sam, &w.sec] {
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

/// A resolved bundle's files, path → content.
fn files(resolved: &Value) -> std::collections::BTreeMap<String, String> {
    resolved["files"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|file| {
            (
                file["path"].as_str().unwrap_or_default().to_owned(),
                file["content"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

// ── The acceptance criterion, first half ─────────────────────────────────────

/// **A skill reaches a client only through the review the pack in force
/// asks for** — and unlike a prompt or a pack, that is true under *every*
/// pack, because the invariant floor asks for a security reviewer and two
/// distinct approvers (ADR-0051 decision 18).
#[tokio::test]
async fn a_skill_reaches_a_client_only_through_review_under_every_pack() {
    let Some(w) = world().await else { return };

    // 1. Authored. Nothing a client installs has moved.
    let (status, draft) = author(&w, &w.alice, w.platform, V1_BODY).await;
    assert_eq!(status, StatusCode::OK, "author the draft: {draft}");
    assert_eq!(draft["name"], json!("code-review"), "{draft}");
    assert!(
        draft["published_commit"].is_null(),
        "nothing is published yet: {draft}"
    );
    // The frontmatter parse is what the registry knows the skill *by*, and
    // it came out of the artefact rather than beside it.
    assert_eq!(
        draft["frontmatter"]["name"],
        json!("code-review"),
        "{draft}"
    );
    assert_eq!(
        draft["frontmatter"]["allowed-tools"],
        json!(["Read", "Bash(git diff *)"]),
        "the client keys a reviewer must see are kept: {draft}"
    );

    let (status, absent) = resolve(&w, &w.bea, "code-review").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a draft is not a version anyone installs: {absent}"
    );

    // 2. The direct route refuses, and the refusal is the floor speaking.
    let (status, refused) = post(
        &w.app,
        &format!("/v1/channels/{}/publish", w.platform),
        &w.cora,
        json!({"skill_names": ["code-review"], "message": "ship it"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a curator alone cannot publish executable code: {refused}"
    );
    let refusal = detail(&refused);
    assert!(
        refusal.contains("security-reviewer"),
        "the refusal names the role the floor requires: {refusal}"
    );
    assert!(
        refusal.contains("proposal"),
        "and where to go with it: {refusal}"
    );

    // 3. **Decision 18.** The same refusal under `standard`, whose whole
    //    content is that publication is cheaper there — for memory, for
    //    prompts and for packs, and deliberately not for code. Before this
    //    feature the cell resolved to one signature, so one person holding
    //    both roles shipped a script alone.
    let mut tx = w.pool.begin().await.expect("begin");
    policy_assignments::assign(&mut *tx, w.tenant, w.org, "standard")
        .await
        .expect("assign standard at the org");
    tx.commit().await.expect("commit assignment");

    let (status, refused_standard) = post(
        &w.app,
        &format!("/v1/channels/{}/publish", w.platform),
        &w.cora,
        json!({"skill_names": ["code-review"], "message": "ship it"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "`standard` does not make shipping code a one-signature act: {refused_standard}"
    );
    assert!(
        detail(&refused_standard).contains("security-reviewer"),
        "{}",
        detail(&refused_standard)
    );

    let mut tx = w.pool.begin().await.expect("begin");
    policy_assignments::unassign(&mut *tx, w.tenant, w.org)
        .await
        .expect("clear the assignment");
    tx.commit().await.expect("commit clear");

    // 4. The review carries it, and bea's very next call resolves it.
    let commit = review_and_publish(&w, w.platform, "code-review", "code review skill").await;
    let (status, served) = resolve(&w, &w.bea, "code-review").await;
    assert_eq!(status, StatusCode::OK, "{served}");
    assert_eq!(served["commit"], json!(commit), "{served}");
    let bundle = files(&served);
    assert_eq!(
        bundle.len(),
        2,
        "the whole bundle, not part of it: {served}"
    );
    assert!(bundle["SKILL.md"].contains(V1_BODY), "{served}");
    assert_eq!(bundle["scripts/check.py"], SCRIPT, "{served}");

    // 5. **The AC from the reader's side.** alice edits; the author's own
    //    draft read returns the edit; the consumer keeps being served the
    //    reviewed bytes at the reviewed commit.
    let (status, edited) = author(&w, &w.alice, w.platform, V2_BODY).await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    let (status, own_draft) = get(
        &w.app,
        &format!(
            "/v1/skills/code-review?scope_id={}&channel=draft",
            w.platform
        ),
        &w.alice,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{own_draft}");
    assert!(
        files(&own_draft)["SKILL.md"].contains(V2_BODY),
        "the author reads their own edit: {own_draft}"
    );

    let (status, still) = resolve(&w, &w.bea, "code-review").await;
    assert_eq!(status, StatusCode::OK, "{still}");
    assert_eq!(still["commit"], json!(commit), "the commit has not moved");
    let bundle = files(&still);
    assert!(
        bundle["SKILL.md"].contains(V1_BODY),
        "the consumer keeps the reviewed version: {still}"
    );
    assert!(
        !bundle["SKILL.md"].contains(V2_BODY),
        "and not one word of the edit: {still}"
    );

    // 6. A second review moves it, and only then.
    let second = review_and_publish(&w, w.platform, "code-review", "review the rewrite").await;
    assert_ne!(second, commit);
    let (status, moved) = resolve(&w, &w.bea, "code-review").await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert!(files(&moved)["SKILL.md"].contains(V2_BODY), "{moved}");
}

// ── The acceptance criterion, second half ────────────────────────────────────

/// **"Installs unmodified" is a hash comparison**, and this is the half the
/// server owns: what a resolve returns is the reviewed bytes, and every
/// file's content address recomputes from them.
///
/// The recomputation runs the *client's* arithmetic — `SkillAsset::address`,
/// the same function `synveda skill install` uses — against the number the
/// commit named. That matters because a materialised bundle carries no
/// watermark of its own (ADR-0051 force 2): this hash is its whole
/// provenance, and a test that trusted the server's own field would be
/// asserting the server agrees with itself.
#[tokio::test]
async fn every_served_file_hashes_to_the_address_the_commit_named() {
    let Some(w) = world().await else { return };
    author(&w, &w.alice, w.platform, V1_BODY).await;
    review_and_publish(&w, w.platform, "code-review", "code review skill").await;

    let (status, served) = resolve(&w, &w.bea, "code-review").await;
    assert_eq!(status, StatusCode::OK, "{served}");
    let scope_id: ScopeId = served["scope_id"]
        .as_str()
        .expect("scope id")
        .parse()
        .expect("parse scope id");
    let sensitivity: Sensitivity = served["sensitivity"]
        .as_str()
        .expect("tier")
        .parse()
        .expect("parse tier");

    let mut seen = 0;
    for file in served["files"].as_array().expect("files") {
        let asset = SkillAsset {
            scope_id,
            skill: "code-review".parse().expect("skill name"),
            sensitivity,
            file: SkillFile {
                path: file["path"].as_str().expect("path").parse().expect("parse"),
                content: file["content"].as_str().expect("content").to_owned(),
            },
        };
        assert_eq!(
            asset.address().to_hex(),
            file["object_hash"].as_str().expect("object hash"),
            "{} does not hash to the address the commit named",
            file["path"]
        );
        seen += 1;
    }
    assert_eq!(seen, 2, "both files were checked");

    // And the bundle is exactly the reviewed files: nothing added, no
    // receipt, no manifest (ADR-0051 option 7). An install writes what is
    // here and nothing else, which is what makes the claim checkable.
    let paths: Vec<&str> = served["files"]
        .as_array()
        .expect("files")
        .iter()
        .map(|file| file["path"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(paths, vec!["SKILL.md", "scripts/check.py"], "{served}");
}

/// **A skill's content never becomes a record and never composes.**
///
/// ADR-0049 option 4's third reason — a prompt is fetched by name where a
/// record is ranked by relevance — was inverted by PRMT-2 for packs, whose
/// published chunks *are* pinned records. It is restored here (decision 9),
/// and the difference is visible from the only place it matters: a session.
#[tokio::test]
async fn a_published_skill_composes_into_nothing() {
    let Some(w) = world().await else { return };
    author(&w, &w.alice, w.platform, V1_BODY).await;
    review_and_publish(&w, w.platform, "code-review", "code review skill").await;

    let (status, block) = post(
        &w.app,
        "/v1/inject",
        &w.bea,
        json!({"task": "review this diff for defects", "session_id": "sess-skil1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{block}");
    let text = block["text"].as_str().unwrap_or_default();
    assert!(
        !text.contains(V1_BODY) && !text.contains("checked"),
        "a skill's content must not reach a block — the client's own \
         progressive disclosure is the loader: {text}"
    );
    // Not even as an unreviewed derived entry, which is the failure mode the
    // pack feature had to exclude in SQL.
    assert!(
        !text.contains("code-review"),
        "and its name is not advertised either — that is SKIL-4's feature, \
         and shipping half of it here would make that AC untestable: {text}"
    );
    // The watermark cites no skill channel: composition never read one.
    let channels = block["channels"].as_array().cloned().unwrap_or_default();
    assert!(
        !channels
            .iter()
            .any(|channel| channel["ref"] == json!("skill/published")),
        "composition read no skill channel: {:?}",
        block["channels"]
    );

    // And a recall — whose universe is wider than inject's by design — finds
    // nothing either, because there are no records to find.
    let (status, recalled) = post(
        &w.app,
        "/v1/recall",
        &w.bea,
        json!({"query": "review a diff and report defects", "limit": 10}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{recalled}");
    assert!(
        !recalled.to_string().contains(V1_BODY),
        "a skill's body is in no corpus: {recalled}"
    );
}

// ── The spec's own rules, enforced where they can still be fixed ─────────────

/// **A validation this product skips is a refusal a third-party client
/// delivers to a user who has already published** (ADR-0051 decision 5).
#[tokio::test]
async fn the_open_specs_rules_are_enforced_at_authoring() {
    let Some(w) = world().await else { return };

    // The name grammar is the spec's, which is stricter than the product's:
    // `code_review` is a legal prompt and pack name and is refused here.
    let (status, bad_name) = author_files(
        &w,
        &w.alice,
        w.platform,
        "code_review",
        &[("SKILL.md", manifest(V1_BODY))],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad_name}");

    // The frontmatter `name` must equal the skill's — the spec's "must match
    // the directory", which is also this registry's key.
    let (status, mismatch) = author_files(
        &w,
        &w.alice,
        w.platform,
        "diff-review",
        &[("SKILL.md", manifest(V1_BODY))],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{mismatch}");
    assert!(
        detail(&mismatch).contains("but the skill is"),
        "{}",
        detail(&mismatch)
    );

    // No SKILL.md at all.
    let (status, headless) = author_files(
        &w,
        &w.alice,
        w.platform,
        "code-review",
        &[("reference.md", "just notes".to_owned())],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{headless}");
    assert!(
        detail(&headless).contains("SKILL.md"),
        "{}",
        detail(&headless)
    );

    // An empty description — what a client loads at every session, so an
    // empty one ships a skill nothing will reach for.
    let (status, mute) = author_files(
        &w,
        &w.alice,
        w.platform,
        "code-review",
        &[(
            "SKILL.md",
            "---\nname: code-review\ndescription: \"\"\n---\n# Body\n".to_owned(),
        )],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{mute}");
    assert!(detail(&mute).contains("description"), "{}", detail(&mute));

    // A path a materialisation would let escape its directory.
    let (status, escape) = author_files(
        &w,
        &w.alice,
        w.platform,
        "code-review",
        &[
            ("SKILL.md", manifest(V1_BODY)),
            ("../escape.py", "print(1)".to_owned()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{escape}");

    // And the one that is not obvious: two legal paths, two distinct
    // objects, one file on a case-folding filesystem.
    let (status, folded) = author_files(
        &w,
        &w.alice,
        w.platform,
        "code-review",
        &[
            ("SKILL.md", manifest(V1_BODY)),
            ("scripts/Run.py", "print(1)".to_owned()),
            ("scripts/run.py", "print(2)".to_owned()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{folded}");
    assert!(
        detail(&folded).contains("differ only in case"),
        "{}",
        detail(&folded)
    );

    // `restricted` is unrepresentable, for prompts' and packs' reason.
    let (status, top) = post(
        &w.app,
        "/v1/skills",
        &w.alice,
        json!({
            "scope_id": w.platform,
            "name": "code-review",
            "sensitivity": "restricted",
            "files": [{"path": "SKILL.md", "content": manifest(V1_BODY)}],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{top}");
    assert!(detail(&top).contains("restricted"), "{}", detail(&top));

    // Nothing above was stored: the registry is empty at this scope.
    let (status, listing) = get(
        &w.app,
        &format!("/v1/skills?scope_id={}", w.platform),
        &w.alice,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listing}");
    assert_eq!(
        listing["skills"].as_array().map_or(0, Vec::len),
        0,
        "a refused bundle leaves no draft behind: {listing}"
    );
}

// ── Resolution ───────────────────────────────────────────────────────────────

/// **The gradient is a filesystem fact.** The name is the installed
/// directory name and a client's skills root is flat, so a team's
/// `code-review` and the org's cannot both exist on disk — this walk is what
/// decides which one does (ADR-0051 decision 6).
#[tokio::test]
async fn resolution_walks_the_chain_nearest_first_and_never_leaks_existence() {
    let Some(w) = world().await else { return };

    // The org publishes one; everyone on the chain resolves it.
    author(&w, &w.cora, w.org, "the org's own review procedure").await;
    review_and_publish(&w, w.org, "code-review", "org code review").await;
    let (status, from_org) = resolve(&w, &w.bea, "code-review").await;
    assert_eq!(status, StatusCode::OK, "{from_org}");
    assert!(
        files(&from_org)["SKILL.md"].contains("the org's own review procedure"),
        "{from_org}"
    );

    // The team publishes its own, and the nearer one wins for its members.
    author(&w, &w.alice, w.platform, "the platform team's stricter one").await;
    review_and_publish(&w, w.platform, "code-review", "platform code review").await;
    let (status, from_team) = resolve(&w, &w.bea, "code-review").await;
    assert_eq!(status, StatusCode::OK, "{from_team}");
    assert!(
        files(&from_team)["SKILL.md"].contains("stricter"),
        "the nearer copy overrides: {from_team}"
    );

    // Someone in the other team is not on that chain and gets the org's.
    let (status, for_dave) = resolve(&w, &w.dave, "code-review").await;
    assert_eq!(status, StatusCode::OK, "{for_dave}");
    assert!(
        files(&for_dave)["SKILL.md"].contains("the org's own"),
        "a sibling team's skill is off dave's chain: {for_dave}"
    );

    // A name nothing publishes is the uniform 404 rather than an oracle.
    let (status, nothing) = resolve(&w, &w.bea, "not-a-skill").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{nothing}");
    // And so is a name that exists only as somebody else's draft.
    author_files(
        &w,
        &w.dave,
        w.eng,
        "secret-tool",
        &[(
            "SKILL.md",
            "---\nname: secret-tool\ndescription: Not for you.\n---\n# x\n".to_owned(),
        )],
    )
    .await;
    let (status, drafted) = resolve(&w, &w.bea, "secret-tool").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "an unreviewed draft is not a version anyone resolves: {drafted}"
    );
}

/// **A pin freezes bytes and a rewind refuses it**, PRMT-1's rule inherited
/// whole — and for skills it is what makes an install receipt reproducible
/// while keeping FLOW-7's sixty seconds true of an asset on laptops.
#[tokio::test]
async fn a_pinned_install_holds_and_a_rewind_refuses_it_naming_both_commits() {
    let Some(w) = world().await else { return };
    author(&w, &w.alice, w.platform, V1_BODY).await;
    let first = review_and_publish(&w, w.platform, "code-review", "v1").await;
    author(&w, &w.alice, w.platform, V2_BODY).await;
    let second = review_and_publish(&w, w.platform, "code-review", "v2").await;

    // The floating read follows publications.
    let (status, floating) = resolve(&w, &w.bea, "code-review").await;
    assert_eq!(status, StatusCode::OK, "{floating}");
    assert_eq!(floating["commit"], json!(second), "{floating}");

    // The pinned one holds — this is a receipt reinstalling what it recorded.
    let pinned_uri = format!(
        "/v1/skills/code-review?scope_id={}&commit={first}",
        w.platform
    );
    let (status, pinned) = get(&w.app, &pinned_uri, &w.bea).await;
    assert_eq!(status, StatusCode::OK, "{pinned}");
    assert_eq!(pinned["origin"], json!("pinned-commit"), "{pinned}");
    assert!(files(&pinned)["SKILL.md"].contains(V1_BODY), "{pinned}");

    // A rewind takes it off the first-parent line.
    let (status, rolled) = post(
        &w.app,
        &format!("/v1/channels/{}/rollback", w.platform),
        &w.cora,
        json!({
            "asset": "skill",
            "from_commit": second,
            "to_commit": first,
            "message": "the rewrite was wrong",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "a skill channel rewinds: {rolled}");

    let (status, refused) = get(
        &w.app,
        &format!(
            "/v1/skills/code-review?scope_id={}&commit={second}",
            w.platform
        ),
        &w.bea,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a pin cannot outlive a withdrawal: {refused}"
    );
    let message = detail(&refused);
    assert!(message.contains(&second[..12]), "{message}");
    assert!(message.contains("skill/published"), "{message}");

    // …and the floating read heals on the very next call.
    let (status, healed) = resolve(&w, &w.bea, "code-review").await;
    assert_eq!(status, StatusCode::OK, "{healed}");
    assert!(files(&healed)["SKILL.md"].contains(V1_BODY), "{healed}");
}

// ── The scanner, the bundle, and the chain ───────────────────────────────────

/// **No secret reaches a client's disk** (ADR-0051 decision 14). MEM-2's
/// scanner runs over every file before anything is stored, and the
/// guarantee is stronger than the pack feature's for having a different
/// destination: a pack's secret would have reached vector space.
#[tokio::test]
async fn a_bundle_carrying_a_credential_is_stopped_at_authoring() {
    let Some(w) = world().await else { return };
    let secret = "AKIAIOSFODNN7EXAMPLE";
    let (status, refused) = author_files(
        &w,
        &w.alice,
        w.platform,
        "code-review",
        &[
            ("SKILL.md", manifest(V1_BODY)),
            (
                "scripts/check.py",
                format!("AWS_KEY = '{secret}'\nprint('go')\n"),
            ),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    let message = detail(&refused);
    assert!(message.contains("scripts/check.py"), "{message}");
    assert!(message.contains("redaction scanner"), "{message}");

    // Nothing was stored, so nothing can be published or installed.
    let (status, listing) = get(
        &w.app,
        &format!("/v1/skills?scope_id={}", w.platform),
        &w.alice,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listing}");
    assert_eq!(listing["skills"].as_array().map_or(0, Vec::len), 0);

    // The quarantine event names the rule and never the matched text.
    let chain = events(&w.pool, w.tenant).await;
    let quarantined = chain
        .iter()
        .find(|event| event.action == "skill.quarantined")
        .expect("a skill.quarantined event");
    assert!(
        !quarantined.payload.to_string().contains(secret),
        "the finding's text is in no payload: {}",
        quarantined.payload
    );
    assert_eq!(quarantined.payload["path"], json!("scripts/check.py"));
}

/// **The request is the bundle** (ADR-0051 decision 17), and the boundary
/// that makes the DELETE grant safe: a dropped file leaves the draft, and a
/// publication approved against it refuses rather than shipping a bundle
/// nobody reviewed.
#[tokio::test]
async fn a_file_dropped_from_the_request_leaves_the_draft_and_cannot_ship_past_its_review() {
    let Some(w) = world().await else { return };
    author(&w, &w.alice, w.platform, V1_BODY).await;

    // Open a proposal against the two-file bundle…
    let (status, opened) = post(
        &w.app,
        "/v1/proposals",
        &w.alice,
        json!({
            "scope_id": w.platform,
            "skill_names": ["code-review"],
            "title": "code review skill",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{opened}");
    let id = opened["id"].as_str().expect("proposal id").to_owned();
    let (status, before) = get(&w.app, &format!("/v1/proposals/{id}"), &w.sec).await;
    assert_eq!(status, StatusCode::OK, "{before}");
    assert_eq!(
        before["members"].as_array().map_or(0, Vec::len),
        2,
        "a proposal names the bundle, so every file is a member: {before}"
    );

    // …then drop the script from the bundle. A client loads a skill whole,
    // so the request is the bundle and the file is gone.
    let (status, shrunk) = author_files(
        &w,
        &w.alice,
        w.platform,
        "code-review",
        &[("SKILL.md", manifest(V1_BODY))],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{shrunk}");
    assert_eq!(shrunk["removed"], json!(1), "{shrunk}");
    assert_eq!(
        shrunk["files"].as_array().map_or(0, Vec::len),
        1,
        "{shrunk}"
    );

    // The approvals still stand, and the publication refuses: approvals bind
    // bytes, and one of the bytes they bound is no longer held.
    for token in [&w.sam, &w.sec] {
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
        &w.cora,
        json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a bundle cannot ship past its own approval: {refused}"
    );
    assert!(
        detail(&refused).contains("scripts/check.py"),
        "{}",
        detail(&refused)
    );
}

/// **Every act is on the chain, and no payload carries file content.**
#[tokio::test]
async fn every_act_is_chained_and_no_payload_carries_file_content() {
    let Some(w) = world().await else { return };
    author(&w, &w.alice, w.platform, V1_BODY).await;
    review_and_publish(&w, w.platform, "code-review", "code review skill").await;
    resolve(&w, &w.bea, "code-review").await;

    let chain = events(&w.pool, w.tenant).await;
    let action = |name: &str| chain.iter().find(|event| event.action == name);

    let authored = action("skill.authored").expect("a skill.authored event");
    assert_eq!(authored.payload["skill"], json!("code-review"));
    assert_eq!(authored.payload["asset"], json!("skill"));
    assert_eq!(
        authored.payload["files"].as_array().map_or(0, Vec::len),
        2,
        "the addresses and the counts: {}",
        authored.payload
    );

    let resolved = action("skill.resolved").expect("a skill.resolved event");
    assert_eq!(resolved.payload["skill"], json!("code-review"));
    assert!(
        resolved.payload["commit"].is_string(),
        "the citation an install records: {}",
        resolved.payload
    );

    // A publication is the same event a memory publication emits — the same
    // governed act with the same consequence, so a second action asserting
    // it would be a fact an auditor has to reconcile (ADR-0019 decision 4).
    let published = chain
        .iter()
        .find(|event| {
            event.action == "vedaflow.channel.published" && event.payload["asset"] == json!("skill")
        })
        .expect("a vedaflow.channel.published with asset=skill");
    assert_eq!(published.payload["channel"], json!("skill/published"));

    // There is deliberately no `skill.installed`: an install is a
    // client-side act on bytes an audited resolve already served.
    assert!(
        chain.iter().all(|event| event.action != "skill.installed"),
        "an event the server cannot verify is a fact an auditor has to \
         reconcile (ADR-0051 decision 16)"
    );

    // The sweep. No payload on the whole chain carries file content or
    // SKILL.md text.
    for event in &chain {
        let payload = event.payload.to_string();
        for phrase in [V1_BODY, "import sys", "Review a diff and report"] {
            assert!(
                !payload.contains(phrase),
                "{} carries file content ({phrase}): {payload}",
                event.action
            );
        }
    }

    // And the chain verifies over all of it.
    let mut tx = synveda_store::rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("tenant tx");
    let report = synveda_audit::verify(&mut tx, w.tenant)
        .await
        .expect("verify the chain");
    assert!(
        matches!(report, synveda_audit::ChainVerification::Valid { .. }),
        "{report}"
    );
}

/// **A skill proposal is reviewable without the console** — FLOW-6's claim,
/// asserted for the fourth asset kind, with a per-file diff.
#[tokio::test]
async fn a_skill_proposal_renders_a_per_file_diff() {
    let Some(w) = world().await else { return };
    author(&w, &w.alice, w.platform, V1_BODY).await;
    review_and_publish(&w, w.platform, "code-review", "v1").await;
    author(&w, &w.alice, w.platform, V2_BODY).await;

    let (status, opened) = post(
        &w.app,
        "/v1/proposals",
        &w.alice,
        json!({
            "scope_id": w.platform,
            "skill_names": ["code-review"],
            "title": "the rewrite",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{opened}");
    let id = opened["id"].as_str().expect("proposal id").to_owned();

    let (status, detail) = get(&w.app, &format!("/v1/proposals/{id}"), &w.sec).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    let members = detail["members"].as_array().expect("members");
    assert_eq!(members.len(), 2, "{detail}");

    let manifest_member = members
        .iter()
        .find(|member| member["member"] == json!("code-review/SKILL.md"))
        .expect("the SKILL.md member");
    assert_eq!(manifest_member["asset"], json!("skill"));
    assert_eq!(
        manifest_member["effect"],
        json!("update"),
        "the edited file is an update: {manifest_member}"
    );
    // The *file's* bytes, not the envelope's: a reviewer reads the script.
    assert!(
        manifest_member["proposed"]
            .as_str()
            .expect("proposed")
            .contains(V2_BODY),
        "{manifest_member}"
    );
    assert!(
        manifest_member["baseline"]["text"]
            .as_str()
            .expect("baseline")
            .contains(V1_BODY),
        "{manifest_member}"
    );

    let script = members
        .iter()
        .find(|member| member["member"] == json!("code-review/scripts/check.py"))
        .expect("the script member");
    assert_eq!(
        script["effect"],
        json!("none"),
        "an unchanged file says so rather than re-rendering: {script}"
    );
}
