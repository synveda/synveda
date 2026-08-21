//! Gateway entry point. Configuration is environment-only for now:
//! `DATABASE_URL` (required), `SYNVEDA_DB_MAX_CONNECTIONS` (default 8 —
//! one pool shared by the request handlers *and* the background workers;
//! see the comment at the call site for what that has and has not been
//! measured to cost), `SYNVEDA_LISTEN_ADDR` (default `127.0.0.1:8120`),
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
//! defaults per implementation. The embedder (MEM-4, ADR-0023) is
//! selected by `SYNVEDA_EMBEDDER` (`deterministic` [default] | `tei` —
//! deliberately no `off`: embed-or-fail is unconditional); `tei`
//! requires `SYNVEDA_TEI_URL` (the dev compose serves
//! `http://localhost:8110`) and honours `SYNVEDA_EMBEDDER_MODEL`
//! (default `BAAI/bge-m3`). Worker pacing:
//! `SYNVEDA_EXTRACTION_POLL_MS` (default 1000),
//! `SYNVEDA_EXTRACTION_BATCH` (default 16),
//! `SYNVEDA_EXTRACTION_VT_SECS` (default 60),
//! `SYNVEDA_EXTRACTION_MAX_READS` (default 5).
//!
//! The auto-promotion engine (FLOW-4, ADR-0033) runs every
//! `SYNVEDA_PROMOTION_INTERVAL_SECS` (default 300) and folds up to
//! `SYNVEDA_PROMOTION_BATCH` (default 1024) audit events per tenant per
//! pass. It cannot be turned off because it does nothing until a pack
//! carries promotion rules, and no embedded pack does.
//!
//! The search index sidecar (CTX-1, ADR-0024) lives under
//! `SYNVEDA_SEARCH_INDEX_DIR` (default `./data/search-index`; one
//! subdirectory per tenant — deleting a tenant's directory is the
//! rebuild procedure, and the directory must share the database's
//! encryption-at-rest story). Its indexer task polls every
//! `SYNVEDA_SEARCH_POLL_MS` (default 1000), which bounds BM25
//! visibility lag; the dense leg reads Postgres directly and never lags.
//!
//! The inject route (CTX-3, ADR-0026) embeds the caller's task through
//! the same configured embedder under `SYNVEDA_INJECT_EMBED_TIMEOUT_MS`
//! (default 100): expiry or failure degrades that inject to the sparse
//! leg (marked in `X-Synveda-Degraded`), never fails it.
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
    // Eight was the number from the first day and it is still the default;
    // what changed is that it can no longer only be changed by recompiling.
    //
    // 233bca9 added this setting on the finding that EVAL-3's LongMemEval
    // run had wedged the gateway on a pool the background workers share
    // with the request handlers. **29ae21f withdrew that finding.** The
    // machine was sleeping — unattended runs, `pmset` `sleep 1` — the VM
    // froze with it, and `acquire` timed out while no code was running at
    // all; two observers on opposite sides of the container boundary froze
    // in lockstep for 7m45s and resumed to report a healthy database. A
    // larger pool "helped" only by coasting longer on connections opened
    // before the freeze. There is no evidence in this repository that this
    // pool wedges under sustained ingestion.
    //
    // The setting stays on its own merits. The workers and the handlers do
    // draw from one pool, and a deployment-shaped number an operator can
    // only change by recompiling is one that will be wrong where nobody
    // can look — OPS-2 (ADR-0062 decision 6) makes it a chart value, sized
    // against the server's own `max_connections`.
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

    // The extraction worker (MEM-3, ADR-0022 decision 1): the observe
    // queue's consumer, embedded so SMB mode stays one process. It shares
    // the gateway's scope-chain cache — hierarchy-move invalidations must
    // reach the worker's authorization reads.
    let extractor = extractor_from_env()?;
    // Shared with the inject route (CTX-3, ADR-0026 decision 3): the
    // query-embedding call and the pipeline's record vectors carry one
    // config-declared model identity.
    let embedder = Arc::new(embedder_from_env()?);
    let extraction_worker = extractor.map(|extractor| {
        tracing::info!(
            extractor = extractor.method(),
            embedder = embedder.method(),
            embedding_model = embedder.model(),
            "extraction worker starting (MEM-3/MEM-4, ADR-0022/0023)"
        );
        synveda_ingest::worker::spawn(
            synveda_ingest::worker::WorkerDeps {
                pool: pool.clone(),
                pdp: Arc::clone(&pdp),
                extractor,
                embedder: embedder.as_ref().clone(),
            },
            extraction_config_from_env(),
        )
    });
    if extraction_worker.is_none() {
        tracing::warn!("SYNVEDA_EXTRACTOR=off: observe signals will accumulate unconsumed");
    }

    // The auto-promotion engine (FLOW-4, ADR-0033): a second background
    // loop, on a cadence of minutes rather than the extraction worker's
    // second. It writes nothing on the read path — it folds
    // `context.injected` events the gateway already records into a usage
    // projection, and opens FLOW-3 proposals under the material owner's
    // authority when a pack's rules fire.
    let promotion_config = promotion_config_from_env();
    tracing::info!(
        interval_secs = promotion_config.interval.as_secs(),
        batch = promotion_config.batch,
        "promotion engine starting (FLOW-4, ADR-0033)"
    );
    let promotion_engine = synveda_ingest::promotion::spawn(
        synveda_ingest::promotion::SweepDeps {
            pool: pool.clone(),
            pdp: Arc::clone(&pdp),
        },
        promotion_config,
    );

    // The retention sweep (MEM-6, ADR-0040 decision 14): the third
    // background loop. It enforces nothing — the read path already
    // refused expired material in the query that asked — so what it does
    // is disposal: the temporal delete, the destruction of closed
    // versions past a second horizon, and the observe staging plane
    // MEM-1 and MEM-2 have been accumulating since they landed. In the
    // default configuration no pack sets a record horizon, so a pass
    // expires nothing and destroys nothing; the staging plane is the one
    // thing every pack disposes of.
    let retention_config = retention_config_from_env();
    tracing::info!(
        interval_secs = retention_config.interval.as_secs(),
        batch = retention_config.batch,
        "retention sweep starting (MEM-6, ADR-0040)"
    );
    let retention_sweep = synveda_ingest::retention::spawn(
        synveda_ingest::retention::SweepDeps {
            pool: pool.clone(),
            pdp: Arc::clone(&pdp),
        },
        retention_config,
    );

    // The lapse expiry sweep (AUTHZ-4, ADR-0037 decision 4). Bookkeeping:
    // it chains `policy.lapse.expired` for windows that have closed, and
    // every grant it touches stopped deciding reads at `expires_at`
    // whether or not this loop is running. The default cadence is
    // deliberately slack for that reason — a late audit line is the only
    // thing a slow sweep costs.
    let lapse_sweep_secs = match std::env::var("SYNVEDA_LAPSE_SWEEP_SECS") {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|_| "SYNVEDA_LAPSE_SWEEP_SECS must be a positive integer")?,
        Err(_) => 60,
    };
    let lapse_sweep = synveda_gateway::lapses::spawn_expiry_sweep(
        pool.clone(),
        Duration::from_secs(lapse_sweep_secs.max(1)),
    );

    // The search index sidecar and its indexer (CTX-1, ADR-0024): a
    // boot failure here means the index root is unusable — refuse to
    // boot rather than serve a read path whose lexical leg can never
    // converge.
    let index_root = std::env::var("SYNVEDA_SEARCH_INDEX_DIR")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "./data/search-index".to_owned());
    let search_index = Arc::new(synveda_retrieval::SearchIndex::open(&index_root)?);
    let indexer_config = synveda_retrieval::IndexerConfig {
        poll_interval: std::env::var("SYNVEDA_SEARCH_POLL_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|ms| *ms > 0)
            .map(Duration::from_millis)
            .unwrap_or(synveda_retrieval::IndexerConfig::default().poll_interval),
        ..synveda_retrieval::IndexerConfig::default()
    };
    tracing::info!(
        index_root,
        poll_ms = indexer_config.poll_interval.as_millis() as u64,
        "search indexer starting (CTX-1, ADR-0024)"
    );
    let search_indexer =
        synveda_retrieval::indexer::spawn(pool.clone(), Arc::clone(&search_index), indexer_config);

    // The inject route's embed deadline (CTX-3, ADR-0026 decision 3).
    let inject_embed_timeout_ms = std::env::var("SYNVEDA_INJECT_EMBED_TIMEOUT_MS")
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
        search_index,
        embedder,
        inject_embed_timeout: Duration::from_millis(inject_embed_timeout_ms),
        keys,
    };

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
    search_indexer.abort();
    promotion_engine.abort();
    retention_sweep.abort();
    lapse_sweep.abort();
    if let Some(sync) = directory_sync {
        sync.abort();
    }
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

/// Builds the configured embedder from `SYNVEDA_EMBEDDER` and its
/// companions (module header). There is no `off`: wherever the worker
/// runs, records commit embed-or-fail (ADR-0023 decision 6).
/// Misconfiguration is a startup error, the extractor discipline.
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

/// Worker pacing from `SYNVEDA_EXTRACTION_*`, with the defaults the
/// module header documents. Unparseable values fall back to defaults:
/// pacing is tuning, never a fail-closed control.
/// `SYNVEDA_PROMOTION_INTERVAL_SECS` / `SYNVEDA_PROMOTION_BATCH`, with
/// the engine's defaults for anything unset or unparseable.
fn promotion_config_from_env() -> synveda_ingest::promotion::SweepConfig {
    let defaults = synveda_ingest::promotion::SweepConfig::default();
    synveda_ingest::promotion::SweepConfig {
        interval: std::env::var("SYNVEDA_PROMOTION_INTERVAL_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|secs| *secs > 0)
            .map_or(defaults.interval, Duration::from_secs),
        batch: std::env::var("SYNVEDA_PROMOTION_BATCH")
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|batch| *batch > 0)
            .unwrap_or(defaults.batch),
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

fn retention_config_from_env() -> synveda_ingest::retention::SweepConfig {
    let defaults = synveda_ingest::retention::SweepConfig::default();
    synveda_ingest::retention::SweepConfig {
        interval: std::env::var("SYNVEDA_RETENTION_INTERVAL_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|secs| *secs > 0)
            .map_or(defaults.interval, Duration::from_secs),
        batch: std::env::var("SYNVEDA_RETENTION_BATCH")
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|batch| *batch > 0)
            .unwrap_or(defaults.batch),
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use synveda_identity::directory::{DirectorySyncConfig, Secret};

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
