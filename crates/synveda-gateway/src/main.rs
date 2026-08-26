//! Gateway entry point. Configuration is environment-only for now:
//! `DATABASE_URL` (required), `SYNVEDA_DB_MAX_CONNECTIONS` (default 8 —
//! one pool shared by the request handlers and remaining background tasks;
//! see the comment at the call site), `SYNVEDA_LISTEN_ADDR` (default
//! `127.0.0.1:8120`),
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
//! The retired record extractor is not started (CPR-16, ADR-0081). CPR-18's
//! capture worker polls frozen session-event batches and produces reviewable
//! candidates; `SYNVEDA_EXTRACTOR` selects `deterministic` (default),
//! `claude` or `vllm`. The embedder is selected by `SYNVEDA_EMBEDDER`
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
//! The directory pull sync (AUTH-5, ADR-0060) runs only for issuers that
//! carry a `directory_sync` entry in `SYNVEDA_OIDC_ISSUERS`, and only when
//! that issuer binds its tenant statically — a pull runs on a timer with no
//! request to read a `tid` claim from. It paces on
//! `SYNVEDA_DIRECTORY_SYNC_INTERVAL_SECS` (default 3600) and is tuned by
//! `SYNVEDA_DIRECTORY_ABSENCE_PASSES` (default 2 — consecutive *complete*
//! passes an absence must survive before anybody is sealed),
//! `SYNVEDA_DIRECTORY_BREAKER_FRACTION` (default 0.10) and
//! `SYNVEDA_DIRECTORY_BREAKER_FLOOR` (default 5). The last two are the
//! circuit breaker: a pass proposing more than that share of a tenant, above
//! that floor, seals nobody until a human authorises it.
//!
//! The standard `OTEL_*` variables configure the OTLP exporter (default
//! endpoint `http://localhost:4317` — Jaeger in the dev compose).

use std::sync::Arc;
use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{self, AppState};
use synveda_gateway::{authz, telemetry};
use synveda_identity::{DisabledVerifier, Hs256Verifier, LoginFlow, OidcVerifier, TokenVerifier};
use synveda_ingest::embedding::Embedder as _;
use synveda_ingest::extraction::Extractor as _;
use synveda_policy::Pdp;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let telemetry = telemetry::init("synveda-gateway")?;
    let metrics = telemetry::init_metrics()?;

    let database_url = std::env::var("DATABASE_URL")
        .map_err(|_| "DATABASE_URL must be set (dev default is in the Makefile)")?;
    // Workers and request handlers share this bounded pool. Deployment
    // profiles size it against Postgres `max_connections` (ADR-0062).
    let max_connections = std::env::var("SYNVEDA_DB_MAX_CONNECTIONS")
        .ok()
        .map(|raw| {
            raw.parse::<u32>()
                .map_err(|_| {
                    format!("SYNVEDA_DB_MAX_CONNECTIONS must be a positive integer, got `{raw}`")
                })
                .and_then(|value| {
                    (value > 0)
                        .then_some(value)
                        .ok_or_else(|| "SYNVEDA_DB_MAX_CONNECTIONS must be at least 1".to_owned())
                })
        })
        .transpose()?
        .unwrap_or(8);
    // connect_lazy: the gateway boots without a database so /readyz can
    // report the outage instead of the process crash-looping.
    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .connect_lazy(&database_url)?;

    // The schema epoch guard (CPR-2, ADR-0068 decision 3, ADR-0069). This
    // product is pre-1.0 and the context-platform redesign is a hard cut:
    // nothing translates a database from before it, so one written before it
    // is refused here rather than half-read later.
    //
    // The two arms are the two different things "the epoch is not the one I
    // serve" can mean, and conflating them would break the boot contract
    // above. A reachable database at the wrong epoch is a *verdict*: the
    // process must not start, because every route below would serve rows in a
    // model it does not implement. A database that cannot be reached at all is
    // a don't-know, and the design here is that the gateway boots anyway so
    // `/readyz` reports the outage (ADR-0007) — so the verdict is taken again
    // on every readiness probe (`app::readyz`), which is what stops a database
    // that came up late from slipping past a check that ran while it was down.
    match synveda_store::epoch::verify(&pool).await {
        Ok(metadata) => tracing::info!(
            schema.epoch = metadata.epoch,
            schema.migration_head = %metadata.migration_head,
            schema.created_at = %metadata.created_at,
            schema.created_by_version = %metadata.created_by_version,
            "schema epoch accepted (CPR-2, ADR-0069)"
        ),
        Err(outage) if !outage.is_refusal() => tracing::warn!(
            error = %outage,
            "the schema epoch could not be checked at boot; /readyz will refuse \
             until it can be"
        ),
        Err(refusal) => {
            // Printed rather than only returned: this is a multi-line
            // instruction for a person, and `main`'s `Box<dyn Error>` renders
            // through `Debug`, which would hand them one line of `\n`s.
            eprintln!("\nsynveda-gateway: {refusal}\n");
            tracing::error!(error = %refusal, "refusing to serve this database");
            return Err("the database is not at the schema epoch this build serves".into());
        }
    }

    // The pool says nothing about itself, and an operator watching every
    // `/v1` surface answer 503 has no way to learn whether this is why.
    // Added on the diagnosis 29ae21f withdrew, and kept because it is what
    // made the real failure legible: a per-interval line is also a clock,
    // and this one ticks whether or not a request does, so a gap in it is
    // a statement about the process that no request-path log can make.
    //
    // Silent while there is headroom — a periodic line nobody needs is a
    // line nobody reads — and one warning per interval once the pool is
    // full with nothing idle, which is the condition that precedes the
    // timeouts rather than the timeouts themselves.
    tokio::spawn({
        let pool = pool.clone();
        async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(5));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                let (size, idle) = (pool.size(), pool.num_idle());
                if size >= max_connections && idle == 0 {
                    tracing::warn!(
                        size,
                        idle,
                        max = max_connections,
                        "database pool saturated: every connection is checked out, so requests \
                         are queueing on acquire and will time out"
                    );
                } else {
                    tracing::debug!(size, idle, max = max_connections, "database pool");
                }
            }
        }
    });

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

    // Populated only in OIDC mode: a pull sync reads the directory behind
    // an issuer, and the dev HS256 mode has no issuer to read.
    let mut directory_connectors: std::collections::HashMap<
        synveda_types::TenantId,
        Box<dyn synveda_identity::directory::DirectoryConnector>,
    > = std::collections::HashMap::new();

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
                // Built before the verifier consumes the list, and refused
                // at boot rather than at the first tick: a directory sync
                // that is misconfigured must not look like a directory that
                // has nobody in it (AUTH-5, ADR-0060).
                directory_connectors = build_directory_connectors(&issuers)?;
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

    // The key plane (TEN-4, ADR-0064). `Kms::Disabled` when no KEK is
    // configured, which is fail-closed rather than fail-to-boot: `/v1`
    // bearer traffic never touches a sealed column, so a deployment that has
    // not set a key keeps serving and the surfaces that need one say which
    // key is missing.
    let keys = Arc::new(synveda_store::keys::KeyRing::new(kms_from_env()?));
    match keys.kms() {
        synveda_crypto::Kms::Disabled => tracing::warn!(
            "no SYNVEDA_KMS_KEY: console sessions and per-tenant secrets are \
             unavailable (TEN-4, ADR-0064). `synveda kms keygen` mints one."
        ),
        kms => {
            // Provisioning the deployment key at boot rather than on first
            // use: a login is a bad moment to discover the key plane is
            // empty, and this is idempotent.
            let key_ref = synveda_crypto::KeyManagement::key_ref(kms).to_owned();
            match keys
                .provision(&pool, synveda_crypto::KeyScope::Deployment)
                .await
            {
                Ok(version) => tracing::info!(
                    key.version = version.get(),
                    kek.ref = key_ref,
                    "deployment encryption key ready (TEN-4, ADR-0064)"
                ),
                // Not fatal, and deliberately: a database that is not ready
                // yet is what `/readyz` is for, and the next seal retries.
                Err(error) => tracing::warn!(
                    %error,
                    "could not provision the deployment encryption key at boot"
                ),
            }
        }
    }

    // Extraction emits reviewable candidates; the embedder serves Knowledge
    // search and context planning.
    let capture_extractor = Arc::new(extractor_from_env()?);
    let embedder = Arc::new(embedder_from_env()?);
    tracing::info!(
        extractor = capture_extractor.method(),
        embedder = embedder.method(),
        embedding_model = embedder.model(),
        "capture extractor and Knowledge embedder ready"
    );

    // Governed relaxation expiry bookkeeping (CPR-31, ADR-0090). Database
    // time already ended authority at the hard boundary; this loop records
    // the content-free system event once.
    let relaxation_sweep_secs = match std::env::var("SYNVEDA_RELAXATION_SWEEP_SECS") {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|_| "SYNVEDA_RELAXATION_SWEEP_SECS must be a positive integer")?,
        Err(_) => 60,
    };
    let relaxation_sweep = synveda_gateway::relaxations::spawn_expiry_sweep(
        pool.clone(),
        Duration::from_secs(relaxation_sweep_secs.max(1)),
    );

    // CPR-17's Knowledge revision sidecar. Unlike the retired record worker,
    // this is index maintenance rather than a domain mutation: immutable
    // revision text is embedded outside a transaction and the derivative row
    // converges idempotently. An unavailable TEI instance degrades search to
    // lexical and is retried; it never blocks a VedaFlow Knowledge commit.
    let knowledge_index_config = synveda_gateway::knowledge_index::Config {
        poll_interval: std::env::var("SYNVEDA_KNOWLEDGE_EMBED_POLL_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|millis| *millis > 0)
            .map(Duration::from_millis)
            .unwrap_or_else(|| synveda_gateway::knowledge_index::Config::default().poll_interval),
        batch: std::env::var("SYNVEDA_KNOWLEDGE_EMBED_BATCH")
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|batch| *batch > 0)
            .unwrap_or_else(|| synveda_gateway::knowledge_index::Config::default().batch),
    };
    tracing::info!(
        model = embedder.model(),
        method = embedder.method(),
        poll_ms = knowledge_index_config.poll_interval.as_millis() as u64,
        batch = knowledge_index_config.batch,
        "Knowledge revision embedding sweep starting (CPR-17, ADR-0082)"
    );
    let knowledge_indexer = synveda_gateway::knowledge_index::spawn(
        pool.clone(),
        Arc::clone(&embedder),
        knowledge_index_config,
    );

    // The context planner's embedding deadline. Failure is an explicit
    // lexical degradation recorded on the ContextRun.
    let context_embed_timeout_ms = std::env::var("SYNVEDA_CONTEXT_EMBED_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|ms| *ms > 0)
        .unwrap_or(100);

    // Built before the background loops that share it: the directory pull
    // sync reaches the product only through the same `AppState` a request
    // does, which is what keeps it on the reconciler's side of the PDP
    // rather than beside it.
    let app_state = AppState {
        pool,
        metrics,
        verifier,
        login,
        public_origin,
        pdp,
        service_token_max_ttl: Duration::from_secs(service_token_max_ttl_secs),
        embedder,
        context_embed_timeout: Duration::from_millis(context_embed_timeout_ms),
        keys,
    };

    let capture_config = capture_config_from_env();
    tracing::info!(
        extractor = capture_extractor.method(),
        poll_ms = capture_config.poll_interval.as_millis() as u64,
        lease_secs = capture_config.lease_duration.as_secs(),
        batches_per_tenant = capture_config.batches_per_tenant,
        "session capture worker starting (CPR-18, ADR-0083)"
    );
    let capture_worker = synveda_ingest::capture_worker::spawn(
        synveda_ingest::capture_worker::Deps {
            pool: app_state.pool.clone(),
            pdp: Arc::clone(&app_state.pdp),
            extractor: capture_extractor,
        },
        capture_config,
    );

    // The directory pull sync (AUTH-5, ADR-0060). Spawned only when an
    // issuer configures one, because an empty connector map is a loop that
    // wakes up to do nothing — and because "no tenant is being pulled"
    // should be visible as a missing log line rather than as a sweep
    // reporting zero every hour.
    //
    // Since TEN-4 a tenant's connector can also come from its own sealed
    // `tenant_secrets` row (ADR-0064 decision 9), and a deployment whose
    // credentials live *only* there configures no issuer connector at all —
    // so it would fail the test above. `SYNVEDA_DIRECTORY_SYNC=on` is that
    // deployment's switch. It is an explicit flag rather than a scan for
    // stored rows because `tenant_secrets` is under forced RLS: asking "does
    // any tenant have one" means one transaction per tenant, and a boot that
    // is O(tenants) to answer a yes/no question is a boot that gets slower
    // for the customers who have the most of them.
    let force_directory_sync = std::env::var("SYNVEDA_DIRECTORY_SYNC")
        .is_ok_and(|value| matches!(value.trim(), "on" | "true" | "1"));
    let directory_sync = if directory_connectors.is_empty() && !force_directory_sync {
        None
    } else {
        let config = directory_sync_config_from_env()?;
        tracing::info!(
            tenants = directory_connectors.len(),
            stored_credentials = force_directory_sync,
            interval_secs = config.interval.as_secs(),
            absence_passes = config.absence_passes,
            breaker_fraction = config.breaker_fraction,
            breaker_floor = config.breaker_floor,
            "directory pull sync starting (AUTH-5, ADR-0060): absence needs \
             this many consecutive complete passes before anybody is sealed"
        );
        Some(synveda_gateway::directory_sync::spawn(
            app_state.clone(),
            directory_connectors,
            config,
        ))
    };

    let addr = std::env::var("SYNVEDA_LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:8120".to_owned());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "synveda-gateway listening");

    axum::serve(listener, app::router(app_state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    refresher.abort();
    knowledge_indexer.abort();
    capture_worker.abort();
    relaxation_sweep.abort();
    if let Some(sync) = directory_sync {
        sync.abort();
    }

    // Flush batched spans before exit; a killed process loses the tail.
    telemetry.shutdown();
    Ok(())
}

async fn shutdown_signal() {
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

/// Builds the KMS from `SYNVEDA_KMS_KEY` (64 hex characters) and
/// `SYNVEDA_KMS_KEY_REF` (a name, default `local:default`).
///
/// Absent means [`synveda_crypto::Kms::Disabled`] — fail-closed, not
/// fail-to-boot (ADR-0064; the `DisabledVerifier` shape). A KEK that is
/// *present and malformed* is a startup error, because that is somebody
/// trying to configure this and getting it wrong, and booting past it would
/// hand them a deployment that looks configured and seals nothing.
fn kms_from_env() -> Result<synveda_crypto::Kms, String> {
    let Some(key) = std::env::var("SYNVEDA_KMS_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(synveda_crypto::Kms::Disabled);
    };
    let key_ref = std::env::var("SYNVEDA_KMS_KEY_REF")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "local:default".to_string());
    // The error is rendered without the value: a malformed key is still a
    // key somebody meant to keep.
    synveda_crypto::LocalKms::from_hex(&key, key_ref)
        .map(synveda_crypto::Kms::Local)
        .map_err(|err| format!("SYNVEDA_KMS_KEY is not usable: {err}"))
}

/// Builds the CPR-18 extractor. There is deliberately no `off`: a terminal
/// session has durably asked for candidate extraction, so silently leaving
/// every batch pending would be a broken runtime rather than a deployment
/// profile.
fn extractor_from_env() -> Result<synveda_ingest::extraction::AnyExtractor, String> {
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
        "deterministic" => Ok(AnyExtractor::Deterministic(DeterministicExtractor::new())),
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
            Ok(AnyExtractor::Claude(ClaudeExtractor::new(
                api_key, model, base_url,
            )))
        }
        "vllm" => {
            let base_url = std::env::var("SYNVEDA_VLLM_BASE_URL")
                .ok()
                .filter(|value| !value.is_empty())
                .ok_or("SYNVEDA_EXTRACTOR=vllm requires SYNVEDA_VLLM_BASE_URL")?;
            let model = model.ok_or("SYNVEDA_EXTRACTOR=vllm requires SYNVEDA_EXTRACTOR_MODEL")?;
            Ok(AnyExtractor::Vllm(VllmExtractor::new(model, base_url)))
        }
        other => Err(format!(
            "SYNVEDA_EXTRACTOR must be deterministic|claude|vllm, got {other:?}"
        )),
    }
}

/// Capture polling is operational tuning, not an authority boundary. Invalid
/// values fall back to conservative defaults; the extractor selection above
/// fails closed because a malformed provider choice would change behaviour.
fn capture_config_from_env() -> synveda_ingest::capture_worker::Config {
    let defaults = synveda_ingest::capture_worker::Config::default();
    synveda_ingest::capture_worker::Config {
        poll_interval: std::env::var("SYNVEDA_CAPTURE_POLL_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .map(Duration::from_millis)
            .unwrap_or(defaults.poll_interval),
        lease_duration: std::env::var("SYNVEDA_CAPTURE_LEASE_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .map(Duration::from_secs)
            .unwrap_or(defaults.lease_duration),
        batches_per_tenant: std::env::var("SYNVEDA_CAPTURE_BATCHES_PER_TENANT")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(defaults.batches_per_tenant),
        lease_owner: std::env::var("SYNVEDA_CAPTURE_LEASE_OWNER")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(defaults.lease_owner),
    }
}

/// Builds the configured embedder from `SYNVEDA_EMBEDDER` and its
/// companions (module header). There is no `off`: Knowledge indexing and
/// context composition share one explicit implementation identity.
/// Misconfiguration is a startup error, like extractor selection.
fn embedder_from_env() -> Result<synveda_ingest::embedding::AnyEmbedder, String> {
    use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder, TeiEmbedder};
    let selected = std::env::var("SYNVEDA_EMBEDDER")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "deterministic".to_owned());
    match selected.as_str() {
        "deterministic" => Ok(AnyEmbedder::Deterministic(DeterministicEmbedder::new())),
        "tei" => {
            let base_url = std::env::var("SYNVEDA_TEI_URL")
                .ok()
                .filter(|value| !value.is_empty())
                .ok_or("SYNVEDA_EMBEDDER=tei requires SYNVEDA_TEI_URL")?;
            let model = std::env::var("SYNVEDA_EMBEDDER_MODEL")
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| TeiEmbedder::DEFAULT_MODEL.to_owned());
            Ok(AnyEmbedder::Tei(TeiEmbedder::new(model, base_url)))
        }
        other => Err(format!(
            "SYNVEDA_EMBEDDER must be deterministic|tei, got {other:?}"
        )),
    }
}

/// Which tenant each configured connector reads for (AUTH-5, ADR-0060).
///
/// **A pull sync requires a statically bound issuer**, and this is the one
/// place that rule is enforced. `TenantBinding::Claim` resolves a tenant from
/// a token presented on a request; a pull sync runs on a timer with no
/// request in front of it, so there is no claim to read and no way to know
/// which tenants that issuer serves without one. The alternatives to
/// refusing are both worse: guessing a tenant, or booting into a sync that
/// silently pulls nobody — and a directory sync that reads nothing looks
/// exactly like a directory that has nobody in it, which is the state that
/// would eventually seal a company.
///
/// # Errors
/// A claim-bound issuer configuring a pull sync, two issuers claiming one
/// tenant, or a connector that cannot be constructed.
fn build_directory_connectors(
    issuers: &[synveda_identity::IssuerConfig],
) -> Result<
    std::collections::HashMap<
        synveda_types::TenantId,
        Box<dyn synveda_identity::directory::DirectoryConnector>,
    >,
    Box<dyn std::error::Error>,
> {
    let mut connectors = std::collections::HashMap::new();
    for issuer in issuers {
        let Some(config) = &issuer.directory_sync else {
            continue;
        };
        let synveda_identity::TenantBinding::Static { tenant_id } = &issuer.tenant else {
            return Err(format!(
                "issuer {} configures `directory_sync` with a claim-bound tenant: a \
                 pull sync runs on a timer with no request to read a claim from, so \
                 it needs `tenant: {{\"static\": ...}}` (AUTH-5, ADR-0060)",
                issuer.issuer
            )
            .into());
        };
        let connector = synveda_identity::directory::connector(config)?;
        tracing::info!(
            issuer = issuer.issuer,
            tenant.id = %tenant_id,
            connector = connector.name(),
            "directory pull sync configured"
        );
        if connectors.insert(*tenant_id, connector).is_some() {
            return Err(format!(
                "tenant {tenant_id} is pull-synced by two issuers: one directory is \
                 the authority for one tenant (ADR-0060 decision 5)"
            )
            .into());
        }
    }
    Ok(connectors)
}

/// The pass's tuning. Every knob has a default that is safe rather than
/// eager, because the thing being tuned decides whether people get sealed.
///
/// # Errors
/// A value that parses but cannot mean anything — a fraction outside `0..=1`
/// or a non-positive absence threshold. Refused at boot rather than clamped:
/// somebody who wrote `SYNVEDA_DIRECTORY_ABSENCE_PASSES=0` meant something,
/// and it was not "seal on the first missed page".
fn directory_sync_config_from_env()
-> Result<synveda_gateway::directory_sync::SyncConfig, Box<dyn std::error::Error>> {
    let defaults = synveda_gateway::directory_sync::SyncConfig::default();
    let absence_passes = match std::env::var("SYNVEDA_DIRECTORY_ABSENCE_PASSES") {
        Ok(value) => {
            let parsed: i32 = value
                .parse()
                .map_err(|_| "SYNVEDA_DIRECTORY_ABSENCE_PASSES must be an integer")?;
            if parsed < 1 {
                return Err("SYNVEDA_DIRECTORY_ABSENCE_PASSES must be at least 1: a \
                            threshold of zero seals somebody the first time one page \
                            of a directory read is throttled (ADR-0060 decision 3.2)"
                    .into());
            }
            parsed
        }
        Err(_) => defaults.absence_passes,
    };
    let breaker_fraction = match std::env::var("SYNVEDA_DIRECTORY_BREAKER_FRACTION") {
        Ok(value) => {
            let parsed: f64 = value
                .parse()
                .map_err(|_| "SYNVEDA_DIRECTORY_BREAKER_FRACTION must be a number")?;
            if !(0.0..=1.0).contains(&parsed) {
                return Err("SYNVEDA_DIRECTORY_BREAKER_FRACTION must be between 0 and 1".into());
            }
            parsed
        }
        Err(_) => defaults.breaker_fraction,
    };
    Ok(synveda_gateway::directory_sync::SyncConfig {
        interval: std::env::var("SYNVEDA_DIRECTORY_SYNC_INTERVAL_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|secs| *secs > 0)
            .map_or(defaults.interval, Duration::from_secs),
        absence_passes,
        breaker_fraction,
        breaker_floor: std::env::var("SYNVEDA_DIRECTORY_BREAKER_FLOOR")
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|floor| *floor >= 0)
            .unwrap_or(defaults.breaker_floor),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use synveda_identity::directory::{DirectorySyncConfig, Secret};

    #[cfg(unix)]
    use std::process::{Child, Command, Stdio};
    #[cfg(unix)]
    use std::thread;
    #[cfg(unix)]
    use std::time::Instant;

    #[cfg(unix)]
    struct ChildGuard(Child);

    #[cfg(unix)]
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn issuer(tenant: synveda_identity::TenantBinding) -> synveda_identity::IssuerConfig {
        let json = r#"[{"issuer":"https://idp.example","client_id":"c"}]"#;
        let mut parsed = synveda_identity::parse_issuers(json).expect("parse");
        let mut config = parsed.remove(0);
        config.tenant = tenant;
        config
    }

    fn okta() -> DirectorySyncConfig {
        DirectorySyncConfig::Okta {
            org_url: "https://example.okta.com".to_owned(),
            api_token: Secret::new("token"),
        }
    }

    #[test]
    fn a_pull_sync_needs_a_statically_bound_issuer() {
        // A claim-bound issuer resolves its tenant from a token on a
        // request. A pull sync has no request, so there is no claim and no
        // way to know which tenants it serves — and the two ways of
        // carrying on regardless are both worse than refusing: guess a
        // tenant, or boot into a sync that pulls nobody. The second is the
        // dangerous one, because a directory read that returns nothing
        // looks exactly like a directory with nobody in it.
        let mut claim_bound = issuer(synveda_identity::TenantBinding::Claim {
            name: "tid".to_owned(),
        });
        claim_bound.directory_sync = Some(okta());
        // `.err()` rather than `expect_err`: the Ok side holds boxed trait
        // objects and is not `Debug`.
        let message = build_directory_connectors(std::slice::from_ref(&claim_bound))
            .err()
            .expect("a claim-bound pull sync is refused")
            .to_string();
        assert!(
            message.contains("static"),
            "and the error says what to do about it: {message}"
        );

        let tenant_id = synveda_types::TenantId::new();
        let mut bound = issuer(synveda_identity::TenantBinding::Static { tenant_id });
        bound.directory_sync = Some(okta());
        let built = build_directory_connectors(std::slice::from_ref(&bound)).expect("built");
        assert_eq!(built.len(), 1);
        assert_eq!(built[&tenant_id].name(), "okta");
    }

    #[test]
    fn an_issuer_with_no_directory_sync_contributes_no_connector() {
        // The common case: OIDC login configured, nothing pulled. It must
        // not produce an empty-but-present sync that wakes hourly to do
        // nothing.
        let plain = issuer(synveda_identity::TenantBinding::Static {
            tenant_id: synveda_types::TenantId::new(),
        });
        assert!(
            build_directory_connectors(std::slice::from_ref(&plain))
                .expect("built")
                .is_empty()
        );
    }

    #[test]
    fn two_issuers_cannot_pull_one_tenant() {
        // One directory is the authority for one tenant (ADR-0060 decision
        // 5). Two would race to decide who has left.
        let tenant_id = synveda_types::TenantId::new();
        let mut first = issuer(synveda_identity::TenantBinding::Static { tenant_id });
        first.directory_sync = Some(okta());
        let mut second = issuer(synveda_identity::TenantBinding::Static { tenant_id });
        second.issuer = "https://other.example".to_owned();
        second.directory_sync = Some(okta());
        let refused = build_directory_connectors(&[first, second]);
        assert!(refused.is_err(), "one tenant, one directory authority");
    }

    #[cfg(unix)]
    #[test]
    fn sigterm_is_delivered_to_the_gateway_handler() {
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
                "tests::sigterm_is_delivered_to_the_gateway_handler",
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

        let pid = child.0.id().to_string();
        let sent = Command::new("kill")
            .args(["-TERM", pid.as_str()])
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

    #[test]
    fn an_absence_threshold_of_zero_is_refused_rather_than_clamped() {
        // Somebody who wrote this meant something, and it was not "seal on
        // the first missed page" — which is what a silent clamp to 1 would
        // have given them.
        unsafe { std::env::set_var("SYNVEDA_DIRECTORY_ABSENCE_PASSES", "0") };
        let refused = directory_sync_config_from_env();
        unsafe { std::env::remove_var("SYNVEDA_DIRECTORY_ABSENCE_PASSES") };
        assert!(refused.is_err(), "a threshold of zero is refused at boot");
    }
}
