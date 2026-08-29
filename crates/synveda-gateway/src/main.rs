//! Gateway entry point. Configuration is environment-only for now:
//! `DATABASE_URL` (required), `SYNVEDA_DB_MAX_CONNECTIONS` (default 8),
//! `SYNVEDA_LISTEN_ADDR` (default
//! `127.0.0.1:8120`),
//! and one auth mode (ADR-0010 — setting both is a startup error):
//! `SYNVEDA_OIDC_ISSUERS` (JSON trust-entry array; enables OIDC verification
//! and `/auth/*`, with `SYNVEDA_PUBLIC_URL` naming this gateway in redirect
//! URIs, default `http://127.0.0.1:8120`) or `SYNVEDA_DEV_JWT_SECRET` (the
//! HS256 dev mode, ADR-0008). Neither set means every `/v1` request is
//! rejected. `SYNVEDA_POLICY_REFRESH_SECS` (default 5, range 1..=3600) paces
//! the policy pack refresher (AUTHZ-1, ADR-0012).
//! `SYNVEDA_SERVICE_TOKEN_MAX_TTL_SECS`
//! (default 3600) caps service identities' token lifetime at the
//! enforcement seam (AUTH-3, ADR-0018).
//!
//! Background Capture, Knowledge indexing, relaxation expiry and directory
//! pull run only in `synveda-worker` (CPR-45, ADR-0102). The gateway retains
//! synchronous request work. Its embedder is selected by `SYNVEDA_EMBEDDER`
//! (`deterministic` [default] | `tei` —
//! deliberately no `off`: embed-or-fail is unconditional); `tei`
//! requires `SYNVEDA_TEI_URL` (the dev compose serves
//! `http://localhost:8110`) and honours `SYNVEDA_EMBEDDER_MODEL`
//! (default `BAAI/bge-m3`).
//!
//! Context planning embeds the caller's task through
//! the same configured embedder under `SYNVEDA_CONTEXT_EMBED_TIMEOUT_MS`
//! (default 100): expiry or failure degrades the run to lexical Knowledge
//! search and is persisted, never hidden.
//!
//! The standard `OTEL_*` variables configure the OTLP exporter (default
//! endpoint `http://localhost:4317` — Jaeger in the dev compose).

#[cfg(all(feature = "test-support", not(test), not(debug_assertions)))]
compile_error!("the gateway release binary cannot include the test-support feature");

use std::sync::Arc;
use std::time::Duration;

use synveda_gateway::app::{self, AppState};
use synveda_gateway::authority::{self, AuthorityGate, AuthorityMonitor, CheckOutcome};
use synveda_gateway::{authz, runtime_config, shutdown, telemetry};
use synveda_identity::{DisabledVerifier, Hs256Verifier, LoginFlow, OidcVerifier, TokenVerifier};
use synveda_ingest::embedding::Embedder as _;
use synveda_policy::Pdp;

const SHUTDOWN_ABORT_RESERVE: Duration = Duration::from_secs(1);
const BACKGROUND_RETRY_INTERVAL: Duration = Duration::from_secs(2);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let telemetry = telemetry::init("synveda-gateway")?;
    let metrics = telemetry::init_metrics()?;

    let database_url = runtime_config::required_setting("DATABASE_URL")?;
    // The gateway and worker have distinct bounded pools. Deployment
    // profiles budget both against PostgreSQL `max_connections` (ADR-0062).
    let max_connections =
        runtime_config::positive_connection_limit("SYNVEDA_DB_MAX_CONNECTIONS", 8)?;
    let database_roles = runtime_config::database_roles()?;
    let connect_options = synveda_store::database_url::parse("DATABASE_URL", &database_url)?
        .application_name("synveda-gateway");
    let database_role = connect_options.get_username();
    if database_role != database_roles.gateway() {
        return Err("the gateway database authority was conclusively refused".into());
    }
    // connect_lazy: the gateway boots without a database so /readyz can
    // report the outage instead of the process crash-looping.
    let (pool_options, pool_refusal) = runtime_config::runtime_pool_options(
        max_connections,
        authority::CHECK_TIMEOUT,
        database_roles.gateway().to_owned(),
    );
    let pool = pool_options.connect_lazy_with(connect_options);

    // The gateway's own public URL. Read once here rather than inside the
    // OIDC arm, because CNSL-1's Origin check needs it in every auth mode:
    // a console session is refused under a dev verifier too, and it is
    // refused for the right reason rather than because nothing was
    // configured to compare against.
    let public_url = std::env::var("SYNVEDA_PUBLIC_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8120".to_owned())
        .trim_end_matches('/')
        .to_owned();
    // `Origin` is scheme://host[:port] and never carries a path, so a
    // public URL that has one would never match. Derived rather than
    // demanded as a second setting: two settings that must agree are two
    // settings that will not (ADR-0055's second finding, one layer down).
    let public_origin = url::Url::parse(&public_url)
        .ok()
        .and_then(|url| {
            url.origin()
                .is_tuple()
                .then(|| url.origin().ascii_serialization())
        })
        .ok_or("SYNVEDA_PUBLIC_URL must be an absolute http(s) URL")?;

    // One auth mode, never two (ADR-0010); fail closed when neither is
    // configured (ADR-0008).
    let oidc_issuers =
        runtime_config::setting("SYNVEDA_OIDC_ISSUERS")?.filter(|value| !value.is_empty());
    let dev_secret =
        runtime_config::setting("SYNVEDA_DEV_JWT_SECRET")?.filter(|value| !value.is_empty());
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
                let redirect_uri = format!("{public_url}/auth/callback");
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
    let refresh_interval =
        runtime_config::bounded_duration_setting("SYNVEDA_POLICY_REFRESH_SECS", 5, 1, 3_600)?;
    let shutdown_grace =
        runtime_config::bounded_duration_setting("SYNVEDA_GATEWAY_SHUTDOWN_SECS", 30, 2, 300)?;

    // The service-token lifetime cap (AUTH-3, ADR-0018 decision 5).
    let service_token_max_ttl_secs = match std::env::var("SYNVEDA_SERVICE_TOKEN_MAX_TTL_SECS") {
        Ok(value) => value
            .parse::<u64>()
            .ok()
            .filter(|secs| *secs > 0)
            .ok_or("SYNVEDA_SERVICE_TOKEN_MAX_TTL_SECS must be a positive integer")?,
        Err(_) => 3600,
    };

    // The key plane (TEN-4, ADR-0064). `Kms::Disabled` when no KEK is
    // configured, which is fail-closed rather than fail-to-boot: `/v1`
    // bearer traffic never touches a sealed column, so a deployment that has
    // not set a key keeps serving and the surfaces that need one say which
    // key is missing.
    let keys = Arc::new(synveda_store::keys::KeyRing::new(
        runtime_config::kms_from_env()?,
    ));
    let deployment_key_ref = match keys.kms() {
        synveda_crypto::Kms::Disabled => {
            tracing::warn!(
                "no SYNVEDA_KMS_KEY: console sessions and per-tenant secrets are \
                 unavailable (TEN-4, ADR-0064). `synveda kms keygen` mints one."
            );
            None
        }
        kms => Some(synveda_crypto::KeyManagement::key_ref(kms).to_owned()),
    };

    // Request-time context planning keeps the same explicit embedder identity
    // as the worker's Knowledge indexer without running that loop here.
    let embedder = Arc::new(runtime_config::embedder_from_env()?);
    tracing::info!(
        embedder = embedder.method(),
        embedding_model = embedder.model(),
        "request-time Knowledge embedder ready"
    );

    // The context planner's embedding deadline. Failure is an explicit
    // lexical degradation recorded on the ContextRun.
    let context_embed_timeout_ms = std::env::var("SYNVEDA_CONTEXT_EMBED_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|ms| *ms > 0)
        .unwrap_or(100);

    // Request-plane state only. Timed background work constructs narrower
    // dependencies in `synveda-worker`; it does not construct `LoginFlow` or
    // public-origin state. The worker reads issuer entries only to configure
    // optional tenant-bound directory connectors.
    let app_state = AppState {
        pool: pool.clone(),
        metrics,
        verifier,
        login,
        public_origin,
        pdp: Arc::clone(&pdp),
        service_token_max_ttl: Duration::from_secs(service_token_max_ttl_secs),
        embedder,
        context_embed_timeout: Duration::from_millis(context_embed_timeout_ms),
        keys: Arc::clone(&keys),
    };

    let authority = AuthorityMonitor::new(
        pool.clone(),
        pool_refusal,
        database_roles.gateway().to_owned(),
        database_roles.clone(),
    );
    let gate = authority.gate();
    let addr = std::env::var("SYNVEDA_LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:8120".to_owned());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "synveda-gateway listening");

    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let mut http_stop = stop_rx.clone();
    let http_gate = gate.clone();
    let mut server = tokio::spawn(async move {
        axum::serve(listener, app::governed_router(app_state, http_gate))
            .with_graceful_shutdown(async move {
                while !*http_stop.borrow() && http_stop.changed().await.is_ok() {}
            })
            .await
    });
    let mut sentinel = tokio::spawn(authority::run_sentinel(authority, stop_rx.clone()));
    let mut background = tokio::spawn(run_gateway_background(
        pool.clone(),
        Arc::clone(&pdp),
        Arc::clone(&keys),
        deployment_key_ref,
        refresh_interval,
        gate.clone(),
        stop_rx.clone(),
    ));
    let mut pool_monitor = tokio::spawn(run_pool_monitor(pool.clone(), max_connections, stop_rx));
    let signal = shutdown::signal();
    tokio::pin!(signal);

    enum ExitReason {
        Signal,
        Sentinel(Result<CheckOutcome, tokio::task::JoinError>),
        Background(Result<Result<(), String>, tokio::task::JoinError>),
        Pool,
        Server(Result<Result<(), std::io::Error>, tokio::task::JoinError>),
    }

    let reason = tokio::select! {
        () = &mut signal => ExitReason::Signal,
        result = &mut sentinel => ExitReason::Sentinel(result),
        result = &mut background => ExitReason::Background(result),
        _result = &mut pool_monitor => ExitReason::Pool,
        result = &mut server => ExitReason::Server(result),
    };

    let _ = stop_tx.send(true);
    let mut sentinel_finished = matches!(&reason, ExitReason::Sentinel(_));
    let mut background_finished = matches!(&reason, ExitReason::Background(_));
    let mut pool_monitor_finished = matches!(&reason, ExitReason::Pool);
    let mut server_finished = matches!(&reason, ExitReason::Server(_));
    let mut cleanup_error = None;
    let cleanup_grace = shutdown_grace.saturating_sub(SHUTDOWN_ABORT_RESERVE);
    let cleanup = tokio::time::timeout(cleanup_grace, async {
        if !sentinel_finished {
            let _ = (&mut sentinel).await;
            sentinel_finished = true;
        }
        if !background_finished {
            match (&mut background).await {
                Ok(Ok(())) => {}
                _ => cleanup_error = Some("gateway background supervisor failed during shutdown"),
            }
            background_finished = true;
        }
        if !pool_monitor_finished {
            let _ = (&mut pool_monitor).await;
            pool_monitor_finished = true;
        }
        if !server_finished {
            match (&mut server).await {
                Ok(Ok(())) => {}
                _ => cleanup_error = Some("gateway HTTP supervisor failed during shutdown"),
            }
            server_finished = true;
        }
        pool.close().await;
    })
    .await;
    if cleanup.is_err() {
        if !sentinel_finished {
            sentinel.abort();
        }
        if !background_finished {
            background.abort();
        }
        if !pool_monitor_finished {
            pool_monitor.abort();
        }
        if !server_finished {
            server.abort();
        }
        let _ = tokio::time::timeout(SHUTDOWN_ABORT_RESERVE, async {
            if !sentinel_finished {
                let _ = sentinel.await;
            }
            if !background_finished {
                let _ = background.await;
            }
            if !pool_monitor_finished {
                let _ = pool_monitor.await;
            }
            if !server_finished {
                let _ = server.await;
            }
        })
        .await;
        cleanup_error = Some("gateway shutdown reached its hard deadline");
        tracing::warn!(
            grace_secs = shutdown_grace.as_secs(),
            "gateway shutdown reached its hard deadline; unfinished supervisors were cancelled"
        );
    }

    // Flush batched spans before exit; a killed process loses the tail.
    telemetry.shutdown();
    if let Some(error) = cleanup_error {
        return Err(error.into());
    }
    match reason {
        ExitReason::Signal => Ok(()),
        ExitReason::Sentinel(Ok(CheckOutcome::Refused { .. })) => {
            Err("the gateway database authority was conclusively refused".into())
        }
        ExitReason::Sentinel(_) => {
            Err("the gateway authority sentinel stopped unexpectedly".into())
        }
        ExitReason::Background(Ok(Err(reason))) => Err(reason.into()),
        ExitReason::Background(_) => {
            Err("the gateway background supervisor stopped unexpectedly".into())
        }
        ExitReason::Pool => Err("the gateway pool monitor stopped unexpectedly".into()),
        ExitReason::Server(Ok(Ok(()))) => {
            Err("the gateway HTTP server stopped unexpectedly".into())
        }
        ExitReason::Server(Ok(Err(error))) => Err(error.into()),
        ExitReason::Server(Err(error)) => {
            tracing::error!(%error, "gateway HTTP supervisor stopped unexpectedly");
            Err("the gateway HTTP supervisor stopped unexpectedly".into())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BackgroundGenerationEnd {
    AuthorityClosed,
    AuthorityRefused,
    Shutdown,
}

async fn run_gateway_background(
    pool: sqlx::PgPool,
    pdp: Arc<Pdp>,
    keys: Arc<synveda_store::keys::KeyRing>,
    deployment_key_ref: Option<String>,
    refresh_interval: Duration,
    gate: AuthorityGate,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<(), String> {
    loop {
        let generation = tokio::select! {
            biased;
            () = shutdown::requested(&mut shutdown) => return Ok(()),
            generation = gate.wait_until_open() => match generation {
                Ok(generation) => generation,
                Err(()) if gate.is_terminal() => {
                    return Err("the gateway database authority was conclusively refused".to_owned());
                }
                Err(()) => return Err("the gateway authority gate lost its sentinel".to_owned()),
            },
        };
        let mut permit = gate.permit();
        if !permit.is_for(generation) {
            continue;
        }

        loop {
            let convergence = tokio::time::timeout(
                authority::CHECK_TIMEOUT,
                authz::converge_packs_once(&pool, &pdp),
            );
            tokio::pin!(convergence);
            let converged = tokio::select! {
                biased;
                () = permit.revoked() => false,
                () = shutdown::requested(&mut shutdown) => return Ok(()),
                result = &mut convergence => matches!(result, Ok(Ok(()))),
            };
            if !permit.is_open() {
                break;
            }
            if converged {
                break;
            }
            tracing::warn!("gateway policy-pack convergence unavailable");
            tokio::select! {
                biased;
                () = permit.revoked() => break,
                () = shutdown::requested(&mut shutdown) => return Ok(()),
                () = tokio::time::sleep(BACKGROUND_RETRY_INTERVAL) => {}
            }
        }
        if !permit.is_for(generation) {
            if gate.is_terminal() {
                return Err("the gateway database authority was conclusively refused".to_owned());
            }
            continue;
        }

        if let Some(key_ref) = deployment_key_ref.as_deref() {
            let provision = tokio::time::timeout(
                authority::CHECK_TIMEOUT,
                keys.provision(&pool, synveda_crypto::KeyScope::Deployment),
            );
            tokio::pin!(provision);
            let result = tokio::select! {
                biased;
                () = permit.revoked() => None,
                () = shutdown::requested(&mut shutdown) => return Ok(()),
                result = &mut provision => Some(result),
            };
            match result {
                None if gate.is_terminal() => {
                    return Err(
                        "the gateway database authority was conclusively refused".to_owned()
                    );
                }
                None => continue,
                Some(Ok(Ok(version))) => tracing::info!(
                    key.version = version.get(),
                    kek.ref = key_ref,
                    authority.generation = generation,
                    "deployment encryption key ready (TEN-4, ADR-0064)"
                ),
                Some(Ok(Err(_))) => {
                    tracing::warn!("deployment encryption key provisioning unavailable")
                }
                Some(Err(_)) => tracing::warn!(
                    timeout_ms = authority::CHECK_TIMEOUT.as_millis() as u64,
                    "deployment encryption key provisioning timed out"
                ),
            }
        }
        if !permit.is_for(generation) {
            continue;
        }

        let (generation_stop_tx, generation_stop_rx) = tokio::sync::watch::channel(false);
        let refresher = authz::run_pack_refresher(
            pool.clone(),
            Arc::clone(&pdp),
            refresh_interval,
            generation_stop_rx,
        );
        tokio::pin!(refresher);
        let end = tokio::select! {
            biased;
            () = permit.revoked() => {
                if gate.is_terminal() {
                    BackgroundGenerationEnd::AuthorityRefused
                } else {
                    BackgroundGenerationEnd::AuthorityClosed
                }
            }
            () = shutdown::requested(&mut shutdown) => BackgroundGenerationEnd::Shutdown,
            () = &mut refresher => {
                return Err("the gateway policy refresher stopped unexpectedly".to_owned());
            }
        };
        let _ = generation_stop_tx.send(true);
        match end {
            BackgroundGenerationEnd::AuthorityClosed => continue,
            BackgroundGenerationEnd::AuthorityRefused => {
                return Err("the gateway database authority was conclusively refused".to_owned());
            }
            BackgroundGenerationEnd::Shutdown => return Ok(()),
        }
    }
}

async fn run_pool_monitor(
    pool: sqlx::PgPool,
    max_connections: u32,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(5));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            () = shutdown::requested(&mut shutdown) => return,
            _ = ticker.tick() => {
                let (size, idle) = (pool.size(), pool.num_idle());
                if size >= max_connections && idle == 0 {
                    tracing::warn!(
                        size,
                        idle,
                        max = max_connections,
                        "database pool saturated: every connection is checked out, so requests are queueing"
                    );
                } else {
                    tracing::debug!(size, idle, max = max_connections, "database pool");
                }
            }
        }
    }
}
