//! Gateway entry point. Configuration is environment-only for now:
//! `DATABASE_URL` (required), `SYNVEDA_LISTEN_ADDR` (default `127.0.0.1:8120`),
//! `SYNVEDA_DEV_JWT_SECRET` (the pre-AUTH-1 dev token secret, ADR-0008 —
//! unset means every `/v1` request is rejected), and the standard `OTEL_*`
//! variables for the OTLP exporter (default endpoint
//! `http://localhost:4317` — Jaeger in the dev compose).

use std::sync::Arc;

use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{self, AppState};
use synveda_gateway::telemetry;
use synveda_identity::{DisabledVerifier, Hs256Verifier, TokenVerifier};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let telemetry = telemetry::init("synveda-gateway")?;
    let metrics = telemetry::init_metrics()?;

    let database_url = std::env::var("DATABASE_URL")
        .map_err(|_| "DATABASE_URL must be set (dev default is in the Makefile)")?;
    // connect_lazy: the gateway boots without a database so /readyz can
    // report the outage instead of the process crash-looping.
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect_lazy(&database_url)?;

    // Fail closed (ADR-0008): without a secret the /v1 plane rejects
    // everything rather than admitting anything. AUTH-1 replaces this with
    // OIDC/JWKS verification behind the same trait.
    let verifier: Arc<dyn TokenVerifier> = match std::env::var("SYNVEDA_DEV_JWT_SECRET") {
        Ok(secret) if !secret.is_empty() => Arc::new(Hs256Verifier::new(secret.as_bytes())),
        _ => {
            tracing::warn!(
                "SYNVEDA_DEV_JWT_SECRET is not set; every /v1 request will be rejected 401"
            );
            Arc::new(DisabledVerifier)
        }
    };

    let addr = std::env::var("SYNVEDA_LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:8120".to_owned());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "synveda-gateway listening");

    axum::serve(
        listener,
        app::router(AppState {
            pool,
            metrics,
            verifier,
        }),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    // Flush batched spans before exit; a killed process loses the tail.
    telemetry.shutdown();
    Ok(())
}

/// Ctrl-C covers dev on every platform; SIGTERM handling arrives with the
/// deployment profiles (OPS-1).
async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        tracing::error!(error = %err, "failed to install the Ctrl-C handler");
    }
}
