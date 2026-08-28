//! Process signal handling shared by the gateway and core worker.

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
pub(crate) async fn requested(shutdown: &mut tokio::sync::watch::Receiver<bool>) {
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
