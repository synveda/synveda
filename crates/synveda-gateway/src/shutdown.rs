//! Process signal handling shared by the gateway and core worker.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Maximum time for a routing probe to observe withdrawn gateway readiness.
const GATEWAY_PROBE_DRAIN_WINDOW: Duration = Duration::from_secs(10);

/// One-way request admission for process shutdown, separate from authority.
///
/// A normal drain refuses new work but lets already-admitted requests finish
/// under their live authority permits. Closing the authority gate instead
/// would cancel those requests and is reserved for an actual authority loss.
#[derive(Clone)]
pub struct GatewayAdmission {
    accepting: Arc<AtomicBool>,
}

impl Default for GatewayAdmission {
    fn default() -> Self {
        Self::new()
    }
}

impl GatewayAdmission {
    /// Starts an accepting gateway process.
    #[must_use]
    pub fn new() -> Self {
        Self {
            accepting: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Permanently refuses new application work and readiness.
    pub fn withdraw(&self) {
        self.accepting.store(false, Ordering::Release);
    }

    /// Whether the process may admit a new application request.
    #[must_use]
    pub fn is_accepting(&self) -> bool {
        self.accepting.load(Ordering::Acquire)
    }
}

/// Leaves one cooperative second and one forced-join second inside the bound.
#[must_use]
pub fn gateway_probe_drain_window(shutdown_grace: Duration) -> Duration {
    GATEWAY_PROBE_DRAIN_WINDOW.min(shutdown_grace.saturating_sub(Duration::from_secs(2)))
}

/// Waits for the first supported process-termination signal.
///
/// Failure to install one handler does not make the process exit as though a
/// shutdown had been requested; the other supported signal remains live.
pub async fn signal() {
    #[cfg(unix)]
    tokio::select! {
        () = wait_for_ctrl_c() => tracing::info!(signal = "SIGINT", "shutdown requested"),
        () = wait_for_sigterm() => tracing::info!(signal = "SIGTERM", "shutdown requested"),
    }

    #[cfg(not(unix))]
    {
        wait_for_ctrl_c().await;
        tracing::info!(signal = "SIGINT", "shutdown requested");
    }
}

/// Completes when a cooperative worker-stop channel is set or closed.
pub async fn requested(shutdown: &mut tokio::sync::watch::Receiver<bool>) {
    loop {
        if *shutdown.borrow() || shutdown.changed().await.is_err() {
            return;
        }
    }
}

async fn wait_for_ctrl_c() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to install the Ctrl-C handler");
        std::future::pending::<()>().await;
    }
}

#[cfg(unix)]
async fn wait_for_sigterm() {
    let mut signal = match install_sigterm() {
        Ok(signal) => signal,
        Err(error) => {
            tracing::error!(%error, "failed to install the SIGTERM handler");
            std::future::pending::<()>().await;
            return;
        }
    };
    if signal.recv().await.is_none() {
        tracing::error!("SIGTERM handler closed without receiving a signal");
        std::future::pending::<()>().await;
    }
}

#[cfg(unix)]
fn install_sigterm() -> std::io::Result<tokio::signal::unix::Signal> {
    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
}

#[cfg(all(test, unix))]
mod tests {
    use std::process::{Child, Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn gateway_admission_withdrawal_is_shared_and_one_way() {
        let admission = GatewayAdmission::new();
        let replica = admission.clone();
        assert!(replica.is_accepting());
        admission.withdraw();
        assert!(!replica.is_accepting());
        replica.withdraw();
        assert!(!admission.is_accepting());
    }

    #[test]
    fn gateway_probe_window_preserves_cleanup_and_abort_reserve() {
        assert_eq!(
            gateway_probe_drain_window(Duration::from_secs(30)),
            Duration::from_secs(10)
        );
        assert_eq!(
            gateway_probe_drain_window(Duration::from_secs(8)),
            Duration::from_secs(6)
        );
        assert_eq!(
            gateway_probe_drain_window(Duration::from_secs(2)),
            Duration::ZERO
        );
    }

    struct ChildGuard(Child);

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn sigterm_is_delivered_to_the_shared_handler() {
        const CHILD_READY: &str = "SYNVEDA_SIGTERM_TEST_READY";
        if let Some(path) = std::env::var_os(CHILD_READY) {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build signal test runtime");
            runtime.block_on(async {
                let mut signal = install_sigterm().expect("install SIGTERM handler");
                std::fs::write(path, b"ready").expect("publish handler readiness");
                assert!(signal.recv().await.is_some(), "receive SIGTERM");
            });
            return;
        }

        let ready = std::env::temp_dir().join(format!(
            "synveda-sigterm-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let child = Command::new(std::env::current_exe().expect("locate test binary"))
            .args([
                "--exact",
                "shutdown::tests::sigterm_is_delivered_to_the_shared_handler",
                "--nocapture",
            ])
            .env(CHILD_READY, &ready)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start signal test child");
        let mut child = ChildGuard(child);
        let ready_deadline = Instant::now() + Duration::from_secs(5);
        while !ready.exists() {
            if let Some(status) = child.0.try_wait().expect("read child status") {
                panic!("signal test child exited before readiness: {status}");
            }
            assert!(
                Instant::now() < ready_deadline,
                "signal handler was not ready"
            );
            thread::sleep(Duration::from_millis(10));
        }

        let sent = Command::new("kill")
            .args(["-TERM", &child.0.id().to_string()])
            .status()
            .expect("send SIGTERM");
        assert!(sent.success(), "kill -TERM failed");

        let exit_deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match child.0.try_wait().expect("read child status") {
                Some(status) => {
                    let _ = std::fs::remove_file(&ready);
                    assert!(status.success(), "signal test child exited with {status}");
                    break;
                }
                None if Instant::now() < exit_deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                None => panic!("signal test child did not exit after SIGTERM"),
            }
        }
    }
}
