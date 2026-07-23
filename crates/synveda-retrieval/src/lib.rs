//! The read path: hybrid retrieval (pgvector ANN + Tantivy BM25, RRF
//! fusion — CTX-1, ADR-0024) and, with CTX-2, the composition engine
//! (scope gradient, pinned-first, token budget, channel rules). No LLM
//! calls on this path (tech plan §3): the crate's only network peer is
//! Postgres, and the query embedding is the caller's input.
//!
//! Retrieval is policy-shaped before it touches an index: the engine's
//! only entry takes an allowed-scope set, produced in the product paths
//! by [`authz::permitted_chain_scopes`] — one PDP `MemoryRead` decision
//! per candidate scope (seed §2.2 is never bypassed). The Tantivy
//! sidecar is maintained by [`indexer`]; Postgres current truth decides
//! what hydrates, so a lagging sidecar can only miss, never leak.
//!
//! The crate also carries the read path's readiness probe — the "core"
//! leg of the gateway→core→store trace (FND-5, ADR-0007).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod authz;
pub mod hybrid;
pub mod index;
pub mod indexer;

pub use authz::{MemoryReadInputs, permitted_chain_scopes};
pub use hybrid::{QueryVector, RetrievedRecord, SearchFilter, SearchRequest, hybrid_search};
pub use index::{SEARCH_SCHEMA_VERSION, SearchIndex, SparseHit};
pub use indexer::{IndexerConfig, SweepSummary, TenantSweep};

use sqlx::PgPool;
use synveda_types::Result;

/// Counter: sidecar sweeps per tenant, labelled `outcome` =
/// `updated` | `empty` | `error`. Emitted here, described by the
/// gateway where the recorder lives (ADR-0007).
pub const SEARCH_INDEX_SWEEPS_TOTAL: &str = "synveda_search_index_sweeps_total";

/// Counter: sidecar document operations, labelled `op` =
/// `upsert` | `delete`.
pub const SEARCH_INDEX_DOCS_TOTAL: &str = "synveda_search_index_docs_total";

/// Counter: hybrid searches, labelled `mode` = `hybrid` |
/// `sparse_only` | `dense_only` | `empty_filter`.
pub const RETRIEVAL_SEARCHES_TOTAL: &str = "synveda_retrieval_searches_total";

/// Histogram: per-leg latency, labelled `leg` = `dense` | `sparse` |
/// `hydrate`.
pub const RETRIEVAL_LEG_SECONDS: &str = "synveda_retrieval_leg_duration_seconds";

/// Verifies the read path can reach its storage backend. Ops-plane only: no
/// records are read, so nothing here needs (or may bypass) the PDP (seed §2.2).
#[tracing::instrument(name = "retrieval.readiness", skip_all, err(Display))]
pub async fn readiness(pool: &PgPool) -> Result<()> {
    synveda_store::ping(pool).await
}
