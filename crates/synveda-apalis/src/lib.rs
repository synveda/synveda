//! Experimental Apalis transport leaf for one Skill-validation operation.
//!
//! Synveda PostgreSQL remains authoritative for tenant identity, operation
//! state, retries, cancellation and the unique business effect. Apalis carries
//! only an untrusted tenant routing id, operation id and version; the worker
//! rechecks tenant association under forced RLS. The adapter can be removed
//! without changing the public API or domain crates.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use apalis::prelude::{
    BoxDynError, Data, Error as ApalisError, Monitor, Storage, WorkerBuilder, WorkerBuilderExt,
    WorkerFactoryFn,
};
use apalis_sql::Config;
use apalis_sql::postgres::PostgresStorage;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::get;
use axum::{Router, response::IntoResponse};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use synveda_gateway::authority::{AuthorityGate, AuthorityMonitor};
use synveda_gateway::skill_validation::{self, ExecuteOutcome};
use synveda_gateway::{authz, runtime_config, shutdown, telemetry};
use synveda_policy::Pdp;
use synveda_store::operations::{
    self as operations, DispatchAcknowledgement, DispatchDeferral, DispatchSelection,
};
use synveda_store::{rls, tenants};
use synveda_types::{DurableOperationId, TenantId};
use tokio::sync::{oneshot, watch};

const QUEUE_NAMESPACE: &str = "synveda.skill_validation.v1";
const DISPATCH_LEASE: Duration = Duration::from_secs(30);
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(60);
const DISPATCH_RETRY: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_secs(1);
const MONITOR_DRAIN_TIMEOUT: Duration = Duration::from_secs(25);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
const APALIS_TASKS_TOTAL: &str = "synveda_apalis_tasks_total";
const APALIS_DISPATCH_TOTAL: &str = "synveda_apalis_dispatch_total";
const APALIS_READY: &str = "synveda_apalis_worker_ready";
const QUEUE_HOST: &str = "apalis-postgres";
const QUEUE_PORT: u16 = 5432;
const QUEUE_DATABASE: &str = "synveda_apalis";
const QUEUE_OWNER: &str = "synveda_apalis_owner";
const QUEUE_RUNTIME: &str = "synveda_apalis";

const CONVERGE_RUNTIME_ROLE: &str = r#"
DO $synveda$
DECLARE
    runtime_credential text;
    granted_role text;
BEGIN
    SELECT secret
      INTO STRICT runtime_credential
      FROM pg_temp.synveda_apalis_runtime_credential;
    PERFORM pg_catalog.set_config('password_encryption', 'scram-sha-256', true);

    BEGIN
        IF EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'synveda_apalis') THEN
            EXECUTE format(
                'ALTER ROLE synveda_apalis WITH LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS PASSWORD %L VALID UNTIL ''infinity''',
                runtime_credential
            );
        ELSE
            EXECUTE format(
                'CREATE ROLE synveda_apalis WITH LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS PASSWORD %L VALID UNTIL ''infinity''',
                runtime_credential
            );
        END IF;
    EXCEPTION WHEN query_canceled OR assert_failure OR OTHERS THEN
        RAISE EXCEPTION USING
            ERRCODE = 'P0001',
            MESSAGE = 'Apalis runtime credential convergence was refused';
    END;

    FOR granted_role IN
        SELECT granted.rolname
          FROM pg_catalog.pg_auth_members membership
          JOIN pg_catalog.pg_roles member_role ON member_role.oid = membership.member
          JOIN pg_catalog.pg_roles granted ON granted.oid = membership.roleid
         WHERE member_role.rolname = 'synveda_apalis'
    LOOP
        EXECUTE format('REVOKE %I FROM synveda_apalis', granted_role);
    END LOOP;
END
$synveda$;
"#;

const CONVERGE_RUNTIME_ACL: &str = r#"
REVOKE ALL ON DATABASE synveda_apalis FROM PUBLIC;
REVOKE ALL ON DATABASE synveda_apalis FROM synveda_apalis;
GRANT CONNECT ON DATABASE synveda_apalis TO synveda_apalis;
REVOKE ALL ON DATABASE postgres, template1 FROM PUBLIC;
REVOKE ALL ON DATABASE postgres, template1 FROM synveda_apalis;

REVOKE ALL ON SCHEMA public FROM PUBLIC;
REVOKE ALL ON SCHEMA public FROM synveda_apalis;
REVOKE ALL ON SCHEMA apalis FROM PUBLIC;
REVOKE ALL ON SCHEMA apalis FROM synveda_apalis;
GRANT USAGE ON SCHEMA apalis TO synveda_apalis;

REVOKE ALL ON ALL TABLES IN SCHEMA apalis FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA apalis FROM synveda_apalis;
GRANT SELECT, INSERT, UPDATE ON TABLE apalis.jobs, apalis.workers TO synveda_apalis;

REVOKE ALL ON ALL FUNCTIONS IN SCHEMA apalis FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA apalis FROM synveda_apalis;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA public FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA public FROM synveda_apalis;
GRANT EXECUTE ON FUNCTION apalis.get_jobs(text, text, integer) TO synveda_apalis;

ALTER ROLE synveda_apalis RESET ALL;
ALTER ROLE synveda_apalis SET search_path = pg_catalog;
"#;

const VERIFY_RUNTIME_ACL: &str = r#"
SELECT
    current_user = 'synveda_apalis'
    AND EXISTS (
        SELECT 1
          FROM pg_catalog.pg_roles
         WHERE rolname = current_user
           AND NOT rolsuper
           AND NOT rolcreatedb
           AND NOT rolcreaterole
           AND NOT rolinherit
           AND NOT rolreplication
           AND NOT rolbypassrls
    )
    AND NOT EXISTS (
        SELECT 1
          FROM pg_catalog.pg_auth_members membership
          JOIN pg_catalog.pg_roles member_role ON member_role.oid = membership.member
         WHERE member_role.rolname = current_user
    )
    AND has_database_privilege(current_user, 'synveda_apalis', 'CONNECT')
    AND NOT has_database_privilege(current_user, 'synveda_apalis', 'TEMPORARY')
    AND NOT has_database_privilege(current_user, 'postgres', 'CONNECT')
    AND NOT has_database_privilege(current_user, 'template1', 'CONNECT')
    AND NOT has_schema_privilege(current_user, 'public', 'USAGE')
    AND has_schema_privilege(current_user, 'apalis', 'USAGE')
    AND NOT has_schema_privilege(current_user, 'apalis', 'CREATE')
    AND has_table_privilege(current_user, 'apalis.jobs', 'SELECT')
    AND has_table_privilege(current_user, 'apalis.jobs', 'INSERT')
    AND has_table_privilege(current_user, 'apalis.jobs', 'UPDATE')
    AND NOT has_table_privilege(current_user, 'apalis.jobs', 'DELETE,TRUNCATE,REFERENCES,TRIGGER')
    AND has_table_privilege(current_user, 'apalis.workers', 'SELECT')
    AND has_table_privilege(current_user, 'apalis.workers', 'INSERT')
    AND has_table_privilege(current_user, 'apalis.workers', 'UPDATE')
    AND NOT has_table_privilege(current_user, 'apalis.workers', 'DELETE,TRUNCATE,REFERENCES,TRIGGER')
    AND has_function_privilege(current_user, 'apalis.get_jobs(text,text,integer)', 'EXECUTE')
    AND NOT EXISTS (
        SELECT 1
          FROM pg_catalog.pg_proc procedure
          JOIN pg_catalog.pg_namespace namespace ON namespace.oid = procedure.pronamespace
         WHERE namespace.nspname = 'public'
           AND has_function_privilege(current_user, procedure.oid, 'EXECUTE')
    )
    AND NOT has_table_privilege(current_user, 'public._sqlx_migrations', 'SELECT')
"#;

#[derive(Clone, Copy, PartialEq, Eq)]
enum FinishedTask {
    Monitor,
    Dispatcher,
    Policy,
    Health,
    Sentinel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct SkillValidationTask {
    tenant_id: TenantId,
    operation_id: DurableOperationId,
    operation_version: u16,
}

#[derive(Clone)]
struct HandlerContext {
    product_pool: sqlx::PgPool,
    runtime: skill_validation::Runtime,
    authority: AuthorityGate,
}

#[derive(Clone)]
struct Health {
    authority: AuthorityGate,
    queue_pool: sqlx::PgPool,
    ready: Arc<AtomicBool>,
    metrics: metrics_exporter_prometheus::PrometheusHandle,
}

/// Initializes telemetry and runs the disabled-by-default experimental leaf.
pub async fn run_worker() -> Result<(), Box<dyn std::error::Error>> {
    let telemetry = telemetry::init("synveda-apalis-worker")?;
    let metrics = match telemetry::init_metrics() {
        Ok(metrics) => metrics,
        Err(error) => {
            telemetry.shutdown();
            return Err(error.into());
        }
    };
    describe_metrics();
    let result = run_process(metrics).await;
    telemetry.shutdown();
    result
}

/// Migrates the private Apalis transport database and converges its runtime role.
///
/// This one-shot command is the only process that accepts the queue owner
/// credential. The credential is never passed to the long-running worker.
pub async fn run_migrate() -> Result<(), Box<dyn std::error::Error>> {
    let owner_password = runtime_config::required_setting("SYNVEDA_APALIS_OWNER_PASSWORD")?;
    let runtime_password = runtime_config::required_setting("SYNVEDA_APALIS_DATABASE_PASSWORD")?;
    validate_queue_secret("SYNVEDA_APALIS_OWNER_PASSWORD", &owner_password)?;
    validate_queue_secret("SYNVEDA_APALIS_DATABASE_PASSWORD", &runtime_password)?;
    if owner_password == runtime_password {
        return Err("the Apalis owner and runtime credentials must differ".into());
    }

    let owner_pool =
        queue_pool_for(QUEUE_OWNER, &owner_password, "synveda-apalis-migrate", 1).await?;
    let owner_is_expected = sqlx::query_scalar::<_, bool>(
        "SELECT current_user = 'synveda_apalis_owner' \
                AND current_database() = 'synveda_apalis' \
                AND EXISTS (SELECT 1 FROM pg_catalog.pg_roles \
                            WHERE rolname = current_user AND rolcanlogin AND rolsuper)",
    )
    .fetch_one(&owner_pool)
    .await?;
    if !owner_is_expected {
        return Err("the Apalis migration database authority was refused".into());
    }
    PostgresStorage::setup(&owner_pool).await?;

    let mut tx = owner_pool.begin().await?;
    sqlx::query(
        "CREATE TEMP TABLE synveda_apalis_runtime_credential (secret text NOT NULL) ON COMMIT DROP",
    )
    .execute(&mut *tx)
    .await?;
    let mut copy = tx
        .copy_in_raw(
            "COPY pg_temp.synveda_apalis_runtime_credential (secret) FROM STDIN WITH (FORMAT text)",
        )
        .await?;
    copy.send(runtime_password.as_bytes()).await?;
    copy.send(&b"\n"[..]).await?;
    if copy.finish().await? != 1 {
        return Err("the Apalis runtime credential convergence input was refused".into());
    }
    sqlx::query(CONVERGE_RUNTIME_ROLE).execute(&mut *tx).await?;
    sqlx::raw_sql(CONVERGE_RUNTIME_ACL)
        .execute(&mut *tx)
        .await?;
    let credential_is_scram = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_authid \
                        WHERE rolname = 'synveda_apalis' \
                          AND rolpassword LIKE 'SCRAM-SHA-256$%' \
                          AND rolvaliduntil = 'infinity'::timestamptz)",
    )
    .fetch_one(&mut *tx)
    .await?;
    if !credential_is_scram {
        return Err("the Apalis runtime credential storage was refused".into());
    }
    tx.commit().await?;

    let runtime_pool = queue_pool_for(
        QUEUE_RUNTIME,
        &runtime_password,
        "synveda-apalis-preflight",
        1,
    )
    .await?;
    let valid = sqlx::query_scalar::<_, bool>(VERIFY_RUNTIME_ACL)
        .fetch_one(&runtime_pool)
        .await?;
    runtime_pool.close().await;
    owner_pool.close().await;
    if !valid {
        return Err("the Apalis runtime database authority was refused".into());
    }
    tracing::info!(
        queue = QUEUE_NAMESPACE,
        "experimental Apalis queue converged"
    );
    Ok(())
}

async fn run_process(
    metrics: metrics_exporter_prometheus::PrometheusHandle,
) -> Result<(), Box<dyn std::error::Error>> {
    let database_url = runtime_config::required_setting("DATABASE_URL")?;
    let database_roles = runtime_config::database_roles()?;
    let product_options = synveda_store::database_url::parse("DATABASE_URL", &database_url)?
        .application_name("synveda-apalis-worker");
    if product_options.get_username() != database_roles.worker() {
        return Err("the Apalis worker database authority was conclusively refused".into());
    }
    let max_connections =
        runtime_config::positive_connection_limit("SYNVEDA_APALIS_PRODUCT_DB_MAX_CONNECTIONS", 4)?;
    let (pool_options, refusal) = runtime_config::runtime_pool_options(
        max_connections,
        Duration::from_secs(5),
        database_roles.worker().to_owned(),
    );
    let product_pool = pool_options.connect_lazy_with(product_options);
    let authority = AuthorityMonitor::new_apalis_worker(
        product_pool.clone(),
        refusal,
        database_roles.worker().to_owned(),
        database_roles,
    );
    let gate = authority.gate();
    let (stop_tx, stop_rx) = watch::channel(false);
    let mut sentinel = tokio::spawn(synveda_gateway::authority::run_sentinel(
        authority,
        stop_rx.clone(),
    ));
    let signal = shutdown::signal();
    tokio::pin!(signal);
    tokio::select! {
        opened = gate.wait_until_open() => {
            opened.map_err(|()| "the Apalis worker database authority was refused")?;
        }
        () = &mut signal => {
            let _ = stop_tx.send(true);
            let _ = sentinel.await;
            product_pool.close().await;
            return Ok(());
        }
        result = &mut sentinel => {
            return Err(format!("the Apalis worker authority sentinel stopped during boot: {result:?}").into());
        }
    }

    let pdp = Arc::new(Pdp::new()?);
    authz::converge_packs_once(&product_pool, &pdp).await?;
    let queue_pool = runtime_queue_pool().await?;
    let queue_config = Config::new(QUEUE_NAMESPACE)
        .set_buffer_size(1)
        .set_keep_alive(Duration::from_secs(10))
        .set_reenqueue_orphaned_after(Duration::from_secs(45));
    let queue = PostgresStorage::<Value>::new_with_config(queue_pool.clone(), queue_config);

    let listen = listen_address()?;
    let listener = tokio::net::TcpListener::bind(listen).await?;
    let ready = Arc::new(AtomicBool::new(false));
    let health = Health {
        authority: gate.clone(),
        queue_pool: queue_pool.clone(),
        ready: Arc::clone(&ready),
        metrics,
    };
    let (health_stop_tx, health_stop_rx) = oneshot::channel();
    let mut health_task = tokio::spawn(serve_health(listener, health, health_stop_rx));

    let runtime = skill_validation::Runtime::new(product_pool.clone(), Arc::clone(&pdp))
        .with_worker_id(format!("apalis-{}", DurableOperationId::new()));
    let context = HandlerContext {
        product_pool: product_pool.clone(),
        runtime,
        authority: gate.clone(),
    };
    let worker = WorkerBuilder::new("synveda-skill-validation")
        .concurrency(1)
        .enable_tracing()
        .data(context)
        .backend(queue.clone())
        .build_fn(handle_task);
    let monitor = Monitor::new()
        .register(worker)
        .shutdown_timeout(MONITOR_DRAIN_TIMEOUT);
    let mut monitor_task = tokio::spawn(monitor.run_with_signal(stop_signal(stop_rx.clone())));
    let dispatcher_id = format!("apalis-dispatch-{}", DurableOperationId::new());
    let mut dispatcher_task = tokio::spawn(run_dispatcher(
        product_pool.clone(),
        queue,
        gate.clone(),
        stop_rx.clone(),
        dispatcher_id,
    ));
    let refresh_interval =
        runtime_config::bounded_duration_setting("SYNVEDA_POLICY_REFRESH_SECS", 5, 1, 3_600)?;
    let mut policy_task = tokio::spawn(authz::run_pack_refresher(
        product_pool.clone(),
        pdp,
        refresh_interval,
        stop_rx.clone(),
    ));
    ready.store(true, Ordering::Release);
    metrics::gauge!(APALIS_READY).set(1.0);
    tracing::info!(queue = QUEUE_NAMESPACE, "experimental Apalis worker ready");

    let (failure, finished) = tokio::select! {
        () = &mut signal => (None, None),
        () = gate.wait_until_closed() => (
            Some("product database authority closed".to_owned()),
            None,
        ),
        result = &mut monitor_task => (
            Some(format!("Apalis monitor stopped unexpectedly: {result:?}")),
            Some(FinishedTask::Monitor),
        ),
        result = &mut dispatcher_task => (
            Some(format!("Apalis dispatcher stopped unexpectedly: {result:?}")),
            Some(FinishedTask::Dispatcher),
        ),
        result = &mut policy_task => (
            Some(format!("policy refresher stopped unexpectedly: {result:?}")),
            Some(FinishedTask::Policy),
        ),
        result = &mut health_task => (
            Some(format!("Apalis health server stopped unexpectedly: {result:?}")),
            Some(FinishedTask::Health),
        ),
        result = &mut sentinel => (
            Some(format!("Apalis authority sentinel stopped unexpectedly: {result:?}")),
            Some(FinishedTask::Sentinel),
        ),
    };

    ready.store(false, Ordering::Release);
    metrics::gauge!(APALIS_READY).set(0.0);
    let _ = stop_tx.send(true);
    let _ = health_stop_tx.send(());
    let cleanup = tokio::time::timeout(SHUTDOWN_TIMEOUT, async {
        if finished != Some(FinishedTask::Monitor) {
            let _ = (&mut monitor_task).await;
        }
        if finished != Some(FinishedTask::Dispatcher) {
            let _ = (&mut dispatcher_task).await;
        }
        if finished != Some(FinishedTask::Policy) {
            let _ = (&mut policy_task).await;
        }
        if finished != Some(FinishedTask::Health) {
            let _ = (&mut health_task).await;
        }
        if finished != Some(FinishedTask::Sentinel) {
            let _ = (&mut sentinel).await;
        }
    })
    .await;
    if cleanup.is_err() {
        monitor_task.abort();
        dispatcher_task.abort();
        policy_task.abort();
        health_task.abort();
        sentinel.abort();
    }
    queue_pool.close().await;
    product_pool.close().await;
    match failure {
        Some(reason) => Err(reason.into()),
        None => Ok(()),
    }
}

async fn runtime_queue_pool() -> Result<sqlx::PgPool, Box<dyn std::error::Error>> {
    for forbidden in [
        "SYNVEDA_APALIS_OWNER_PASSWORD",
        "SYNVEDA_APALIS_OWNER_PASSWORD_FILE",
    ] {
        if std::env::var_os(forbidden).is_some() {
            return Err("the Apalis worker must not receive its queue owner credential".into());
        }
    }
    for unsupported in [
        "SYNVEDA_APALIS_DATABASE_HOST",
        "SYNVEDA_APALIS_DATABASE_PORT",
        "SYNVEDA_APALIS_DATABASE_NAME",
        "SYNVEDA_APALIS_DATABASE_USER",
    ] {
        if std::env::var_os(unsupported).is_some() {
            return Err("the experimental Apalis queue endpoint is fixed and private".into());
        }
    }
    let password = runtime_config::required_setting("SYNVEDA_APALIS_DATABASE_PASSWORD")?;
    validate_queue_secret("SYNVEDA_APALIS_DATABASE_PASSWORD", &password)?;
    queue_pool_for(QUEUE_RUNTIME, &password, "synveda-apalis-queue", 4).await
}

async fn queue_pool_for(
    user: &str,
    password: &str,
    application_name: &str,
    max_connections: u32,
) -> Result<sqlx::PgPool, Box<dyn std::error::Error>> {
    let options = PgConnectOptions::new()
        .host(QUEUE_HOST)
        .port(QUEUE_PORT)
        .database(QUEUE_DATABASE)
        .username(user)
        .password(password)
        .ssl_mode(PgSslMode::Disable)
        .application_name(application_name);
    Ok(PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(options)
        .await?)
}

fn validate_queue_secret(name: &str, value: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!(
            "{name} must contain exactly 64 lower-case hexadecimal characters"
        ));
    }
    Ok(())
}

fn setting_or(name: &str, default: &str) -> Result<String, String> {
    Ok(runtime_config::setting(name)?.unwrap_or_else(|| default.to_owned()))
}

async fn run_dispatcher(
    product_pool: sqlx::PgPool,
    mut queue: PostgresStorage<Value>,
    authority: AuthorityGate,
    mut shutdown: watch::Receiver<bool>,
    dispatcher_id: String,
) {
    let mut ticker = tokio::time::interval(POLL_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
            _ = ticker.tick() => {}
        }
        if *shutdown.borrow() || !authority.is_open() {
            return;
        }
        let active = match tenants::active(&product_pool).await {
            Ok(active) => active,
            Err(error) => {
                tracing::warn!(%error, "Apalis dispatcher tenant inventory failed");
                metrics::counter!(APALIS_DISPATCH_TOTAL, "outcome" => "tenant_inventory_error")
                    .increment(1);
                continue;
            }
        };
        for tenant in active {
            if *shutdown.borrow() || !authority.is_open() {
                return;
            }
            let mut permit = authority.permit();
            if !permit.is_open() {
                return;
            }
            let dispatch = dispatch_one(&product_pool, &mut queue, tenant.id, &dispatcher_id);
            tokio::pin!(dispatch);
            let result = tokio::select! {
                biased;
                () = permit.revoked() => return,
                result = &mut dispatch => result,
            };
            if let Err(error) = result {
                tracing::warn!(tenant.id = %tenant.id, %error, "Apalis dispatch failed");
                metrics::counter!(APALIS_DISPATCH_TOTAL, "outcome" => "error").increment(1);
            }
        }
    }
}

#[tracing::instrument(
    name = "apalis.dispatch",
    skip_all,
    fields(operation.kind = "skill_validation")
)]
async fn dispatch_one(
    pool: &sqlx::PgPool,
    queue: &mut PostgresStorage<Value>,
    tenant: TenantId,
    owner: &str,
) -> synveda_types::Result<()> {
    let mut tx = rls::begin_tenant_tx(pool, tenant).await?;
    let selected = operations::claim_dispatch(
        &mut tx,
        tenant,
        owner,
        seconds(DISPATCH_LEASE),
        seconds(DELIVERY_TIMEOUT),
    )
    .await?;
    let Some(selected) = selected else {
        tx.rollback().await.map_err(transaction_error)?;
        return Ok(());
    };
    let claim = match selected {
        DispatchSelection::DeadLettered(terminal) => {
            skill_validation::record_dispatch_dead_letter(&mut tx, tenant, terminal).await?;
            tx.commit().await.map_err(transaction_error)?;
            metrics::counter!(APALIS_DISPATCH_TOTAL, "outcome" => "dead_lettered").increment(1);
            return Ok(());
        }
        DispatchSelection::Claimed(claim) => claim,
    };
    tx.commit().await.map_err(transaction_error)?;
    let task = serde_json::json!({
        "tenant_id": claim.tenant_id,
        "operation_id": claim.operation_id,
        "operation_version": claim.operation_version,
    });
    let submission = queue.push(task).await.map(|_| ());
    let mut tx = rls::begin_tenant_tx(pool, tenant).await?;
    match submission {
        Ok(()) => {
            let outcome = operations::acknowledge_dispatch(&mut tx, &claim).await?;
            let label = match outcome {
                DispatchAcknowledgement::Acknowledged => "submitted",
                DispatchAcknowledgement::AlreadyCompleted => "already_completed",
                DispatchAcknowledgement::LostFence => "lost_fence",
            };
            metrics::counter!(APALIS_DISPATCH_TOTAL, "outcome" => label).increment(1);
        }
        Err(_) => {
            let outcome = operations::defer_dispatch(
                &mut tx,
                &claim,
                seconds(DISPATCH_RETRY),
                "queue_unavailable",
            )
            .await?;
            let label = match outcome {
                DispatchDeferral::RetryScheduled => "retry_scheduled",
                DispatchDeferral::ConsumerRunning => "consumer_running",
                DispatchDeferral::DeadLettered(terminal) => {
                    skill_validation::record_dispatch_dead_letter(&mut tx, tenant, terminal)
                        .await?;
                    "dead_lettered"
                }
                DispatchDeferral::AlreadyCompleted => "already_completed",
                DispatchDeferral::LostFence => "lost_fence",
            };
            metrics::counter!(APALIS_DISPATCH_TOTAL, "outcome" => label).increment(1);
        }
    }
    tx.commit().await.map_err(transaction_error)?;
    Ok(())
}

async fn handle_task(task: Value, context: Data<HandlerContext>) -> Result<(), ApalisError> {
    let task = match serde_json::from_value::<SkillValidationTask>(task) {
        Ok(task) => task,
        Err(_) => {
            tracing::warn!("malformed opaque Apalis task refused");
            metrics::counter!(APALIS_TASKS_TOTAL, "outcome" => "malformed").increment(1);
            return Err(abort_task("malformed_task"));
        }
    };
    let mut permit = context.authority.permit();
    if !permit.is_open() {
        metrics::counter!(APALIS_TASKS_TOTAL, "outcome" => "authority_closed").increment(1);
        return Err(retry_task("authority_closed"));
    }
    let execution = handle_authorized_task(&task, &context);
    tokio::pin!(execution);
    let outcome = tokio::select! {
        biased;
        () = permit.revoked() => Err(retry_task("authority_closed")),
        outcome = &mut execution => outcome,
    };
    match outcome {
        Ok(outcome) => {
            metrics::counter!(APALIS_TASKS_TOTAL, "outcome" => task_outcome(outcome)).increment(1);
            Ok(())
        }
        Err(error) => {
            metrics::counter!(APALIS_TASKS_TOTAL, "outcome" => "retryable_error").increment(1);
            Err(error)
        }
    }
}

async fn handle_authorized_task(
    task: &SkillValidationTask,
    context: &HandlerContext,
) -> Result<ExecuteOutcome, ApalisError> {
    let outcome = skill_validation::execute_operation(
        &context.runtime,
        task.tenant_id,
        Some(task.operation_id),
        task.operation_version,
    )
    .await;
    match outcome {
        Ok(ExecuteOutcome::LeaseLost | ExecuteOutcome::NotClaimed) => {
            recover_delivery(task, context).await
        }
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            tracing::warn!(tenant.id = %task.tenant_id, %error, "Apalis task execution failed");
            recover_delivery(task, context).await
        }
    }
}

async fn recover_delivery(
    task: &SkillValidationTask,
    context: &HandlerContext,
) -> Result<ExecuteOutcome, ApalisError> {
    if !context.authority.is_open() {
        return Err(retry_task("authority_closed"));
    }
    let mut tx = rls::begin_tenant_tx(&context.product_pool, task.tenant_id)
        .await
        .map_err(|_| retry_task("product_error"))?;
    let state = operations::recover_interrupted_delivery(
        &mut tx,
        task.tenant_id,
        task.operation_id,
        "delivery_interrupted",
    )
    .await
    .map_err(|_| retry_task("product_error"))?;
    let Some(state) = state else {
        let _ = tx.rollback().await;
        return Err(abort_task("operation_not_found"));
    };
    tx.commit().await.map_err(|_| retry_task("product_error"))?;
    if state.is_terminal() {
        return Ok(ExecuteOutcome::NotClaimed);
    }
    Ok(ExecuteOutcome::RetryScheduled)
}

fn retry_task(code: &'static str) -> ApalisError {
    let error: BoxDynError = Box::new(std::io::Error::other(code));
    ApalisError::Failed(Arc::new(error))
}

fn abort_task(code: &'static str) -> ApalisError {
    let error: BoxDynError = Box::new(std::io::Error::other(code));
    ApalisError::Abort(Arc::new(error))
}

const fn task_outcome(outcome: ExecuteOutcome) -> &'static str {
    match outcome {
        ExecuteOutcome::NotClaimed => "not_claimed",
        ExecuteOutcome::Succeeded => "succeeded",
        ExecuteOutcome::RetryScheduled => "retry_scheduled",
        ExecuteOutcome::DeadLettered => "dead_lettered",
        ExecuteOutcome::LeaseLost => "lease_lost",
    }
}

async fn stop_signal(mut shutdown: watch::Receiver<bool>) -> Result<(), std::io::Error> {
    loop {
        if *shutdown.borrow_and_update() {
            return Ok(());
        }
        shutdown
            .changed()
            .await
            .map_err(|_| std::io::Error::other("Apalis shutdown channel closed"))?;
    }
}

async fn serve_health(
    listener: tokio::net::TcpListener,
    health: Health,
    stop: oneshot::Receiver<()>,
) -> Result<(), std::io::Error> {
    let router = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics))
        .with_state(health);
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = stop.await;
        })
        .await
}

async fn healthz() -> StatusCode {
    StatusCode::OK
}

async fn readyz(State(health): State<Health>) -> StatusCode {
    if !health.ready.load(Ordering::Acquire) || !health.authority.is_open() {
        return StatusCode::SERVICE_UNAVAILABLE;
    }
    match tokio::time::timeout(Duration::from_secs(1), health.queue_pool.acquire()).await {
        Ok(Ok(_)) => StatusCode::OK,
        Ok(Err(_)) | Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

async fn metrics(State(health): State<Health>) -> Response {
    health.metrics.render().into_response()
}

fn listen_address() -> Result<std::net::SocketAddr, Box<dyn std::error::Error>> {
    let raw = setting_or("SYNVEDA_APALIS_LISTEN_ADDR", "127.0.0.1:8122")?;
    let address = raw
        .parse::<std::net::SocketAddr>()
        .map_err(|_| "SYNVEDA_APALIS_LISTEN_ADDR must be an IP socket address")?;
    let allow_non_loopback =
        match runtime_config::setting("SYNVEDA_APALIS_ALLOW_NON_LOOPBACK_HEALTH")?.as_deref() {
            None | Some("false") => false,
            Some("true") => true,
            Some(_) => {
                return Err(
                    "SYNVEDA_APALIS_ALLOW_NON_LOOPBACK_HEALTH must be exactly true or false".into(),
                );
            }
        };
    if !address.ip().is_loopback() && (!allow_non_loopback || !address.ip().is_unspecified()) {
        return Err("SYNVEDA_APALIS_LISTEN_ADDR must use loopback unless the private health-listener relaxation admits an unspecified address".into());
    }
    Ok(address)
}

fn seconds(duration: Duration) -> i64 {
    i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
}

fn transaction_error(error: sqlx::Error) -> synveda_types::Error {
    synveda_types::Error::Storage {
        message: format!("finish Apalis operation transaction: {error}"),
    }
}

fn describe_metrics() {
    metrics::describe_gauge!(
        APALIS_READY,
        "Experimental Apalis Skill-validation worker readiness"
    );
    metrics::describe_counter!(
        APALIS_TASKS_TOTAL,
        "Experimental Apalis tasks by closed execution outcome"
    );
    metrics::describe_counter!(
        APALIS_DISPATCH_TOTAL,
        "Experimental Apalis outbox submissions by closed outcome"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_queue_sql_boundary_is_pinned() {
        assert_eq!(
            (
                QUEUE_HOST,
                QUEUE_PORT,
                QUEUE_DATABASE,
                QUEUE_OWNER,
                QUEUE_RUNTIME,
            ),
            (
                "apalis-postgres",
                5432,
                "synveda_apalis",
                "synveda_apalis_owner",
                "synveda_apalis",
            )
        );

        let grants = CONVERGE_RUNTIME_ACL
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("GRANT "))
            .collect::<Vec<_>>();
        assert_eq!(
            grants,
            [
                "GRANT CONNECT ON DATABASE synveda_apalis TO synveda_apalis;",
                "GRANT USAGE ON SCHEMA apalis TO synveda_apalis;",
                "GRANT SELECT, INSERT, UPDATE ON TABLE apalis.jobs, apalis.workers TO synveda_apalis;",
                "GRANT EXECUTE ON FUNCTION apalis.get_jobs(text, text, integer) TO synveda_apalis;",
            ]
        );

        let role_flags =
            "WITH LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS";
        assert_eq!(CONVERGE_RUNTIME_ROLE.matches(role_flags).count(), 2);
        for required in [
            "FROM pg_catalog.pg_auth_members membership",
            "EXECUTE format('REVOKE %I FROM synveda_apalis', granted_role);",
            "REVOKE ALL ON DATABASE synveda_apalis FROM PUBLIC;",
            "REVOKE ALL ON DATABASE synveda_apalis FROM synveda_apalis;",
            "REVOKE ALL ON DATABASE postgres, template1 FROM PUBLIC;",
            "REVOKE ALL ON DATABASE postgres, template1 FROM synveda_apalis;",
            "REVOKE ALL ON SCHEMA public FROM PUBLIC;",
            "REVOKE ALL ON SCHEMA public FROM synveda_apalis;",
            "REVOKE ALL ON SCHEMA apalis FROM PUBLIC;",
            "REVOKE ALL ON SCHEMA apalis FROM synveda_apalis;",
            "REVOKE ALL ON ALL TABLES IN SCHEMA apalis FROM PUBLIC;",
            "REVOKE ALL ON ALL TABLES IN SCHEMA apalis FROM synveda_apalis;",
            "REVOKE ALL ON ALL FUNCTIONS IN SCHEMA apalis FROM PUBLIC;",
            "REVOKE ALL ON ALL FUNCTIONS IN SCHEMA apalis FROM synveda_apalis;",
            "REVOKE ALL ON ALL FUNCTIONS IN SCHEMA public FROM PUBLIC;",
            "REVOKE ALL ON ALL FUNCTIONS IN SCHEMA public FROM synveda_apalis;",
        ] {
            assert!(
                CONVERGE_RUNTIME_ROLE.contains(required) || CONVERGE_RUNTIME_ACL.contains(required),
                "private queue convergence lost `{required}`"
            );
        }

        for required in [
            "AND NOT rolsuper",
            "AND NOT rolcreatedb",
            "AND NOT rolcreaterole",
            "AND NOT rolinherit",
            "AND NOT rolreplication",
            "AND NOT rolbypassrls",
            "FROM pg_catalog.pg_auth_members membership",
            "has_database_privilege(current_user, 'synveda_apalis', 'CONNECT')",
            "NOT has_database_privilege(current_user, 'synveda_apalis', 'TEMPORARY')",
            "NOT has_database_privilege(current_user, 'postgres', 'CONNECT')",
            "NOT has_database_privilege(current_user, 'template1', 'CONNECT')",
            "NOT has_schema_privilege(current_user, 'public', 'USAGE')",
            "has_schema_privilege(current_user, 'apalis', 'USAGE')",
            "NOT has_schema_privilege(current_user, 'apalis', 'CREATE')",
            "has_table_privilege(current_user, 'apalis.jobs', 'SELECT')",
            "has_table_privilege(current_user, 'apalis.jobs', 'INSERT')",
            "has_table_privilege(current_user, 'apalis.jobs', 'UPDATE')",
            "NOT has_table_privilege(current_user, 'apalis.jobs', 'DELETE,TRUNCATE,REFERENCES,TRIGGER')",
            "has_table_privilege(current_user, 'apalis.workers', 'SELECT')",
            "has_table_privilege(current_user, 'apalis.workers', 'INSERT')",
            "has_table_privilege(current_user, 'apalis.workers', 'UPDATE')",
            "NOT has_table_privilege(current_user, 'apalis.workers', 'DELETE,TRUNCATE,REFERENCES,TRIGGER')",
            "has_function_privilege(current_user, 'apalis.get_jobs(text,text,integer)', 'EXECUTE')",
            "namespace.nspname = 'public'",
            "NOT has_table_privilege(current_user, 'public._sqlx_migrations', 'SELECT')",
        ] {
            assert!(
                VERIFY_RUNTIME_ACL.contains(required),
                "private queue verification lost `{required}`"
            );
        }

        for forbidden in [
            "durable_operations",
            "operation_outbox",
            "operation_attempts",
            "skill_",
            "audit_",
            "tenants",
        ] {
            for sql in [
                CONVERGE_RUNTIME_ROLE,
                CONVERGE_RUNTIME_ACL,
                VERIFY_RUNTIME_ACL,
            ] {
                assert!(
                    !sql.contains(forbidden),
                    "private queue SQL named product state `{forbidden}`"
                );
            }
        }
    }

    #[test]
    fn task_payload_is_exact_and_content_free() {
        let task = SkillValidationTask {
            tenant_id: TenantId::new(),
            operation_id: DurableOperationId::new(),
            operation_version: 1,
        };
        let value = serde_json::to_value(&task).expect("serialize task");
        let object = value.as_object().expect("task object");
        assert_eq!(
            object.keys().map(String::as_str).collect::<Vec<_>>(),
            ["tenant_id", "operation_id", "operation_version"]
        );
        for forbidden in ["scope_id", "principal_id", "content", "credential"] {
            assert!(!object.contains_key(forbidden));
        }
        assert_eq!(
            serde_json::from_value::<SkillValidationTask>(value).unwrap(),
            task
        );
    }

    #[test]
    fn task_payload_rejects_unknown_or_missing_fields() {
        let id = DurableOperationId::new();
        assert!(
            serde_json::from_value::<SkillValidationTask>(serde_json::json!({
                "tenant_id": TenantId::new(),
                "operation_id": id,
                "operation_version": 1,
                "unexpected": true,
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<SkillValidationTask>(serde_json::json!({
                "tenant_id": TenantId::new(),
                "operation_id": id,
            }))
            .is_err()
        );
    }

    #[test]
    fn task_outcome_labels_are_closed() {
        assert_eq!(task_outcome(ExecuteOutcome::Succeeded), "succeeded");
        assert_eq!(
            task_outcome(ExecuteOutcome::RetryScheduled),
            "retry_scheduled"
        );
        assert_eq!(task_outcome(ExecuteOutcome::DeadLettered), "dead_lettered");
        assert_eq!(task_outcome(ExecuteOutcome::LeaseLost), "lease_lost");
        assert_eq!(task_outcome(ExecuteOutcome::NotClaimed), "not_claimed");
    }
}
