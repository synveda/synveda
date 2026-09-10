//! Restart-safe immutable Knowledge revision embedding sweep (CPR-17,
//! ADR-0082 decision 3).
//!
//! Governance commits do not wait on an embedding service. This worker reads
//! a bounded batch of immutable revision text under tenant RLS, drops the
//! transaction before calling the configured embedder, then inserts the
//! resulting derivative rows idempotently in a fresh tenant transaction. A
//! failed dependency call leaves every revision eligible for the next sweep;
//! a concurrent sweep converges through the sidecar's primary key.

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use synveda_ingest::embedding::{AnyEmbedder, Embedder as _};
use synveda_store::{knowledge_search, rls, tenants};
use synveda_types::{Error, Result, TenantId};

/// Indexed-vector transitions, labelled `inserted`, `duplicate` or `error`.
pub const KNOWLEDGE_EMBEDDINGS_TOTAL: &str = "synveda_knowledge_embeddings_total";
/// Per-tenant sweep outcomes, labelled `updated`, `empty` or `error`.
pub const KNOWLEDGE_EMBED_SWEEPS_TOTAL: &str = "synveda_knowledge_embed_sweeps_total";

/// Background pacing and batch bound.
#[derive(Debug, Clone)]
pub struct Config {
    /// Delay between complete tenant sweeps.
    pub poll_interval: Duration,
    /// Immutable revisions embedded in one dependency call.
    pub batch: i64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(1),
            batch: 64,
        }
    }
}

/// One tenant sweep's exact outcome.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TenantSweep {
    /// New sidecar rows committed.
    pub inserted: u64,
    /// Rows another concurrent sweep had already committed.
    pub duplicates: u64,
}

/// One all-tenant pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepSummary {
    /// Active tenants visited.
    pub tenants: usize,
    /// New sidecar rows.
    pub inserted: u64,
    /// Concurrent/idempotent duplicates.
    pub duplicates: u64,
    /// Tenant sweeps that failed and will retry.
    pub errors: usize,
}

/// Runs one immediate sweep and then one per configured interval until
/// shutdown.
///
/// Shutdown cancels the current tenant future. Provider output is written
/// only after the embedding call returns and inside a tenant transaction, so
/// cancellation cannot leave a partial sidecar write.
pub(crate) async fn run(
    pool: PgPool,
    embedder: Arc<AnyEmbedder>,
    config: Config,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let mut ticker = tokio::time::interval(config.poll_interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
            _ = ticker.tick() => {}
        }
        if *shutdown.borrow() {
            return;
        }
        match sweep_until_shutdown(&pool, &embedder, &config, &mut shutdown).await {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => tracing::warn!(%error, "Knowledge embedding sweep failed; retrying"),
        }
    }
}

async fn sweep_until_shutdown(
    pool: &PgPool,
    embedder: &AnyEmbedder,
    config: &Config,
    shutdown: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<bool> {
    let active = tokio::select! {
        biased;
        () = crate::shutdown::requested(shutdown) => return Ok(false),
        result = tenants::active(pool) => result?,
    };
    for tenant in active {
        if *shutdown.borrow() {
            return Ok(false);
        }
        let result = tokio::select! {
            biased;
            () = crate::shutdown::requested(shutdown) => {
                tracing::info!(tenant.id = %tenant.id, "Knowledge embedding tenant pass cancelled for worker shutdown");
                return Ok(false);
            }
            result = sweep_tenant(pool, embedder, tenant.id, config.batch) => result,
        };
        match result {
            Ok(sweep) => {
                let outcome = if sweep == TenantSweep::default() {
                    "empty"
                } else {
                    "updated"
                };
                metrics::counter!(KNOWLEDGE_EMBED_SWEEPS_TOTAL, "outcome" => outcome).increment(1);
            }
            Err(error) => {
                metrics::counter!(KNOWLEDGE_EMBED_SWEEPS_TOTAL, "outcome" => "error").increment(1);
                tracing::warn!(
                    tenant.id = %tenant.id,
                    model = embedder.model(),
                    %error,
                    "tenant Knowledge embedding sweep failed; retrying"
                );
            }
        }
    }
    Ok(true)
}

/// Sweeps every active tenant without allowing one tenant or one dependency
/// failure to starve the rest.
#[tracing::instrument(name = "knowledge.embedding.sweep", skip_all, err(Display))]
pub async fn sweep_once(
    pool: &PgPool,
    embedder: &AnyEmbedder,
    config: &Config,
) -> Result<SweepSummary> {
    let mut summary = SweepSummary::default();
    for tenant in tenants::active(pool).await? {
        summary.tenants += 1;
        match sweep_tenant(pool, embedder, tenant.id, config.batch).await {
            Ok(sweep) => {
                summary.inserted += sweep.inserted;
                summary.duplicates += sweep.duplicates;
                let outcome = if sweep == TenantSweep::default() {
                    "empty"
                } else {
                    "updated"
                };
                metrics::counter!(KNOWLEDGE_EMBED_SWEEPS_TOTAL, "outcome" => outcome).increment(1);
            }
            Err(error) => {
                summary.errors += 1;
                metrics::counter!(KNOWLEDGE_EMBED_SWEEPS_TOTAL, "outcome" => "error").increment(1);
                tracing::warn!(
                    tenant.id = %tenant.id,
                    model = embedder.model(),
                    %error,
                    "tenant Knowledge embedding sweep failed; retrying"
                );
            }
        }
    }
    Ok(summary)
}

/// Embeds one tenant's next immutable batch. Public for deterministic tests
/// and acceptance demos; production reaches it only through [`run`].
#[tracing::instrument(
    name = "knowledge.embedding.sweep_tenant",
    skip_all,
    fields(tenant.id = %tenant_id, model = embedder.model(), batch),
    err(Display)
)]
pub async fn sweep_tenant(
    pool: &PgPool,
    embedder: &AnyEmbedder,
    tenant_id: TenantId,
    batch: i64,
) -> Result<TenantSweep> {
    let model = embedder.model().to_owned();
    let mut read = rls::begin_tenant_tx(pool, tenant_id).await?;
    let pending =
        knowledge_search::unembedded_revisions(&mut read, tenant_id, &model, batch.clamp(1, 512))
            .await?;
    drop(read);
    if pending.is_empty() {
        return Ok(TenantSweep::default());
    }

    let inputs: Vec<String> = pending
        .iter()
        .map(|revision| revision.text.clone())
        .collect();
    let vectors = embedder.embed(&inputs).await?;
    if vectors.len() != pending.len() {
        return Err(Error::Dependency {
            service: embedder.method().to_owned(),
            message: format!(
                "Knowledge index requested {} vectors and received {}",
                pending.len(),
                vectors.len()
            ),
        });
    }

    let mut write = rls::begin_tenant_tx(pool, tenant_id).await?;
    let mut sweep = TenantSweep::default();
    for (revision, vector) in pending.iter().zip(&vectors) {
        if knowledge_search::insert_embedding(
            &mut write,
            tenant_id,
            revision.revision_id,
            &model,
            vector,
        )
        .await?
        {
            sweep.inserted += 1;
            metrics::counter!(KNOWLEDGE_EMBEDDINGS_TOTAL, "outcome" => "inserted").increment(1);
        } else {
            sweep.duplicates += 1;
            metrics::counter!(KNOWLEDGE_EMBEDDINGS_TOTAL, "outcome" => "duplicate").increment(1);
        }
    }
    write.commit().await.map_err(|error| Error::Storage {
        message: format!("commit Knowledge embedding sweep: {error}"),
    })?;
    Ok(sweep)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_bound_dependency_work() {
        let config = Config::default();
        assert_eq!(config.poll_interval, Duration::from_secs(1));
        assert_eq!(config.batch, 64);
    }
}
