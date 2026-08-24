//! CPR-14: the Claude Code lifecycle acceptance harness (ADR-0079).
//!
//! **Tier 2 of three.** The three tiers, and what each can and cannot prove:
//!
//! | Tier | Client | Gateway | Runs in |
//! |---|---|---|---|
//! | 1 — captured/mock | the real hook binary, captured frames | a mock HTTP server | `make ci` (`pnpm -r test` → `dist/driver.mjs`) |
//! | 2 — captured/live | the real hook binary, captured frames | **this gateway, this schema, this PDP, this chain** | `make db-test` / `make claude-acceptance` |
//! | 3 — live client | **`claude -p` itself** | a real gateway | `make claude-acceptance-live`, never CI |
//!
//! Tier 1 proves the hook contract. Tier 3 proves the frames are the frames.
//! This tier is the one that can say *the database and the audit chain are
//! correct*, because it holds both ends: it serves the real router over a real
//! socket and then reads the rows the hooks caused.
//!
//! What "authentic" means here: the frames under
//! `adapters/claude-code/fixtures/hooks/` and the transcript under
//! `fixtures/transcripts/tool-turn.jsonl` are **recorded from Claude Code
//! 2.1.241**, shapes verbatim and content synthetic, by tier 3's `--capture`
//! mode. Nothing in this file invents a payload shape.
//!
//! The hook is spawned exactly as `adapters/claude-code/hooks/hooks.json`
//! registers it — `node <plugin-root>/dist/hook.mjs <mode>`, payload on stdin
//! — so what is exercised is the product and not a Rust re-implementation of
//! it.
//!
//! Skips, with a message, when `DATABASE_URL` is unset (CI has no database) or
//! when `adapters/claude-code/dist/hook.mjs` has not been built.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_audit::ChainVerification;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::{Hs256Verifier, ProvisioningClaims};
use synveda_ingest::capture_worker::{self, Config as CaptureConfig, Deps as CaptureDeps};
use synveda_ingest::extraction::{AnyExtractor, DeterministicExtractor};
use synveda_types::TenantId;
use tokio::io::AsyncWriteExt;
use tokio::sync::oneshot;

const SECRET: &[u8] = b"cpr-14-claude-lifecycle-secret";
/// The identity the hooks run as: holds `administrator` at the tenant root,
/// which is the grant a first login mints (CPR-7's `synveda-admins` rule).
const OPERATOR: &str = "cpr14-operator";

/// The committed timeline golden the console renders (ADR-0079 decision 5).
/// Regenerate with `SYNVEDA_WRITE_CLAUDE_TIMELINE=1 make claude-acceptance`.
const TIMELINE_GOLDEN: &str = "../../console/src/fixtures/claude-timeline.json";

/// One harness session id, so two runs of this suite do not collide in the
/// spool directory. Replaced by a placeholder before anything is compared.
fn harness_session_id() -> String {
    format!("cpr14-{}", TenantId::new().as_uuid().simple())
}

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

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolve the workspace root")
}

fn state(url: &str) -> AppState {
    AppState {
        pool: PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(5))
            .connect_lazy(url)
            .expect("parse database url"),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp: Arc::new(synveda_policy::Pdp::new().expect("build the embedded PDP")),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index: Arc::new(
            synveda_retrieval::SearchIndex::open(
                std::env::temp_dir()
                    .join("synveda-cpr14-tests")
                    .join(TenantId::new().to_string()),
            )
            .expect("open search index"),
        ),
        embedder: Arc::new(synveda_ingest::embedding::AnyEmbedder::Deterministic(
            synveda_ingest::embedding::DeterministicEmbedder::new(),
        )),
        inject_embed_timeout: Duration::from_millis(200),
        keys: Arc::new(synveda_store::keys::KeyRing::new(
            synveda_crypto::Kms::Disabled,
        )),
    }
}

/// A gateway that can be taken down and brought back up on the same address —
/// which is what makes the outage in this suite an outage rather than a typo.
struct Gateway {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    joined: Option<tokio::task::JoinHandle<()>>,
    state: AppState,
}

impl Gateway {
    async fn start(state: AppState, addr: Option<SocketAddr>) -> Self {
        let listener = match addr {
            None => tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind the gateway"),
            Some(addr) => {
                // The previous bind of this port has just closed; a rebind can
                // still lose to the kernel finishing with it. Retry briefly
                // rather than fail an acceptance run on a socket race.
                let mut last = None;
                let mut bound = None;
                for _ in 0..50 {
                    match tokio::net::TcpListener::bind(addr).await {
                        Ok(listener) => {
                            bound = Some(listener);
                            break;
                        }
                        Err(err) => {
                            last = Some(err);
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                    }
                }
                bound.unwrap_or_else(|| {
                    panic!("rebind the gateway on {addr}: {}", last.expect("an error"))
                })
            }
        };
        let addr = listener.local_addr().expect("gateway address");
        let (tx, rx) = oneshot::channel();
        let app = router(state.clone());
        let joined = tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await;
        });
        Self {
            addr,
            shutdown: Some(tx),
            joined: Some(joined),
            state,
        }
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Takes the gateway down and waits for the socket to close.
    async fn stop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(joined) = self.joined.take() {
            let _ = joined.await;
        }
    }

    /// Brings it back on the same address.
    async fn restart(&mut self) {
        let restarted = Gateway::start(self.state.clone(), Some(self.addr)).await;
        self.shutdown = restarted.shutdown;
        self.joined = restarted.joined;
    }
}

/// Everything one acceptance run needs, and nothing shared between runs.
struct Harness {
    gateway: Gateway,
    pool: PgPool,
    tenant_id: TenantId,
    token: String,
    workspace_id: String,
    project_id: String,
    /// The scratch machine: HOME, XDG config, XDG state, the project.
    home: PathBuf,
    project: PathBuf,
    transcript: PathBuf,
    external_session_id: String,
    hook: PathBuf,
    fixtures: PathBuf,
}

/// Connects, migrates, admits a tenant, provisions the operator through the
/// production login seam, opens a gateway and a scratch machine — or answers
/// `None` with a reason. Every governed asset after tenant admission goes
/// through a public route and the PDP.
async fn harness() -> Option<Harness> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping the Claude lifecycle harness: DATABASE_URL is not set \
                 (run `make dev-up` then `make claude-acceptance`)"
            );
            return None;
        }
    };
    let root = repo_root();
    let hook = root.join("adapters/claude-code/dist/hook.mjs");
    if !hook.exists() {
        eprintln!(
            "skipping the Claude lifecycle harness: {} is not built \
             (run `pnpm --filter @synveda/claude-code-adapter build`)",
            hook.display()
        );
        return None;
    }

    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");
    let tenant_id = TenantId::new();
    let slug = format!("cpr14-{}", tenant_id.as_uuid().simple());
    let tenant = synveda_store::tenants::create(
        &pool,
        tenant_id,
        &slug,
        "CPR-14 Claude lifecycle",
        synveda_types::TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    let state = state(&url);
    synveda_gateway::provision::provision(
        &state,
        &tenant,
        OPERATOR,
        &ProvisioningClaims {
            groups: vec!["synveda-admins".to_owned()],
            display_name: Some("CPR-14 operator".to_owned()),
            ..ProvisioningClaims::default()
        },
    )
    .await
    .expect("provision the first admin through the login seam");
    let gateway = Gateway::start(state.clone(), None).await;
    let token = Hs256Verifier::new(SECRET).issue(OPERATOR, tenant_id, Duration::from_secs(3600));

    // The workspace goes through the public route, under the PDP, like
    // anything else a person would create before pointing a client at it.
    let workspace: Value = post(
        &gateway.url(),
        &token,
        "/v1/workspaces",
        Some(&format!("ws-{slug}")),
        json!({"slug": slug, "display_name": "Acme API"}),
    )
    .await
    .1;
    let workspace_id = workspace["id"]
        .as_str()
        .unwrap_or_else(|| panic!("workspace id in {workspace}"))
        .to_owned();

    let (project_status, project_view) = post(
        &gateway.url(),
        &token,
        &format!("/v1/workspaces/{workspace_id}/projects"),
        Some(&format!("pr-{slug}")),
        json!({"slug": "acceptance", "display_name": "Acceptance"}),
    )
    .await;
    assert_eq!(project_status, 201, "{project_view}");
    let project_id = project_view["id"]
        .as_str()
        .unwrap_or_else(|| panic!("project id in {project_view}"))
        .to_owned();

    // Exercise the complete session -> capture -> VedaFlow -> Knowledge path
    // before opening the adapter run. CPR-18 deliberately does not bridge the
    // accepted item back into the temporary record-backed context composer;
    // CPR-20 replaces that read seam with Knowledge retrieval.
    seed_memory_through_session_api(&gateway, &state, &token, &workspace_id, &project_id).await;

    let home = std::env::temp_dir().join(format!("synveda-cpr14-{}", tenant_id.as_uuid().simple()));
    let project = home.join("Source/acme-api");
    let transcript = home.join(".claude/projects/acme-api/session.jsonl");
    std::fs::create_dir_all(&project).expect("create the scratch project");
    std::fs::create_dir_all(transcript.parent().expect("transcript parent"))
        .expect("create the transcript directory");
    std::fs::write(&transcript, "").expect("create an empty transcript");

    Some(Harness {
        gateway,
        pool,
        tenant_id,
        token,
        workspace_id,
        project_id,
        project,
        transcript,
        external_session_id: harness_session_id(),
        hook,
        fixtures: root.join("adapters/claude-code/fixtures"),
        home,
    })
}

/// Creates one pre-existing Knowledge item through the complete capture and
/// VedaFlow path. This is intentionally more work than inserting a row:
/// direct table mutation would bypass the PDP and invalidate the acceptance
/// evidence.
async fn seed_memory_through_session_api(
    gateway: &Gateway,
    state: &AppState,
    token: &str,
    workspace_id: &str,
    project_id: &str,
) {
    let (status, opened) = post(
        &gateway.url(),
        token,
        "/v1/sessions",
        Some("cpr14-seed-open"),
        json!({
            "workspace_id": workspace_id,
            "project_id": project_id,
            "client_name": "cpr14-seed",
            "external_session_id": "cpr14-seed"
        }),
    )
    .await;
    assert_eq!(status, 201, "{opened}");
    let session_id = opened["id"].as_str().expect("seed session id");
    let (status, appended) = post(
        &gateway.url(),
        token,
        &format!("/v1/sessions/{session_id}/events"),
        None,
        json!({"events": [{
            "event_type": "message.assistant",
            "client_event_id": "cpr14-seed-event",
            "occurred_at": "2026-08-23T20:59:00Z",
            "payload": {"text": "Deploys go through make deploy; never push to main directly."}
        }]}),
    )
    .await;
    assert_eq!(status, 200, "{appended}");

    let (status, batch) = post(
        &gateway.url(),
        token,
        &format!("/v1/sessions/{session_id}/capture-batches"),
        Some("cpr14-seed-capture"),
        json!({}),
    )
    .await;
    assert_eq!(status, 201, "{batch}");
    let batch_id = batch["id"].as_str().expect("seed capture batch id");

    let deps = CaptureDeps {
        pool: state.pool.clone(),
        pdp: Arc::clone(&state.pdp),
        extractor: Arc::new(AnyExtractor::Deterministic(DeterministicExtractor::new())),
    };
    let config = CaptureConfig {
        poll_interval: Duration::from_millis(1),
        lease_duration: Duration::from_secs(30),
        batches_per_tenant: 32,
        lease_owner: "cpr14-seed".to_owned(),
    };
    for _ in 0..20 {
        let summary = capture_worker::sweep_once(&deps, &config)
            .await
            .expect("capture worker pass");
        if summary.completed > 0 {
            break;
        }
    }
    let (status, candidates) = get(
        &gateway.url(),
        token,
        &format!("/v1/capture-candidates?batch_id={batch_id}"),
    )
    .await;
    assert_eq!(status, 200, "{candidates}");
    let candidate_id = candidates["candidates"]
        .as_array()
        .and_then(|items| items.first())
        .and_then(|candidate| candidate["id"].as_str())
        .unwrap_or_else(|| panic!("deterministic extraction produced no candidate: {candidates}"));
    let (status, accepted) = post(
        &gateway.url(),
        token,
        &format!("/v1/capture-candidates/{candidate_id}/accept"),
        Some("cpr14-seed-accept"),
        json!({}),
    )
    .await;
    assert!(matches!(status, 200 | 201), "{accepted}");
    assert!(
        matches!(
            accepted["candidate"]["resulting_outcome"].as_str(),
            Some("applied" | "pending_review")
        ),
        "the candidate entered VedaFlow: {accepted}"
    );
}

/// What one hook run produced.
struct HookRun {
    status: Option<i32>,
    stdout: String,
    elapsed: Duration,
}

impl HookRun {
    /// The `additionalContext` the harness would have handed the model.
    fn context(&self) -> Option<String> {
        let parsed: Value = serde_json::from_str(self.stdout.trim()).ok()?;
        parsed
            .get("hookSpecificOutput")?
            .get("additionalContext")?
            .as_str()
            .map(str::to_owned)
    }
}

impl Harness {
    /// One recorded frame, repointed at this scratch machine.
    fn frame(&self, name: &str) -> Value {
        let raw = std::fs::read_to_string(self.fixtures.join(format!("hooks/{name}.json")))
            .unwrap_or_else(|err| panic!("read the recorded frame {name}: {err}"));
        let mut frame: Value = serde_json::from_str(&raw).expect("parse the recorded frame");
        frame["session_id"] = json!(self.external_session_id);
        frame["cwd"] = json!(self.project.to_string_lossy());
        frame["transcript_path"] = json!(self.transcript.to_string_lossy());
        frame
    }

    /// Appends a recorded transcript fixture to this run's transcript, which is
    /// what the client itself does as a turn proceeds.
    fn append_transcript(&self, fixture: &str) {
        let raw = std::fs::read_to_string(self.fixtures.join(format!("transcripts/{fixture}")))
            .unwrap_or_else(|err| panic!("read the recorded transcript {fixture}: {err}"));
        let mut existing = std::fs::read_to_string(&self.transcript).unwrap_or_default();
        existing.push_str(&raw);
        std::fs::write(&self.transcript, existing).expect("extend the transcript");
    }

    /// Uses the adapter's supported per-project configuration seam. The
    /// replay needs one SessionStart whose only job is backlog delivery; it
    /// turns injection back on before the recovery start whose composition is
    /// part of the acceptance evidence.
    fn set_inject(&self, enabled: bool) {
        let directory = self.project.join(".synveda");
        std::fs::create_dir_all(&directory).expect("create project config directory");
        std::fs::write(
            directory.join("config.json"),
            json!({"inject": enabled}).to_string(),
        )
        .expect("write project config");
    }

    /// Runs the hook the way `hooks.json` registers it.
    async fn hook(&self, mode: &str, frame: &Value, gateway: &str) -> HookRun {
        let started = Instant::now();
        let mut child = tokio::process::Command::new("node")
            .arg(&self.hook)
            .arg(mode)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("XDG_STATE_HOME", self.home.join(".local/state"))
            .env("SYNVEDA_GATEWAY", gateway)
            .env("SYNVEDA_TOKEN", &self.token)
            .env("SYNVEDA_WORKSPACE", &self.workspace_id)
            .env("SYNVEDA_PROJECT", &self.project_id)
            // The governed skills entry shells out to a CLI this suite does not
            // build; it is tier 3's to exercise, and off here so a missing
            // binary is not read as a lifecycle failure.
            .env("SYNVEDA_TIMEOUT_MS", "5000")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn the hook");
        let payload = frame.to_string();
        child
            .stdin
            .as_mut()
            .expect("hook stdin")
            .write_all(payload.as_bytes())
            .await
            .expect("write the frame");
        child.stdin.take();
        let output = child.wait_with_output().await.expect("hook exits");
        HookRun {
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            elapsed: started.elapsed(),
        }
    }

    async fn get(&self, path: &str) -> (u16, Value) {
        get(&self.gateway.url(), &self.token, path).await
    }

    /// The one session this run opened, straight from the table.
    async fn session_row(&self) -> Option<(uuid::Uuid, String, Option<String>, i64)> {
        sqlx::query_as::<_, (uuid::Uuid, String, Option<String>, i64)>(
            "select s.id, s.status::text, s.end_reason, \
             (select count(*) from session_events e where e.session_id = s.id) \
             from sessions s where s.tenant_id = $1 and s.external_session_id = $2",
        )
        .bind(self.tenant_id.as_uuid())
        .bind(&self.external_session_id)
        .fetch_optional(&self.pool)
        .await
        .expect("read the session row")
    }

    /// Every appended event, in server-assigned order.
    async fn events(&self) -> Vec<(i64, String, String)> {
        sqlx::query_as::<_, (i64, String, String)>(
            "select e.sequence, e.event_type::text, e.client_event_id \
             from session_events e join sessions s on s.id = e.session_id \
             where s.tenant_id = $1 and s.external_session_id = $2 order by e.sequence",
        )
        .bind(self.tenant_id.as_uuid())
        .bind(&self.external_session_id)
        .fetch_all(&self.pool)
        .await
        .expect("read the events")
    }

    async fn event_hashes(&self) -> Vec<(String, String)> {
        sqlx::query_as::<_, (String, String)>(
            "select e.client_event_id, e.payload_hash \
             from session_events e join sessions s on s.id = e.session_id \
             where s.tenant_id = $1 and s.external_session_id = $2 order by e.sequence",
        )
        .bind(self.tenant_id.as_uuid())
        .bind(&self.external_session_id)
        .fetch_all(&self.pool)
        .await
        .expect("read the server event hashes")
    }

    async fn chain_actions(&self) -> Vec<String> {
        sqlx::query_scalar::<_, String>(
            "select action from audit_log where tenant_id = $1 order by seq",
        )
        .bind(self.tenant_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .expect("read the chain")
    }

    /// What the spool holds for this run, if anything, and the file that owns
    /// it. The path is needed only for permission evidence.
    fn spool_file(&self) -> Option<(PathBuf, Value)> {
        let dir = self.home.join(".local/state/synveda/spool");
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let raw = std::fs::read_to_string(entry.path()).ok()?;
            let parsed: Value = serde_json::from_str(&raw).ok()?;
            if parsed["external_session_id"].as_str() == Some(self.external_session_id.as_str()) {
                return Some((entry.path(), parsed));
            }
        }
        None
    }

    fn spool(&self) -> Option<Value> {
        self.spool_file().map(|(_, spool)| spool)
    }

    fn logged(&self, event: &str) -> Vec<Value> {
        let path = self.home.join(".local/state/synveda/adapter.log");
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|row| row["event"] == event)
            .collect()
    }

    /// Content-free diagnostics for the live gate: hook and adapter event
    /// names only, never transcript fields or log values.
    fn captured_hook_names(&self) -> Vec<String> {
        let mut names = std::fs::read_dir(self.home.join("captures"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
            .filter_map(|raw| serde_json::from_str::<Value>(&raw).ok())
            .filter_map(|frame| {
                let name = frame.get("hook_event_name")?.as_str()?;
                Some(match name {
                    "SessionStart" | "Stop" | "PreCompact" | "SessionEnd" => name.to_owned(),
                    _ => "<unknown>".to_owned(),
                })
            })
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    fn adapter_log_event_names(&self) -> Vec<String> {
        let path = self.home.join(".local/state/synveda/adapter.log");
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter_map(|row| row.get("event")?.as_str().map(safe_diagnostic_name))
            .collect()
    }

    fn cleanup(&self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        // Tier 3 may hold a transient Claude credential plus raw hook frames.
        // Keep explicit happy-path cleanup, but make every assertion failure
        // clean the isolated machine too.
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn spool_payload_hash(payload: &Value) -> String {
    let canonical = synveda_types::json::canonicalise(payload).to_string();
    format!("{:x}", Sha256::digest(canonical.as_bytes()))
}

async fn post(base: &str, token: &str, path: &str, key: Option<&str>, body: Value) -> (u16, Value) {
    let mut request = reqwest::Client::new()
        .post(format!("{base}{path}"))
        .bearer_auth(token)
        .json(&body);
    if let Some(key) = key {
        request = request.header("idempotency-key", key);
    }
    read(request).await
}

async fn get(base: &str, token: &str, path: &str) -> (u16, Value) {
    read(
        reqwest::Client::new()
            .get(format!("{base}{path}"))
            .bearer_auth(token),
    )
    .await
}

async fn read(request: reqwest::RequestBuilder) -> (u16, Value) {
    let response = request.send().await.expect("the gateway answers");
    let status = response.status().as_u16();
    let body = response.text().await.expect("collect the body");
    (status, serde_json::from_str(&body).unwrap_or(Value::Null))
}

// ── The lifecycle, once, end to end ──────────────────────────────────────────

/// The ten claims of CPR-14, in the order a real conversation makes them.
///
/// One test rather than ten, because they are ten assertions about **one
/// run**: an outage that queues events is only interesting if the run it
/// queued them for is the run that later delivers them, and ten tests would
/// each have to rebuild the nine steps before their own.
#[tokio::test]
async fn a_claude_code_session_is_a_governed_run_from_start_to_end() {
    let _guard = serial().await;
    let Some(mut h) = harness().await else { return };
    let live = h.gateway.url();

    // ── 1. A captured SessionStart creates a session ─────────────────────────
    let start = h.frame("session-start-startup");
    let first = h.hook("session-start", &start, &live).await;
    assert_eq!(first.status, Some(0), "a hook always exits 0");

    let (session_id, status, _, _) = h
        .session_row()
        .await
        .expect("SessionStart opened exactly one run");
    assert_eq!(status, "active");

    let (code, view) = h.get(&format!("/v1/sessions/{session_id}")).await;
    assert_eq!(code, 200, "{view}");
    assert_eq!(view["client_name"], "claude-code");
    assert_eq!(view["agent_name"], "claude-code");
    assert_eq!(view["external_session_id"], h.external_session_id.as_str());
    assert_eq!(view["workspace_id"], h.workspace_id.as_str());
    assert_eq!(view["project_id"], h.project_id.as_str());
    assert!(
        view["client_installation_id"].is_string(),
        "the run names the installation that opened it: {view}"
    );
    // The run's scope is *derived* from the workspace and never sent by the
    // client — the property the whole session plane rests on (ADR-0076).
    assert!(view["scope_id"].is_string(), "{view}");
    assert_eq!(
        view["principal_id"], OPERATOR,
        "the run is attributed to the bearer's subject, not to anything the client said"
    );
    // A real headless SessionStart carries no `model`, so nothing invents one.
    assert!(
        view["model_name"].is_null(),
        "a headless start names no model: {view}"
    );

    // ── 2. Context comes through the session context endpoint ────────────────
    let context_run: (i32, String, String) = sqlx::query_as(
        "select entry_count, rendered, block_hash \
         from session_context_runs \
         where tenant_id = $1 and session_id = $2",
    )
    .bind(h.tenant_id.as_uuid())
    .bind(session_id)
    .fetch_one(&h.pool)
    .await
    .expect("SessionStart context run");
    assert_eq!(
        context_run.0, 0,
        "CPR-18 publishes Knowledge without dual-writing the temporary record index"
    );
    assert!(
        context_run.1.is_empty(),
        "the legacy composer must not translate accepted Knowledge: {context_run:?}"
    );
    assert!(
        first.context().is_none(),
        "the hook must not manufacture context when composition selected nothing"
    );
    assert!(
        context_run.2.len() == 64 && context_run.2.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "even an empty composition has an auditable hash: {context_run:?}"
    );

    // ── 3. Stop is durable locally; the next start delivers ─────────────────
    h.append_transcript("tool-turn.jsonl");
    let stop = h.frame("stop");
    let normal_stop = h.hook("turn", &stop, &live).await;
    assert_eq!(normal_stop.status, Some(0));

    let stopped_spool = h.spool().expect("Stop wrote the durable spool");
    let stopped_entries = stopped_spool["entries"].as_array().expect("spool entries");
    assert_eq!(stopped_entries.len(), 4, "the whole tool turn is durable");
    assert!(
        stopped_entries.iter().all(|entry| {
            entry["acknowledged"] != json!(true) && entry["delivery_attempts"] == json!(0)
        }),
        "Stop performs no delivery attempt: {stopped_spool}"
    );
    assert!(
        h.events().await.is_empty(),
        "the synchronous Stop boundary ends before the gateway"
    );

    // A supported project setting turns context injection off for this one
    // start, leaving it as the eligible lifecycle retry the delivery design
    // promises rather than manufacturing a call into an adapter function.
    h.set_inject(false);
    let delivery_start = h.frame("session-start-startup");
    let delivered = h.hook("session-start", &delivery_start, &live).await;
    assert_eq!(delivered.status, Some(0));
    h.set_inject(true);

    let after_turn = h.events().await;
    let types: Vec<&str> = after_turn.iter().map(|(_, t, _)| t.as_str()).collect();
    assert_eq!(
        types,
        vec![
            "message.user",
            "tool.invoked",
            "tool.result",
            "message.assistant"
        ],
        "a real tool-using turn arrives as four ordered events, the call among them"
    );
    assert_eq!(
        after_turn
            .iter()
            .map(|(seq, _, _)| *seq)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4],
        "sequences are the server's, contiguous from one"
    );

    // ── 5. A gateway outage queues events locally ────────────────────────────
    //
    // The second turn first reaches the spool exactly as the client writes it.
    // Only then does the gateway go away, matching the recovery contract's
    // boundary rather than racing the hook which establishes it.
    h.append_transcript("second-turn.jsonl");
    let outage = h.hook("turn", &stop, &live).await;
    assert_eq!(
        outage.status,
        Some(0),
        "the durable Stop never fails a session"
    );
    h.gateway.stop().await;

    let (spool_path, spooled) = h.spool_file().expect("the turn is on disk");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = spool_path.parent().expect("spool directory");
        assert_eq!(
            std::fs::metadata(directory)
                .expect("spool directory metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700,
            "payload-bearing spool directory is private"
        );
        assert_eq!(
            std::fs::metadata(&spool_path)
                .expect("spool metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600,
            "payload-bearing spool file is private"
        );
    }
    let entries = spooled["entries"].as_array().expect("spool entries");
    for entry in entries {
        assert_eq!(
            entry["payload_hash"].as_str(),
            Some(spool_payload_hash(&entry["payload"]).as_str()),
            "the local SHA-256 attests the payload bytes: {entry}"
        );
    }
    let unacknowledged = spooled["entries"]
        .as_array()
        .expect("spool entries")
        .iter()
        .filter(|entry| entry["acknowledged"] != json!(true))
        .count();
    assert_eq!(
        unacknowledged, 2,
        "the second turn is held, durably, and nothing else is: {spooled}"
    );
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry["acknowledged"] == json!(true))
            .count(),
        4,
        "acknowledged entries remain distinguishable from the pending tail"
    );
    assert!(
        entries
            .iter()
            .filter(|entry| entry["acknowledged"] != json!(true))
            .all(|entry| entry["delivery_attempts"] == json!(0)),
        "Stop does not contact the unavailable gateway: {spooled}"
    );
    assert_eq!(
        h.events().await.len(),
        4,
        "an outage delivers nothing — the events are on disk, not at the gateway"
    );

    // ── 6 and 7. Recovery redelivers an overlapping batch ────────────────────
    //
    // First deliver one of the two held entries through the same public append
    // route, but deliberately do not update the spool — the exact state after
    // a server commit whose acknowledgement was lost. The next SessionStart
    // sends the two-entry backlog: duplicate + appended in one batch.
    h.gateway.restart().await;
    let pending_entries: Vec<Value> = entries
        .iter()
        .filter(|entry| entry["acknowledged"] != json!(true))
        .cloned()
        .collect();
    let first_pending = pending_entries.first().expect("one pending entry");
    let append_started = Instant::now();
    let (partial_status, partial) = post(
        &live,
        &h.token,
        &format!("/v1/sessions/{session_id}/events"),
        None,
        json!({"events": [{
            "event_type": first_pending["event_type"],
            "client_event_id": first_pending["client_event_id"],
            "occurred_at": first_pending["occurred_at"],
            "payload": first_pending["payload"]
        }]}),
    )
    .await;
    let append_latency = append_started.elapsed();
    assert_eq!(partial_status, 200, "{partial}");
    assert_eq!(partial["appended"], 1, "{partial}");
    assert_eq!(partial["events"][0]["event"]["sequence"], 5, "{partial}");
    assert_eq!(
        partial["events"][0]["event"]["payload_hash"]
            .as_str()
            .map(str::len),
        Some(64),
        "the server returns its authoritative BLAKE3 digest: {partial}"
    );

    let resumed = h.frame("session-start-startup");
    let recovered = h.hook("session-start", &resumed, &live).await;
    assert_eq!(recovered.status, Some(0));

    let after_backlog = h.events().await;
    assert_eq!(
        after_backlog
            .iter()
            .map(|(_, t, _)| t.as_str())
            .collect::<Vec<_>>(),
        vec![
            "message.user",
            "tool.invoked",
            "tool.result",
            "message.assistant",
            "message.user",
            "message.assistant"
        ],
        "the backlog lands after what preceded it, once"
    );
    let ids: Vec<&str> = after_backlog.iter().map(|(_, _, id)| id.as_str()).collect();
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        ids.len(),
        "exactly once means no client event id appears twice: {ids:?}"
    );
    let recovered_spool = h
        .spool()
        .expect("recovery retains acknowledgement evidence");
    let recovered_entries = recovered_spool["entries"]
        .as_array()
        .expect("spool entries");
    let overlap: Vec<&Value> = recovered_entries
        .iter()
        .filter(|entry| {
            pending_entries
                .iter()
                .any(|pending| pending["client_event_id"] == entry["client_event_id"])
        })
        .collect();
    assert_eq!(overlap.len(), 2);
    assert_eq!(overlap[0]["outcome"], "duplicate", "{recovered_spool}");
    assert_eq!(overlap[1]["outcome"], "appended", "{recovered_spool}");
    assert!(
        overlap
            .iter()
            .all(|entry| entry["acknowledged"] == json!(true)),
        "both halves of the overlap are acknowledged: {recovered_spool}"
    );
    assert_eq!(
        after_backlog[4].0, 5,
        "the duplicate kept its original position"
    );
    assert_eq!(
        after_backlog[5].0, 6,
        "the new event took the next position"
    );

    // ── 4 and 8. A headless completion flushes, and SessionEnd ends the run ──
    //
    // The `SessionEnd` frame is the one a `claude -p` completion actually
    // emits — `reason: "other"`, which is not `clear` and not `logout`.
    let end = h.frame("session-end-headless");
    let ended = h.hook("turn", &end, &live).await;
    assert_eq!(ended.status, Some(0));

    let (_, status, end_reason, event_count) = h.session_row().await.expect("the run survives");
    assert_eq!(
        status, "ended",
        "a drained close ends rather than lingering"
    );
    assert_eq!(
        end_reason.as_deref(),
        Some("other"),
        "the client's own reason is carried, not a reason this product invented"
    );
    assert_eq!(event_count, 6, "nothing arrived late and nothing was lost");
    let (code, view) = h.get(&format!("/v1/sessions/{session_id}")).await;
    assert_eq!(code, 200, "{view}");
    assert!(view["ended_at"].is_string(), "{view}");

    // ── 9. The console timeline displays it ──────────────────────────────────
    let (code, timeline) = h.get(&format!("/v1/sessions/{session_id}/timeline")).await;
    assert_eq!(code, 200, "{timeline}");
    assert_eq!(timeline["truncated"], json!(false));
    let counts = timeline["event_counts"]
        .as_object()
        .expect("event counts")
        .iter()
        .map(|(name, count)| (name.clone(), count.as_i64().unwrap_or_default()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(counts.get("message.user"), Some(&2));
    assert_eq!(counts.get("message.assistant"), Some(&2));
    assert_eq!(counts.get("tool.invoked"), Some(&1));
    assert_eq!(counts.get("tool.result"), Some(&1));
    let entries = timeline["entries"].as_array().expect("timeline entries");
    let compositions = entries
        .iter()
        .filter(|entry| entry["kind"] == "context_run")
        .count();
    assert_eq!(
        compositions, 2,
        "each SessionStart composed — a resumed conversation asks again, over a \
         corpus that may have moved: {timeline}"
    );
    assert_eq!(
        entries.len(),
        8,
        "six events and two compositions, merged: {timeline}"
    );
    // Every event here was replayed from a recorded transcript and delivered
    // now, so the two clocks are minutes apart and the server says so. That is
    // exactly what a spooled batch looks like, and the flag names no cause.
    assert!(
        entries
            .iter()
            .filter(|entry| entry["kind"] == "event")
            .all(|entry| entry["delayed"] == json!(true) && entry["received_at"].is_string()),
        "a replayed event reports both clocks and the gap between them: {timeline}"
    );
    assert!(
        entries
            .iter()
            .filter(|entry| entry["kind"] == "context_run")
            .all(|entry| entry["delayed"] == json!(false) && entry["received_at"].is_null()),
        "a composition happens here, so it has one clock: {timeline}"
    );
    // The payload text is never on a timeline; a summary is.
    let rendered = timeline.to_string();
    assert!(
        !rendered.contains("retry budget is 3 attempts"),
        "a timeline summarises and never carries payload text: {rendered}"
    );
    check_timeline_golden(&timeline, &session_id.to_string(), &h.workspace_id);

    // ── 10. Audit events and database state ──────────────────────────────────
    let chain = h.chain_actions().await;
    for action in [
        "session.opened",
        "session.context.composed",
        "session.events.appended",
        "session.ended",
    ] {
        assert!(
            chain.iter().any(|entry| entry == action),
            "{action} is missing from the chain: {chain:?}"
        );
    }
    for action in ["session.opened", "session.ended"] {
        let count: i64 = sqlx::query_scalar(
            "select count(*) from audit_log \
             where tenant_id = $1 and action = $2 and payload->'session'->>'id' = $3",
        )
        .bind(h.tenant_id.as_uuid())
        .bind(action)
        .bind(session_id.to_string())
        .fetch_one(&h.pool)
        .await
        .expect("count this run's lifecycle audit action");
        assert_eq!(
            count, 1,
            "one {action} for the accepted run; the seed session is separate: {chain:?}"
        );
    }
    let mut tx = synveda_store::rls::begin_tenant_tx(&h.pool, h.tenant_id)
        .await
        .expect("begin tenant tx");
    let verification = synveda_audit::verify(&mut tx, h.tenant_id)
        .await
        .expect("verify the chain");
    tx.commit().await.expect("commit the read");
    assert!(
        matches!(verification, ChainVerification::Valid { .. }),
        "the chain must verify after a whole session: {verification:?}"
    );
    // Nothing a transcript said reaches the chain — the chain carries counts.
    let payloads: Vec<String> =
        sqlx::query_scalar("select payload::text from audit_log where tenant_id = $1")
            .bind(h.tenant_id.as_uuid())
            .fetch_all(&h.pool)
            .await
            .expect("read the append payloads");
    for payload in &payloads {
        for sensitive in [
            "full jitter",
            "Read notes.txt",
            "Write that budget down",
            "Deploys go through",
        ] {
            assert!(
                !payload.contains(sensitive),
                "the chain is not the transcript store: {payload}"
            );
        }
    }
    let hashes = h.event_hashes().await;
    assert_eq!(hashes.len(), 6);
    assert!(
        hashes.iter().all(|(_, hash)| {
            hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        }),
        "each stored event carries the server's BLAKE3 digest: {hashes:?}"
    );
    // Terminal close freezes the complete eligible event set into one
    // restart-safe batch. It does not wait on a model and it does not publish
    // unreviewed output as Knowledge.
    let capture: (String, i32, i64) = sqlx::query_as(
        "select state, event_count, \
                (select count(*) from capture_candidates candidate \
                 where candidate.tenant_id = batch.tenant_id \
                   and candidate.batch_id = batch.id) \
         from capture_batches batch \
         where tenant_id = $1 and session_id = $2",
    )
    .bind(h.tenant_id.as_uuid())
    .bind(session_id)
    .fetch_one(&h.pool)
    .await
    .expect("terminal capture batch");
    assert_eq!(capture.0, "pending", "the hook never waits on extraction");
    assert_eq!(capture.1, 6, "the batch freezes every eligible event once");
    assert_eq!(capture.2, 0, "unprocessed output is not active Knowledge");

    // Performance evidence is emitted by this separately runnable target;
    // limits are the existing hook and route budgets, not values raised to
    // make this machine pass.
    let context_ms: Vec<u64> = h
        .logged("context.ok")
        .iter()
        .filter_map(|row| row["elapsed_ms"].as_u64())
        .collect();
    assert_eq!(context_ms.len(), 2, "both starts measured composition");
    assert!(
        first.elapsed < Duration::from_secs(8),
        "SessionStart: {:?}",
        first.elapsed
    );
    assert!(
        normal_stop.elapsed < Duration::from_secs(5),
        "Stop hook: {:?}",
        normal_stop.elapsed
    );
    assert!(
        ended.elapsed < Duration::from_secs(8),
        "SessionEnd: {:?}",
        ended.elapsed
    );
    assert!(
        recovered.elapsed < Duration::from_secs(8),
        "bounded backlog recovery: {:?}",
        recovered.elapsed
    );
    assert!(
        append_latency < Duration::from_secs(2),
        "append: {append_latency:?}"
    );
    assert!(
        context_ms.iter().all(|elapsed| *elapsed < 5_000),
        "context-run latency exceeded the configured request timeout: {context_ms:?}"
    );
    eprintln!(
        "CPR-14 measurements: session_start_ms={} stop_ms={} session_end_ms={} \
         append_ms={} context_run_ms={context_ms:?} backlog_recovery_ms={}",
        first.elapsed.as_millis(),
        normal_stop.elapsed.as_millis(),
        ended.elapsed.as_millis(),
        append_latency.as_millis(),
        recovered.elapsed.as_millis(),
    );

    h.cleanup();
}

/// Tier 3: the vendor executable, the marketplace installer it owns, and the
/// installed hooks/MCP entry. Never runs in ordinary CI: the wrapper refuses
/// with a named preflight result when Claude credentials are unavailable.
#[tokio::test]
#[ignore = "requires an authenticated Claude Code executable; run make claude-acceptance-live"]
async fn an_installed_claude_executable_completes_the_session_plane() {
    assert_eq!(
        std::env::var("SYNVEDA_CLAUDE_LIVE").as_deref(),
        Ok("1"),
        "run through make claude-acceptance-live"
    );
    let claude = find_program(
        std::env::var_os("SYNVEDA_CLAUDE_BIN")
            .as_deref()
            .unwrap_or_else(|| std::ffi::OsStr::new("claude")),
    )
    .expect("authenticated claude executable on PATH");
    let mut h = harness().await.expect("fresh current-epoch live harness");
    // Claude Code requires an actual UUID for --session-id.
    h.external_session_id = uuid::Uuid::new_v4().to_string();
    let root = repo_root();
    let cli = root.join("target/debug/synveda");
    assert!(
        cli.is_file(),
        "{} is missing; the live wrapper builds it",
        cli.display()
    );

    prepare_isolated_claude_config(&h);

    let version = live_command(&h, &claude, &["--version"], &h.project, &cli).await;
    assert!(
        version.status.success(),
        "claude --version: {}",
        version.stderr
    );
    let auth = live_command(&h, &claude, &["auth", "status"], &h.project, &cli).await;
    let environment_credential = [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
    ]
    .iter()
    .any(|key| std::env::var_os(key).is_some());
    assert!(
        (auth.status.success() && auth.stdout.contains("\"loggedIn\": true"))
            || environment_credential,
        "the isolated Claude configuration is not authenticated (status={}, stdout_bytes={}, stderr_bytes={})",
        auth.status,
        auth.stdout.len(),
        auth.stderr.len(),
    );

    // Package the checkout, then install it through synveda -> claude plugin.
    // The packager produces the release archive, so the extraction here is the
    // same boundary an installed release crosses.
    let package = h.home.join("package");
    std::fs::create_dir_all(&package).expect("package scratch");
    let package_text = package.to_string_lossy().into_owned();
    let package_script = root.join("scripts/package-plugin.sh");
    let packaged = live_command(
        &h,
        Path::new("/bin/bash"),
        &[
            package_script.to_string_lossy().as_ref(),
            "0.2.0",
            package_text.as_str(),
        ],
        &root,
        &cli,
    )
    .await;
    assert!(
        packaged.status.success(),
        "package the plugin: {}{}",
        packaged.stdout,
        packaged.stderr
    );
    let archive = package.join("synveda-plugin-0.2.0.tar.gz");
    let extracted = live_command(
        &h,
        Path::new("/usr/bin/tar"),
        &[
            "-xzf",
            archive.to_string_lossy().as_ref(),
            "-C",
            package_text.as_str(),
        ],
        &root,
        &cli,
    )
    .await;
    assert!(
        extracted.status.success(),
        "extract plugin: {}",
        extracted.stderr
    );
    let marketplace = package.join("plugin");
    let installed = live_command(
        &h,
        &cli,
        &[
            "plugin",
            "install",
            "--client",
            "claude-code",
            "--from",
            marketplace.to_string_lossy().as_ref(),
            "--scope",
            "user",
        ],
        &h.project,
        &cli,
    )
    .await;
    assert!(
        installed.status.success(),
        "install plugin: {}{}",
        installed.stdout,
        installed.stderr
    );

    let listed = live_command(&h, &claude, &["plugin", "list"], &h.project, &cli).await;
    assert!(
        listed.status.success()
            && listed.stdout.contains("synveda@synveda")
            && listed.stdout.to_ascii_lowercase().contains("enabled"),
        "Claude Code does not report the plugin enabled: {}{}",
        listed.stdout,
        listed.stderr
    );
    let details = live_command(
        &h,
        &claude,
        &["plugin", "details", "synveda@synveda"],
        &h.project,
        &cli,
    )
    .await;
    for component in [
        "SessionStart",
        "Stop",
        "PreCompact",
        "SessionEnd",
        "MCP servers (1)",
    ] {
        assert!(
            details.stdout.contains(component),
            "Claude Code's component inventory omitted {component}: {}{}",
            details.stdout,
            details.stderr
        );
    }

    std::fs::write(
        h.project.join("notes.txt"),
        "retry-budget: 3 attempts with full jitter\n",
    )
    .expect("live prompt fixture");
    let client_started = Instant::now();
    let client = live_command(
        &h,
        &claude,
        &[
            "-p",
            // `--allowedTools` is variadic, so the positional prompt must
            // precede it rather than be consumed as another tool name.
            "Use the Read tool exactly once to read the relative file notes.txt. Do not answer before the tool result. Then answer in one short sentence.",
            "--session-id",
            h.external_session_id.as_str(),
            "--output-format",
            "json",
            "--tools",
            "Read",
            "--allowedTools",
            "Read",
        ],
        &h.project,
        &cli,
    )
    .await;
    let client_elapsed = client_started.elapsed();
    assert!(
        client.status.success(),
        "real Claude Code session failed ({})",
        safe_claude_diagnostic(&client),
    );

    let deadline = Instant::now() + Duration::from_secs(8);
    let (session_id, status, end_reason, event_count) = loop {
        if let Some(row) = h.session_row().await
            && row.1 == "ended"
        {
            break row;
        }
        if Instant::now() >= deadline {
            let observed = h
                .session_row()
                .await
                .map(|(_, status, end_reason, events)| (status, end_reason.is_some(), events));
            panic!(
                "SessionEnd did not leave an ended session (observed={observed:?}, captured_hooks={:?}, adapter_log_events={:?})",
                h.captured_hook_names(),
                h.adapter_log_event_names(),
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(status, "ended");
    assert_eq!(
        end_reason.as_deref(),
        Some("other"),
        "normal headless completion has Claude Code's stable exit reason"
    );
    assert!(
        event_count >= 4,
        "user, tool call/result and assistant persisted"
    );
    let kinds: Vec<String> = h
        .events()
        .await
        .into_iter()
        .map(|(_, kind, _)| kind)
        .collect();
    for kind in [
        "message.user",
        "tool.invoked",
        "tool.result",
        "message.assistant",
    ] {
        assert!(
            kinds.iter().any(|stored| stored == kind),
            "{kind}: {kinds:?}"
        );
    }
    assert_eq!(
        kinds.len(),
        event_count as usize,
        "the event rows and session count agree"
    );
    let events = h.events().await;
    assert!(
        events
            .iter()
            .enumerate()
            .all(|(index, event)| event.0 == (index + 1) as i64),
        "server-assigned event sequence is contiguous: {events:?}"
    );
    let client_ids = events
        .iter()
        .map(|event| event.2.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        client_ids.len(),
        events.len(),
        "client event ids are unique"
    );
    let hashes = h.event_hashes().await;
    assert_eq!(hashes.len(), events.len());
    assert!(hashes.iter().all(|(_, hash)| {
        hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
    }));
    let context_runs: i64 = sqlx::query_scalar(
        "select count(*) from session_context_runs where tenant_id = $1 and session_id = $2",
    )
    .bind(h.tenant_id.as_uuid())
    .bind(session_id)
    .fetch_one(&h.pool)
    .await
    .expect("live context runs");
    assert_eq!(context_runs, 1, "SessionStart composed exactly once");
    let actions = h.chain_actions().await;
    for action in [
        "session.opened",
        "session.context.composed",
        "session.events.appended",
        "session.ended",
    ] {
        assert!(actions.iter().any(|stored| stored == action), "{action}");
    }
    let mut tx = synveda_store::rls::begin_tenant_tx(&h.pool, h.tenant_id)
        .await
        .expect("begin live audit verification");
    let verification = synveda_audit::verify(&mut tx, h.tenant_id)
        .await
        .expect("verify the live chain");
    tx.commit().await.expect("commit the live audit read");
    assert!(matches!(verification, ChainVerification::Valid { .. }));

    let (_, timeline) = h.get(&format!("/v1/sessions/{session_id}/timeline")).await;
    let timeline_text = timeline.to_string();
    for sensitive in ["retry-budget", "notes.txt", "Use the Read tool"] {
        assert!(
            !timeline_text.contains(sensitive),
            "the live timeline contains transcript content"
        );
    }
    let audit_payloads: Vec<String> =
        sqlx::query_scalar("select payload::text from audit_log where tenant_id = $1")
            .bind(h.tenant_id.as_uuid())
            .fetch_all(&h.pool)
            .await
            .expect("read live audit payloads");
    for payload in audit_payloads {
        for sensitive in ["retry-budget", "notes.txt", "Use the Read tool"] {
            assert!(
                !payload.contains(sensitive),
                "the live audit chain contains transcript content"
            );
        }
    }
    let completion_logs = h.logged("turn.done");
    assert!(
        completion_logs.iter().any(|row| {
            row["hook"] == "SessionEnd"
                && row["pending"] == json!(0)
                && row["complete"] == json!(true)
        }),
        "SessionEnd did not acknowledge and complete its spool: {completion_logs:?}"
    );
    assert!(
        h.spool().is_none(),
        "a fully acknowledged, closed live spool is retired"
    );
    let hook_durations = h.logged("hook.done");
    let hook_ms = |hook: &str, mode: &str| {
        hook_durations
            .iter()
            .find(|row| row["hook"] == hook && row["mode"] == mode)
            .and_then(|row| row["elapsed_ms"].as_u64())
            .unwrap_or_else(|| panic!("missing {hook}/{mode} duration in {hook_durations:?}"))
    };
    let session_start_ms = hook_ms("SessionStart", "start");
    let stop_ms = hook_ms("Stop", "turn");
    let session_end_ms = hook_ms("SessionEnd", "turn");
    let append_ms = h
        .logged("deliver.batch")
        .into_iter()
        .find(|row| row["ok"] == json!(true) && row["events"].as_u64().unwrap_or(0) > 0)
        .and_then(|row| row["elapsed_ms"].as_u64())
        .expect("successful live append duration");
    let context_run_ms = h
        .logged("context.ok")
        .into_iter()
        .find_map(|row| row["elapsed_ms"].as_u64())
        .expect("successful live context-run duration");
    assert!(
        session_start_ms < 8_000,
        "SessionStart: {session_start_ms}ms"
    );
    assert!(stop_ms < 5_000, "Stop: {stop_ms}ms");
    assert!(session_end_ms < 8_000, "SessionEnd: {session_end_ms}ms");
    assert!(append_ms < 5_000, "append request: {append_ms}ms");
    assert!(context_run_ms < 5_000, "context run: {context_run_ms}ms");
    let captures = std::fs::read_dir(h.home.join("captures"))
        .expect("captured real hook frames")
        .filter_map(Result::ok)
        .count();
    assert!(
        captures >= 3,
        "SessionStart, activity and SessionEnd frames captured"
    );
    let captured_hooks = h.captured_hook_names();
    for hook in ["SessionStart", "Stop", "SessionEnd"] {
        assert!(
            captured_hooks.iter().any(|captured| captured == hook),
            "Claude Code did not emit {hook}: {captured_hooks:?}"
        );
    }

    let report_dir = root.join("target/cpr14-live");
    std::fs::create_dir_all(&report_dir).expect("live report directory");
    std::fs::write(
        report_dir.join("last-run.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&json!({
                "mode": "live",
                "claude_code": version.stdout.trim(),
                "plugin": "synveda@synveda 0.2.0 enabled",
                "synveda": env!("CARGO_PKG_VERSION"),
                "os": host_version(),
                "session_id": session_id,
                "events": event_count,
                "context_runs": context_runs,
                "captured_hook_frames": captures,
                "client_duration_ms": client_elapsed.as_millis(),
                "session_start_ms": session_start_ms,
                "stop_ms": stop_ms,
                "session_end_ms": session_end_ms,
                "append_ms": append_ms,
                "context_run_ms": context_run_ms,
            }))
            .expect("live report")
        ),
    )
    .expect("persist non-sensitive live report");
    eprintln!(
        "CPR-14 live: claude={} plugin=0.2.0 events={} context_runs={} client_ms={} session_start_ms={} stop_ms={} session_end_ms={} append_ms={} context_run_ms={}",
        version.stdout.trim(),
        event_count,
        context_runs,
        client_elapsed.as_millis(),
        session_start_ms,
        stop_ms,
        session_end_ms,
        append_ms,
        context_run_ms,
    );
    h.cleanup();
}

struct CommandOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

fn safe_diagnostic_name(name: &str) -> String {
    if name.len() <= 96
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.:-".contains(&byte))
    {
        name.to_owned()
    } else {
        "<redacted>".to_owned()
    }
}

/// Reports enough of Claude's single-result envelope to classify a client
/// failure without ever printing the result text, tool inputs, or hook frames.
fn safe_claude_diagnostic(output: &CommandOutput) -> String {
    let parsed = serde_json::from_str::<Value>(&output.stdout).ok();
    let field = |name: &str| parsed.as_ref().and_then(|value| value.get(name)).cloned();
    let safe_field = |name: &str| {
        field(name).map_or(Value::Null, |value| match value {
            Value::String(ref text)
                if text.len() <= 64
                    && text
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_.:-".contains(&byte)) =>
            {
                value
            }
            Value::Bool(_) | Value::Number(_) | Value::Null => value,
            _ => Value::String("<redacted>".to_owned()),
        })
    };
    let mut top_level_fields: Vec<_> = parsed
        .as_ref()
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|object| object.keys())
        .map(|name| {
            if name.len() <= 64
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_.:-".contains(&byte))
            {
                name.to_owned()
            } else {
                "<redacted>".to_owned()
            }
        })
        .collect();
    top_level_fields.sort();
    let permission_denials = parsed
        .as_ref()
        .and_then(|value| value.get("permission_denials"))
        .and_then(Value::as_array);
    let denial_tools: Vec<_> = permission_denials
        .into_iter()
        .flatten()
        .filter_map(|denial| denial.get("tool_name").and_then(Value::as_str))
        .map(|name| {
            if name.len() <= 96
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_.:-".contains(&byte))
            {
                name.to_owned()
            } else {
                "<redacted>".to_owned()
            }
        })
        .collect();
    let denial_count = permission_denials.map_or(0, Vec::len);
    let error_count = parsed
        .as_ref()
        .and_then(|value| value.get("errors"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let hook_error_count = parsed
        .as_ref()
        .and_then(|value| value.get("hook_errors"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    // Classify values, not the whole JSON encoding. Every result envelope has
    // a `permission_denials` field, so scanning field names labelled every
    // unrelated failure as a permission problem.
    let mut diagnostic_parts = vec![output.stderr.as_str()];
    if let Some(result) = parsed
        .as_ref()
        .and_then(|value| value.get("result"))
        .and_then(Value::as_str)
    {
        diagnostic_parts.push(result);
    } else if parsed.is_none() {
        diagnostic_parts.push(output.stdout.as_str());
    }
    let diagnostic_values = parsed
        .as_ref()
        .into_iter()
        .flat_map(|value| [value.get("errors"), value.get("permission_denials")])
        .flatten()
        .map(Value::to_string)
        .collect::<Vec<_>>();
    diagnostic_parts.extend(diagnostic_values.iter().map(String::as_str));
    let diagnostic_text = diagnostic_parts.join("\n").to_ascii_lowercase();
    let category = if [
        "authentication_failed",
        "authentication_error",
        "invalid authentication",
        "oauth token",
        "login expired",
        "failed to authenticate",
        "please run /login",
        "claude_code_oauth_token",
        "oauth token revoked",
        "invalid authorization",
    ]
    .iter()
    .any(|needle| diagnostic_text.contains(needle))
    {
        "authentication"
    } else if [
        "session limit",
        "weekly limit",
        "credit balance",
        "rate_limit",
        "rate limit",
        "429",
    ]
    .iter()
    .any(|needle| diagnostic_text.contains(needle))
    {
        "usage_limit"
    } else if ["overloaded", "529", "internal server error"]
        .iter()
        .any(|needle| diagnostic_text.contains(needle))
    {
        "service"
    } else if diagnostic_text.contains("api error") {
        "api"
    } else if [
        "unable to connect",
        "connection refused",
        "network",
        "tls certificate",
    ]
    .iter()
    .any(|needle| diagnostic_text.contains(needle))
    {
        "network"
    } else if denial_count > 0
        || ["permission denied", "bypasspermissions", "root/sudo"]
            .iter()
            .any(|needle| diagnostic_text.contains(needle))
    {
        "permission"
    } else if ["workspace", "trust"]
        .iter()
        .any(|needle| diagnostic_text.contains(needle))
    {
        "workspace_trust"
    } else if ["hook", "plugin", "mcp server"]
        .iter()
        .any(|needle| diagnostic_text.contains(needle))
    {
        "integration"
    } else if ["stdin", "positional argument", "output-format", "--print"]
        .iter()
        .any(|needle| diagnostic_text.contains(needle))
    {
        "invocation"
    } else if ["current directory", "no such file", "not found"]
        .iter()
        .any(|needle| diagnostic_text.contains(needle))
    {
        "filesystem"
    } else {
        "unknown"
    };
    let stdout_hash = Sha256::digest(output.stdout.as_bytes());
    let stderr_hash = Sha256::digest(output.stderr.as_bytes());
    format!(
        "status={}, category={}, stdout_bytes={}, stderr_bytes={}, stdout_sha256={stdout_hash:x}, stderr_sha256={stderr_hash:x}, fields={top_level_fields:?}, type={}, subtype={}, is_error={}, num_turns={}, stop_reason={}, terminal_reason={}, api_error_status={}, prevented_continuation={}, hook_errors={hook_error_count}, result_bytes={}, duration_ms={}, permission_denials={denial_count}, denial_tools={denial_tools:?}, errors={error_count}",
        output.status,
        category,
        output.stdout.len(),
        output.stderr.len(),
        safe_field("type"),
        safe_field("subtype"),
        safe_field("is_error"),
        safe_field("num_turns"),
        safe_field("stop_reason"),
        safe_field("terminal_reason"),
        safe_field("api_error_status"),
        safe_field("prevented_continuation"),
        field("result")
            .and_then(|value| value.as_str().map(str::len))
            .unwrap_or(0),
        safe_field("duration_ms"),
    )
}

#[cfg(unix)]
#[test]
fn claude_failure_diagnostics_classify_without_disclosing_result_text() {
    use std::os::unix::process::ExitStatusExt as _;

    let secret = "private-model-result-must-not-be-logged";
    let output = CommandOutput {
        status: std::process::ExitStatus::from_raw(256),
        stdout: json!({
            "type": "result",
            "subtype": "success",
            "is_error": true,
            "num_turns": 1,
            "result": format!(
                "CLAUDE_CODE_OAUTH_TOKEN has invalid authorization: {secret}"
            ),
            "permission_denials": [{"tool_name": "Read", "tool_input": {"path": secret}}],
            "errors": []
        })
        .to_string(),
        stderr: String::new(),
    };

    let diagnostic = safe_claude_diagnostic(&output);
    assert!(diagnostic.contains("category=authentication"));
    assert!(diagnostic.contains("permission_denials=1"));
    assert!(diagnostic.contains("denial_tools=[\"Read\"]"));
    assert!(!diagnostic.contains(secret));
    assert!(!diagnostic.contains("invalid authorization:"));
}

async fn live_command(
    harness: &Harness,
    program: &Path,
    args: &[&str],
    cwd: &Path,
    cli: &Path,
) -> CommandOutput {
    let mut command = tokio::process::Command::new(program);
    command.args(args).current_dir(cwd).env_clear();
    for key in [
        "PATH",
        "TMPDIR",
        "LANG",
        "LC_ALL",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "NO_PROXY",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .env("HOME", &harness.home)
        .env("CLAUDE_CONFIG_DIR", harness.home.join(".claude-config"))
        .env("XDG_CONFIG_HOME", harness.home.join(".config"))
        .env("XDG_STATE_HOME", harness.home.join(".local/state"))
        .env("SYNVEDA_GATEWAY", harness.gateway.url())
        .env("SYNVEDA_TOKEN", &harness.token)
        .env("SYNVEDA_WORKSPACE", &harness.workspace_id)
        .env("SYNVEDA_PROJECT", &harness.project_id)
        .env("SYNVEDA_CLI", cli)
        .env("SYNVEDA_CAPTURE_DIR", harness.home.join("captures"))
        // Claude Code's overall SessionEnd budget defaults to 1.5 seconds and
        // plugin hook timeouts do not raise it. Use the acceptance gate's
        // existing eight-second ceiling so the adapter's three-second bounded
        // flush can finish rather than being killed by its host.
        .env("CLAUDE_CODE_SESSIONEND_HOOKS_TIMEOUT_MS", "8000");
    let output = command.output().await.expect("run live command");
    CommandOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn prepare_isolated_claude_config(harness: &Harness) {
    let target = harness.home.join(".claude-config");
    std::fs::create_dir_all(&target).expect("isolated Claude configuration");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700))
            .expect("private Claude configuration");
    }
    let Some(source) = std::env::var_os("SYNVEDA_CLAUDE_CREDENTIALS_FILE") else {
        return;
    };
    let source = PathBuf::from(source);
    assert!(
        source.is_file(),
        "the Claude credential handoff is not a file"
    );
    let copied = target.join(".credentials.json");
    std::fs::copy(source, &copied).expect("copy the credential into isolated configuration");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&copied, std::fs::Permissions::from_mode(0o600))
            .expect("private isolated credential");
    }
}

fn host_version() -> String {
    let uname = std::process::Command::new("uname")
        .args(["-srvmp"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH));
    if std::env::consts::OS != "macos" {
        return uname;
    }
    let sw_vers = std::process::Command::new("sw_vers")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .trim()
                .replace('\n', "; ")
        });
    sw_vers.map_or(uname.clone(), |version| format!("{version}; {uname}"))
}

fn find_program(program: &std::ffi::OsStr) -> Option<PathBuf> {
    let direct = PathBuf::from(program);
    if direct.components().count() > 1 {
        return direct.is_file().then_some(direct);
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|directory| directory.join(program))
        .find(|candidate| candidate.is_file())
}

/// The timeline the console renders, pinned.
///
/// The console cannot reach a database, so the only way its timeline test can
/// render *this* integration's output rather than a hand-written imitation is
/// for this run to publish what it received. Ids and instants are replaced by
/// placeholders — what is pinned is the shape and the summaries, which is what
/// the console reads.
fn check_timeline_golden(timeline: &Value, session_id: &str, workspace_id: &str) {
    let mut normalised = timeline.clone();
    normalise(&mut normalised, session_id, workspace_id);
    let rendered = format!(
        "{}\n",
        serde_json::to_string_pretty(&normalised).expect("render the golden")
    );
    if std::env::var("SYNVEDA_WRITE_CLAUDE_TIMELINE").is_ok() {
        std::fs::write(TIMELINE_GOLDEN, &rendered).expect("write the timeline golden");
        eprintln!("wrote {TIMELINE_GOLDEN}");
        return;
    }
    let committed = std::fs::read_to_string(TIMELINE_GOLDEN).unwrap_or_else(|err| {
        panic!(
            "{TIMELINE_GOLDEN} is missing ({err}). Regenerate it with \
             `SYNVEDA_WRITE_CLAUDE_TIMELINE=1 make claude-acceptance`."
        )
    });
    assert_eq!(
        committed, rendered,
        "the timeline this integration produces has changed shape. If that is \
         intended, regenerate the console's fixture with \
         `SYNVEDA_WRITE_CLAUDE_TIMELINE=1 make claude-acceptance`."
    );
}

/// Replaces everything a second run would spell differently.
fn normalise(value: &mut Value, session_id: &str, workspace_id: &str) {
    match value {
        Value::String(text) => {
            if text == session_id {
                *text = "SESSION".to_owned();
            } else if text == workspace_id {
                *text = "WORKSPACE".to_owned();
            } else if is_instant(text) {
                *text = "INSTANT".to_owned();
            } else if is_uuid(text) {
                *text = "ID".to_owned();
            } else if let Some(rest) = text.strip_prefix("cpr14-") {
                // The harness session id, and the workspace slug derived from it.
                let _ = rest;
                *text = "EXTERNAL".to_owned();
            }
        }
        Value::Array(items) => {
            for item in items {
                normalise(item, session_id, workspace_id);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                normalise(item, session_id, workspace_id);
            }
        }
        _ => {}
    }
}

fn is_instant(text: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(text).is_ok()
}

fn is_uuid(text: &str) -> bool {
    uuid::Uuid::parse_str(text).is_ok()
}
