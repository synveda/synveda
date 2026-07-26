-- MEM-5: always-on dedup & conflict detection (ADR-0039).
--
-- Two tables and one column:
--
--   record_signatures     the MinHash signature of a record's content and the
--                         LSH bands it collides on — the lexical half of
--                         nomination, and the half that works when the
--                         embedder's geometry does not (ADR-0023 decision 6
--                         says the hash embedder's carries no meaning, and it
--                         is what dev, demos and `make eval` run on).
--   record_supersessions  the explicit edge: which record closed which, on
--                         whose judgement, at what similarity. The feature
--                         text's "supersession edges", relational rather than
--                         AGE because ADR-0029's G5 makes Cypher unsqlxable
--                         and this edge is read by the write path inside the
--                         record's own transaction (ADR-0039 option 7).
--   policy_packs.dedup    a stored pack's optional DedupConfig, exactly as
--                         `redaction` (0013) and `composition` (0017): null
--                         means the product default.
--
-- What is deliberately *not* here: any change to `records`. A supersession is
-- `valid_to` moving on a row that already has the column, which is what
-- ADR-0006 built valid time for and what ADR-0022 decision 7 left to this
-- feature. Nothing touches the bitemporal pair's structural rule.

-- ── The lexical nominator ───────────────────────────────────────────────────

-- One row per current record, written in the same statement as the record
-- (synveda_store::records::insert/update). No deferred constraint trigger,
-- unlike record_embeddings: an embedding-less record is unrepresentable in
-- the read path, while a signature-less one is merely invisible to this
-- nominator — degraded, not broken (ADR-0039 decision 3). Rows written before
-- this migration are not backfilled, as ADR-0023 recorded for the MEM-3
-- window.
create table record_signatures (
    record_id  uuid        not null,
    tenant_id  uuid        not null,
    -- The MinHash signature: MINHASH_PERMUTATIONS values over the record's
    -- normalised word set. Kept beside the bands because Jaccard is estimable
    -- from it without reading content — MEM-6 and EVAL-2 both want that, and
    -- it costs one array.
    signature  bigint[]    not null,
    -- The band hashes the signature groups into: overlap here is the
    -- nomination predicate, and the GIN index below is what makes it one.
    bands      bigint[]    not null,
    signed_at  timestamptz not null default now(),

    constraint record_signatures_pk primary key (record_id),
    constraint record_signatures_record_fk
        foreign key (record_id) references records (id) on delete cascade,
    constraint record_signatures_signature_check
        check (cardinality(signature) > 0),
    constraint record_signatures_bands_check
        check (cardinality(bands) > 0)
);

-- The nomination index: `bands && $1`. GIN over bigint[] is exactly the
-- shape array-overlap wants, and it keeps LSH to one indexed predicate
-- instead of a bucket table with a row per band (ADR-0039 decision 3).
create index record_signatures_bands_idx on record_signatures using gin (bands);

grant select, insert, update on record_signatures to synveda_app;

alter table record_signatures enable row level security;
alter table record_signatures force row level security;

create policy record_signatures_tenant_isolation on record_signatures
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

-- ── The supersession edge ───────────────────────────────────────────────────

-- Append-only by grant: an edge is a historical fact about a decision that
-- was taken, and reopening a window is a new bitemporal state on the record,
-- not the retraction of a row that says what happened (ADR-0039 compliance
-- notes). No DELETE grant; rows leave only through their records' cascades.
create table record_supersessions (
    tenant_id      uuid        not null,
    superseded_id  uuid        not null,
    superseding_id uuid        not null,
    -- Why the judge said so, and which judge. `method` is the seam's name
    -- (`deterministic` today; an LLM judge takes the same column), `reason`
    -- the short machine-readable verdict class.
    method         text        not null,
    reason         text        not null,
    -- The signals, as integer per-mille. Floats are not stored here for the
    -- same reason the audit payload refuses them (ADR-0019 decision 2): a
    -- number that jsonb or a client may reshape is a number that cannot be
    -- compared later. Null where a leg could not run — a neighbour embedded
    -- by another model has no comparable cosine.
    jaccard_permille integer,
    cosine_permille  integer,
    -- The window this edge closed: the instant `superseded_id.valid_to`
    -- moved to. Recorded here as well as on the record because the record's
    -- window can move again (MEM-6), and an edge that cannot say what it did
    -- is an edge an auditor cannot use.
    closed_at      timestamptz not null,
    decided_at     timestamptz not null default now(),

    constraint record_supersessions_pk primary key (superseded_id, superseding_id),
    constraint record_supersessions_superseded_fk
        foreign key (superseded_id) references records (id) on delete cascade,
    constraint record_supersessions_superseding_fk
        foreign key (superseding_id) references records (id) on delete cascade,
    -- A record superseding itself is a code bug that would make the record
    -- both current and closed by its own hand.
    constraint record_supersessions_distinct_check
        check (superseded_id <> superseding_id),
    constraint record_supersessions_method_check
        check (method <> ''),
    constraint record_supersessions_reason_check
        check (reason <> ''),
    constraint record_supersessions_jaccard_check
        check (jaccard_permille is null or jaccard_permille between 0 and 1000),
    constraint record_supersessions_cosine_check
        check (cosine_permille is null or cosine_permille between -1000 and 1000)
);

-- "What did this record close" and "what closed this record" are both asked:
-- the first by the write path when it reports a group, the second by an
-- auditor holding a record id and by CTX-5's as-of surface.
create index record_supersessions_superseding_idx
    on record_supersessions (tenant_id, superseding_id);

grant select, insert on record_supersessions to synveda_app;

alter table record_supersessions enable row level security;
alter table record_supersessions force row level security;

create policy record_supersessions_tenant_isolation on record_supersessions
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

-- ── The pack's dedup configuration ──────────────────────────────────────────

-- policy_packs.dedup — a stored pack's optional DedupConfig (mode plus the
-- three thresholds and the nomination depth); null means the product default,
-- which is supersession on. Embedded product packs carry compiled-in configs
-- and no row, exactly like policy_packs.redaction (0013) and .composition
-- (0017).
alter table policy_packs add column dedup jsonb;
