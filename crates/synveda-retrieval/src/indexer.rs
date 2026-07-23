//! The search indexer (CTX-1, ADR-0024 decision 4): a gateway-embedded
//! task that keeps each tenant's Tantivy sidecar converged with the
//! bitemporal record tables by polling a per-tenant transaction-time
//! watermark.
//!
//! Each sweep, per active tenant, inside a tenant RLS transaction: ids
//! changed since `watermark − overlap` are collected from the pair
//! (`records.tx_from` for new current versions, `records_history.tx_to`
//! for closed ones), each id's *current* version decides upsert or
//! delete, Tantivy commits, the reader reloads, and the watermark
//! advances to `max(old, max stamp seen, db_now − overlap)` — server
//! clock, never the client's. The overlap window re-scans idempotently,
//! covering writers whose stamp predates a concurrent sweep's read; a
//! writer holding its transaction open longer than the overlap is
//! outside the design (worker transactions are milliseconds), and the
//! recovery — like every other index doubt — is deleting the tenant's
//! index directory.
//!
//! Sweep failures are logged and retried next tick, per tenant: index
//! maintenance degrades to staleness (the hydration re-check keeps
//! stale one-sided, ADR-0024 decision 6), it never takes the gateway
//! down. Polling over LISTEN/NOTIFY mirrors ADR-0022's transport
//! decision, with the same recorded upgrade path.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use synveda_store::{rls, search, tenants};
use synveda_types::{RecordId, Result, TenantId};

use crate::index::{SearchIndex, record_document, record_term, sweep_writer};
use crate::{SEARCH_INDEX_DOCS_TOTAL, SEARCH_INDEX_SWEEPS_TOTAL};

/// Indexer pacing.
#[derive(Debug, Clone)]
pub struct IndexerConfig {
    /// Delay between sweeps (default 1s). Bounds BM25 visibility lag.
    pub poll_interval: Duration,
    /// The change-scan overlap window (default 10s, ADR-0024
    /// decision 4).
    pub overlap: Duration,
    /// Ids re-read per database round-trip within a sweep (default 512).
    pub batch: usize,
}

impl Default for IndexerConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(1),
            overlap: Duration::from_secs(10),
            batch: 512,
        }
    }
}

/// One tenant sweep's document operations.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TenantSweep {
    /// Documents written (insert or replace).
    pub upserts: u64,
    /// Documents removed (temporally deleted records).
    pub deletes: u64,
}

/// One full sweep's outcome across tenants.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SweepSummary {
    /// Tenants visited.
    pub tenants: usize,
    /// Documents written.
    pub upserts: u64,
    /// Documents removed.
    pub deletes: u64,
    /// Tenants whose sweep failed (logged; retried next tick).
    pub errors: usize,
}

/// Spawns the indexer loop: one sweep immediately, then one per
/// `poll_interval` (the pack-refresher pattern).
pub fn spawn(
    pool: PgPool,
    index: Arc<SearchIndex>,
    config: IndexerConfig,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(config.poll_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(error) = sweep_once(&pool, &index, &config).await {
                tracing::warn!(error = %error, "search index sweep failed");
            }
        }
    })
}

/// One sweep over every active tenant. Per-tenant failures are counted
/// and logged, never propagated — one tenant's corrupt index must not
/// starve the rest.
#[tracing::instrument(name = "retrieval.index.sweep", skip_all, err(Display))]
pub async fn sweep_once(
    pool: &PgPool,
    index: &SearchIndex,
    config: &IndexerConfig,
) -> Result<SweepSummary> {
    let mut summary = SweepSummary::default();
    for tenant in tenants::active(pool).await? {
        summary.tenants += 1;
        match sweep_tenant(pool, index, tenant.id, config).await {
            Ok(sweep) => {
                summary.upserts += sweep.upserts;
                summary.deletes += sweep.deletes;
                let outcome = if sweep == TenantSweep::default() {
                    "empty"
                } else {
                    "updated"
                };
                metrics::counter!(SEARCH_INDEX_SWEEPS_TOTAL, "outcome" => outcome).increment(1);
            }
            Err(error) => {
                summary.errors += 1;
                metrics::counter!(SEARCH_INDEX_SWEEPS_TOTAL, "outcome" => "error").increment(1);
                tracing::warn!(
                    tenant.id = %tenant.id,
                    error = %error,
                    "tenant search index sweep failed; retried next tick"
                );
            }
        }
    }
    Ok(summary)
}

/// One tenant's sweep (module docs). Public so tests and demos can
/// drive convergence deterministically instead of racing the loop.
#[tracing::instrument(
    name = "retrieval.index.sweep_tenant",
    skip_all,
    fields(tenant.id = %tenant_id, changed = tracing::field::Empty),
    err(Display)
)]
pub async fn sweep_tenant(
    pool: &PgPool,
    index: &SearchIndex,
    tenant_id: TenantId,
    config: &IndexerConfig,
) -> Result<TenantSweep> {
    let overlap =
        chrono::Duration::from_std(config.overlap).unwrap_or(chrono::Duration::seconds(10));
    let (slot, watermark) = index.open_for_write(tenant_id)?;
    let mut tx = rls::begin_tenant_tx(pool, tenant_id).await?;
    let scan = search::changes_since(&mut tx, tenant_id, watermark - overlap).await?;
    tracing::Span::current().record("changed", scan.ids.len());
    if scan.ids.is_empty() {
        return Ok(TenantSweep::default());
    }
    let mut sweep = TenantSweep::default();
    let mut writer = sweep_writer(&slot.index)?;
    for chunk in scan.ids.chunks(config.batch.max(1)) {
        let rows = search::for_index(&mut tx, tenant_id, chunk).await?;
        let present: HashSet<RecordId> = rows.iter().map(|row| row.id).collect();
        for row in &rows {
            writer.delete_term(record_term(slot.fields, row.id));
            writer
                .add_document(record_document(slot.fields, row))
                .map_err(|err| synveda_types::Error::Internal {
                    message: format!("search index add: {err}"),
                })?;
            sweep.upserts += 1;
        }
        for id in chunk {
            if !present.contains(id) {
                writer.delete_term(record_term(slot.fields, *id));
                sweep.deletes += 1;
            }
        }
    }
    // Read-only transaction; dropping it rolls back, GUC included.
    drop(tx);
    writer
        .commit()
        .map_err(|err| synveda_types::Error::Internal {
            message: format!("search index commit: {err}"),
        })?;
    slot.reader
        .reload()
        .map_err(|err| synveda_types::Error::Internal {
            message: format!("search index reload: {err}"),
        })?;
    // Idle-time advance rides the server clock so post-burst re-scans
    // die out within one overlap window (module docs). Only after the
    // Tantivy commit: a crash before this line re-scans idempotently.
    let advanced = [watermark, scan.db_now - overlap]
        .into_iter()
        .chain(scan.max_stamp)
        .max()
        .unwrap_or(watermark);
    index.store_watermark(tenant_id, advanced)?;
    metrics::counter!(SEARCH_INDEX_DOCS_TOTAL, "op" => "upsert").increment(sweep.upserts);
    metrics::counter!(SEARCH_INDEX_DOCS_TOTAL, "op" => "delete").increment(sweep.deletes);
    Ok(sweep)
}
