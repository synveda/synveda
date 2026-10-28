//! Same-machine latency and isolation evidence for gateway admission.
//!
//! A measured tenant runs the representative Session workflow as four paced
//! agents (two principals, two sessions each; every iteration appends one
//! event and composes one ContextRun) while a second tenant floods the same
//! gateway from many clients that ignore `Retry-After`. Four phases repeat
//! that workload: quiet, noisy, noisy with a CPU-bound database and noisy with
//! a stalled embedding provider. The report gives p50/p95/p99 and status
//! counts for the measured tenant, the noisy tenant's admitted and refused
//! counts, sampled pool pressure and remembered identities. With enforced
//! limits it asserts the measured tenant was never refused or failed; the
//! numbers are evidence for a before/after comparison, not a service
//! objective.
//!
//! Run it explicitly with `--ignored`. `SYNVEDA_ADMISSION_LOAD_SECS` sets
//! each phase's duration, `SYNVEDA_ADMISSION_LOAD_NOISY` the noisy client
//! count, `SYNVEDA_ADMISSION_LOAD_PERIOD_MS` each measured agent's iteration
//! period, `SYNVEDA_ADMISSION_LOAD_MODE` `enforce` (default) or `observe`, and
//! `SYNVEDA_ADMISSION_LOAD_REPORT` a path for the machine-readable report.

#[path = "support/context_world.rs"]
mod context_world;
use context_world::*;

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use synveda_gateway::app::behavior_test_router_with_limits;
use synveda_gateway::limits::{Limiter, Limits, Mode};

fn setting(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn measured_limiter() -> Limiter {
    let mut limits = Limits::default();
    if std::env::var("SYNVEDA_ADMISSION_LOAD_MODE").as_deref() == Ok("observe") {
        limits.mode = Mode::Observe;
    }
    Limiter::new(limits).expect("default admission limits")
}

fn measured_router(state: AppState, limiter: &Limiter) -> Router {
    behavior_test_router_with_limits(state, limiter.clone())
}

fn enforcing(limiter: &Limiter) -> bool {
    limiter.limits().mode == Mode::Enforce
}

fn remembered(limiter: &Limiter) -> Value {
    let (principals, tenants, sources) = limiter.remembered();
    json!({"principals": principals, "tenants": tenants, "sources": sources})
}

fn quantiles(mut values: Vec<Duration>) -> Value {
    if values.is_empty() {
        return json!({"n": 0});
    }
    values.sort_unstable();
    let at = |quantile: f64| {
        let index = ((values.len() as f64) * quantile).ceil() as usize;
        values[index.saturating_sub(1).min(values.len() - 1)].as_secs_f64() * 1_000.0
    };
    json!({
        "n": values.len(),
        "p50_ms": at(0.50),
        "p95_ms": at(0.95),
        "p99_ms": at(0.99),
        "max_ms": values[values.len() - 1].as_secs_f64() * 1_000.0,
    })
}

#[derive(Default)]
struct Tally {
    statuses: Mutex<BTreeMap<u16, u64>>,
    appends: Mutex<Vec<Duration>>,
    composes: Mutex<Vec<Duration>>,
}

impl Tally {
    fn record(&self, status: StatusCode) {
        *self
            .statuses
            .lock()
            .expect("tally lock")
            .entry(status.as_u16())
            .or_default() += 1;
    }

    fn snapshot(&self) -> BTreeMap<u16, u64> {
        self.statuses.lock().expect("tally lock").clone()
    }
}

#[derive(Clone)]
struct Agent {
    token: String,
    session: String,
}

struct Tenant {
    token: String,
    project: String,
    workspace: String,
}

async fn open_session(app: &Router, tenant: &Tenant, token: &str, key: &str) -> String {
    let (status, session) = call(
        app,
        "POST",
        "/v1/sessions",
        token,
        Some(key),
        Some(json!({
            "workspace_id": tenant.workspace,
            "project_id": tenant.project,
            "client_name": "admission-load",
            "external_session_id": key,
            "task_summary": "Measured admission workload"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{session}");
    session["id"].as_str().expect("session id").to_owned()
}

async fn append(app: &Router, token: &str, session: &str, id: &str) -> StatusCode {
    let body = json!({"events": [{
        "event_type": "message.assistant",
        "client_event_id": id,
        "occurred_at": "2026-10-27T12:00:00Z",
        "payload": {"text": "A measured assistant turn about the iris-41 release checklist."}
    }]});
    call(
        app,
        "POST",
        &format!("/v1/sessions/{session}/events"),
        token,
        None,
        Some(body),
    )
    .await
    .0
}

async fn compose(app: &Router, token: &str, session: &str, key: &str) -> StatusCode {
    call(
        app,
        "POST",
        &format!("/v1/sessions/{session}/context-runs"),
        token,
        Some(key),
        Some(json!({"query": "correlation header convention", "budget_tokens": 1500})),
    )
    .await
    .0
}

/// Accepts connections and never answers, so every embedding attempt waits
/// for the gateway's own context-embedding deadline.
async fn stalled_provider() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stalled provider");
    let address = listener.local_addr().expect("stalled provider address");
    let task = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            if held.len() < 4_096 {
                held.push(socket);
            }
        }
    });
    (format!("http://{address}"), task)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Quiet,
    Noisy,
    NoisyDatabase,
    NoisyProvider,
}

impl Phase {
    const fn name(self) -> &'static str {
        match self {
            Self::Quiet => "quiet",
            Self::Noisy => "noisy",
            Self::NoisyDatabase => "noisy_database_cpu",
            Self::NoisyProvider => "noisy_provider_stall",
        }
    }
}

struct Phases {
    normal: Router,
    stalled: Router,
    limiter: Limiter,
    pool: sqlx::PgPool,
    burner: sqlx::PgPool,
    measured: Vec<Agent>,
    noisy: Vec<Agent>,
    duration: Duration,
    period: Duration,
}

fn flood(
    app: &Router,
    agents: &[Agent],
    ordinal: usize,
    stop: &Arc<AtomicBool>,
) -> (Arc<Tally>, Arc<AtomicU64>, Vec<tokio::task::JoinHandle<()>>) {
    let tally = Arc::new(Tally::default());
    let admitted = Arc::new(AtomicU64::new(0));
    let mut tasks = Vec::new();
    for (worker, agent) in agents.iter().cloned().enumerate() {
        let (app, stop, tally, admitted) = (
            app.clone(),
            Arc::clone(stop),
            Arc::clone(&tally),
            Arc::clone(&admitted),
        );
        tasks.push(tokio::spawn(async move {
            let mut sequence = 0_u64;
            while !stop.load(Ordering::Relaxed) {
                sequence += 1;
                let id = format!("noisy-{ordinal}-{worker}-{sequence}");
                let status = if sequence.is_multiple_of(2) {
                    append(&app, &agent.token, &agent.session, &id).await
                } else {
                    compose(&app, &agent.token, &agent.session, &id).await
                };
                tally.record(status);
                if status.is_success() {
                    admitted.fetch_add(1, Ordering::Relaxed);
                }
            }
        }));
    }
    (tally, admitted, tasks)
}

fn burn(burner: &sqlx::PgPool, stop: &Arc<AtomicBool>) -> Vec<tokio::task::JoinHandle<()>> {
    (0..4)
        .map(|_| {
            let (burner, stop) = (burner.clone(), Arc::clone(stop));
            tokio::spawn(async move {
                while !stop.load(Ordering::Relaxed) {
                    let _: Option<i64> =
                        sqlx::query_scalar("select count(*) from generate_series(1, 1500000)")
                            .fetch_one(&burner)
                            .await
                            .ok();
                }
            })
        })
        .collect()
}

/// Samples taken, samples with every connection checked out, and the most
/// connections seen in use.
type PoolSamples = Arc<Mutex<(u64, u64, u32)>>;

fn sample_pool(
    pool: &sqlx::PgPool,
    stop: &Arc<AtomicBool>,
) -> (PoolSamples, tokio::task::JoinHandle<()>) {
    let samples = Arc::new(Mutex::new((0_u64, 0_u64, 0_u32)));
    let (pool, stop, shared) = (pool.clone(), Arc::clone(stop), Arc::clone(&samples));
    let task = tokio::spawn(async move {
        while !stop.load(Ordering::Relaxed) {
            let in_use = pool.size().saturating_sub(pool.num_idle() as u32);
            let saturated = pool.num_idle() == 0 && pool.size() >= 8;
            {
                let mut guard = shared.lock().expect("pool sample lock");
                guard.0 += 1;
                guard.1 += u64::from(saturated);
                guard.2 = guard.2.max(in_use);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
    (samples, task)
}

async fn measure(app: &Router, agents: &[Agent], ordinal: usize, phases: &Phases) -> Arc<Tally> {
    let tally = Arc::new(Tally::default());
    let mut tasks = Vec::new();
    for (index, agent) in agents.iter().cloned().enumerate() {
        let (app, tally) = (app.clone(), Arc::clone(&tally));
        let (duration, period) = (phases.duration, phases.period);
        tasks.push(tokio::spawn(async move {
            let started = Instant::now();
            let mut ticker = tokio::time::interval(period);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut iteration = 0_u64;
            while started.elapsed() < duration {
                ticker.tick().await;
                iteration += 1;
                let id = format!("measured-{ordinal}-{index}-{iteration}");
                let at = Instant::now();
                let status = append(&app, &agent.token, &agent.session, &id).await;
                tally.appends.lock().expect("lock").push(at.elapsed());
                tally.record(status);
                let at = Instant::now();
                let status = compose(&app, &agent.token, &agent.session, &id).await;
                tally.composes.lock().expect("lock").push(at.elapsed());
                tally.record(status);
            }
        }));
    }
    for task in tasks {
        task.await.expect("measured agent joined");
    }
    tally
}

async fn run_phase(phases: &Phases, phase: Phase, ordinal: usize) -> Value {
    let app = if phase == Phase::NoisyProvider {
        phases.stalled.clone()
    } else {
        phases.normal.clone()
    };
    let stop = Arc::new(AtomicBool::new(false));
    let (noisy, admitted, mut background) = if phase == Phase::Quiet {
        (
            Arc::new(Tally::default()),
            Arc::new(AtomicU64::new(0)),
            Vec::new(),
        )
    } else {
        flood(&app, &phases.noisy, ordinal, &stop)
    };
    if phase == Phase::NoisyDatabase {
        background.extend(burn(&phases.burner, &stop));
    }
    let (pool_samples, sampler) = sample_pool(&phases.pool, &stop);
    let started = Instant::now();
    let measured = measure(&app, &phases.measured, ordinal, phases).await;
    let elapsed = started.elapsed().as_secs_f64();
    stop.store(true, Ordering::Relaxed);
    for task in background {
        task.await.expect("background worker joined");
    }
    sampler.await.expect("pool sampler joined");
    let (samples, saturated, max_in_use) = *pool_samples.lock().expect("pool sample lock");
    let appends = std::mem::take(&mut *measured.appends.lock().expect("lock"));
    let composes = std::mem::take(&mut *measured.composes.lock().expect("lock"));
    let report = json!({
        "phase": phase.name(),
        "seconds": elapsed,
        "measured": {
            "agents": phases.measured.len(),
            "append": quantiles(appends),
            "context_run": quantiles(composes),
            "statuses": measured.snapshot(),
        },
        "noisy": {
            "clients": phases.noisy.len(),
            "statuses": noisy.snapshot(),
            "admitted_per_second": admitted.load(Ordering::Relaxed) as f64 / elapsed,
        },
        "pool": {
            "max_connections": 8,
            "samples": samples,
            "saturated_fraction": if samples == 0 { 0.0 } else { saturated as f64 / samples as f64 },
            "max_in_use": max_in_use,
        },
        "remembered": remembered(&phases.limiter),
    });
    eprintln!("admission-load {report}");
    report
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "manual same-machine admission load and isolation measurement"]
async fn one_noisy_tenant_against_the_representative_workflow() {
    let _guard = serial().await;
    let Some(victim) = admitted_world().await else {
        return;
    };
    let Some(noisy) = admitted_world().await else {
        return;
    };
    corpus(&victim).await;
    corpus(&noisy).await;
    let duration = Duration::from_secs(setting("SYNVEDA_ADMISSION_LOAD_SECS", 8));
    let period = Duration::from_millis(setting("SYNVEDA_ADMISSION_LOAD_PERIOD_MS", 600));
    let clients = setting("SYNVEDA_ADMISSION_LOAD_NOISY", 24) as usize;

    let limiter = measured_limiter();
    let shared = state(&victim.database_url);
    let pool = shared.pool.clone();
    let normal = measured_router(shared.clone(), &limiter);
    let (provider, provider_task) = stalled_provider().await;
    let mut stalled_state = shared.clone();
    stalled_state.embedder = Arc::new(synveda_ingest::embedding::AnyEmbedder::Tei(
        synveda_ingest::embedding::TeiEmbedder::new("stalled".to_owned(), provider)
            .expect("stalled embedder"),
    ));
    let stalled = measured_router(stalled_state, &limiter);

    let victim_tenant = Tenant {
        token: victim.alice_token.clone(),
        project: victim.project_id.clone(),
        workspace: victim.workspace_id.clone(),
    };
    let mut measured = Vec::new();
    for (name, token) in [("alice", &victim.alice_token), ("bob", &victim.bob_token)] {
        for lane in 0..2 {
            let key = format!("admission-measured-{name}-{lane}");
            let session = open_session(&victim.app, &victim_tenant, token, &key).await;
            measured.push(Agent {
                token: token.clone(),
                session,
            });
        }
    }
    let noisy_tenant = Tenant {
        token: noisy.alice_token.clone(),
        project: noisy.project_id.clone(),
        workspace: noisy.workspace_id.clone(),
    };
    let mut flooders = Vec::with_capacity(clients);
    for client in 0..clients {
        let key = format!("admission-noisy-{client}");
        let session = open_session(&noisy.app, &noisy_tenant, &noisy_tenant.token, &key).await;
        flooders.push(Agent {
            token: noisy_tenant.token.clone(),
            session,
        });
    }
    let burner = PgPoolOptions::new()
        .max_connections(4)
        .connect(&victim.database_url)
        .await
        .expect("connect database load pool");
    let phases = Phases {
        normal,
        stalled,
        limiter: limiter.clone(),
        pool,
        burner,
        measured,
        noisy: flooders,
        duration,
        period,
    };

    let mut reports = Vec::new();
    for (ordinal, phase) in [
        Phase::Quiet,
        Phase::Noisy,
        Phase::NoisyDatabase,
        Phase::NoisyProvider,
    ]
    .into_iter()
    .enumerate()
    {
        reports.push(run_phase(&phases, phase, ordinal).await);
    }
    provider_task.abort();

    let report = json!({"enforced": enforcing(&limiter), "phases": reports});
    if let Ok(path) = std::env::var("SYNVEDA_ADMISSION_LOAD_REPORT") {
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&report).expect("render report"),
        )
        .expect("write admission load report");
    }
    if enforcing(&limiter) {
        for phase in report["phases"].as_array().expect("phases") {
            let statuses = phase["measured"]["statuses"]
                .as_object()
                .expect("measured statuses");
            assert!(
                statuses.keys().all(|status| status.starts_with('2')),
                "the measured tenant was refused or failed: {phase}"
            );
        }
    }
}
