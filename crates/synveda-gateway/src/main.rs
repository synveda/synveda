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

use std::sync::Arc;
use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{self, AppState};
use synveda_gateway::{authz, runtime_config, shutdown, telemetry};
use synveda_identity::{DisabledVerifier, Hs256Verifier, LoginFlow, OidcVerifier, TokenVerifier};
use synveda_ingest::embedding::Embedder as _;
use synveda_policy::Pdp;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let telemetry = telemetry::init("synveda-gateway")?;
    let metrics = telemetry::init_metrics()?;

    let database_url = runtime_config::required_setting("DATABASE_URL")?;
    // The gateway and worker have distinct bounded pools. Deployment
    // profiles budget both against PostgreSQL `max_connections` (ADR-0062).
    let max_connections =
        runtime_config::positive_connection_limit("SYNVEDA_DB_MAX_CONNECTIONS", 8)?;
    let connect_options = synveda_store::database_url::parse("DATABASE_URL", &database_url)?
        .application_name("synveda-gateway");
    // connect_lazy: the gateway boots without a database so /readyz can
    // report the outage instead of the process crash-looping.
    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .connect_lazy_with(connect_options);

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
    let pool_monitor = tokio::spawn({
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
    let refresher = authz::spawn_pack_refresher(pool.clone(), Arc::clone(&pdp), refresh_interval);

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

    let addr = std::env::var("SYNVEDA_LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:8120".to_owned());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "synveda-gateway listening");

    axum::serve(listener, app::router(app_state))
        .with_graceful_shutdown(shutdown::signal())
        .await?;
    refresher.abort();
    pool_monitor.abort();

    // Flush batched spans before exit; a killed process loses the tail.
    telemetry.shutdown();
    Ok(())
}
