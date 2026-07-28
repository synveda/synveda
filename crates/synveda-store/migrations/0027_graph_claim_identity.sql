-- GRPH-2: a claim's identity, enforced (ADR-0044 decision 11).
--
-- Migration 0026 shipped the edge table without uniqueness, so two identical
-- `mentions` edges between the same record and the same name are
-- representable. Every other write path in this repo made re-assertion
-- structurally idempotent — `record_supersessions` on its primary key (0024),
-- VedaFlow objects on `(tenant_id, hash)` (0018), `graph_vertices` on its
-- resolution key (0026) — and a linker is exactly the shape of code that gets
-- re-driven: a redelivered signal, a re-linking sweep, a second contributor's
-- backfill. This is the reviewed diff ADR-0043 decision 10 said GRPH-2 would
-- bring, and it adds no column: the claim's identity was always
-- (tenant, graph, kind, src, dst); nothing was enforcing it.
--
-- The predicate is what keeps supersession legal. ADR-0043 decision 4 retires
-- an edge by closing its valid window and inserting the replacement, and both
-- rows stay in `graph_edges` — only transaction-time versioning moves rows to
-- the history table. So an unconditional unique index would refuse the second
-- half of every supersession. `where valid_to is null` says the thing actually
-- meant: at most one *open* claim of a given relation between a given pair.
--
-- `graph_edges_history` gets no such index and must not: a closed version is a
-- record of what was once claimed, and the same relation may have been opened
-- and closed any number of times.
--
-- Deliberately not here: no index on `source_record_id`. "Which claims came
-- from this record" is a question nothing asks yet — the linker knows what it
-- wrote, and GRPH-3 seeds from `graph_vertices_record_idx` — and shipping an
-- index variant before a query that needs it is what CTX-1 refused (ADR-0024).

create unique index graph_edges_open_claim_unique
    on graph_edges (tenant_id, graph, kind, src_id, dst_id)
    where valid_to is null;
