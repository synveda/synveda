//! The parity corpus (CNSL-1, ADR-0056 decision 7).
//!
//! CNSL-1's acceptance criterion is one clause — **full review parity with
//! CLI** — and the only version of that word worth having is one a test can
//! fail. Two renderers that agree on the day they are written is not parity;
//! it is a coincidence with a maintenance schedule. So the corpus comes
//! first, before either renderer is written against it, and this file is
//! what makes the corpus *the gateway's* rather than an author's idea of it.
//!
//! # What this test is
//!
//! It drives the real `/v1` API through the real router — no mock, no
//! hand-built `ProposalDetail` — and settles each recorded payload against
//! `console/fixtures/`. Two modes:
//!
//! ```text
//! make db-test                                  # verify: the corpus is what the gateway serves
//! SYNVEDA_RECORD_FIXTURES=1 make db-test        # re-record it
//! ```
//!
//! Verification is the point. A corpus nobody checks is a set of payloads
//! that drift out of the shape the product actually serves, and then both
//! renderers agree about a response nobody receives — which is the same
//! failure the AC is trying to prevent, one level down. The guard is why the
//! recording is a test rather than a script.
//!
//! # What is in it, and why each case is there
//!
//! Five cases are recorded from the gateway and one is synthesised, and the
//! set is chosen to cover the judgements a review makes rather than the
//! shapes a serialiser emits:
//!
//! - **`memory-update`** — FLOW-6's own shape, and the only case where the
//!   three member contents differ from each other: the bytes under review,
//!   the baseline they would overwrite, and the record as it stands now
//!   (ADR-0035 decisions 5 and 8). It also carries a `none` member, because
//!   "publishing changes nothing about this one" and "publishing replaces
//!   this one" must not render the same.
//! - **`skill-clean`** — a scan that ran and found nothing, and a bundle
//!   over the bar with a checklist bound to its bytes. The happy path is in
//!   the corpus because "found nothing" and "no scan here" are different
//!   facts and a renderer that conflates them fails only on this case.
//! - **`skill-blocking-scan`** — authored under a pack whose floor permits
//!   the `high` band and reviewed under one that refuses it, which is the
//!   real way a proposal comes to be blocked: nothing about the bundle
//!   changed, the pack did. It carries a blocking finding *and* two
//!   non-blocking ones, so a renderer that paints them all alike fails.
//! - **`skill-below-bar`** — two shortfalls at once, so the corpus pins
//!   that a refusal names every bar it missed rather than the first.
//! - **`skill-checklist-stale`** — a checklist answered against an earlier
//!   draft, and therefore **not found**: `requires_checklist` true and
//!   `checklist` absent (ADR-0053 decision 4). The ADR names this case
//!   specifically, because a parity suite that only covers the happy path
//!   proves the two renderers agree about nothing difficult.
//! - **`skill-unknown-severity`** — *synthesised*, because the gateway
//!   cannot produce it: it is what a **newer** gateway would serve, with two
//!   severity bands outside `ScanSeverity`'s three — one above the pack's
//!   threshold and one below it. That pair is the sharp case. A client
//!   meeting an unfamiliar severity has to guess, and the only safe guess is
//!   to rank it above everything, which is what the CLI's fallback does and
//!   must; the guess is wrong for the band below the threshold. ADR-0056
//!   decision 5 moved the verdict to the gateway so that no client has to
//!   make it, and this case is where a renderer that went back to guessing
//!   fails. Its shape is checked against a recorded sibling here, so a field
//!   added to `ProposalDetail` cannot leave it quietly behind.
//!
//! # Normalisation
//!
//! Ids, commit and object addresses and timestamps are replaced with stable
//! stand-ins, or no two runs would ever produce the same bytes. The
//! substitution is **shape-preserving on purpose**: a record id is replaced
//! by something that is still UUID-shaped, an object address by something
//! still 64 hex characters, an instant by a real RFC 3339 instant written to
//! the precision the original carried. Both renderers key behaviour off
//! those shapes — the CLI abbreviates a uuid-shaped member name and does not
//! abbreviate a path — so a corpus that normalised a uuid to `uuid-01` would
//! be a corpus in which that rule is never exercised and both surfaces agree
//! on a rendering neither produces.
//!
//! The one part of this that is not obvious until it fails: a member's
//! `proposed` and `baseline.text` are canonical JSON objects carried **as
//! strings**, so the ids inside them are payload too. A scrubber that walked
//! only the outer document leaves a corpus that changes every run for
//! reasons nobody can see in a diff — which is how it was found.
//!
//! Tests need a live Postgres: they read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database), the house convention.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Map, Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::{Hs256Verifier, personal_slug};
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder};
use synveda_policy::Pdp;
use synveda_retrieval::index::SearchIndex;
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::{hierarchy, identities, policy_assignments, role_bindings, tenants};
use synveda_types::{
    HierarchyNode, Identity, IdentityId, IdentityKind, PackConfig, RecordClass, RecordId,
    RecordKind, Role, ScopeId, ScopeKind, Sensitivity, SkillQualityConfig, SkillScanConfig,
    TenantId, TenantStatus,
};
use tower::ServiceExt;

const SECRET: &[u8] = b"cnsl-1-parity-secret";

/// The env var that flips recording on. Anything other than `1` verifies.
const RECORD: &str = "SYNVEDA_RECORD_FIXTURES";

// ── The corpus on disk ───────────────────────────────────────────────────────

/// `console/fixtures/`, from this crate's manifest directory.
fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../console/fixtures")
        .canonicalize()
        .expect("console/fixtures exists in the workspace")
}

fn recording() -> bool {
    std::env::var(RECORD).is_ok_and(|value| value == "1")
}

/// Writes the case when recording, and holds the gateway to it otherwise.
///
/// The failure message names the command that re-records, because the
/// alternative is a contributor hand-editing a fixture until a test passes
/// — which is precisely the way this corpus stops being evidence (ADR-0056's
/// reversal trigger for decision 7).
fn settle(name: &str, payload: &Value) {
    let path = corpus_dir().join(format!("{name}.json"));
    let recorded = format!(
        "{}\n",
        serde_json::to_string_pretty(payload).expect("payload serialises")
    );
    if recording() {
        std::fs::write(&path, &recorded).unwrap_or_else(|err| panic!("write {name}: {err}"));
        eprintln!("recorded console/fixtures/{name}.json");
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "console/fixtures/{name}.json is missing ({err}) — record it with \
             `{RECORD}=1 make db-test`"
        )
    });
    if committed != recorded {
        panic!(
            "console/fixtures/{name}.json is not what the gateway serves.\n\n\
             If the payload changed on purpose, re-record with `{RECORD}=1 make db-test` \
             and read the diff: every renderer that consumes this corpus is being asked \
             to change with it.\n\n\
             served:\n{recorded}"
        );
    }
}

// ── Normalisation ────────────────────────────────────────────────────────────

/// Replaces the volatile parts of a payload with stable, *same-shaped*
/// stand-ins, remembering each substitution so the payload stays internally
/// consistent: a member's `record_id` and the approval that names the same
/// identity still match after scrubbing.
#[derive(Default)]
struct Stable {
    seen: BTreeMap<String, String>,
    ids: usize,
    hashes: usize,
    times: usize,
}

impl Stable {
    fn scrub(&mut self, value: &mut Value) {
        match value {
            Value::String(text) => {
                // A member's `proposed` and its baseline's `text` are
                // canonical JSON objects carried **as strings** — the bytes
                // under review, which is why they are transported rather
                // than re-derived (ADR-0035 decision 6). The ids inside
                // them are payload too, and a scrubber that walked only the
                // outer document would leave a corpus that changes every
                // run for reasons nobody can see in a diff.
                if let Some(mut embedded) = as_json_object(text) {
                    self.scrub(&mut embedded);
                    *text = serde_json::to_string(&embedded).expect("re-serialises");
                    return;
                }
                if let Some(replacement) = self.substitute(text) {
                    *text = replacement;
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.scrub(item);
                }
            }
            Value::Object(map) => {
                // `serde_json::Map` iterates in a deterministic order, so two
                // runs assign the same stand-in to the same field.
                for (_, field) in map.iter_mut() {
                    self.scrub(field);
                }
            }
            _ => {}
        }
    }

    fn substitute(&mut self, raw: &str) -> Option<String> {
        if let Some(existing) = self.seen.get(raw) {
            return Some(existing.clone());
        }
        let replacement = if is_uuid(raw) {
            self.ids += 1;
            // Still parses as a UUID, and still 36 characters of hex and
            // hyphens — which is the test a renderer uses to decide whether
            // to abbreviate it.
            format!("00000000-0000-4000-8000-{:012}", self.ids)
        } else if is_object_address(raw) {
            self.hashes += 1;
            // Still 64 hex characters, so an abbreviation to twelve is still
            // an abbreviation of an address.
            format!("{:064x}", self.hashes)
        } else if as_instant(raw).is_some() {
            self.times += 1;
            // Still an instant, and still written to the precision the
            // original was: a canonical object's `valid_from` carries
            // microseconds and a `created_at` does not, and a corpus that
            // flattened the two would be one in which no renderer ever
            // meets the format the product emits. What is *not* promised is
            // that the stand-ins preserve the originals' relative order —
            // only that equal inputs stay equal.
            let base = Utc
                .with_ymd_and_hms(2026, 1, 1, 9, 0, 0)
                .single()
                .expect("a real instant");
            let precision = if raw.contains('.') {
                SecondsFormat::Micros
            } else {
                SecondsFormat::Secs
            };
            (base + chrono::Duration::seconds(self.times as i64 * 60))
                .to_rfc3339_opts(precision, true)
        } else {
            return None;
        };
        self.seen.insert(raw.to_owned(), replacement.clone());
        Some(replacement)
    }
}

fn is_uuid(text: &str) -> bool {
    text.len() == 36
        && text.chars().enumerate().all(|(index, character)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                character == '-'
            } else {
                character.is_ascii_hexdigit()
            }
        })
}

fn is_object_address(text: &str) -> bool {
    text.len() == 64 && text.chars().all(|character| character.is_ascii_hexdigit())
}

fn as_instant(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
}

/// A string that is itself a JSON **object**, which is how a member's
/// canonical bytes travel. An array or a bare scalar is left alone: a
/// reviewer's comment that happens to be the word `null` is a comment.
fn as_json_object(text: &str) -> Option<Value> {
    if !text.starts_with('{') {
        return None;
    }
    serde_json::from_str::<Value>(text)
        .ok()
        .filter(Value::is_object)
}

/// A recorded case: the detail as served, scrubbed.
fn record(mut detail: Value) -> Value {
    Stable::default().scrub(&mut detail);
    detail
}

// ── The world the cases are built in ─────────────────────────────────────────

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn index_root() -> PathBuf {
    std::env::temp_dir()
        .join("synveda-cnsl1-parity")
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
        scope_chains: Arc::new(synveda_store::ScopeChainCache::new()),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index: Arc::new(SearchIndex::open(index_root()).expect("open sidecar")),
        embedder: Arc::new(AnyEmbedder::Deterministic(DeterministicEmbedder::new())),
        inject_embed_timeout: Duration::from_millis(100),
    }
}

fn issue(subject: &str, tenant_id: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue(subject, tenant_id, Duration::from_secs(600))
}

/// One tenant, `acme → eng → platform`, and the cast a skill publication
/// takes: an author, a curator who runs the effect, a steward, and the
/// security reviewer the floor asks for on every skill.
struct World {
    pool: PgPool,
    tenant: TenantId,
    app: Router,
    pdp: Arc<Pdp>,
    platform: ScopeId,
    alice: String,
    cora: String,
    sam: String,
    sec: String,
}

async fn world() -> Option<World> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!(
            "skipping the CNSL-1 parity corpus: DATABASE_URL is not set \
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
    let slug = format!("cnsl1-{}", tenant.as_uuid().simple());
    tenants::create(
        &pool,
        tenant,
        &slug,
        "CNSL-1 corpus tenant",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");

    let mut tx = pool.begin().await.expect("begin");
    let org = node(&mut tx, tenant, None, ScopeKind::Org, "acme").await;
    let eng = node(&mut tx, tenant, Some(org.id), ScopeKind::Department, "eng").await;
    let platform = node(&mut tx, tenant, Some(eng.id), ScopeKind::Team, "platform").await;
    node(
        &mut tx,
        tenant,
        Some(org.id),
        ScopeKind::Team,
        identities::QUARANTINE_SLUG,
    )
    .await;
    tx.commit().await.expect("commit hierarchy");

    for subject in ["alice", "cora", "sam", "sec"] {
        seed_user(&pool, tenant, subject, platform.id).await;
    }
    bind(&pool, tenant, "alice", platform.id, Role::Contributor).await;
    bind(&pool, tenant, "cora", platform.id, Role::Curator).await;
    bind(&pool, tenant, "sam", platform.id, Role::Steward).await;
    bind(&pool, tenant, "sec", platform.id, Role::SecurityReviewer).await;

    let pdp = Arc::new(Pdp::new().expect("build the embedded PDP"));
    let app = router(state(&url, pdp.clone()));
    Some(World {
        pool,
        tenant,
        app,
        pdp,
        platform: platform.id,
        alice: issue("alice", tenant),
        cora: issue("cora", tenant),
        sam: issue("sam", tenant),
        sec: issue("sec", tenant),
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

/// Installs a pack that permits everything the PDP is asked and configures
/// the two gates this corpus varies.
///
/// The Cedar source is permissive because the corpus is about what a review
/// *renders*, not about who may see it — AUTHZ-1's golden matrix is where
/// authorisation is pinned, and duplicating it here would be a second
/// implementation of a decision that already has one.
async fn install_pack(
    w: &World,
    name: &str,
    version: i64,
    scan: SkillScanConfig,
    quality: SkillQualityConfig,
) {
    w.pdp
        .install_source(
            w.tenant,
            name,
            version,
            "permit (principal, action, resource) when { resource in principal.tenant };",
            PackConfig {
                scan: Some(scan),
                quality: Some(quality),
                ..Default::default()
            },
        )
        .expect("install pack");
    policy_assignments::set_default(&w.pool, w.tenant, name)
        .await
        .expect("set default pack");
}

// ── HTTP ─────────────────────────────────────────────────────────────────────

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

async fn get(app: &Router, token: &str, uri: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("build request");
    call(app, request).await
}

/// `GET /v1/proposals/{id}` as the security reviewer, who is the reviewer
/// every one of these proposals is waiting on.
async fn detail(w: &World, id: &str) -> Value {
    let (status, body) = get(&w.app, &w.sec, &format!("/v1/proposals/{id}")).await;
    assert_eq!(status, StatusCode::OK, "read the proposal: {body}");
    body
}

// ── Bundles ──────────────────────────────────────────────────────────────────

/// A bundle that clears `regulated-strict`'s bar: it says when to use the
/// skill, has steps, and carries a worked example.
fn full_bundle(name: &str, body: &str) -> Vec<(&'static str, String)> {
    vec![(
        "SKILL.md",
        format!(
            "---\n\
             name: {name}\n\
             description: Use when reviewing a change to the on-call runbook.\n\
             ---\n\
             \n\
             # {name}\n\
             \n\
             {body}\n\
             \n\
             ## Steps\n\
             \n\
             1. Read what is in front of you.\n\
             2. Do the thing this skill is for.\n\
             3. Report what changed.\n\
             \n\
             ## Example\n\
             \n\
             ```sh\n\
             echo 'ran {name}'\n\
             ```\n"
        ),
    )]
}

/// A bundle with nothing in it but a name: no section, no example, nothing
/// a rubric can credit. This is the below-the-bar case, and it is thin on
/// purpose rather than by omission.
fn thin_bundle(name: &str) -> Vec<(&'static str, String)> {
    vec![(
        "SKILL.md",
        format!("---\nname: {name}\ndescription: does a thing\n---\n\ndo the thing\n"),
    )]
}

/// A setup script that trips three rules at two severities: `sudo` is the
/// `high` band a pack decides on, and the install and the fetch are notices
/// a reviewer weighs.
const SETUP: &str = "#!/usr/bin/env bash\n\
                     set -euo pipefail\n\
                     sudo apt-get install -y ripgrep\n\
                     curl -sS https://example.invalid/rules.json -o rules.json\n";

async fn author(w: &World, name: &str, files: &[(&'static str, String)]) -> Value {
    let (status, authored) = post(
        &w.app,
        &w.alice,
        "/v1/skills",
        json!({
            "scope_id": w.platform,
            "name": name,
            "files": files.iter().map(|(path, content)| json!({
                "path": path,
                "content": content,
            })).collect::<Vec<_>>(),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "author {name}: {authored}");
    authored
}

/// Opens a proposal over one authored skill and returns its id.
async fn propose_skill(w: &World, name: &str, title: &str) -> String {
    let (status, opened) = post(
        &w.app,
        &w.alice,
        "/v1/proposals",
        json!({"scope_id": w.platform, "skill_names": [name], "title": title}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "propose {name}: {opened}");
    opened["id"].as_str().expect("proposal id").to_owned()
}

/// Answers every checklist item `yes`, binding the answers to the bytes the
/// proposal names right now.
async fn record_checklist(w: &World, id: &str) -> Value {
    let (status, checked) = post(
        &w.app,
        &w.sam,
        &format!("/v1/proposals/{id}/checklist"),
        json!({
            "answers": {
                "tested": "yes",
                "instructions-correct": "yes",
                "scope-appropriate": "yes",
                "not-duplicate": "yes",
                "dependencies-available": "yes",
            },
            "note": "ran it against last week's incident and it reproduced the fix",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "record the checklist: {checked}");
    checked
}

async fn approve(w: &World, token: &str, id: &str, comment: &str) {
    let (status, approved) = post(
        &w.app,
        token,
        &format!("/v1/proposals/{id}/approve"),
        json!({"comment": comment}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "approve: {approved}");
}

// ── Memory ───────────────────────────────────────────────────────────────────

fn record_state(scope: ScopeId, owner: IdentityId, content: &str) -> RecordState {
    RecordState {
        scope_id: scope,
        owner_id: owner,
        kind: RecordKind::Derived,
        class: RecordClass::Procedure,
        content: content.to_owned(),
        sensitivity: Sensitivity::Internal,
        provenance: json!({"source": "the CNSL-1 parity corpus"}),
        // A fixed instant rather than `now`, so the only thing normalisation
        // has to stabilise is the row's own bookkeeping.
        valid_from: Utc
            .with_ymd_and_hms(2026, 1, 1, 8, 0, 0)
            .single()
            .expect("a real instant"),
        valid_to: None,
    }
}

fn embedding() -> RecordEmbedding {
    RecordEmbedding {
        model: DeterministicEmbedder::MODEL.to_owned(),
        vector: vec![0.25; 16],
    }
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
        &record_state(scope, owner, content),
        &embedding(),
    )
    .await
    .expect("insert record");
    id
}

async fn rewrite(
    pool: &PgPool,
    record: RecordId,
    scope: ScopeId,
    owner: IdentityId,
    content: &str,
) {
    records::update(
        pool,
        record,
        &record_state(scope, owner, content),
        &embedding(),
    )
    .await
    .expect("rewrite record")
    .expect("the record exists");
}

// ── The cases ────────────────────────────────────────────────────────────────

const RUNBOOK: &str = "check the on-call rota\nrotate the signing key\nfile the change record";
const REVISED: &str =
    "check the on-call rota\nrotate the signing key every 90 days\nfile the change record";
const STANDING: &str = "escalate to the platform lead before touching production";

/// The memory case: a channel that already holds one version, an edit that
/// would replace it, and a second member publication would not touch.
async fn memory_update(w: &World) -> Value {
    let alice = identities::by_subject(&w.pool, w.tenant, "alice")
        .await
        .expect("read alice")
        .expect("alice exists");

    let runbook = seed_record(&w.pool, w.tenant, w.platform, alice.id, RUNBOOK).await;
    let standing = seed_record(&w.pool, w.tenant, w.platform, alice.id, STANDING).await;

    // Publish both, so the channel has a baseline to overwrite.
    let (status, opened) = post(
        &w.app,
        &w.alice,
        "/v1/proposals",
        json!({
            "scope_id": w.platform,
            "record_ids": [runbook, standing],
            "title": "the on-call runbook and the standing instruction",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "open the first proposal: {opened}");
    let first = opened["id"].as_str().expect("proposal id").to_owned();
    approve(w, &w.sec, &first, "read it").await;
    approve(w, &w.cora, &first, "ship it").await;
    let (status, published) = post(
        &w.app,
        &w.cora,
        &format!("/v1/proposals/{first}/publish"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "publish the baseline: {published}");

    // Edit one of them and propose both again: one member updates, one is
    // already at the address the channel names and changes nothing.
    rewrite(&w.pool, runbook, w.platform, alice.id, REVISED).await;
    let (status, opened) = post(
        &w.app,
        &w.alice,
        "/v1/proposals",
        json!({
            "scope_id": w.platform,
            "record_ids": [runbook, standing],
            "title": "rotate the signing key on a schedule",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "open the second proposal: {opened}");
    let second = opened["id"].as_str().expect("proposal id").to_owned();

    // One approval cast and the requirement not yet met, so the corpus has
    // both an approval to render and an `outstanding` line to render.
    approve(
        w,
        &w.sec,
        &second,
        "the 90 days matches the policy we agreed",
    )
    .await;

    record(detail(w, &second).await)
}

/// The clean case: a scan that found nothing, and a bundle over the bar with
/// a checklist bound to exactly its bytes.
async fn skill_clean(w: &World) -> Value {
    author(
        w,
        "release-notes",
        &full_bundle(
            "release-notes",
            "Summarise what shipped, for the people who did not ship it.",
        ),
    )
    .await;
    let id = propose_skill(w, "release-notes", "the release-notes skill").await;
    record_checklist(w, &id).await;
    approve(w, &w.sec, &id, "nothing executable in it").await;
    record(detail(w, &id).await)
}

/// The blocking case: authored under a pack whose floor permits the `high`
/// band, reviewed under one that refuses it. Nothing about the bundle
/// changed between the two reads — the pack did, which is how a proposal
/// comes to be blocked in the field.
async fn skill_blocking_scan(w: &World) -> Value {
    install_pack(
        w,
        "corpus-permissive",
        1,
        SkillScanConfig::FLOOR,
        SkillQualityConfig::STRICT,
    )
    .await;
    let mut files = full_bundle("bootstrap", "Prepare a machine to run the checks.");
    files.push(("scripts/setup.sh", SETUP.to_owned()));
    author(w, "bootstrap", &files).await;
    let id = propose_skill(w, "bootstrap", "the bootstrap skill").await;
    record_checklist(w, &id).await;

    install_pack(
        w,
        "corpus-strict",
        1,
        SkillScanConfig::STRICT,
        SkillQualityConfig::STRICT,
    )
    .await;
    record(detail(w, &id).await)
}

/// Two bars missed at once: the rubric's, and the checklist the pack
/// requires and nobody has answered.
async fn skill_below_bar(w: &World) -> Value {
    author(w, "quick-note", &thin_bundle("quick-note")).await;
    let id = propose_skill(w, "quick-note", "the quick-note skill").await;
    record(detail(w, &id).await)
}

/// A checklist answered against an earlier draft, and therefore not found.
///
/// No invalidation and no sweep: the edited bundle simply has a different
/// digest, and the lookup by digest finds nothing (ADR-0053 decision 4).
async fn skill_checklist_stale(w: &World) -> Value {
    author(
        w,
        "triage",
        &full_bundle("triage", "Sort an incoming report into a queue."),
    )
    .await;
    let first = propose_skill(w, "triage", "the triage skill").await;
    record_checklist(w, &first).await;

    // The author edits beneath the review. A fresh proposal over the new
    // bytes is the case a reviewer meets: the pack still requires a
    // checklist, and the one that was answered describes bytes that are no
    // longer what would publish.
    author(
        w,
        "triage",
        &full_bundle(
            "triage",
            "Sort an incoming report into a queue, and say why it landed there.",
        ),
    )
    .await;
    let second = propose_skill(w, "triage", "the triage skill, revised").await;
    record(detail(w, &second).await)
}

// ── The test ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_parity_corpus_is_what_the_gateway_serves() {
    let Some(w) = world().await else { return };

    // Order matters only through the pack: the blocking case installs one
    // and leaves it in force, so the cases that want the product default
    // run before it.
    settle("memory-update", &memory_update(&w).await);
    settle("skill-clean", &skill_clean(&w).await);
    settle("skill-below-bar", &skill_below_bar(&w).await);
    settle("skill-checklist-stale", &skill_checklist_stale(&w).await);
    settle("skill-blocking-scan", &skill_blocking_scan(&w).await);
}

/// The synthesised case has no gateway to be recorded from — it is what a
/// **newer** gateway would serve — so what holds it honest is its shape.
///
/// A field added to `ProposalDetail` or to a scan finding would otherwise
/// leave it behind silently, and a renderer written against a corpus with
/// one stale member is a renderer with an untested branch.
#[test]
fn the_synthesised_case_has_the_shape_the_gateway_serves() {
    let dir = corpus_dir();
    let synthesised = read_case(&dir, "skill-unknown-severity");
    let recorded = read_case(&dir, "skill-blocking-scan");

    assert_eq!(
        keys(&synthesised),
        keys(&recorded),
        "the synthesised case must carry the same top-level fields as a recorded one",
    );
    assert_eq!(
        keys(&synthesised["scan"]),
        keys(&recorded["scan"]),
        "and the same scan report fields",
    );
    for finding in synthesised["scan"]["findings"]
        .as_array()
        .expect("findings")
    {
        assert_eq!(
            keys(finding),
            keys(&recorded["scan"]["findings"][0]),
            "and the same finding fields",
        );
    }

    // The point of the case: severities outside the three this build knows,
    // **on both sides of the threshold**. One unknown band above it and one
    // below is what makes the case sharp — a client that ranks an
    // unfamiliar name above everything, which is exactly what the CLI's
    // fallback does and must, would paint the lower one as a refusal. The
    // gateway says it is not one, and ADR-0056 decision 5 is the rule that
    // the gateway's answer is the answer.
    let unknown: Vec<&Value> = synthesised["scan"]["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter(|finding| {
            !matches!(
                finding["severity"].as_str(),
                Some("notice" | "high" | "critical")
            )
        })
        .collect();
    let verdicts: Vec<&Value> = unknown.iter().map(|finding| &finding["blocking"]).collect();
    assert!(
        verdicts.contains(&&json!(true)) && verdicts.contains(&&json!(false)),
        "an unknown severity must appear on both sides of the pack's threshold, \
         or the case cannot fail a renderer that guesses: {synthesised}",
    );
}

fn read_case(dir: &Path, name: &str) -> Value {
    let path = dir.join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read console/fixtures/{name}.json: {err}"));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("{name}.json is not JSON: {err}"))
}

fn keys(value: &Value) -> Vec<&str> {
    value
        .as_object()
        .map(Map::keys)
        .map(|keys| keys.map(String::as_str).collect())
        .unwrap_or_default()
}
