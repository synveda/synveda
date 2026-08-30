//! Provider-neutral OIDC discovery and JWKS deployment preflight.
//!
//! This process has no database or provider-administration authority. It
//! consumes the same issuer and public-origin configuration as the gateway
//! and exits only after every configured issuer proves the exact bounded
//! discovery and signing-key metadata contract (CPR-45, ADR-0102). A real
//! authorization-code/PKCE exchange remains an acceptance-test concern.

#![forbid(unsafe_code)]

use std::process::ExitCode;
use std::time::Duration;

const RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
const EX_CONFIG: u8 = 78;
const EX_TEMPFAIL: u8 = 75;

enum DiagnosticOutcome {
    Passed(usize),
    Refused,
}

fn main() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            eprintln!("OIDC diagnostic runtime was unavailable");
            return ExitCode::from(EX_TEMPFAIL);
        }
    };
    let outcome = runtime.block_on(run());
    // Even if a transitive dependency accidentally reintroduces blocking
    // work, diagnostic process teardown remains bounded. The shipped reqwest
    // client also uses async DNS, so ordinary operation leaves no such task.
    runtime.shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT);
    outcome
}

async fn run() -> ExitCode {
    let outcome = match load_contract() {
        Ok(verifier) => match verifier.initialize().await {
            Ok(()) => DiagnosticOutcome::Passed(verifier.issuers().count()),
            Err(error) if error.is_retryable() => {
                eprintln!(
                    "OIDC {} diagnostic remained unavailable through its deployment deadline",
                    error.stage()
                );
                return ExitCode::from(EX_TEMPFAIL);
            }
            Err(error) => {
                eprintln!("{error}");
                DiagnosticOutcome::Refused
            }
        },
        Err(()) => DiagnosticOutcome::Refused,
    };

    match outcome {
        DiagnosticOutcome::Passed(count) => {
            eprintln!("OIDC diagnostic passed for {count} configured issuer(s)");
            ExitCode::SUCCESS
        }
        DiagnosticOutcome::Refused => {
            eprintln!("OIDC diagnostic configuration or provider contract was refused");
            ExitCode::from(EX_CONFIG)
        }
    }
}

fn load_contract() -> Result<synveda_identity::OidcVerifier, ()> {
    // The gateway and this one-shot must derive the identical credential-free
    // callback even though the diagnostic never opens a browser itself.
    synveda_gateway::runtime_config::public_application_url().map_err(|_| ())?;
    let json = synveda_gateway::runtime_config::required_setting("SYNVEDA_OIDC_ISSUERS")
        .map_err(|_| ())?;
    let issuers = synveda_identity::parse_issuers(&json).map_err(|_| ())?;
    synveda_gateway::runtime_config::validate_oidc_directory_references(&issuers)
        .map_err(|_| ())?;
    let expected =
        synveda_gateway::runtime_config::required_setting("SYNVEDA_OIDC_EXPECTED_ISSUER")
            .map_err(|_| ())?;
    let contract = synveda_identity::OidcVerifier::new_with_insecure_development_http(
        issuers.clone(),
        synveda_gateway::runtime_config::insecure_development_http_enabled().map_err(|_| ())?,
    )
    .map_err(|_| ())?;
    if contract.sole_issuer() != Some(expected.as_str()) {
        return Err(());
    }
    Ok(contract)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_shutdown_does_not_wait_for_blocking_work() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        runtime.spawn_blocking(move || {
            started_tx.send(()).expect("report blocking task start");
            std::thread::sleep(Duration::from_millis(200));
        });
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("blocking task started");

        let started = std::time::Instant::now();
        runtime.shutdown_timeout(Duration::from_millis(5));
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "runtime shutdown waited for non-cancellable blocking work"
        );
    }
}
