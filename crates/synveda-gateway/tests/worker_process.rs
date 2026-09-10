//! CPR-45 real-process acceptance for the private core worker.
//!
//! Database-backed cases use the exact worker login and a separate narrow
//! lifecycle administrator supplied by `make db-test`. Ordinary workspace
//! tests never receive that administrator through `DATABASE_URL`.

#![cfg(unix)]

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use synveda_gateway::authority::{CHECK_INTERVAL, CHECK_TIMEOUT};

const CHILD_STDERR_BOUND: u64 = 65_536;

struct ChildGuard(Child);

impl ChildGuard {
    fn stderr(&mut self) -> String {
        let mut output = String::new();
        if let Some(stderr) = self.0.stderr.as_mut() {
            let _ = stderr.take(CHILD_STDERR_BOUND).read_to_string(&mut output);
        }
        output
    }

    fn stop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
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

fn configured_roles_json() -> Option<String> {
    match (
        std::env::var("SYNVEDA_DATABASE_ROLES").ok(),
        std::env::var("SYNVEDA_DATABASE_ROLES_FILE").ok(),
    ) {
        (Some(value), None) => Some(value),
        (None, Some(path)) => Some(
            std::fs::read_to_string(path)
                .expect("read test database role contract")
                .trim_end()
                .to_owned(),
        ),
        _ => None,
    }
}

fn spawn_worker(database_url_file: &Path, roles_json: &str, listen_addr: &str) -> ChildGuard {
    let child = Command::new(env!("CARGO_BIN_EXE_synveda-worker"))
        .env_remove("DATABASE_URL")
        .env("DATABASE_URL_FILE", database_url_file)
        .env("SYNVEDA_DATABASE_ROLES", roles_json)
        .env_remove("SYNVEDA_DATABASE_ROLES_FILE")
        .env_remove("SYNVEDA_EXPECTED_DATABASE_ROLE")
        .env_remove("SYNVEDA_EXPECTED_DATABASE_ROLE_FILE")
        .env("SYNVEDA_WORKER_LISTEN_ADDR", listen_addr)
        .env("SYNVEDA_WORKER_SHUTDOWN_SECS", "3")
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

fn private_database_url_file(setting: &str) -> Option<PathBuf> {
    std::env::var_os(setting).map(PathBuf::from)
}

fn private_database_url(path: &Path) -> String {
    std::fs::read_to_string(path)
        .expect("read isolated test database URL")
        .trim_end_matches('\n')
        .to_owned()
}

fn spawn_with_invalid_database_url(executable: &str, database_url: &str) -> ChildGuard {
    let roles = r#"{"migrator":"migrator","gateway":"gateway","worker":"worker","administrators":["administrator"],"administrative_memberships":[],"forbidden_databases":["postgres"],"isolated_peer_roles":[]}"#;
    let child = Command::new(executable)
        .env("DATABASE_URL", database_url)
        .env_remove("DATABASE_URL_FILE")
        .env("SYNVEDA_DATABASE_ROLES", roles)
        .env_remove("SYNVEDA_DATABASE_ROLES_FILE")
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

fn spawn_with_database_topology(executable: &str, roles: &str, required_peer: &str) -> ChildGuard {
    let child = Command::new(executable)
        .env(
            "DATABASE_URL",
            "postgres://unavailable:opaque@127.0.0.1:1/synveda",
        )
        .env_remove("DATABASE_URL_FILE")
        .env("SYNVEDA_DATABASE_ROLES", roles)
        .env_remove("SYNVEDA_DATABASE_ROLES_FILE")
        .env("SYNVEDA_DATABASE_REQUIRED_PEER", required_peer)
        .env_remove("SYNVEDA_DATABASE_REQUIRED_PEER_FILE")
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
        .expect("spawn product process with database topology contract");
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

struct ExitObservation {
    status: Option<std::process::ExitStatus>,
    stderr: String,
}

async fn observe_exit(child: &mut ChildGuard, budget: Duration) -> ExitObservation {
    let deadline = Instant::now() + budget;
    loop {
        if let Some(status) = child.0.try_wait().expect("read worker status") {
            return ExitObservation {
                status: Some(status),
                stderr: child.stderr(),
            };
        }
        if Instant::now() >= deadline {
            child.stop();
            return ExitObservation {
                status: None,
                stderr: child.stderr(),
            };
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn assert_terminal_refusal(observation: &ExitObservation, label: &str) {
    assert!(
        !observation.stderr.contains("postgres://")
            && !observation.stderr.contains("postgresql://")
            && !observation.stderr.contains("password="),
        "{label} rendered database connection material"
    );
    let status = observation
        .status
        .as_ref()
        .unwrap_or_else(|| panic!("{label} did not exit within the authority refusal bound"));
    assert!(!status.success(), "{label} was accepted as worker");
    assert!(
        observation
            .stderr
            .contains("worker database authority was conclusively refused"),
        "{label} did not emit the generic terminal refusal"
    );
}

async fn observe_worker_refusal(database_url_file: &Path, roles: &str) -> ExitObservation {
    let addr = free_loopback_addr();
    let mut worker = spawn_worker(database_url_file, roles, &addr);
    observe_exit(
        &mut worker,
        CHECK_INTERVAL + CHECK_TIMEOUT + Duration::from_secs(5),
    )
    .await
}

async fn assert_worker_refused(database_url_file: &Path, roles: &str, label: &str) {
    let observation = observe_worker_refusal(database_url_file, roles).await;
    assert_terminal_refusal(&observation, label);
}

fn quoted_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

#[tokio::test]
async fn boot_outage_is_live_not_ready_and_sigterm_exits_cleanly() {
    let addr = free_loopback_addr();
    let roles = r#"{"migrator":"migrator","gateway":"gateway","worker":"unavailable","administrators":["administrator"],"administrative_memberships":[],"forbidden_databases":["postgres"],"isolated_peer_roles":[]}"#;
    let database_url_file =
        std::env::temp_dir().join(format!("synveda-worker-outage-url-{}", std::process::id()));
    std::fs::write(
        &database_url_file,
        "postgres://unavailable:opaque@127.0.0.1:1/unavailable\n",
    )
    .expect("write outage database URL file");
    let mut worker = spawn_worker(&database_url_file, roles, &addr);

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

    send_sigterm(&worker);
    let status = wait_for_exit(&mut worker, Duration::from_secs(5)).await;
    assert!(
        status.success(),
        "worker exited with {status}: {}",
        worker.stderr()
    );
    std::fs::remove_file(database_url_file).ok();
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
async fn gateway_and_worker_refuse_a_one_sided_required_peer_before_connecting() {
    let one_sided_roles = [
        r#"{"migrator":"migrator","gateway":"gateway","worker":"worker","administrators":["administrator"],"administrative_memberships":[],"forbidden_databases":["postgres","template1"],"isolated_peer_roles":["keycloak"]}"#,
        r#"{"migrator":"migrator","gateway":"gateway","worker":"worker","administrators":["administrator"],"administrative_memberships":[],"forbidden_databases":["keycloak","postgres","template1"],"isolated_peer_roles":[]}"#,
    ];
    for executable in [
        env!("CARGO_BIN_EXE_synveda-gateway"),
        env!("CARGO_BIN_EXE_synveda-worker"),
    ] {
        for roles in one_sided_roles {
            let mut process = spawn_with_database_topology(executable, roles, "keycloak");
            let status = wait_for_exit(&mut process, Duration::from_secs(5)).await;
            let stderr = process.stderr();
            assert!(!status.success(), "one-sided peer topology was accepted");
            assert!(
                stderr.contains(
                    "SYNVEDA_DATABASE_REQUIRED_PEER is absent from the configured bidirectional isolation contract"
                ),
                "content-free topology refusal missing: {stderr}"
            );
            assert!(
                !stderr.contains("connection refused") && !stderr.contains("Connection refused"),
                "process reached PostgreSQL before refusing topology: {stderr}"
            );
        }
    }
}

#[tokio::test]
async fn worker_login_mismatch_is_a_generic_terminal_refusal_before_connecting() {
    let database_url_file = std::env::temp_dir().join(format!(
        "synveda-worker-login-mismatch-{}",
        std::process::id()
    ));
    std::fs::write(
        &database_url_file,
        "postgres://gateway:opaque@127.0.0.1:1/synveda\n",
    )
    .expect("write mismatched worker database URL file");
    let roles = r#"{"migrator":"migrator","gateway":"gateway","worker":"worker","administrators":["administrator"],"administrative_memberships":[],"forbidden_databases":["postgres"],"isolated_peer_roles":[]}"#;
    let observation = observe_worker_refusal(&database_url_file, roles).await;
    std::fs::remove_file(database_url_file).ok();
    assert_terminal_refusal(&observation, "gateway login");
    assert!(
        !observation.stderr.contains("connection refused")
            && !observation.stderr.contains("Connection refused"),
        "gateway login reached PostgreSQL before the terminal refusal"
    );
}

#[tokio::test]
async fn exact_worker_role_is_ready_and_authority_drift_is_terminal() {
    let (Some(admin_url_file), Some(worker_url_file), Some(gateway_url_file), Some(roles_json)) = (
        private_database_url_file("SYNVEDA_TEST_ADMIN_DATABASE_URL_FILE"),
        private_database_url_file("SYNVEDA_TEST_WORKER_DATABASE_URL_FILE"),
        private_database_url_file("SYNVEDA_TEST_GATEWAY_DATABASE_URL_FILE"),
        configured_roles_json(),
    ) else {
        eprintln!("isolated exact-role fixture not set; skipping worker process acceptance");
        return;
    };
    let roles = synveda_store::runtime_role::DatabaseRoles::parse_json(&roles_json)
        .expect("parse exact test roles");
    let worker_role = quoted_identifier(roles.worker());
    let admin_url = private_database_url(&admin_url_file);
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&admin_url)
        .await
        .expect("connect as isolated test administrator");

    let addr = free_loopback_addr();
    let mut worker = spawn_worker(&worker_url_file, &roles_json, &addr);
    wait_for_status(
        &mut worker,
        &format!("http://{addr}/readyz"),
        StatusCode::OK,
        Duration::from_secs(15),
    )
    .await;
    send_sigterm(&worker);
    let status = wait_for_exit(&mut worker, Duration::from_secs(5)).await;
    assert!(status.success(), "exact worker did not drain cleanly");

    assert_worker_refused(&gateway_url_file, &roles_json, "gateway login").await;
    assert_worker_refused(&admin_url_file, &roles_json, "database administrator").await;

    let addr = free_loopback_addr();
    let mut drifted = spawn_worker(&worker_url_file, &roles_json, &addr);
    wait_for_status(
        &mut drifted,
        &format!("http://{addr}/readyz"),
        StatusCode::OK,
        Duration::from_secs(15),
    )
    .await;
    sqlx::query(&format!("alter role {worker_role} bypassrls"))
        .execute(&admin)
        .await
        .expect("introduce conclusive worker drift");
    let observation = observe_exit(
        &mut drifted,
        CHECK_INTERVAL + CHECK_TIMEOUT + Duration::from_secs(5),
    )
    .await;
    sqlx::query(&format!("alter role {worker_role} nobypassrls"))
        .execute(&admin)
        .await
        .expect("restore worker role");
    assert_terminal_refusal(&observation, "worker with BYPASSRLS");

    sqlx::query(&format!("grant pg_read_all_data to {worker_role}"))
        .execute(&admin)
        .await
        .expect("add unexpected worker membership");
    let observation = observe_worker_refusal(&worker_url_file, &roles_json).await;
    sqlx::query(&format!("revoke pg_read_all_data from {worker_role}"))
        .execute(&admin)
        .await
        .expect("restore worker membership");
    assert_terminal_refusal(&observation, "worker with extra membership");

    sqlx::query(&format!(
        "grant synveda_app to {worker_role} with admin true, inherit true, set true"
    ))
    .execute(&admin)
    .await
    .expect("add unsafe worker grant option");
    let observation = observe_worker_refusal(&worker_url_file, &roles_json).await;
    sqlx::query(&format!(
        "revoke admin option for synveda_app from {worker_role}"
    ))
    .execute(&admin)
    .await
    .expect("remove unsafe worker grant option");
    assert_terminal_refusal(&observation, "worker with ADMIN OPTION");

    sqlx::query(&format!(
        "alter role {worker_role} set default_transaction_read_only = on"
    ))
    .execute(&admin)
    .await
    .expect("make worker sessions read-only");
    let observation = observe_worker_refusal(&worker_url_file, &roles_json).await;
    sqlx::query(&format!(
        "alter role {worker_role} reset default_transaction_read_only"
    ))
    .execute(&admin)
    .await
    .expect("restore writable worker sessions");
    assert_terminal_refusal(&observation, "read-only worker session");

    sqlx::query(&format!(
        "grant update (slug) on table tenants to {worker_role}"
    ))
    .execute(&admin)
    .await
    .expect("add direct worker column ACL");
    let observation = observe_worker_refusal(&worker_url_file, &roles_json).await;
    sqlx::query(&format!(
        "revoke update (slug) on table tenants from {worker_role}"
    ))
    .execute(&admin)
    .await
    .expect("remove direct worker column ACL");
    assert_terminal_refusal(&observation, "worker with a direct column ACL");

    sqlx::query("revoke execute on function pg_catalog.pg_control_system() from public")
        .execute(&admin)
        .await
        .expect("remove the fixed identity-catalog permission");
    let observation = observe_worker_refusal(&worker_url_file, &roles_json).await;
    sqlx::query("grant execute on function pg_catalog.pg_control_system() to public")
        .execute(&admin)
        .await
        .expect("restore the fixed identity-catalog permission");
    assert_terminal_refusal(
        &observation,
        "worker with a permanent identity-query refusal",
    );

    admin.close().await;
}
