-- GRPH-1: the multi-graph schema — indexed adjacency in Postgres (ADR-0043).
--
-- Three tables and one view:
--
--   graph_vertices        identity. One row per thing the graph can talk
--                         about, `(tenant_id, graph, kind, key)` unique so
--                         GRPH-2's entity resolution has a place to converge.
--                         Not bitemporal: a vertex asserts that a thing exists
--                         and is named, which is not a claim about the world
--                         that can be superseded (ADR-0043 decision 5).
--   graph_edges           claims, and every revisable statement is one. A
--                         bitemporal row of exactly the `records` shape —
--                         the same pattern as migration 0001, not an
--                         imitation of it, so the graph answers `as_of`
--                         through the same view shape `records_versions`
--                         gave CTX-5 (decision 3).
--   graph_edges_history   the closed versions, archived by the triggers below.
--   graph_edges_versions  every version the database has ever known.
--
-- Migration 0001's structural rule governs this pair as it governs that one,
-- and this migration doubles the surface it applies to: an alteration to
-- `graph_edges` must make the identical change to `graph_edges_history`, to
-- `graph_edges_versions`, and to the explicit column lists in the archive
-- trigger functions, in the same migration. The trigger functions are written
-- out per table because they enumerate columns; that duplication is the price
-- decision 3 accepted for the shared shape.
--
-- What is deliberately *not* here:
--
--   * No AGE. ADR-0004 chose per-tenant Cypher graphs; GRPH-4 measured the
--     relational baseline 3–8× faster at 2.5× less storage on the traversal
--     GRPH-3 will actually issue, and ADR-0043 overturned the engine while
--     keeping the named graphs. No crate calls the extension.
--   * No `properties jsonb` on edges. An edge property nobody queries is a
--     column nobody reviewed; GRPH-2 adds the columns it needs as a reviewed
--     diff (decision 10). `method`, `confidence_permille` and
--     `source_record_id` are the provenance of an assertion, typed.
--   * No `scope_id` on either table, and this one is load-bearing. The graph
--     is never a scope producer (decision 12): expansion runs *before*
--     admission and hands candidate ids into ADR-0042 decision 12's fused
--     list, which `admit` narrows and never widens. A scope column here would
--     be an authorisation input the PDP never granted — the exact shape by
--     which a knowledge graph becomes a policy bypass.
--   * No mirror of `record_supersessions`. That table stays the system of
--     record for its claim because the write path reads it inside the
--     record's own transaction (ADR-0039 option 7); traversal reaches it
--     through a projection GRPH-2 owns. One system of record per claim
--     (decision 11).
--   * No identity- or scope-backed vertex *columns*. Decision 5 makes a
--     record-backed vertex representable because `records.class` already
--     carries `entity` and `episode` and GRPH-2 resolves against them; a
--     vertex that names an identity or a scope is expressible today as
--     `(kind, key)` and earns a foreign-key column when a feature actually
--     writes one — decision 10's discipline, applied to this table.

-- ── Identity ────────────────────────────────────────────────────────────────

create table graph_vertices (
    id         uuid        not null,
    tenant_id  uuid        not null,
    -- ADR-0004's named graphs, surviving its engine: the semantic partition
    -- MAGMA's finding is actually about. Mandatory here and mandatory in the
    -- traversal API, which takes a `Graph` by value with no default and no
    -- `Option` (decision 2).
    graph      text        not null,
    -- What sort of thing this is ('person', 'org', 'episode', …). Open
    -- vocabulary: entity types are the extraction pipeline's business, not a
    -- closed product enum like `records.class`.
    kind       text        not null,
    -- The resolution key GRPH-2 converges on — the normalised form, unique
    -- within (tenant, graph, kind).
    key        text        not null,
    -- The display form the key was normalised from. One column so a caller
    -- showing an expansion result need not join a record that may not exist.
    label      text        not null,
    -- The backing record, when this vertex is a thing the corpus already
    -- holds rather than one the graph invented. Cascade: a vertex whose
    -- record is destroyed by retention is a name for nothing.
    --
    -- `records` is keyed by `id` alone, so this reference is not composite
    -- and a cross-tenant backing is prevented by RLS and by the write path
    -- rather than by the schema — the shape `record_embeddings` (0015) and
    -- `record_signatures` (0024) already ship. The graph's *own* references
    -- below are composite, which is where decision 7's guarantee lives.
    record_id  uuid,
    created_at timestamptz not null default now(),

    constraint graph_vertices_pk primary key (id),
    -- The composite target that makes a cross-tenant *and* cross-graph edge
    -- unrepresentable below. `graph` is in the key so an edge cannot join two
    -- semantic domains — ADR-0043 accepted a discriminator defended in Rust
    -- alone; including it here costs one index and answers ADR-0004 option
    -- 2's leak-by-omission objection structurally on the write side. The read
    -- side stays defended by the mandatory `Graph` argument.
    constraint graph_vertices_tenant_graph_id_unique unique (tenant_id, graph, id),
    constraint graph_vertices_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint graph_vertices_record_fk
        foreign key (record_id) references records (id) on delete cascade,
    -- Where entity resolution converges (decision 5).
    constraint graph_vertices_key_unique unique (tenant_id, graph, kind, key),
    constraint graph_vertices_graph_check
        check (graph in ('entity', 'episode', 'provenance')),
    constraint graph_vertices_kind_check check (length(kind) between 1 and 64),
    constraint graph_vertices_key_check check (length(key) between 1 and 512),
    constraint graph_vertices_label_check check (length(label) between 1 and 512)
);

-- "Which vertices back this record" — how GRPH-3 turns a retrieved record
-- into an expansion seed, and how GRPH-2 checks whether linking already ran.
create index graph_vertices_record_idx
    on graph_vertices (tenant_id, record_id)
    where record_id is not null;

-- ── Claims ──────────────────────────────────────────────────────────────────

create table graph_edges (
    id                  uuid        not null,
    tenant_id           uuid        not null,
    graph               text        not null,
    -- The relation type ('mentions', 'reports_to', 'occurred_during', …).
    -- Open vocabulary for the same reason `graph_vertices.kind` is.
    kind                text        not null,
    src_id              uuid        not null,
    dst_id              uuid        not null,
    -- Who asserted the claim and how sure they were. `method` is the seam's
    -- name, exactly as `record_supersessions.method` (0024) — `deterministic`
    -- today, an LLM linker taking the same column. Confidence is integer per
    -- mille, never a float: a number jsonb or a client may reshape is a
    -- number that cannot be compared later (the MEM-5 discipline, ADR-0039
    -- decision 2's reasoning inherited).
    method              text        not null,
    confidence_permille integer     not null,
    -- The evidence: the record this claim was extracted from. Null where a
    -- claim has no single source — a projection, or a linker that fused
    -- several. Cascade, like the vertex's backing: an edge outliving its
    -- evidence is an assertion nobody can audit.
    source_record_id    uuid,
    -- Valid time is ordinary application data; transaction time is written
    -- ONLY by the triggers below and cannot be forged by application SQL.
    -- Open-ended bounds are NULL (ADR-0006, migration 0001).
    valid_from          timestamptz not null,
    valid_to            timestamptz,          -- null = no known end of validity
    tx_from             timestamptz not null, -- written by triggers only
    tx_to               timestamptz,          -- null on every current row, by check

    constraint graph_edges_pk primary key (id),
    -- Both endpoints carry a foreign key, and both are composite: an edge
    -- between two tenants — or between two graphs — is unrepresentable, not
    -- merely refused (decision 6, decision 7; the `hierarchy_closure`
    -- pattern). Cascade so a deleted vertex takes its claims with it, through
    -- the archive trigger rather than around it.
    constraint graph_edges_src_fk
        foreign key (tenant_id, graph, src_id)
        references graph_vertices (tenant_id, graph, id) on delete cascade,
    constraint graph_edges_dst_fk
        foreign key (tenant_id, graph, dst_id)
        references graph_vertices (tenant_id, graph, id) on delete cascade,
    constraint graph_edges_source_record_fk
        foreign key (source_record_id) references records (id) on delete cascade,
    constraint graph_edges_graph_check
        check (graph in ('entity', 'episode', 'provenance')),
    constraint graph_edges_kind_check check (length(kind) between 1 and 64),
    constraint graph_edges_method_check check (method <> ''),
    constraint graph_edges_confidence_check
        check (confidence_permille between 0 and 1000),
    -- A thing related to itself is a resolution bug, not a claim
    -- (`record_supersessions_distinct_check`, same reasoning).
    constraint graph_edges_endpoints_check check (src_id <> dst_id),
    constraint graph_edges_valid_period_check
        check (valid_to is null or valid_from < valid_to),
    constraint graph_edges_tx_to_is_null_check
        check (tx_to is null)
);

create table graph_edges_history (
    id                  uuid        not null,
    tenant_id           uuid        not null,
    graph               text        not null,
    kind                text        not null,
    src_id              uuid        not null,
    dst_id              uuid        not null,
    method              text        not null,
    confidence_permille integer     not null,
    source_record_id    uuid,
    valid_from          timestamptz not null,
    valid_to            timestamptz,
    tx_from             timestamptz not null,
    tx_to               timestamptz not null, -- history rows are always closed

    constraint graph_edges_history_pk primary key (id, tx_from),
    -- No foreign keys, exactly as `records_history` has none: a closed
    -- version outlives the vertices and records it names, and history that
    -- disappears when its endpoints do is not history.
    constraint graph_edges_history_graph_check
        check (graph in ('entity', 'episode', 'provenance')),
    constraint graph_edges_history_kind_check check (length(kind) between 1 and 64),
    constraint graph_edges_history_method_check check (method <> ''),
    constraint graph_edges_history_confidence_check
        check (confidence_permille between 0 and 1000),
    constraint graph_edges_history_endpoints_check check (src_id <> dst_id),
    constraint graph_edges_history_valid_period_check
        check (valid_to is null or valid_from < valid_to),
    constraint graph_edges_history_tx_period_check
        check (tx_from < tx_to)
);

-- Every version the database has ever known — the as-of traversal surface,
-- the same shape `records_versions` gives the corpus.
create view graph_edges_versions as
select id, tenant_id, graph, kind, src_id, dst_id, method, confidence_permille,
       source_record_id, valid_from, valid_to, tx_from, tx_to
from graph_edges
union all
select id, tenant_id, graph, kind, src_id, dst_id, method, confidence_permille,
       source_record_id, valid_from, valid_to, tx_from, tx_to
from graph_edges_history;

-- ── The adjacency indexes ───────────────────────────────────────────────────
-- What decision 9's plan assertion defends: the AC suite reads
-- `explain (format json)` for the shipped statements and fails on a
-- sequential scan over `graph_edges`, because a plan that regresses silently
-- is how the discipline dies on contact with the second contributor.
--
-- Expansion is undirected — a seed matches either endpoint — so the traversal
-- is two indexed legs, each `(tenant_id, graph, <endpoint> = any($seeds))`
-- with the temporal predicate as a heap filter. That is the shape GRPH-4
-- measured at 1.24ms (1-hop) and 4.84ms (2-hop) over 10M edges.

create index graph_edges_src_idx on graph_edges (tenant_id, graph, src_id);
create index graph_edges_dst_idx on graph_edges (tenant_id, graph, dst_id);

-- The as-of legs read the view, whose history half needs the same two
-- indexes or a rewind pays a sequential scan the current-time path does not.
create index graph_edges_history_src_idx
    on graph_edges_history (tenant_id, graph, src_id);
create index graph_edges_history_dst_idx
    on graph_edges_history (tenant_id, graph, dst_id);

-- ── Transaction-time maintenance ────────────────────────────────────────────
-- Migration 0001's functions, written out for this table because they
-- enumerate columns. Behaviour is identical, including the clock-anomaly
-- serialization failure and the empty-transaction-period case.

create function graph_edges_tx_insert() returns trigger
language plpgsql as $$
begin
    -- Transaction time is server truth; anything the application supplied
    -- is overwritten.
    new.tx_from := now();
    new.tx_to := null;
    return new;
end;
$$;

create function graph_edges_tx_update() returns trigger
language plpgsql as $$
begin
    -- An edge's identity is its endpoints, its relation type and its
    -- semantic domain. Changing any of them makes it a different claim, and
    -- decision 4 says a different claim is a closed window plus a new row —
    -- never an update that rewrites what the old window meant.
    if new.id is distinct from old.id then
        raise exception 'graph_edges.id is immutable; close the window and insert a new edge';
    end if;
    if new.tenant_id is distinct from old.tenant_id then
        raise exception 'graph_edges.tenant_id is immutable; an edge never changes tenant';
    end if;
    if new.graph is distinct from old.graph then
        raise exception 'graph_edges.graph is immutable; a claim does not change semantic domain';
    end if;
    if new.src_id is distinct from old.src_id or new.dst_id is distinct from old.dst_id then
        raise exception 'graph_edges endpoints are immutable; close the window and insert a new edge';
    end if;
    if new.kind is distinct from old.kind then
        raise exception 'graph_edges.kind is immutable; a different relation is a different claim';
    end if;
    if old.tx_from > now() then
        -- A concurrent transaction with a later clock already committed this
        -- version; closing it "before it began" would record a
        -- negative-length period. Fail like a serialization conflict.
        raise exception 'transaction-time clock anomaly on graph_edges.id=%: version began at %, now() is %; retry',
            old.id, old.tx_from, now()
            using errcode = 'serialization_failure';
    end if;
    if old.tx_from < now() then
        insert into graph_edges_history
            (id, tenant_id, graph, kind, src_id, dst_id, method,
             confidence_permille, source_record_id, valid_from, valid_to,
             tx_from, tx_to)
        values
            (old.id, old.tenant_id, old.graph, old.kind, old.src_id,
             old.dst_id, old.method, old.confidence_permille,
             old.source_record_id, old.valid_from, old.valid_to,
             old.tx_from, now());
    end if;
    -- When old.tx_from = now() the replaced version's transaction period is
    -- empty — it never existed in transaction time — so no history row.
    new.tx_from := now();
    new.tx_to := null;
    return new;
end;
$$;

create function graph_edges_tx_delete() returns trigger
language plpgsql as $$
begin
    if old.tx_from > now() then
        raise exception 'transaction-time clock anomaly on graph_edges.id=%: version began at %, now() is %; retry',
            old.id, old.tx_from, now()
            using errcode = 'serialization_failure';
    end if;
    if old.tx_from < now() then
        insert into graph_edges_history
            (id, tenant_id, graph, kind, src_id, dst_id, method,
             confidence_permille, source_record_id, valid_from, valid_to,
             tx_from, tx_to)
        values
            (old.id, old.tenant_id, old.graph, old.kind, old.src_id,
             old.dst_id, old.method, old.confidence_permille,
             old.source_record_id, old.valid_from, old.valid_to,
             old.tx_from, now());
    end if;
    return old;
end;
$$;

create trigger graph_edges_tx_insert before insert on graph_edges
    for each row execute function graph_edges_tx_insert();
create trigger graph_edges_tx_update before update on graph_edges
    for each row execute function graph_edges_tx_update();
create trigger graph_edges_tx_delete before delete on graph_edges
    for each row execute function graph_edges_tx_delete();

-- ── Guard rails ─────────────────────────────────────────────────────────────
-- Not a security boundary (a superuser can drop triggers) — defence in depth
-- against application bugs, exactly as migration 0001 says of the pair it
-- guards.

create function graph_edges_history_append_only() returns trigger
language plpgsql as $$
begin
    raise exception 'graph_edges_history is append-only (% attempted)', tg_op;
end;
$$;

create trigger graph_edges_history_append_only
    before update or delete on graph_edges_history
    for each row execute function graph_edges_history_append_only();
create trigger graph_edges_history_no_truncate
    before truncate on graph_edges_history
    for each statement execute function graph_edges_history_append_only();

create function graph_edges_block_truncate() returns trigger
language plpgsql as $$
begin
    raise exception 'truncate on graph_edges would bypass history archiving; delete rows instead';
end;
$$;

create trigger graph_edges_no_truncate
    before truncate on graph_edges
    for each statement execute function graph_edges_block_truncate();

-- ── Grants and tenant isolation ─────────────────────────────────────────────
-- Tenant-scoped tables ⇒ forced RLS + policy + least-privilege grants in the
-- same migration (ADR-0009 structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it, and decision 8 puts all
-- three tables into its adversarial suite).

-- No DELETE on graph_vertices: a vertex leaves through its record's cascade,
-- and a merge that retires one is GRPH-2's feature with GRPH-2's grant.
grant select, insert, update on graph_vertices to synveda_app;

-- No DELETE on graph_edges either, and this one is an ADR commitment: an edge
-- is retired by closing its window (decision 4), and ADR-0043's compliance
-- note reserves direct authorship or deletion of an edge for "a new action, a
-- new grant and a new ADR". Cascades still reach these rows — foreign-key
-- actions bypass grants by Postgres semantics — and they run through the
-- archive trigger, so nothing leaves without a history row.
grant select, insert, update on graph_edges to synveda_app;

-- INSERT on the history table is required because the archive triggers above
-- run with invoker rights, exactly as migration 0003 records for
-- records_history. No DELETE: destruction past a horizon is retention's, and
-- when MEM-6's sweep learns to purge graph history it brings its own grant
-- and the `synveda.retention_purge` path through the append-only trigger
-- (migration 0025's shape), rather than a standing privilege here.
grant select, insert on graph_edges_history to synveda_app;
grant select on graph_edges_versions to synveda_app;

alter table graph_vertices enable row level security;
alter table graph_vertices force row level security;
alter table graph_edges enable row level security;
alter table graph_edges force row level security;
alter table graph_edges_history enable row level security;
alter table graph_edges_history force row level security;

create policy graph_vertices_tenant_isolation on graph_vertices
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy graph_edges_tenant_isolation on graph_edges
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy graph_edges_history_tenant_isolation on graph_edges_history
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

-- Without this the as-of surface would evaluate base-table RLS as the view
-- OWNER (which may bypass), silently defeating the backstop for history —
-- the same line migration 0003 draws under records_versions.
alter view graph_edges_versions set (security_invoker = on);
