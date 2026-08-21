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
use synveda_identity::Hs256Verifier;
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder};
use synveda_policy::Pdp;
use synveda_retrieval::index::SearchIndex;
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::{access, identities, policy_assignments, scopes, tenants};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{
    GrantId, Identity, IdentityId, IdentityKind, PackConfig, RecordClass, RecordId, RecordKind,
    ScopeId, Sensitivity, SkillQualityConfig, SkillScanConfig, TenantId, TenantStatus,
};
use tower::ServiceExt;

const SECRET: &[u8] = b"cnsl-1-parity-secret";

/// The env var that flips recording on. Anything other than `1` verifies.
const RECORD: &str = "SYNVEDA_RECORD_FIXTURES";

/// Every case in the corpus, recorded and synthesised alike.
///
/// Named in one place because three things iterate it — the recorder, the
/// facts derivation, and the CLI's renderer suite over in `synveda-cli` —
/// and a case added to one and not the others is a case that proves nothing.
const CASES: &[&str] = &[
    "memory-update",
    "memory-drifted",
    "skill-clean",
    "skill-below-bar",
    "skill-checklist-stale",
    "skill-blocking-scan",
    "skill-unknown-severity",
];

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
/// Settles a case and the facts derived from it together.
///
/// Together rather than in two passes because the facts are a function of
/// the payload and nothing else: recording one without the other leaves a
/// pair on disk that disagree, and the disagreement would surface in the CLI
/// and console suites rather than here, where it belongs.
fn settle(name: &str, payload: &Value) {
    settle_file(&format!("{name}.json"), payload, "the gateway serves");
    settle_file(
        &format!("{name}.facts.json"),
        &facts(payload),
        "the corpus implies",
    );
}

fn settle_file(file: &str, payload: &Value, provenance: &str) {
    let path = corpus_dir().join(file);
    let recorded = format!(
        "{}\n",
        serde_json::to_string_pretty(payload).expect("payload serialises")
    );
    if recording() {
        std::fs::write(&path, &recorded).unwrap_or_else(|err| panic!("write {file}: {err}"));
        eprintln!("recorded console/fixtures/{file}");
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "console/fixtures/{file} is missing ({err}) — record it with \
             `{RECORD}=1 make db-test`"
        )
    });
    if committed != recorded {
        panic!(
            "console/fixtures/{file} is not what {provenance}.\n\n\
             If it changed on purpose, re-record with `{RECORD}=1 make db-test` \
             and read the diff: every renderer that consumes this corpus is being asked \
             to change with it.\n\n\
             derived:\n{recorded}"
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
            //
            // The counter goes at the **front**, and that is not cosmetic.
            // Both surfaces abbreviate an id to its first twelve characters,
            // so stand-ins that differ only in their tail are stand-ins a
            // renderer cannot tell apart — a corpus in which every member
            // shares one label, and a parity suite that would pass while
            // the surface showed the wrong row.
            format!("{:08x}-0000-4000-8000-000000000000", self.ids)
        } else if is_object_address(raw) {
            self.hashes += 1;
            // Still 64 hex characters, so an abbreviation to twelve is still
            // an abbreviation of an address — and distinguishing in its
            // first twelve, for the reason above.
            format!("{:08x}{}", self.hashes, "0".repeat(56))
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
    let root = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("mint root");
    let eng = unit(&mut tx, tenant, root.id, "eng", ScopeKind::OrgUnit).await;
    // `workspace`-shaped: the LOCAL cell of the approval matrix, which is
    // what the old `team` rank meant. An `org_unit` is SHARED and prices a
    // memory publication at a curator *and* an administrator (CPR-7); this
    // suite is about what the console renders, not about that price.
    let platform = unit(&mut tx, tenant, eng.id, "platform", ScopeKind::Workspace).await;
    tx.commit().await.expect("commit scopes");

    for subject in ["alice", "cora", "sam", "sec"] {
        seed_user(&pool, tenant, subject).await;
    }
    bind(&pool, tenant, "alice", platform.id, RoleKey::Member).await;
    bind(&pool, tenant, "cora", platform.id, RoleKey::Curator).await;
    bind(&pool, tenant, "sam", platform.id, RoleKey::Administrator).await;
    bind(&pool, tenant, "sec", platform.id, RoleKey::Reviewer).await;

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

/// One org unit under a parent — the shape every grouping takes now that
/// rank is gone (ADR-0073 decision 4).
async fn unit(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    tenant: TenantId,
    parent: ScopeId,
    slug: &str,
    kind: ScopeKind,
) -> Scope {
    scopes::create(
        &mut *tx,
        &scopes::NewScope {
            id: ScopeId::new(),
            tenant_id: tenant,
            kind,
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

/// Three versions of one record, so the drift case's three contents are
/// three visibly different strings rather than three copies of one.
const DRIFT_V1: &str = "page the platform lead";
const DRIFT_V2: &str = "page the platform lead, then the incident commander";
const DRIFT_V3: &str = "page the incident commander first, then the platform lead";

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

/// The drift case, and the only one in which a member's **three** contents
/// are three different strings.
///
/// A record is published, proposed again, and then edited *while the review
/// is open*. The proposal still names the bytes that were proposed, because
/// approvals bind bytes and not records (ADR-0032 decision 6) — so
/// `baseline` is what the channel holds, `proposed` is what the approvals
/// are about, and `content` is what the record says right now, which is a
/// third thing and belongs to nobody's decision yet. Publishing will refuse.
///
/// Without this case the corpus cannot tell whether a surface renders the
/// record or the proposal, because everywhere else the two agree.
async fn memory_drifted(w: &World) -> Value {
    let alice = identities::by_subject(&w.pool, w.tenant, "alice")
        .await
        .expect("read alice")
        .expect("alice exists");

    let note = seed_record(&w.pool, w.tenant, w.platform, alice.id, DRIFT_V1).await;
    let (status, opened) = post(
        &w.app,
        &w.alice,
        "/v1/proposals",
        json!({
            "scope_id": w.platform,
            "record_ids": [note],
            "title": "the escalation note",
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

    // v2 is proposed…
    rewrite(&w.pool, note, w.platform, alice.id, DRIFT_V2).await;
    let (status, opened) = post(
        &w.app,
        &w.alice,
        "/v1/proposals",
        json!({
            "scope_id": w.platform,
            "record_ids": [note],
            "title": "the escalation note, revised",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "open the second proposal: {opened}");
    let second = opened["id"].as_str().expect("proposal id").to_owned();

    // …and then somebody edits the record underneath the open review.
    rewrite(&w.pool, note, w.platform, alice.id, DRIFT_V3).await;

    let detail = detail(w, &second).await;
    assert_eq!(
        detail["members"][0]["unchanged"],
        json!(false),
        "the case is only worth recording if the member really drifted: {detail}",
    );
    record(detail)
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
    settle("memory-drifted", &memory_drifted(&w).await);
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

// ── The facts a review must name ─────────────────────────────────────────────

/// The review-relevant facts of one case, derived from its recorded payload.
///
/// This is the other half of decision 7. The corpus says what the gateway
/// serves; **these say what a surface has to get out of it**, and both
/// renderers are asserted against the same file. Without them "parity" is a
/// diff of two transcripts, which fails on whitespace and passes on a
/// missing finding.
///
/// # Where the line is drawn
///
/// These are *data*, never layout. ADR-0056's option 3 was rejected — a
/// gateway serialising a display model would either constrain the console to
/// what a terminal can do or ship two display models and call it one — and
/// the same line holds here: the corpus fixes **what must be named**, and
/// each surface owns *how*. So a finding's path, line, rule, severity and
/// verdict are here, and the fact that the CLI paints a blocking one with
/// `Mark::Removed` and the console will use a red chip is not.
///
/// # Derived, not authored
///
/// Every field is a projection of the payload rather than a judgement about
/// it — the judgements moved to the gateway in decisions 5 and 6, which is
/// what makes this safe. `blocking` is copied, not computed; a shortfall's
/// sentence is copied, not composed. If a future field here needed a rule to
/// derive it, that rule would belong on the gateway and not in this file.
///
/// The one thing deliberately absent is the AC's "set of actions offered".
/// Which acts a proposal admits is a function of its state, its pack and the
/// reader's own roles, and only the first is on the wire — so a corpus that
/// claimed it would be inventing the other two. It needs a served field, and
/// that is a decision to take with a screen in front of you rather than in a
/// fixture generator.
/// The text inside a member's bytes, as a reviewer reads it.
///
/// A memory's proposed bytes are a canonical JSON object and a skill file's
/// are the file. Both renderers show the *content* either way — the CLI's
/// diff is field-wise over the object rather than a diff of its braces — so
/// the fact is the content, and the envelope is an implementation detail of
/// how it travelled.
fn readable(value: &Value) -> Value {
    let Some(text) = value.as_str() else {
        return Value::Null;
    };
    as_json_object(text)
        .and_then(|object| {
            object
                .get("content")
                .and_then(Value::as_str)
                .map(String::from)
        })
        .map_or_else(|| value.clone(), Value::String)
}

fn facts(detail: &Value) -> Value {
    let mut out = Map::new();
    out.insert("state".to_owned(), detail["state"].clone());
    out.insert("outstanding".to_owned(), detail["outstanding"].clone());

    out.insert(
        "approvals".to_owned(),
        Value::Array(
            detail["approvals"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|approval| {
                    json!({
                        "subject": approval["approver_subject"],
                        "verdict": approval["verdict"],
                        // A review act cast against an earlier commit is
                        // evidence about other content, and a surface that
                        // rendered it like a live one would be showing a
                        // requirement as met that is not.
                        "counts": approval["counts"],
                    })
                })
                .collect(),
        ),
    );

    out.insert(
        "members".to_owned(),
        Value::Array(
            detail["members"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|member| {
                    let drifted = !member["unchanged"].as_bool().unwrap_or(true);
                    // A member the publication would not touch has no diff
                    // to show, and the *corpus* says so rather than each
                    // renderer deciding it locally — a condition encoded
                    // twice is a condition two surfaces can disagree about,
                    // which is the whole thing decision 7 is for.
                    let shows_a_diff = member["effect"] != json!("none");
                    json!({
                        "name": member["member"],
                        "effect": member["effect"],
                        // `unchanged` inverted, because what a reviewer has
                        // to be told is the exceptional case: the bytes moved
                        // under the review and publishing will refuse.
                        "drifted": drifted,
                        // The three contents ADR-0035 decision 5 puts in front
                        // of a reviewer, as **text a person reads** rather than
                        // as the bytes that carry it: a memory's canonical
                        // object is JSON, and what has to be named is the
                        // content inside it, not the envelope.
                        "proposed": if shows_a_diff { readable(&member["proposed"]) } else { Value::Null },
                        "baseline": match member.get("baseline") {
                            Some(baseline) if shows_a_diff => readable(&baseline["text"]),
                            _ => Value::Null,
                        },
                        // The record as it stands now. Equal to `proposed`
                        // unless somebody edited underneath the review, which
                        // is the only condition under which it is a third
                        // fact rather than a repeat of the second — so it is
                        // carried only then, and a surface is asked to name
                        // it only when it means something.
                        "current": if drifted { readable(&member["content"]) } else { Value::Null },
                    })
                })
                .collect(),
        ),
    );

    if let Some(scan) = detail.get("scan") {
        out.insert(
            "scan".to_owned(),
            json!({
                "blocks_at": scan["blocks_at"],
                "blocked": scan["blocked"],
                // Order is a fact: worst first is what makes a truncated
                // read still lead with the reason.
                "findings": scan["findings"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .map(|finding| json!({
                        "path": finding["path"],
                        "line": finding["line"],
                        "rule": finding["rule"],
                        "severity": finding["severity"],
                        "blocking": finding["blocking"],
                    }))
                    .collect::<Vec<_>>(),
            }),
        );
    }

    if let Some(quality) = detail.get("quality") {
        let checklist = match quality.get("checklist") {
            Some(checklist) if checklist["complete"] == json!(true) => "complete",
            Some(_) => "partial",
            None => "absent",
        };
        out.insert(
            "quality".to_owned(),
            json!({
                // Two numbers, never one. A surface that averaged them could
                // not tell a well-formatted bundle nobody worked through from
                // one somebody did (ADR-0053 decision 1).
                "score": quality["score"],
                "min_score": quality["min_score"],
                "checklist": checklist,
                "checklist_required": quality["requires_checklist"],
                // The gateway's sentences, copied. Composing them here would
                // be a third author of a line ADR-0056 decision 6 exists to
                // give one author.
                "shortfalls": quality["shortfalls"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .map(|shortfall| shortfall["detail"].clone())
                    .collect::<Vec<_>>(),
                "needs_override": quality["needs_override"],
            }),
        );
    }

    Value::Object(out)
}

/// Derives the facts from every committed payload.
///
/// Pure, and therefore not gated on a database: `make ci` checks that the
/// facts still follow from the payloads even where no Postgres exists, which
/// matters because the facts are what both renderers are held to and a stale
/// one would weaken two suites at once.
#[test]
fn the_facts_follow_from_the_corpus() {
    for case in CASES {
        let detail = read_case(&corpus_dir(), case);
        settle_file(
            &format!("{case}.facts.json"),
            &facts(&detail),
            "the corpus implies",
        );
    }
}
