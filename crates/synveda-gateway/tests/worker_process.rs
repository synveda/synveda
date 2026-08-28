//! CPR-45 process acceptance for the private core worker.
//!
//! These tests spawn the shipped binary rather than calling supervisor
//! helpers. The database-backed half uses an ordinary non-owner login and
//! skips only when the repository's documented `DATABASE_URL` prerequisite is
//! absent; `make db-test` always supplies it.

#![cfg(unix)]

use std::io::Read as _;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use reqwest::StatusCode;

const PROCESS_ROLE: &str = "synveda_worker_process_test";
const PROCESS_PASSWORD: &str = "synveda-worker-process-test";

struct ChildGuard(Child);

impl ChildGuard {
    fn stderr(&mut self) -> String {
        let mut output = String::new();
        if let Some(stderr) = self.0.stderr.as_mut() {
            let _ = stderr.read_to_string(&mut output);
        }
        output
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_loopback_addr() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve loopback port");
    let addr = listener.local_addr().expect("read loopback port");
    drop(listener);
    addr.to_string()
}

fn spawn_worker(database_url: &str, expected_role: &str, listen_addr: &str) -> ChildGuard {
    let child = Command::new(env!("CARGO_BIN_EXE_synveda-worker"))
        .env("DATABASE_URL", database_url)
        .env_remove("DATABASE_URL_FILE")
        .env("SYNVEDA_EXPECTED_DATABASE_ROLE", expected_role)
        .env_remove("SYNVEDA_EXPECTED_DATABASE_ROLE_FILE")
        .env("SYNVEDA_WORKER_LISTEN_ADDR", listen_addr)
        .env("SYNVEDA_WORKER_SHUTDOWN_SECS", "2")
        .env("SYNVEDA_EXTRACTOR", "deterministic")
        .env("SYNVEDA_EMBEDDER", "deterministic")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("ANTHROPIC_API_KEY_FILE")
        .env_remove("SYNVEDA_OIDC_ISSUERS")
        .env_remove("SYNVEDA_OIDC_ISSUERS_FILE")
        .env_remove("SYNVEDA_KMS_KEY")
        .env_remove("SYNVEDA_KMS_KEY_FILE")
        .env_remove("SYNVEDA_KMS_KEY_REF")
        .env_remove("SYNVEDA_KMS_KEY_REF_FILE")
        .env("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:9")
        .env("OTEL_BSP_EXPORT_TIMEOUT", "100")
        .env("RUST_LOG", "error")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn synveda-worker");
    ChildGuard(child)
}

fn spawn_with_invalid_database_url(executable: &str, database_url: &str) -> ChildGuard {
    let child = Command::new(executable)
        .env("DATABASE_URL", database_url)
        .env_remove("DATABASE_URL_FILE")
        .env("SYNVEDA_EXPECTED_DATABASE_ROLE", "invalid_config_test")
        .env("SYNVEDA_LISTEN_ADDR", "127.0.0.1:0")
        .env("SYNVEDA_WORKER_LISTEN_ADDR", "127.0.0.1:0")
        .env_remove("SYNVEDA_OIDC_ISSUERS")
        .env_remove("SYNVEDA_OIDC_ISSUERS_FILE")
        .env_remove("SYNVEDA_DEV_JWT_SECRET")
        .env("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:9")
        .env("OTEL_BSP_EXPORT_TIMEOUT", "100")
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn product process with invalid database URL");
    ChildGuard(child)
}

async fn wait_for_status(
    child: &mut ChildGuard,
    url: &str,
    wanted: StatusCode,
    budget: Duration,
) -> reqwest::Response {
    let client = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_millis(200))
        .timeout(Duration::from_secs(3))
        .build()
        .expect("build health client");
    let deadline = Instant::now() + budget;
    loop {
        if let Some(status) = child.0.try_wait().expect("read worker status") {
            let stderr = child.stderr();
            panic!("worker exited before {url} returned {wanted}: {status}\n{stderr}");
        }
        if let Ok(response) = client.get(url).send().await
            && response.status() == wanted
        {
            return response;
        }
        assert!(Instant::now() < deadline, "{url} did not return {wanted}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn send_sigterm(child: &ChildGuard) {
    let status = Command::new("kill")
        .args(["-TERM", &child.0.id().to_string()])
        .status()
        .expect("send SIGTERM");
    assert!(status.success(), "kill -TERM failed");
}

async fn wait_for_exit(child: &mut ChildGuard, budget: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + budget;
    loop {
        if let Some(status) = child.0.try_wait().expect("read worker status") {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "worker did not exit within {budget:?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test]
async fn boot_outage_is_live_not_ready_and_sigterm_exits_cleanly() {
    let addr = free_loopback_addr();
    let mut worker = spawn_worker(
        "postgres://unavailable:opaque@127.0.0.1:1/unavailable",
        "unavailable",
        &addr,
    );

    wait_for_status(
        &mut worker,
        &format!("http://{addr}/healthz"),
        StatusCode::OK,
        Duration::from_secs(10),
    )
    .await;
    wait_for_status(
        &mut worker,
        &format!("http://{addr}/readyz"),
        StatusCode::SERVICE_UNAVAILABLE,
        Duration::from_secs(3),
    )
    .await;
    let metrics = wait_for_status(
        &mut worker,
        &format!("http://{addr}/metrics"),
        StatusCode::OK,
        Duration::from_secs(3),
    )
    .await
    .text()
    .await
    .expect("read worker metrics");
    assert!(metrics.contains("synveda_worker_ready 0"), "{metrics}");

    let started = Instant::now();
    send_sigterm(&worker);
    let status = wait_for_exit(&mut worker, Duration::from_secs(5)).await;
    assert!(
        status.success(),
        "worker exited with {status}: {}",
        worker.stderr()
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn invalid_database_urls_exit_without_logging_their_values() {
    const SENTINEL: &str = "SYNVEDA_PROCESS_DATABASE_SECRET";
    let database_urls = [
        format!("https://invalid:{SENTINEL}@127.0.0.1:1/synveda"),
        format!("postgres://invalid@127.0.0.1:1/synveda?access_token={SENTINEL}"),
    ];
    for executable in [
        env!("CARGO_BIN_EXE_synveda-gateway"),
        env!("CARGO_BIN_EXE_synveda-worker"),
    ] {
        for database_url in &database_urls {
            let mut process = spawn_with_invalid_database_url(executable, database_url);
            let status = wait_for_exit(&mut process, Duration::from_secs(5)).await;
            let stderr = process.stderr();
            assert!(!status.success(), "invalid database URL was accepted");
            assert!(
                !stderr.contains(SENTINEL),
                "database secret leaked: {stderr}"
            );
            assert!(
                stderr.contains("DATABASE_URL is not a valid PostgreSQL connection URL"),
                "content-free refusal missing: {stderr}"
            );
        }
    }
}

#[tokio::test]
async fn exact_non_owner_role_becomes_ready_and_owner_or_wrong_role_is_refused() {
    let Some(admin_url) = std::env::var("DATABASE_URL")
        .ok()
        .filter(|value| !value.is_empty())
    else {
        eprintln!("DATABASE_URL not set; skipping CPR-45 worker process database acceptance");
        return;
    };
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&admin_url)
        .await
        .expect("connect as database owner");
    let mut tx = admin
        .begin()
        .await
        .expect("begin process-role provisioning");
    sqlx::query!("select pg_advisory_xact_lock(hashtext('synveda.test.worker-process-role'))")
        .execute(&mut *tx)
        .await
        .expect("lock process-role provisioning");
    sqlx::query!(
        r#"do $synveda$
           begin
             if not exists (
               select 1 from pg_catalog.pg_roles
                where rolname = 'synveda_worker_process_test'
             ) then
               create role synveda_worker_process_test
                 login password 'synveda-worker-process-test';
             end if;
           end
           $synveda$"#
    )
    .execute(&mut *tx)
    .await
    .expect("create process role");
    sqlx::query!(
        r#"alter role synveda_worker_process_test
              with login inherit nosuperuser nocreatedb nocreaterole
                   noreplication nobypassrls connection limit -1
                   password 'synveda-worker-process-test'"#
    )
    .execute(&mut *tx)
    .await
    .expect("converge process role");
    sqlx::query!(
        r#"do $synveda$
           begin
             if not exists (
               select 1 from pg_catalog.pg_roles
                where rolname = 'synveda_worker_process_grantor'
             ) then
               create role synveda_worker_process_grantor nologin;
             end if;
           end
           $synveda$"#
    )
    .execute(&mut *tx)
    .await
    .expect("create alternate membership grantor");
    sqlx::query!(
        r#"alter role synveda_worker_process_grantor
              with nologin inherit nosuperuser nocreatedb nocreaterole
                   noreplication nobypassrls connection limit -1"#
    )
    .execute(&mut *tx)
    .await
    .expect("converge alternate membership grantor");
    sqlx::query!(
        "grant synveda_app to synveda_worker_process_grantor with admin true, inherit true, set true"
    )
    .execute(&mut *tx)
    .await
    .expect("authorise alternate membership grantor");
    sqlx::query!(
        "revoke synveda_app from synveda_worker_process_test granted by synveda_worker_process_grantor"
    )
    .execute(&mut *tx)
    .await
    .expect("remove stale alternate process-role grant");
    sqlx::query!("revoke pg_read_all_data from synveda_worker_process_test")
        .execute(&mut *tx)
        .await
        .expect("remove stale process-role membership");
    sqlx::query!("drop schema if exists synveda_worker_process_owned cascade")
        .execute(&mut *tx)
        .await
        .expect("remove stale process-role schema");
    sqlx::query!(
        "grant synveda_app to synveda_worker_process_test with admin false, inherit true, set true"
    )
    .execute(&mut *tx)
    .await
    .expect("grant process role");
    tx.commit().await.expect("commit process-role provisioning");

    let mut runtime_url = url::Url::parse(&admin_url).expect("parse admin URL");
    runtime_url
        .set_username(PROCESS_ROLE)
        .expect("set process role");
    runtime_url
        .set_password(Some(PROCESS_PASSWORD))
        .expect("set process password");
    let runtime_url = runtime_url.to_string();

    let addr = free_loopback_addr();
    let mut worker = spawn_worker(&runtime_url, PROCESS_ROLE, &addr);
    wait_for_status(
        &mut worker,
        &format!("http://{addr}/readyz"),
        StatusCode::OK,
        Duration::from_secs(15),
    )
    .await;
    let shutdown_started = Instant::now();
    send_sigterm(&worker);
    let status = wait_for_exit(&mut worker, Duration::from_secs(5)).await;
    assert!(
        status.success(),
        "ready idle worker did not shut down cleanly: {status}: {}",
        worker.stderr()
    );
    assert!(
        shutdown_started.elapsed() < Duration::from_secs(5),
        "ready idle worker exceeded the shutdown bound"
    );

    let drift_addr = free_loopback_addr();
    let mut worker = spawn_worker(&runtime_url, PROCESS_ROLE, &drift_addr);
    wait_for_status(
        &mut worker,
        &format!("http://{drift_addr}/readyz"),
        StatusCode::OK,
        Duration::from_secs(15),
    )
    .await;
    sqlx::query!("alter role synveda_worker_process_test bypassrls")
        .execute(&admin)
        .await
        .expect("drift the live process role");
    let status = wait_for_exit(&mut worker, Duration::from_secs(10)).await;
    sqlx::query!("alter role synveda_worker_process_test nobypassrls")
        .execute(&admin)
        .await
        .expect("restore the live process role");
    assert!(
        !status.success(),
        "worker kept running after conclusive authority drift: {status}: {}",
        worker.stderr()
    );

    let admin_role = url::Url::parse(&admin_url)
        .expect("parse admin URL")
        .username()
        .to_owned();
    let owner_addr = free_loopback_addr();
    let mut owner = spawn_worker(&admin_url, &admin_role, &owner_addr);
    let status = wait_for_exit(&mut owner, Duration::from_secs(10)).await;
    assert!(!status.success(), "database owner was accepted as worker");

    sqlx::query!("alter role synveda_worker_process_test bypassrls")
        .execute(&admin)
        .await
        .expect("make process role elevated");
    let elevated_addr = free_loopback_addr();
    let mut elevated = spawn_worker(&runtime_url, PROCESS_ROLE, &elevated_addr);
    let status = wait_for_exit(&mut elevated, Duration::from_secs(10)).await;
    sqlx::query!("alter role synveda_worker_process_test nobypassrls")
        .execute(&admin)
        .await
        .expect("restore process role");
    assert!(!status.success(), "BYPASSRLS worker role was accepted");

    sqlx::query!("grant pg_read_all_data to synveda_worker_process_test")
        .execute(&admin)
        .await
        .expect("add unexpected process-role membership");
    let membership_addr = free_loopback_addr();
    let mut membership = spawn_worker(&runtime_url, PROCESS_ROLE, &membership_addr);
    let status = wait_for_exit(&mut membership, Duration::from_secs(10)).await;
    sqlx::query!("revoke pg_read_all_data from synveda_worker_process_test")
        .execute(&admin)
        .await
        .expect("restore process-role memberships");
    assert!(
        !status.success(),
        "extra worker role membership was accepted"
    );

    sqlx::query!(
        "grant synveda_app to synveda_worker_process_test with admin true, inherit true, set true"
    )
    .execute(&admin)
    .await
    .expect("grant unsafe administration option");
    let admin_option_addr = free_loopback_addr();
    let mut admin_option = spawn_worker(&runtime_url, PROCESS_ROLE, &admin_option_addr);
    let status = wait_for_exit(&mut admin_option, Duration::from_secs(10)).await;
    sqlx::query!(
        "grant synveda_app to synveda_worker_process_test with admin false, inherit true, set true"
    )
    .execute(&admin)
    .await
    .expect("restore exact process-role membership");
    assert!(
        !status.success(),
        "worker role with synveda_app ADMIN OPTION was accepted"
    );

    sqlx::query!(
        "grant synveda_app to synveda_worker_process_test with admin true, inherit true, set true granted by synveda_worker_process_grantor"
    )
    .execute(&admin)
    .await
    .expect("add a concurrent unsafe grant from another grantor");
    let duplicate_grant_addr = free_loopback_addr();
    let mut duplicate_grant = spawn_worker(&runtime_url, PROCESS_ROLE, &duplicate_grant_addr);
    let status = wait_for_exit(&mut duplicate_grant, Duration::from_secs(10)).await;
    sqlx::query!(
        "revoke synveda_app from synveda_worker_process_test granted by synveda_worker_process_grantor"
    )
    .execute(&admin)
    .await
    .expect("remove the concurrent unsafe grant");
    assert!(
        !status.success(),
        "worker accepted a safe grant alongside an unsafe grant from another grantor"
    );

    sqlx::query!(
        "create schema synveda_worker_process_owned authorization synveda_worker_process_test"
    )
    .execute(&admin)
    .await
    .expect("give process role schema ownership");
    let schema_owner_addr = free_loopback_addr();
    let mut schema_owner = spawn_worker(&runtime_url, PROCESS_ROLE, &schema_owner_addr);
    let status = wait_for_exit(&mut schema_owner, Duration::from_secs(10)).await;
    sqlx::query!("drop schema synveda_worker_process_owned")
        .execute(&admin)
        .await
        .expect("remove process-role schema");
    assert!(!status.success(), "schema-owning worker role was accepted");

    sqlx::query!("alter role synveda_worker_process_test set default_transaction_read_only = on")
        .execute(&admin)
        .await
        .expect("make new process-role sessions read-only");
    let read_only_addr = free_loopback_addr();
    let mut read_only = spawn_worker(&runtime_url, PROCESS_ROLE, &read_only_addr);
    let status = wait_for_exit(&mut read_only, Duration::from_secs(10)).await;
    sqlx::query!("alter role synveda_worker_process_test reset default_transaction_read_only")
        .execute(&admin)
        .await
        .expect("restore writable process-role sessions");
    assert!(
        !status.success(),
        "worker with a read-only database session was accepted"
    );

    let wrong_addr = free_loopback_addr();
    let mut wrong = spawn_worker(&runtime_url, "another_worker_role", &wrong_addr);
    let status = wait_for_exit(&mut wrong, Duration::from_secs(5)).await;
    assert!(!status.success(), "wrong expected role was accepted");

    sqlx::query!("revoke synveda_app from synveda_worker_process_grantor")
        .execute(&admin)
        .await
        .expect("remove alternate grantor capability");
    sqlx::query!("drop role synveda_worker_process_grantor")
        .execute(&admin)
        .await
        .expect("drop alternate membership grantor");
    admin.close().await;
}
