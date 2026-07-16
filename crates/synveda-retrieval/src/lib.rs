//! The read path: hybrid retrieval (pgvector ANN + Tantivy BM25, RRF fusion) and
//! the composition engine (scope gradient, pinned-first, token budget, channel
//! rules). No LLM calls on this path (tech plan §3).
//!
//! Implementation lands with CTX-1/CTX-2.

use synveda_types as _;
