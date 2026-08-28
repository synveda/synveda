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
use sqlx::postgres::PgPoolOptions;
use synveda_ingest::embedding::Embedder as _;
use synveda_ingest::extraction::Extractor as _;
use synveda_policy::Pdp;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinSet;

use crate::app::DirectoryRuntime;
use crate::{authz, directory_sync, knowledge_index, relaxations, runtime_config, shutdown};

const STARTING: u8 = 0;
const RUNNING: u8 = 1;
const DRAINING: u8 = 2;
const FAULTED: u8 = 3;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
const HEARTBEAT_STALE_AFTER: Duration = Duration::from_secs(5);
const AUTHORITY_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const READINESS_TIMEOUT: Duration = Duration::from_secs(2);
const BOOT_RETRY_INTERVAL: Duration = Duration::from_secs(2);

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
    let connect_options = synveda_store::database_url::parse("DATABASE_URL", &database_url)?
        .application_name("synveda-worker");
    let database_role = connect_options.get_username();
    let expected_database_role = runtime_config::setting("SYNVEDA_EXPECTED_DATABASE_ROLE")?
        .filter(|role| !role.is_empty())
        .unwrap_or_else(|| database_role.to_owned());
    if database_role != expected_database_role {
        return Err("DATABASE_URL login does not match SYNVEDA_EXPECTED_DATABASE_ROLE".into());
    }
    // Lazy connection lets the private health surface report an outage while
    // the process waits; no work starts before the boot sentinel succeeds.
    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(5))
        .connect_lazy_with(connect_options);

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
        runtime_config::bounded_duration_setting("SYNVEDA_WORKER_SHUTDOWN_SECS", 75, 1, 300)?;
    let knowledge_config = knowledge_config_from_env()?;
    let (directory_connectors, directory_config) = directory_config_from_env()?;
    if directory_config.is_some()
        && directory_connectors.is_empty()
        && matches!(keys.kms(), synveda_crypto::Kms::Disabled)
    {
        return Err("SYNVEDA_DIRECTORY_SYNC requires a configured KMS key when no issuer carries a static directory connector".into());
    }

    tracing::info!(
        extractor = extractor.method(),
        embedder = embedder.method(),
        embedding_model = embedder.model(),
        db.max_connections = max_connections,
        shutdown_grace_secs = shutdown_grace.as_secs(),
        "core worker configuration accepted"
    );

    let health = WorkerHealth::new(pool.clone(), expected_database_role.clone(), metrics);
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

    let signal = shutdown::signal();
    tokio::pin!(signal);
    let booted = {
        // Keep the database-convergence future in this block. On SIGTERM the
        // losing future must be dropped before `pool.close()`; retaining a
        // pinned, no-longer-polled acquire until function exit can otherwise
        // make shutdown wait on the very work it cancelled.
        let boot = wait_until_authoritative(&pool, &pdp, &expected_database_role);
        tokio::pin!(boot);
        tokio::select! {
            result = &mut boot => {
                result?;
                true
            }
            () = &mut signal => false,
            result = &mut health_task => {
                health.fault();
                return Err(health_exit_error(result));
            }
        }
    };

    if !booted {
        health.begin_drain();
        let _ = health_stop_tx.send(());
        finish_health(health_task).await?;
        pool.close().await;
        return Ok(());
    }

    let (work_stop_tx, work_stop_rx) = watch::channel(false);
    let mut tasks = JoinSet::new();

    spawn_named(
        &mut tasks,
        "heartbeat",
        heartbeat(health.clone(), work_stop_rx.clone()),
    );
    spawn_named(
        &mut tasks,
        "authority-sentinel",
        authority_sentinel(
            pool.clone(),
            expected_database_role.clone(),
            work_stop_rx.clone(),
        ),
    );
    spawn_named(
        &mut tasks,
        "pool-monitor",
        pool_monitor(pool.clone(), max_connections, work_stop_rx.clone()),
    );
    spawn_named(
        &mut tasks,
        "policy-refresh",
        authz::run_pack_refresher(
            pool.clone(),
            Arc::clone(&pdp),
            policy_refresh,
            work_stop_rx.clone(),
        ),
    );
    spawn_named(
        &mut tasks,
        "capture",
        synveda_ingest::capture_worker::run(
            synveda_ingest::capture_worker::Deps {
                pool: pool.clone(),
                pdp: Arc::clone(&pdp),
                extractor,
            },
            capture_config,
            work_stop_rx.clone(),
        ),
    );
    spawn_named(
        &mut tasks,
        "knowledge-index",
        knowledge_index::run(
            pool.clone(),
            embedder,
            knowledge_config,
            work_stop_rx.clone(),
        ),
    );
    spawn_named(
        &mut tasks,
        "relaxation-expiry",
        relaxations::run_expiry_sweep(pool.clone(), relaxation_interval, work_stop_rx.clone()),
    );
    if let Some(config) = directory_config {
        spawn_named(
            &mut tasks,
            "directory-sync",
            directory_sync::run(
                DirectoryRuntime::new(pool.clone(), Arc::clone(&pdp), keys),
                directory_connectors,
                config,
                work_stop_rx,
            ),
        );
    }

    health.beat();
    health.running();
    tracing::info!("core worker running");

    let mut health_finished = false;
    let unexpected = tokio::select! {
        () = &mut signal => None,
        result = tasks.join_next() => {
            health.fault();
            Some(unexpected_task_exit(result))
        }
        result = &mut health_task => {
            health.fault();
            health_finished = true;
            Some(health_exit_error(result).to_string())
        }
    };

    if unexpected.is_none() {
        health.begin_drain();
    }
    let _ = work_stop_tx.send(true);
    let drain = drain_tasks(&mut tasks, shutdown_grace).await;
    let _ = health_stop_tx.send(());
    let health_result = if health_finished {
        Ok(())
    } else {
        finish_health(health_task).await
    };
    pool.close().await;

    if let Some(reason) = unexpected {
        return Err(reason.into());
    }
    if drain? == DrainOutcome::Forced {
        tracing::warn!(
            grace_secs = shutdown_grace.as_secs(),
            "worker shutdown reached its hard deadline; unfinished work was cancelled and joined"
        );
    }
    health_result?;
    tracing::info!("core worker stopped cleanly");
    Ok(())
}

fn spawn_named<F>(tasks: &mut JoinSet<&'static str>, name: &'static str, task: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    tasks.spawn(async move {
        task.await;
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
    let joined = tokio::time::timeout(grace, async {
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
            while tasks.join_next().await.is_some() {}
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

async fn finish_health(
    task: tokio::task::JoinHandle<Result<(), std::io::Error>>,
) -> Result<(), Box<dyn std::error::Error>> {
    match task.await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(format!("worker health server failed: {error}").into()),
        Err(error) => Err(format!("worker health server task failed: {error}").into()),
    }
}

async fn wait_until_authoritative(
    pool: &sqlx::PgPool,
    pdp: &Pdp,
    expected_database_role: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        match synveda_store::epoch::verify(pool).await {
            Ok(metadata) => {
                match synveda_store::runtime_role::verify(pool, expected_database_role).await {
                    Ok(role) => match synveda_store::runtime_role::database_identity(pool).await {
                        Ok(_) => match authz::converge_packs_once(pool, pdp).await {
                            Ok(()) => {
                                tracing::info!(
                                    schema.epoch = metadata.epoch,
                                    schema.migration_head = %metadata.migration_head,
                                    db.role = role.name,
                                    "worker writable database, runtime role and initial policy packs accepted"
                                );
                                return Ok(());
                            }
                            Err(error) => {
                                tracing::warn!(%error, "initial policy-pack convergence unavailable")
                            }
                        },
                        Err(synveda_types::Error::Invalid { message }) => {
                            return Err(message.into());
                        }
                        Err(error) => {
                            tracing::warn!(%error, "worker writable-primary check unavailable")
                        }
                    },
                    Err(synveda_types::Error::Invalid { message }) => return Err(message.into()),
                    Err(error) => tracing::warn!(%error, "worker runtime-role check unavailable"),
                }
            }
            Err(error) if error.is_refusal() => return Err(error.to_string().into()),
            Err(error) => tracing::warn!(%error, "worker database/schema unavailable"),
        }
        tokio::time::sleep(BOOT_RETRY_INTERVAL).await;
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
    pool: sqlx::PgPool,
    expected_database_role: Arc<str>,
    metrics: metrics_exporter_prometheus::PrometheusHandle,
    lifecycle: Arc<AtomicU8>,
    started: Instant,
    heartbeat_ms: Arc<AtomicU64>,
}

impl WorkerHealth {
    fn new(
        pool: sqlx::PgPool,
        expected_database_role: String,
        metrics: metrics_exporter_prometheus::PrometheusHandle,
    ) -> Self {
        metrics::gauge!(crate::telemetry::WORKER_READY).set(0.0);
        let state = Self {
            pool,
            expected_database_role: expected_database_role.into(),
            metrics,
            lifecycle: Arc::new(AtomicU8::new(STARTING)),
            started: Instant::now(),
            heartbeat_ms: Arc::new(AtomicU64::new(0)),
        };
        state.beat();
        state
    }

    fn running(&self) {
        self.lifecycle.store(RUNNING, Ordering::Release);
        metrics::gauge!(crate::telemetry::WORKER_READY).set(1.0);
    }

    fn begin_drain(&self) {
        self.lifecycle.store(DRAINING, Ordering::Release);
        metrics::gauge!(crate::telemetry::WORKER_READY).set(0.0);
        tracing::info!("worker readiness withdrawn; cancelling and joining supervised tasks");
    }

    fn fault(&self) {
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
        self.lifecycle.load(Ordering::Acquire) == RUNNING
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

#[derive(Debug)]
enum AuthorityCheckError {
    Refusal(String),
    Unavailable(String),
}

async fn verify_authority_once(
    pool: &sqlx::PgPool,
    expected_database_role: &str,
) -> Result<(), AuthorityCheckError> {
    match synveda_store::epoch::verify(pool).await {
        Ok(_) => {}
        Err(error) if error.is_refusal() => {
            return Err(AuthorityCheckError::Refusal(error.to_string()));
        }
        Err(error) => return Err(AuthorityCheckError::Unavailable(error.to_string())),
    }
    match synveda_store::runtime_role::verify(pool, expected_database_role).await {
        Ok(_) => {}
        Err(synveda_types::Error::Storage { message }) => {
            return Err(AuthorityCheckError::Unavailable(message));
        }
        Err(error) => return Err(AuthorityCheckError::Refusal(error.to_string())),
    }
    match synveda_store::runtime_role::database_identity(pool).await {
        Ok(_) => Ok(()),
        Err(synveda_types::Error::Storage { message }) => {
            Err(AuthorityCheckError::Unavailable(message))
        }
        Err(error) => Err(AuthorityCheckError::Refusal(error.to_string())),
    }
}

/// Re-proves the immutable schema and runtime authority while work is live.
/// A database outage keeps the process unready and is retried; a conclusive
/// epoch or role refusal ends this critical task so the supervisor faults,
/// cancels every loop and exits non-zero.
async fn authority_sentinel(
    pool: sqlx::PgPool,
    expected_database_role: String,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut ticker = tokio::time::interval(AUTHORITY_CHECK_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
            _ = ticker.tick() => {
                match tokio::time::timeout(
                    READINESS_TIMEOUT,
                    verify_authority_once(&pool, &expected_database_role),
                )
                .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(AuthorityCheckError::Unavailable(reason))) => {
                        tracing::warn!(%reason, "worker authority sentinel unavailable");
                    }
                    Err(_) => {
                        tracing::warn!(
                            timeout_ms = READINESS_TIMEOUT.as_millis() as u64,
                            "worker authority sentinel timed out"
                        );
                    }
                    Ok(Err(AuthorityCheckError::Refusal(reason))) => {
                        tracing::error!(%reason, "worker authority sentinel refused runtime state");
                        return;
                    }
                }
            }
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
    if !state.is_running() {
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
    let ready = tokio::time::timeout(READINESS_TIMEOUT, async {
        synveda_store::runtime_role::verify(&state.pool, &state.expected_database_role).await?;
        synveda_store::runtime_role::database_identity(&state.pool).await?;
        synveda_store::epoch::verify(&state.pool)
            .await
            .map(|_| ())
            .map_err(|error| synveda_types::Error::Storage {
                message: error.to_string(),
            })
    })
    .await;
    match ready {
        Ok(Ok(())) => {
            metrics::gauge!(crate::telemetry::WORKER_READY).set(1.0);
            (StatusCode::OK, "ready").into_response()
        }
        Ok(Err(error)) => {
            metrics::gauge!(crate::telemetry::WORKER_READY).set(0.0);
            tracing::warn!(%error, "worker readiness dependency failed");
            (StatusCode::SERVICE_UNAVAILABLE, "not ready").into_response()
        }
        Err(_) => {
            metrics::gauge!(crate::telemetry::WORKER_READY).set(0.0);
            tracing::warn!(
                timeout_ms = READINESS_TIMEOUT.as_millis() as u64,
                "worker readiness timed out"
            );
            (StatusCode::SERVICE_UNAVAILABLE, "not ready").into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn draining_withdraws_readiness_before_a_current_unit_finishes() {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://invalid:invalid@127.0.0.1:1/invalid")
            .expect("lazy pool");
        let health = WorkerHealth::new(pool, "invalid".to_owned(), test_metrics());
        health.beat();
        health.running();
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

    fn test_metrics() -> metrics_exporter_prometheus::PrometheusHandle {
        metrics_exporter_prometheus::PrometheusBuilder::new()
            .build_recorder()
            .handle()
    }
}
