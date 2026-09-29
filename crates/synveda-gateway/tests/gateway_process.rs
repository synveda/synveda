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

fn spawn_gateway(database_url_file: &Path, addr: &str) -> ChildGuard {
    let child = Command::new(env!("CARGO_BIN_EXE_synveda-gateway"))
        .env_remove("DATABASE_URL")
        .env("DATABASE_URL_FILE", database_url_file)
        .env_remove("SYNVEDA_DATABASE_ROLES")
        .env("SYNVEDA_LISTEN_ADDR", addr)
        .env("SYNVEDA_PUBLIC_URL", format!("http://{addr}"))
        .env_remove("SYNVEDA_PUBLIC_URL_FILE")
        .env("SYNVEDA_GATEWAY_SHUTDOWN_SECS", "15")
        .env("SYNVEDA_POLICY_REFRESH_SECS", "3600")
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
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn synveda-gateway");
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
    let mut child = spawn_gateway(Path::new(&database_url_file), &addr);
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
