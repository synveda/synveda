-- CTX-1: hybrid retrieval (ADR-0024).
--
-- ANN indexing over the MEM-4 embedding sidecar. The column is
-- typmod-less `vector` (models with different dimensions coexist,
-- ADR-0023), and pgvector can only index a fixed-dimension expression —
-- so the index is a partial HNSW expression index per supported
-- dimension, and the dense-leg query casts through the identical
-- expression (ADR-0024 decision 5). The supported set is the shipped
-- embedders: 16 (the deterministic hash embedder) and 1024 (BGE-M3
-- dense via TEI). A deployment pinning a different-dimension model adds
-- its index and query variant as a reviewed diff — the same review that
-- admits the model.
--
-- Cosine ops match the read path's distance; the shipped vectors are
-- L2-normalised at the source (DeterministicEmbedder, TEI's normalised
-- BGE-M3 output), so cosine and inner-product orderings agree.

create index record_embeddings_hnsw_16 on record_embeddings
    using hnsw ((embedding::vector(16)) vector_cosine_ops)
    where dim = 16;

create index record_embeddings_hnsw_1024 on record_embeddings
    using hnsw ((embedding::vector(1024)) vector_cosine_ops)
    where dim = 1024;

-- The search indexer's change feed (ADR-0024 decision 4): the
-- bitemporal pair is tailed by transaction time — new/updated current
-- versions by records.tx_from, closed versions (updates and temporal
-- deletes) by records_history.tx_to. Both scans are per-tenant.

create index records_tenant_tx_from_idx
    on records (tenant_id, tx_from);

create index records_history_tenant_tx_to_idx
    on records_history (tenant_id, tx_to);

-- The dense leg's selective regime: when the allowed-scope slice is
-- small, the planner should prefer an exact scan over the slice to an
-- iterative HNSW crawl. Also serves hydration's re-check and future
-- scope-sliced listings (CTX-2).

create index records_tenant_scope_idx
    on records (tenant_id, scope_id);
