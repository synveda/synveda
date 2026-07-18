//! The read path: hybrid retrieval (pgvector ANN + Tantivy BM25, RRF fusion) and
//! the composition engine (scope gradient, pinned-first, token budget, channel
//! rules). No LLM calls on this path (tech plan §3).
//!
//! Retrieval/composition implementation lands with CTX-1/CTX-2. Today the crate
//! carries the read path's readiness probe — the "core" leg of the
//! gateway→core→store trace (FND-5, ADR-0007).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use sqlx::PgPool;
use synveda_types::Result;

/// Verifies the read path can reach its storage backend. Ops-plane only: no
/// records are read, so nothing here needs (or may bypass) the PDP (seed §2.2).
#[tracing::instrument(name = "retrieval.readiness", skip_all, err(Display))]
pub async fn readiness(pool: &PgPool) -> Result<()> {
    synveda_store::ping(pool).await
}
