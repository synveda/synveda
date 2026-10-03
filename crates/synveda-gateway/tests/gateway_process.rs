//! OPS-7 process-level shutdown acceptance on an isolated exact-role database.

#![cfg(unix)]

use std::io::Read as _;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use reqwest::StatusCode;

struct ChildGuard(Child);

impl ChildGuard {
    fn stderr(&mut self) -> String {
        let mut output = String::new();
        if let Some(stderr) = self.0.stderr.as_mut() {
            let _ = stderr.take(65_536).read_to_string(&mut output);
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

fn spawn_gateway(database_url_file: &Path, addr: &str, kms_key: Option<&str>) -> ChildGuard {
    let mut command = Command::new(env!("CARGO_BIN_EXE_synveda-gateway"));
    command
        .env_remove("DATABASE_URL")
        .env("DATABASE_URL_FILE", database_url_file)
        .env_remove("SYNVEDA_DATABASE_ROLES")
        .env("SYNVEDA_LISTEN_ADDR", addr)
        .env("SYNVEDA_PUBLIC_URL", format!("http://{addr}"))
        .env_remove("SYNVEDA_PUBLIC_URL_FILE")
        .env("SYNVEDA_GATEWAY_SHUTDOWN_SECS", "15")
        .env("SYNVEDA_POLICY_REFRESH_SECS", "15")
        .env("SYNVEDA_EMBEDDER", "deterministic")
        .env_remove("SYNVEDA_OIDC_ISSUERS")
        .env_remove("SYNVEDA_OIDC_ISSUERS_FILE")
        .env_remove("SYNVEDA_DEV_JWT_SECRET")
        .env_remove("SYNVEDA_DEV_JWT_SECRET_FILE")
        .env_remove("SYNVEDA_KMS_KEY")
        .env_remove("SYNVEDA_KMS_KEY_FILE")
        .env_remove("SYNVEDA_KMS_KEY_REF")
        .env_remove("SYNVEDA_KMS_KEY_REF_FILE")
        .env("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:9")
        .env("OTEL_BSP_EXPORT_TIMEOUT", "100")
        .env("RUST_LOG", "error")
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if let Some(key) = kms_key {
        command.env("SYNVEDA_KMS_KEY", key);
    }
    let child = command.spawn().expect("spawn synveda-gateway");
    ChildGuard(child)
}

async fn wait_for_status(
    child: &mut ChildGuard,
    client: &reqwest::Client,
    url: &str,
    wanted: StatusCode,
    budget: Duration,
) {
    let deadline = Instant::now() + budget;
    loop {
        if let Some(status) = child.0.try_wait().expect("read gateway status") {
            let stderr = child.stderr();
            panic!("gateway exited before {url} returned {wanted}: {status}\n{stderr}");
        }
        if let Ok(response) = client.get(url).send().await
            && response.status() == wanted
        {
            return;
        }
        assert!(Instant::now() < deadline, "{url} did not return {wanted}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test]
async fn sigterm_withdraws_readiness_before_http_stops() {
    let Some(database_url_file) = std::env::var_os("SYNVEDA_TEST_DATABASE_URL_FILE") else {
        eprintln!("skipping gateway SIGTERM: run through `make db-test`");
        return;
    };
    assert!(
        std::env::var_os("SYNVEDA_DATABASE_ROLES_FILE").is_some(),
        "isolated database fixture must supply exact runtime roles"
    );
    let addr = free_loopback_addr();
    let mut child = spawn_gateway(Path::new(&database_url_file), &addr, None);
    let client = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_millis(200))
        .timeout(Duration::from_secs(2))
        .build()
        .expect("build loopback client");
    let base = format!("http://{addr}");
    wait_for_status(
        &mut child,
        &client,
        &format!("{base}/readyz"),
        StatusCode::OK,
        Duration::from_secs(10),
    )
    .await;
    wait_for_status(
        &mut child,
        &client,
        &format!("{base}/v1/whoami"),
        StatusCode::UNAUTHORIZED,
        Duration::from_secs(2),
    )
    .await;

    let sent = Command::new("kill")
        .args(["-TERM", &child.0.id().to_string()])
        .status()
        .expect("send SIGTERM");
    assert!(sent.success(), "SIGTERM delivery failed");
    wait_for_status(
        &mut child,
        &client,
        &format!("{base}/readyz"),
        StatusCode::SERVICE_UNAVAILABLE,
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(
        client
            .get(format!("{base}/healthz"))
            .send()
            .await
            .expect("liveness during drain")
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        client
            .get(format!("{base}/v1/whoami"))
            .send()
            .await
            .expect("new work during drain")
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );

    let deadline = Instant::now() + Duration::from_secs(17);
    loop {
        if let Some(status) = child.0.try_wait().expect("read gateway exit") {
            let stderr = child.stderr();
            assert!(status.success(), "gateway exited with {status}: {stderr}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "gateway exceeded its shutdown bound"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test]
async fn deployment_key_failures_keep_admission_closed_until_retry_succeeds() {
    let Some(database_url_file) = std::env::var_os("SYNVEDA_TEST_DATABASE_URL_FILE") else {
        eprintln!("skipping gateway key retry: run through `make db-test`");
        return;
    };
    let migrator_url_file = std::env::var_os("SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE")
        .expect("isolated fixture supplies the migrator URL");
    let migrator_url = std::fs::read_to_string(migrator_url_file).expect("read fixture URL");
    let migrator = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(migrator_url.trim())
        .await
        .expect("connect isolated migrator");
    let mut lock = migrator.begin().await.expect("begin lock transaction");
    sqlx::query("lock table deployment_keys in access exclusive mode")
        .execute(&mut *lock)
        .await
        .expect("hold key table until the first bounded attempt times out");

    let addr = free_loopback_addr();
    let mut child = spawn_gateway(Path::new(&database_url_file), &addr, Some(&"11".repeat(32)));
    let client = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_millis(200))
        .timeout(Duration::from_secs(2))
        .build()
        .expect("build loopback client");
    let base = format!("http://{addr}");
    wait_for_status(
        &mut child,
        &client,
        &format!("{base}/healthz"),
        StatusCode::OK,
        Duration::from_secs(10),
    )
    .await;
    let timeout_metric =
        synveda_gateway::telemetry::GATEWAY_DEPLOYMENT_KEY_PROVISION_ATTEMPTS_TOTAL;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let response = client
            .get(format!("{base}/metrics"))
            .send()
            .await
            .expect("read gateway metrics");
        let exposition = response.text().await.expect("metrics body");
        if exposition
            .lines()
            .any(|line| line.starts_with(timeout_metric) && line.contains("outcome=\"timeout\""))
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "bounded key attempt did not time out"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        client
            .get(format!("{base}/readyz"))
            .send()
            .await
            .expect("read readiness")
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    lock.rollback().await.expect("release key table");
    wait_for_status(
        &mut child,
        &client,
        &format!("{base}/readyz"),
        StatusCode::OK,
        Duration::from_secs(15),
    )
    .await;
    let exposition = client
        .get(format!("{base}/metrics"))
        .send()
        .await
        .expect("read recovered metrics")
        .text()
        .await
        .expect("metrics body");
    assert!(
        exposition
            .lines()
            .any(|line| { line.starts_with(timeout_metric) && line.contains("outcome=\"ok\"") })
    );
    assert!(exposition.lines().any(|line| {
        line.starts_with(synveda_gateway::telemetry::GATEWAY_DEPLOYMENT_KEY_READY)
            && line.ends_with(" 1")
    }));

    // A stored row is not enough: a different KEK cannot unwrap it and must
    // not make the new process ready merely because provision found a row.
    drop(child);
    let wrong_addr = free_loopback_addr();
    let mut wrong_key = spawn_gateway(
        Path::new(&database_url_file),
        &wrong_addr,
        Some(&"22".repeat(32)),
    );
    let wrong_base = format!("http://{wrong_addr}");
    wait_for_status(
        &mut wrong_key,
        &client,
        &format!("{wrong_base}/healthz"),
        StatusCode::OK,
        Duration::from_secs(10),
    )
    .await;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let exposition = client
            .get(format!("{wrong_base}/metrics"))
            .send()
            .await
            .expect("read wrong-key metrics")
            .text()
            .await
            .expect("metrics body");
        if exposition
            .lines()
            .any(|line| line.starts_with(timeout_metric) && line.contains("outcome=\"error\""))
        {
            break;
        }
        assert!(Instant::now() < deadline, "wrong KEK was not refused");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        client
            .get(format!("{wrong_base}/readyz"))
            .send()
            .await
            .expect("wrong-key readiness")
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}
