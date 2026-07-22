//! Gateway entry point. Configuration is environment-only for now:
//! `DATABASE_URL` (required), `SYNVEDA_LISTEN_ADDR` (default `127.0.0.1:8120`),
//! and one auth mode (ADR-0010 — setting both is a startup error):
//! `SYNVEDA_OIDC_ISSUERS` (JSON trust-entry array; enables OIDC verification
//! and `/auth/*`, with `SYNVEDA_PUBLIC_URL` naming this gateway in redirect
//! URIs, default `http://127.0.0.1:8120`) or `SYNVEDA_DEV_JWT_SECRET` (the
//! HS256 dev mode, ADR-0008). Neither set means every `/v1` request is
//! rejected. `SYNVEDA_POLICY_REFRESH_SECS` (default 5) paces the policy
//! pack refresher (AUTHZ-1, ADR-0012). `SYNVEDA_SERVICE_TOKEN_MAX_TTL_SECS`
//! (default 3600) caps service identities' token lifetime at the
//! enforcement seam (AUTH-3, ADR-0018).
//!
//! The extraction worker (MEM-3, ADR-0022) is selected by
//! `SYNVEDA_EXTRACTOR` (`deterministic` [default] | `claude` | `vllm` |
//! `off`). `claude` requires `ANTHROPIC_API_KEY` (never logged) and
//! honours `SYNVEDA_ANTHROPIC_BASE_URL` (default
//! `https://api.anthropic.com`); `vllm` requires `SYNVEDA_VLLM_BASE_URL`
//! and `SYNVEDA_EXTRACTOR_MODEL`; `SYNVEDA_EXTRACTOR_MODEL` otherwise
//! defaults per implementation. Worker pacing:
//! `SYNVEDA_EXTRACTION_POLL_MS` (default 1000),
//! `SYNVEDA_EXTRACTION_BATCH` (default 16),
//! `SYNVEDA_EXTRACTION_VT_SECS` (default 60),
//! `SYNVEDA_EXTRACTION_MAX_READS` (default 5).
//!
//! The standard `OTEL_*` variables configure the OTLP exporter (default
//! endpoint `http://localhost:4317` — Jaeger in the dev compose).

use std::sync::Arc;
use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{self, AppState};
use synveda_gateway::{authz, telemetry};
use synveda_identity::{DisabledVerifier, Hs256Verifier, LoginFlow, OidcVerifier, TokenVerifier};
use synveda_ingest::extraction::Extractor as _;
use synveda_policy::Pdp;

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

    // One auth mode, never two (ADR-0010); fail closed when neither is
    // configured (ADR-0008).
    let oidc_issuers = std::env::var("SYNVEDA_OIDC_ISSUERS")
        .ok()
        .filter(|v| !v.is_empty());
    let dev_secret = std::env::var("SYNVEDA_DEV_JWT_SECRET")
        .ok()
        .filter(|v| !v.is_empty());
    let (verifier, login): (Arc<dyn TokenVerifier>, Option<Arc<LoginFlow>>) =
        match (oidc_issuers, dev_secret) {
            (Some(_), Some(_)) => {
                return Err(
                    "SYNVEDA_OIDC_ISSUERS and SYNVEDA_DEV_JWT_SECRET are mutually \
                            exclusive (ADR-0010): configure exactly one auth mode"
                        .into(),
                );
            }
            (Some(json), None) => {
                let issuers = synveda_identity::parse_issuers(&json)?;
                let public_url = std::env::var("SYNVEDA_PUBLIC_URL")
                    .unwrap_or_else(|_| "http://127.0.0.1:8120".to_owned());
                let redirect_uri = format!("{}/auth/callback", public_url.trim_end_matches('/'));
                let oidc = Arc::new(OidcVerifier::new(issuers)?);
                tracing::info!(
                    redirect_uri,
                    issuers = %oidc.issuers().collect::<Vec<_>>().join(", "),
                    "OIDC auth mode (ADR-0010): /v1 accepts IdP-issued bearer tokens"
                );
                let flow = Arc::new(LoginFlow::new(Arc::clone(&oidc), redirect_uri));
                (oidc, Some(flow))
            }
            (None, Some(secret)) => {
                tracing::warn!("HS256 dev auth mode (ADR-0008): dev/demo only, never a deployment");
                (Arc::new(Hs256Verifier::new(secret.as_bytes())), None)
            }
            (None, None) => {
                tracing::warn!("no auth mode configured; every /v1 request will be rejected 401");
                (Arc::new(DisabledVerifier), None)
            }
        };

    // The embedded PDP (AUTHZ-1, ADR-0012): failure here means the binary's
    // own schema or an embedded product pack is broken — refuse to boot.
    let pdp = Arc::new(Pdp::new()?);
    let refresh_secs = match std::env::var("SYNVEDA_POLICY_REFRESH_SECS") {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|_| "SYNVEDA_POLICY_REFRESH_SECS must be a positive integer")?,
        Err(_) => 5,
    };
    let refresher = authz::spawn_pack_refresher(
        pool.clone(),
        Arc::clone(&pdp),
        Duration::from_secs(refresh_secs.max(1)),
    );

    // The service-token lifetime cap (AUTH-3, ADR-0018 decision 5).
    let service_token_max_ttl_secs = match std::env::var("SYNVEDA_SERVICE_TOKEN_MAX_TTL_SECS") {
        Ok(value) => value
            .parse::<u64>()
            .ok()
            .filter(|secs| *secs > 0)
            .ok_or("SYNVEDA_SERVICE_TOKEN_MAX_TTL_SECS must be a positive integer")?,
        Err(_) => 3600,
    };

    // The extraction worker (MEM-3, ADR-0022 decision 1): the observe
    // queue's consumer, embedded so SMB mode stays one process. It shares
    // the gateway's scope-chain cache — hierarchy-move invalidations must
    // reach the worker's authorization reads.
    let scope_chains = Arc::new(synveda_store::ScopeChainCache::new());
    let extractor = extractor_from_env()?;
    let extraction_worker = extractor.map(|extractor| {
        tracing::info!(
            extractor = extractor.method(),
            "extraction worker starting (MEM-3, ADR-0022)"
        );
        synveda_ingest::worker::spawn(
            synveda_ingest::worker::WorkerDeps {
                pool: pool.clone(),
                pdp: Arc::clone(&pdp),
                chains: Arc::clone(&scope_chains),
                extractor,
            },
            extraction_config_from_env(),
        )
    });
    if extraction_worker.is_none() {
        tracing::warn!("SYNVEDA_EXTRACTOR=off: observe signals will accumulate unconsumed");
    }

    let addr = std::env::var("SYNVEDA_LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:8120".to_owned());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "synveda-gateway listening");

    axum::serve(
        listener,
        app::router(AppState {
            pool,
            metrics,
            verifier,
            login,
            pdp,
            scope_chains,
            service_token_max_ttl: Duration::from_secs(service_token_max_ttl_secs),
        }),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    refresher.abort();
    if let Some(worker) = extraction_worker {
        worker.abort();
    }

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

/// Builds the configured extractor from `SYNVEDA_EXTRACTOR` and its
/// companions (module header). `None` means `off`. Misconfiguration is a
/// startup error: a silently-idle pipeline would break the <60s lag SLO
/// without a symptom.
fn extractor_from_env() -> Result<Option<synveda_ingest::extraction::AnyExtractor>, String> {
    use synveda_ingest::extraction::{
        AnyExtractor, ClaudeExtractor, DeterministicExtractor, VllmExtractor,
    };
    let selected = std::env::var("SYNVEDA_EXTRACTOR")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "deterministic".to_owned());
    let model = std::env::var("SYNVEDA_EXTRACTOR_MODEL")
        .ok()
        .filter(|value| !value.is_empty());
    match selected.as_str() {
        "off" => Ok(None),
        "deterministic" => Ok(Some(AnyExtractor::Deterministic(
            DeterministicExtractor::new(),
        ))),
        "claude" => {
            let api_key = std::env::var("ANTHROPIC_API_KEY")
                .ok()
                .filter(|value| !value.is_empty())
                .ok_or("SYNVEDA_EXTRACTOR=claude requires ANTHROPIC_API_KEY")?;
            let base_url = std::env::var("SYNVEDA_ANTHROPIC_BASE_URL")
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| ClaudeExtractor::DEFAULT_BASE_URL.to_owned());
            let model = model.unwrap_or_else(|| ClaudeExtractor::DEFAULT_MODEL.to_owned());
            Ok(Some(AnyExtractor::Claude(ClaudeExtractor::new(
                api_key, model, base_url,
            ))))
        }
        "vllm" => {
            let base_url = std::env::var("SYNVEDA_VLLM_BASE_URL")
                .ok()
                .filter(|value| !value.is_empty())
                .ok_or("SYNVEDA_EXTRACTOR=vllm requires SYNVEDA_VLLM_BASE_URL")?;
            let model = model.ok_or("SYNVEDA_EXTRACTOR=vllm requires SYNVEDA_EXTRACTOR_MODEL")?;
            Ok(Some(AnyExtractor::Vllm(VllmExtractor::new(
                model, base_url,
            ))))
        }
        other => Err(format!(
            "SYNVEDA_EXTRACTOR must be off|deterministic|claude|vllm, got {other:?}"
        )),
    }
}

/// Worker pacing from `SYNVEDA_EXTRACTION_*`, with the defaults the
/// module header documents. Unparseable values fall back to defaults:
/// pacing is tuning, never a fail-closed control.
fn extraction_config_from_env() -> synveda_ingest::worker::WorkerConfig {
    let defaults = synveda_ingest::worker::WorkerConfig::default();
    let parse = |name: &str| {
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|value| *value > 0)
    };
    synveda_ingest::worker::WorkerConfig {
        poll_interval: parse("SYNVEDA_EXTRACTION_POLL_MS")
            .map(|ms| Duration::from_millis(ms as u64))
            .unwrap_or(defaults.poll_interval),
        batch: parse("SYNVEDA_EXTRACTION_BATCH")
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(defaults.batch),
        vt_secs: parse("SYNVEDA_EXTRACTION_VT_SECS")
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(defaults.vt_secs),
        max_reads: parse("SYNVEDA_EXTRACTION_MAX_READS")
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(defaults.max_reads),
    }
}
