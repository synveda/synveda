//! Supervised core worker process (CPR-45, ADR-0102).
//!
//! This process owns Capture extraction, immutable Knowledge indexing,
//! relaxation-expiry evidence and optional directory pull. Business state,
//! tenant identity and authorization remain in PostgreSQL/Cedar; the worker
//! reconstructs their normal tenant transaction for every unit of work.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use synveda_ingest::embedding::Embedder as _;
use synveda_ingest::extraction::Extractor as _;
use synveda_policy::Pdp;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinSet;

use crate::app::DirectoryRuntime;
use crate::authority::{AuthorityGate, AuthorityMonitor, CheckOutcome};
use crate::{
    authority, authz, directory_sync, knowledge_index, relaxations, runtime_config, shutdown,
};

const STARTING: u8 = 0;
const RUNNING: u8 = 1;
const DRAINING: u8 = 2;
const FAULTED: u8 = 3;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
const HEARTBEAT_STALE_AFTER: Duration = Duration::from_secs(5);
const CONVERGENCE_RETRY_INTERVAL: Duration = Duration::from_secs(2);
const ABORT_JOIN_RESERVE: Duration = Duration::from_secs(1);

/// Initializes telemetry and runs the supervised worker until process
/// shutdown.
pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let telemetry = crate::telemetry::init("synveda-worker")?;
    let metrics = match crate::telemetry::init_metrics() {
        Ok(metrics) => metrics,
        Err(error) => {
            telemetry.shutdown();
            return Err(error.into());
        }
    };
    let result = run_process(metrics).await;
    telemetry.shutdown();
    result
}

async fn run_process(
    metrics: metrics_exporter_prometheus::PrometheusHandle,
) -> Result<(), Box<dyn std::error::Error>> {
    let database_url = runtime_config::required_setting("DATABASE_URL")?;
    let max_connections =
        runtime_config::positive_connection_limit("SYNVEDA_WORKER_DB_MAX_CONNECTIONS", 8)?;
    let database_roles = runtime_config::database_roles()?;
    let connect_options = synveda_store::database_url::parse("DATABASE_URL", &database_url)?
        .application_name("synveda-worker");
    let database_role = connect_options.get_username();
    if database_role != database_roles.worker() {
        return Err("the worker database authority was conclusively refused".into());
    }
    // Lazy connection lets the private health surface report an outage while
    // the process waits; no work starts before the boot sentinel succeeds.
    let (pool_options, pool_refusal) = runtime_config::runtime_pool_options(
        max_connections,
        Duration::from_secs(5),
        database_roles.worker().to_owned(),
    );
    let pool = pool_options.connect_lazy_with(connect_options);
    let pdp = Arc::new(Pdp::new()?);
    let extractor = Arc::new(runtime_config::extractor_from_env()?);
    let embedder = Arc::new(runtime_config::embedder_from_env()?);
    let keys = Arc::new(synveda_store::keys::KeyRing::new(
        runtime_config::kms_from_env()?,
    ));
    let capture_config = runtime_config::capture_config_from_env()?;
    let policy_refresh =
        runtime_config::bounded_duration_setting("SYNVEDA_POLICY_REFRESH_SECS", 5, 1, 3_600)?;
    let relaxation_interval =
        runtime_config::bounded_duration_setting("SYNVEDA_RELAXATION_SWEEP_SECS", 60, 1, 3_600)?;
    let shutdown_grace =
        runtime_config::bounded_duration_setting("SYNVEDA_WORKER_SHUTDOWN_SECS", 75, 3, 300)?;
    let knowledge_config = knowledge_config_from_env()?;
    let (directory_connectors, directory_config) = directory_config_from_env()?;
    if directory_config.is_some()
        && directory_connectors.is_empty()
        && matches!(keys.kms(), synveda_crypto::Kms::Disabled)
    {
        return Err("SYNVEDA_DIRECTORY_SYNC requires a configured KMS key when no issuer carries a static directory connector".into());
    }
    drop(directory_connectors);

    tracing::info!(
        extractor = extractor.method(),
        embedder = embedder.method(),
        embedding_model = embedder.model(),
        db.max_connections = max_connections,
        shutdown_grace_secs = shutdown_grace.as_secs(),
        "core worker configuration accepted"
    );

    let authority = AuthorityMonitor::new_worker(
        pool.clone(),
        pool_refusal,
        database_roles.worker().to_owned(),
        database_roles,
    );
    let gate = authority.gate();
    let health = WorkerHealth::new(gate.clone(), metrics);
    let listen_addr =
        std::env::var("SYNVEDA_WORKER_LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:8121".to_owned());
    let listen_addr = listen_addr
        .parse::<std::net::SocketAddr>()
        .map_err(|_| "SYNVEDA_WORKER_LISTEN_ADDR must be an IP socket address")?;
    if !listen_addr.ip().is_loopback() {
        return Err("SYNVEDA_WORKER_LISTEN_ADDR must use a loopback address".into());
    }
    let listener = tokio::net::TcpListener::bind(&listen_addr).await?;
    let (health_stop_tx, health_stop_rx) = oneshot::channel();
    let mut health_task = tokio::spawn(serve_health(listener, health.clone(), health_stop_rx));
    tracing::info!(addr = %listen_addr, "synveda-worker private health listening");

    let generation = GenerationRuntime {
        pool: pool.clone(),
        pdp,
        extractor,
        embedder,
        keys,
        capture_config,
        knowledge_config,
        policy_refresh,
        relaxation_interval,
        directory_config,
        drain_grace: shutdown_grace
            .saturating_sub(ABORT_JOIN_RESERVE)
            .max(Duration::from_millis(1)),
    };
    let (process_stop_tx, process_stop_rx) = watch::channel(false);
    let mut sentinel_task =
        tokio::spawn(authority::run_sentinel(authority, process_stop_rx.clone()));
    let mut pool_task = tokio::spawn(pool_monitor(
        pool.clone(),
        max_connections,
        process_stop_rx.clone(),
    ));
    let mut heartbeat_task = tokio::spawn(heartbeat(health.clone(), process_stop_rx.clone()));
    let signal = shutdown::signal();
    tokio::pin!(signal);
    let mut health_finished = false;
    let mut sentinel_finished = false;
    let mut pool_finished = false;
    let mut heartbeat_finished = false;
    let mut exit_error = None;
    let mut active_task = Some(tokio::spawn(run_authority_generation(
        generation.clone(),
        gate.clone(),
        health.clone(),
        process_stop_rx.clone(),
    )));

    loop {
        enum SupervisorEvent {
            Generation(Result<Result<GenerationEnd, String>, tokio::task::JoinError>),
            Signal,
            Health(Result<Result<(), std::io::Error>, tokio::task::JoinError>),
            Sentinel(Result<CheckOutcome, tokio::task::JoinError>),
            Pool,
            Heartbeat,
        }
        let event = tokio::select! {
            result = async {
                match active_task.as_mut() {
                    Some(task) => task.await,
                    None => std::future::pending().await,
                }
            } => {
                SupervisorEvent::Generation(result)
            },
            () = &mut signal => SupervisorEvent::Signal,
            result = &mut health_task => SupervisorEvent::Health(result),
            result = &mut sentinel_task => SupervisorEvent::Sentinel(result),
            _result = &mut pool_task => SupervisorEvent::Pool,
            _result = &mut heartbeat_task => SupervisorEvent::Heartbeat,
        };
        match event {
            SupervisorEvent::Generation(Ok(Ok(GenerationEnd::AuthorityClosed))) => {
                active_task = Some(tokio::spawn(run_authority_generation(
                    generation.clone(),
                    gate.clone(),
                    health.clone(),
                    process_stop_rx.clone(),
                )));
                continue;
            }
            SupervisorEvent::Generation(Ok(Ok(GenerationEnd::Shutdown))) => {
                active_task = None;
                break;
            }
            SupervisorEvent::Signal => {
                break;
            }
            SupervisorEvent::Generation(Ok(Ok(GenerationEnd::AuthorityRefused))) => {
                active_task = None;
                exit_error =
                    Some("the worker database authority was conclusively refused".to_owned());
                break;
            }
            SupervisorEvent::Generation(Ok(Err(reason))) => {
                active_task = None;
                health.fault();
                exit_error = Some(reason);
                break;
            }
            SupervisorEvent::Generation(Err(_)) => {
                active_task = None;
                health.fault();
                exit_error = Some("the worker generation supervisor task failed".to_owned());
                break;
            }
            SupervisorEvent::Health(result) => {
                health_finished = true;
                health.fault();
                exit_error = Some(health_exit_error(result).to_string());
                break;
            }
            SupervisorEvent::Sentinel(result) => {
                sentinel_finished = true;
                health.fault();
                exit_error = Some(match result {
                    Ok(CheckOutcome::Refused { .. }) => {
                        "the worker database authority was conclusively refused".to_owned()
                    }
                    Ok(_) => "the worker authority sentinel stopped unexpectedly".to_owned(),
                    Err(_) => "the worker authority sentinel task failed".to_owned(),
                });
                break;
            }
            SupervisorEvent::Pool => {
                pool_finished = true;
                health.fault();
                exit_error = Some("the worker pool monitor stopped unexpectedly".to_owned());
                break;
            }
            SupervisorEvent::Heartbeat => {
                heartbeat_finished = true;
                health.fault();
                exit_error = Some("the worker heartbeat stopped unexpectedly".to_owned());
                break;
            }
        }
    }

    health.begin_drain();
    let _ = process_stop_tx.send(true);
    let _ = health_stop_tx.send(());
    let cleanup_grace = shutdown_grace.saturating_sub(ABORT_JOIN_RESERVE);
    let cleanup = tokio::time::timeout(cleanup_grace, async {
        if let Some(task) = active_task.as_mut() {
            let result = task.await;
            if exit_error.is_none() {
                exit_error = generation_cleanup_error(result);
                if exit_error.is_some() {
                    health.fault();
                }
            }
            active_task = None;
        }
        if !sentinel_finished {
            let _ = (&mut sentinel_task).await;
            sentinel_finished = true;
        }
        if !pool_finished {
            let _ = (&mut pool_task).await;
            pool_finished = true;
        }
        if !heartbeat_finished {
            let _ = (&mut heartbeat_task).await;
            heartbeat_finished = true;
        }
        if !health_finished {
            let _ = (&mut health_task).await;
            health_finished = true;
        }
        pool.close().await;
    })
    .await;
    if cleanup.is_err() {
        sentinel_task.abort();
        if !pool_finished {
            pool_task.abort();
        }
        if !heartbeat_finished {
            heartbeat_task.abort();
        }
        health_task.abort();
        let active_task = active_task.take();
        if let Some(task) = active_task.as_ref() {
            task.abort();
        }
        let _ = tokio::time::timeout(ABORT_JOIN_RESERVE, async {
            if let Some(task) = active_task {
                let _ = task.await;
            }
            if !sentinel_finished {
                let _ = sentinel_task.await;
            }
            if !pool_finished {
                let _ = pool_task.await;
            }
            if !heartbeat_finished {
                let _ = heartbeat_task.await;
            }
            if !health_finished {
                let _ = health_task.await;
            }
        })
        .await;
        tracing::warn!(
            grace_secs = shutdown_grace.as_secs(),
            "worker shutdown reached its hard deadline; unfinished supervisors were cancelled and joined"
        );
        if exit_error.is_none() {
            exit_error = Some("worker shutdown required forced supervisor cancellation".to_owned());
            health.fault();
        }
    }

    if let Some(reason) = exit_error {
        return Err(reason.into());
    }
    tracing::info!("core worker stopped cleanly");
    Ok(())
}

fn generation_cleanup_error(
    result: Result<Result<GenerationEnd, String>, tokio::task::JoinError>,
) -> Option<String> {
    match result {
        Ok(Ok(GenerationEnd::Shutdown | GenerationEnd::AuthorityClosed)) => None,
        Ok(Ok(GenerationEnd::AuthorityRefused)) => {
            Some("the worker database authority was conclusively refused".to_owned())
        }
        Ok(Err(reason)) => Some(reason),
        Err(_) => Some("the worker generation supervisor task failed".to_owned()),
    }
}

#[derive(Clone)]
struct GenerationRuntime {
    pool: sqlx::PgPool,
    pdp: Arc<Pdp>,
    extractor: Arc<synveda_ingest::extraction::AnyExtractor>,
    embedder: Arc<synveda_ingest::embedding::AnyEmbedder>,
    keys: Arc<synveda_store::keys::KeyRing>,
    capture_config: synveda_ingest::capture_worker::Config,
    knowledge_config: knowledge_index::Config,
    policy_refresh: Duration,
    relaxation_interval: Duration,
    directory_config: Option<directory_sync::SyncConfig>,
    drain_grace: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GenerationEnd {
    AuthorityClosed,
    AuthorityRefused,
    Shutdown,
}

async fn run_authority_generation(
    runtime: GenerationRuntime,
    gate: AuthorityGate,
    health: WorkerHealth,
    mut shutdown: watch::Receiver<bool>,
) -> Result<GenerationEnd, String> {
    health.waiting();
    let generation = tokio::select! {
        biased;
        () = shutdown::requested(&mut shutdown) => return Ok(GenerationEnd::Shutdown),
        generation = gate.wait_until_open() => match generation {
            Ok(generation) => generation,
            Err(()) if gate.is_terminal() => return Ok(GenerationEnd::AuthorityRefused),
            Err(()) => return Err("the worker authority gate lost its sentinel".to_owned()),
        },
    };
    let mut permit = gate.permit();
    if gate.open_generation() != Some(generation) || !permit.is_open() {
        return Ok(if gate.is_terminal() {
            GenerationEnd::AuthorityRefused
        } else {
            GenerationEnd::AuthorityClosed
        });
    }

    loop {
        let convergence = tokio::time::timeout(
            authority::CHECK_TIMEOUT,
            authz::converge_packs_once(&runtime.pool, &runtime.pdp),
        );
        tokio::pin!(convergence);
        let converged = tokio::select! {
            biased;
            () = permit.revoked() => {
                return Ok(if gate.is_terminal() {
                    GenerationEnd::AuthorityRefused
                } else {
                    GenerationEnd::AuthorityClosed
                });
            }
            () = shutdown::requested(&mut shutdown) => return Ok(GenerationEnd::Shutdown),
            result = &mut convergence => matches!(result, Ok(Ok(()))),
        };
        if converged {
            break;
        }
        tracing::warn!("initial policy-pack convergence unavailable");
        tokio::select! {
            biased;
            () = permit.revoked() => {
                return Ok(if gate.is_terminal() {
                    GenerationEnd::AuthorityRefused
                } else {
                    GenerationEnd::AuthorityClosed
                });
            }
            () = shutdown::requested(&mut shutdown) => return Ok(GenerationEnd::Shutdown),
            () = tokio::time::sleep(CONVERGENCE_RETRY_INTERVAL) => {}
        }
    }
    if !permit.is_open() {
        return Ok(if gate.is_terminal() {
            GenerationEnd::AuthorityRefused
        } else {
            GenerationEnd::AuthorityClosed
        });
    }

    let directory_connectors = if runtime.directory_config.is_some() {
        directory_config_from_env()
            .map_err(|_| "worker directory configuration could not be rebuilt".to_owned())?
            .0
    } else {
        DirectoryConnectors::new()
    };
    let (work_stop_tx, work_stop_rx) = watch::channel(false);
    let mut tasks = JoinSet::new();
    spawn_governed_named(
        &mut tasks,
        "policy-refresh",
        gate.clone(),
        generation,
        authz::run_pack_refresher(
            runtime.pool.clone(),
            Arc::clone(&runtime.pdp),
            runtime.policy_refresh,
            work_stop_rx.clone(),
        ),
    );
    spawn_governed_named(
        &mut tasks,
        "capture",
        gate.clone(),
        generation,
        synveda_ingest::capture_worker::run(
            synveda_ingest::capture_worker::Deps {
                pool: runtime.pool.clone(),
                pdp: Arc::clone(&runtime.pdp),
                extractor: Arc::clone(&runtime.extractor),
            },
            runtime.capture_config.clone(),
            work_stop_rx.clone(),
        ),
    );
    spawn_governed_named(
        &mut tasks,
        "knowledge-index",
        gate.clone(),
        generation,
        knowledge_index::run(
            runtime.pool.clone(),
            Arc::clone(&runtime.embedder),
            runtime.knowledge_config.clone(),
            work_stop_rx.clone(),
        ),
    );
    spawn_governed_named(
        &mut tasks,
        "relaxation-expiry",
        gate.clone(),
        generation,
        relaxations::run_expiry_sweep(
            runtime.pool.clone(),
            runtime.relaxation_interval,
            work_stop_rx.clone(),
        ),
    );
    if let Some(config) = runtime.directory_config {
        spawn_governed_named(
            &mut tasks,
            "directory-sync",
            gate.clone(),
            generation,
            directory_sync::run(
                DirectoryRuntime::new(
                    runtime.pool.clone(),
                    Arc::clone(&runtime.pdp),
                    Arc::clone(&runtime.keys),
                ),
                directory_connectors,
                config,
                work_stop_rx.clone(),
            ),
        );
    }

    health.beat();
    health.running(generation);
    tracing::info!(authority.generation = generation, "core worker running");
    let end = tokio::select! {
        biased;
        () = permit.revoked() => {
            if gate.is_terminal() {
                GenerationEnd::AuthorityRefused
            } else {
                GenerationEnd::AuthorityClosed
            }
        }
        () = shutdown::requested(&mut shutdown) => GenerationEnd::Shutdown,
        result = tasks.join_next() => {
            health.fault();
            let reason = unexpected_task_exit(result);
            let _ = work_stop_tx.send(true);
            let _ = drain_tasks(&mut tasks, runtime.drain_grace).await;
            return Err(reason);
        }
    };
    health.waiting();
    let _ = work_stop_tx.send(true);
    finish_generation_drain(&mut tasks, runtime.drain_grace, end).await
}

async fn finish_generation_drain(
    tasks: &mut JoinSet<&'static str>,
    grace: Duration,
    end: GenerationEnd,
) -> Result<GenerationEnd, String> {
    match drain_tasks(tasks, grace)
        .await
        .map_err(|_| "worker generation drain failed".to_owned())?
    {
        DrainOutcome::Graceful => Ok(end),
        DrainOutcome::Forced => {
            tracing::warn!(
                grace_secs = grace.as_secs(),
                "worker generation drain reached its hard deadline"
            );
            Err("worker generation shutdown required forced task cancellation".to_owned())
        }
    }
}

#[cfg(test)]
fn spawn_named<F>(tasks: &mut JoinSet<&'static str>, name: &'static str, task: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    tasks.spawn(async move {
        task.await;
        name
    });
}

fn spawn_governed_named<F>(
    tasks: &mut JoinSet<&'static str>,
    name: &'static str,
    gate: AuthorityGate,
    generation: u64,
    task: F,
) where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let mut permit = gate.permit();
    tasks.spawn(async move {
        if permit.is_for(generation) {
            tokio::select! {
                biased;
                () = permit.revoked() => {}
                () = task => {}
            }
        }
        name
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DrainOutcome {
    Graceful,
    Forced,
}

async fn drain_tasks(
    tasks: &mut JoinSet<&'static str>,
    grace: Duration,
) -> Result<DrainOutcome, Box<dyn std::error::Error>> {
    let abort_reserve = grace.min(ABORT_JOIN_RESERVE);
    let graceful_grace = grace.saturating_sub(abort_reserve);
    let joined = tokio::time::timeout(graceful_grace, async {
        let mut first_error = None;
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(name) => tracing::debug!(task = name, "worker task drained"),
                Err(error) => {
                    first_error
                        .get_or_insert_with(|| format!("worker task failed during drain: {error}"));
                }
            }
        }
        first_error.map_or(Ok(DrainOutcome::Graceful), Err)
    })
    .await;
    match joined {
        Ok(result) => result.map_err(Into::into),
        Err(_) => {
            tasks.abort_all();
            tokio::time::timeout(abort_reserve, async {
                while tasks.join_next().await.is_some() {}
            })
            .await
            .map_err(|_| "worker tasks did not join before the hard deadline")?;
            Ok(DrainOutcome::Forced)
        }
    }
}

fn unexpected_task_exit(result: Option<Result<&'static str, tokio::task::JoinError>>) -> String {
    match result {
        Some(Ok(name)) => format!("critical worker task `{name}` returned unexpectedly"),
        Some(Err(error)) => format!("critical worker task failed: {error}"),
        None => "worker supervisor lost every critical task".to_owned(),
    }
}

fn health_exit_error(
    result: Result<Result<(), std::io::Error>, tokio::task::JoinError>,
) -> Box<dyn std::error::Error> {
    match result {
        Ok(Ok(())) => "worker health server returned unexpectedly".into(),
        Ok(Err(error)) => format!("worker health server failed: {error}").into(),
        Err(error) => format!("worker health server task failed: {error}").into(),
    }
}

fn knowledge_config_from_env() -> Result<knowledge_index::Config, String> {
    let defaults = knowledge_index::Config::default();
    Ok(knowledge_index::Config {
        poll_interval: runtime_config::bounded_u64_setting(
            "SYNVEDA_KNOWLEDGE_EMBED_POLL_MS",
            1,
            60_000,
        )?
        .map(Duration::from_millis)
        .unwrap_or(defaults.poll_interval),
        batch: runtime_config::bounded_u64_setting("SYNVEDA_KNOWLEDGE_EMBED_BATCH", 1, 512)?
            .map(|value| {
                i64::try_from(value)
                    .map_err(|_| "SYNVEDA_KNOWLEDGE_EMBED_BATCH exceeds i64".to_owned())
            })
            .transpose()?
            .unwrap_or(defaults.batch),
    })
}

type DirectoryConnectors = std::collections::HashMap<
    synveda_types::TenantId,
    Box<dyn synveda_identity::directory::DirectoryConnector>,
>;

fn directory_config_from_env()
-> Result<(DirectoryConnectors, Option<directory_sync::SyncConfig>), Box<dyn std::error::Error>> {
    let issuers = runtime_config::setting("SYNVEDA_OIDC_ISSUERS")?
        .filter(|value| !value.trim().is_empty())
        .map(|json| synveda_identity::parse_issuers(&json))
        .transpose()?;
    let connectors = issuers
        .as_deref()
        .map(runtime_config::build_directory_connectors)
        .transpose()?
        .unwrap_or_default();
    let forced = std::env::var("SYNVEDA_DIRECTORY_SYNC")
        .is_ok_and(|value| matches!(value.trim(), "on" | "true" | "1"));
    let config = if connectors.is_empty() && !forced {
        None
    } else {
        Some(runtime_config::directory_sync_config_from_env()?)
    };
    Ok((connectors, config))
}

#[derive(Clone)]
struct WorkerHealth {
    authority: AuthorityGate,
    metrics: metrics_exporter_prometheus::PrometheusHandle,
    lifecycle: Arc<AtomicU8>,
    active_generation: Arc<AtomicU64>,
    started: Instant,
    heartbeat_ms: Arc<AtomicU64>,
}

impl WorkerHealth {
    fn new(
        authority: AuthorityGate,
        metrics: metrics_exporter_prometheus::PrometheusHandle,
    ) -> Self {
        metrics::gauge!(crate::telemetry::WORKER_READY).set(0.0);
        let state = Self {
            authority,
            metrics,
            lifecycle: Arc::new(AtomicU8::new(STARTING)),
            active_generation: Arc::new(AtomicU64::new(0)),
            started: Instant::now(),
            heartbeat_ms: Arc::new(AtomicU64::new(0)),
        };
        state.beat();
        state
    }

    fn running(&self, generation: u64) {
        self.active_generation
            .store(generation.max(1), Ordering::Release);
        self.lifecycle.store(RUNNING, Ordering::Release);
        metrics::gauge!(crate::telemetry::WORKER_READY).set(1.0);
    }

    fn waiting(&self) {
        self.active_generation.store(0, Ordering::Release);
        self.lifecycle.store(STARTING, Ordering::Release);
        metrics::gauge!(crate::telemetry::WORKER_READY).set(0.0);
    }

    fn begin_drain(&self) {
        self.active_generation.store(0, Ordering::Release);
        self.lifecycle.store(DRAINING, Ordering::Release);
        metrics::gauge!(crate::telemetry::WORKER_READY).set(0.0);
        tracing::info!("worker readiness withdrawn; cancelling and joining supervised tasks");
    }

    fn fault(&self) {
        self.active_generation.store(0, Ordering::Release);
        self.lifecycle.store(FAULTED, Ordering::Release);
        metrics::gauge!(crate::telemetry::WORKER_READY).set(0.0);
    }

    fn beat(&self) {
        let elapsed = self.started.elapsed().as_millis();
        let elapsed = u64::try_from(elapsed).unwrap_or(u64::MAX);
        self.heartbeat_ms.store(elapsed, Ordering::Release);
        metrics::gauge!(crate::telemetry::WORKER_HEARTBEAT_AGE_SECONDS).set(0.0);
    }

    fn heartbeat_age(&self) -> Duration {
        let then = self.heartbeat_ms.load(Ordering::Acquire);
        let now = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Duration::from_millis(now.saturating_sub(then))
    }

    fn is_running(&self) -> bool {
        let generation = self.active_generation.load(Ordering::Acquire);
        self.lifecycle.load(Ordering::Acquire) == RUNNING
            && generation != 0
            && self.authority.open_generation() == Some(generation)
    }
}

async fn heartbeat(health: WorkerHealth, mut shutdown: watch::Receiver<bool>) {
    let mut ticker = tokio::time::interval(HEARTBEAT_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
            _ = ticker.tick() => health.beat(),
        }
    }
}

async fn pool_monitor(
    pool: sqlx::PgPool,
    max_connections: u32,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(5));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
            _ = ticker.tick() => {
                let (size, idle) = (pool.size(), pool.num_idle());
                if size >= max_connections && idle == 0 {
                    tracing::warn!(
                        size,
                        idle,
                        max = max_connections,
                        "worker database pool saturated"
                    );
                } else {
                    tracing::debug!(size, idle, max = max_connections, "worker database pool");
                }
            }
        }
    }
}

async fn serve_health(
    listener: tokio::net::TcpListener,
    state: WorkerHealth,
    stop: oneshot::Receiver<()>,
) -> Result<(), std::io::Error> {
    let router = Router::new()
        .route("/healthz", get(worker_healthz))
        .route("/readyz", get(worker_readyz))
        .route("/metrics", get(worker_metrics))
        .with_state(state);
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = stop.await;
        })
        .await
}

async fn worker_healthz() -> &'static str {
    "ok"
}

async fn worker_metrics(State(state): State<WorkerHealth>) -> String {
    metrics::gauge!(crate::telemetry::WORKER_HEARTBEAT_AGE_SECONDS)
        .set(state.heartbeat_age().as_secs_f64());
    state.metrics.render()
}

async fn worker_readyz(State(state): State<WorkerHealth>) -> Response {
    if !state.is_running() || !state.authority.is_open() {
        metrics::gauge!(crate::telemetry::WORKER_READY).set(0.0);
        return (StatusCode::SERVICE_UNAVAILABLE, "not ready").into_response();
    }
    let heartbeat_age = state.heartbeat_age();
    metrics::gauge!(crate::telemetry::WORKER_HEARTBEAT_AGE_SECONDS)
        .set(heartbeat_age.as_secs_f64());
    if heartbeat_age > HEARTBEAT_STALE_AFTER {
        metrics::gauge!(crate::telemetry::WORKER_READY).set(0.0);
        return (StatusCode::SERVICE_UNAVAILABLE, "not ready").into_response();
    }
    metrics::gauge!(crate::telemetry::WORKER_READY).set(1.0);
    (StatusCode::OK, "ready").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_cleanup_preserves_generation_failures() {
        assert_eq!(
            generation_cleanup_error(Ok(Ok(GenerationEnd::Shutdown))),
            None
        );
        assert_eq!(
            generation_cleanup_error(Ok(Ok(GenerationEnd::AuthorityClosed))),
            None
        );
        assert_eq!(
            generation_cleanup_error(Ok(Ok(GenerationEnd::AuthorityRefused))),
            Some("the worker database authority was conclusively refused".to_owned())
        );
        assert_eq!(
            generation_cleanup_error(Ok(Err("drain failed".to_owned()))),
            Some("drain failed".to_owned())
        );
    }

    #[tokio::test]
    async fn draining_withdraws_readiness_before_a_current_unit_finishes() {
        let health = WorkerHealth::new(AuthorityGate::open_for_test(), test_metrics());
        health.beat();
        health.running(1);
        let (stop_tx, stop_rx) = watch::channel(false);
        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let mut tasks = JoinSet::new();
        spawn_named(&mut tasks, "current-unit", async move {
            let _ = started_tx.send(());
            let _ = release_rx.await;
            drop(stop_rx);
        });
        started_rx.await.expect("unit started");

        health.begin_drain();
        assert!(!health.is_running(), "readiness is withdrawn first");
        let _ = stop_tx.send(true);
        assert_eq!(tasks.len(), 1, "the current unit is still draining");
        let _ = release_tx.send(());
        drain_tasks(&mut tasks, Duration::from_secs(1))
            .await
            .expect("unit drains");
    }

    #[tokio::test]
    async fn a_stuck_unit_is_aborted_and_joined_at_the_hard_deadline() {
        let mut tasks = JoinSet::new();
        spawn_named(&mut tasks, "stuck", std::future::pending());
        let started = Instant::now();
        let result = drain_tasks(&mut tasks, Duration::from_millis(20)).await;
        assert_eq!(result.expect("bounded abort"), DrainOutcome::Forced);
        assert!(tasks.is_empty(), "aborted task was joined");
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn a_forced_generation_drain_is_bounded_and_fails_closed() {
        let mut tasks = JoinSet::new();
        spawn_named(&mut tasks, "non-cooperative", std::future::pending());
        let started = Instant::now();
        let error = finish_generation_drain(
            &mut tasks,
            Duration::from_millis(20),
            GenerationEnd::Shutdown,
        )
        .await
        .expect_err("forced generation cancellation must be a failed worker exit");
        assert_eq!(
            error,
            "worker generation shutdown required forced task cancellation"
        );
        assert!(
            tasks.is_empty(),
            "the forced task was joined before refusal"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    fn test_metrics() -> metrics_exporter_prometheus::PrometheusHandle {
        metrics_exporter_prometheus::PrometheusBuilder::new()
            .build_recorder()
            .handle()
    }
}
