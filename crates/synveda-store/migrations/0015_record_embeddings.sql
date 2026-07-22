-- MEM-4: transactional embed-or-fail (ADR-0023).
--
-- Every current record carries exactly one embedding in this sidecar
-- table — never a column on the bitemporal pair (history is provenance;
-- a re-embed on model change regenerates vectors rather than archiving
-- stale ones). The column is typmod-less `vector` so models with
-- different dimensions coexist; per-row model + dim record which one
-- produced it. ANN indexing over this table belongs to CTX-1.
--
-- The write path (synveda_store::records::insert/update) writes the
-- record and its embedding in one statement; the deferred constraint
-- trigger below is the schema backstop that makes an embedding-less
-- record impossible to COMMIT, whatever the writer (ADR-0006/0009
-- defence-in-depth tradition). Rows created before this migration (the
-- MEM-3 window) are untouched: the constraint governs new inserts; the
-- re-embed workflow (tech plan §1.3) owns the backfill.

create extension if not exists vector;

create table record_embeddings (
    record_id   uuid        not null,
    tenant_id   uuid        not null,
    model       text        not null,
    dim         integer     not null,
    embedding   vector      not null,
    embedded_at timestamptz not null default now(),

    constraint record_embeddings_pk primary key (record_id),
    constraint record_embeddings_record_fk
        foreign key (record_id) references records (id) on delete cascade,
    constraint record_embeddings_model_check
        check (model <> ''),
    constraint record_embeddings_dim_check
        check (dim > 0 and dim = vector_dims(embedding))
);

-- Tenant isolation backstop (TEN-2, ADR-0009 structural rule): forced
-- RLS in the creating migration. No DELETE grant on purpose — an
-- embedding row leaves only through its record's FK cascade (FK actions
-- bypass RLS and grants by Postgres semantics), so ad-hoc deletion that
-- would strand a record embedding-less has no privilege to run under.
grant select, insert, update on record_embeddings to synveda_app;

alter table record_embeddings enable row level security;
alter table record_embeddings force row level security;

create policy record_embeddings_tenant_isolation on record_embeddings
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

-- ── The embed-or-fail backstop ───────────────────────────────────────────────
-- Deferred to commit so any same-transaction ordering satisfies it; a
-- record whose embedding never arrives cannot commit at all.

create function records_require_embedding() returns trigger
language plpgsql as $$
begin
    if not exists (select from record_embeddings where record_id = new.id) then
        raise exception
            'record % has no embedding: records commit embed-or-fail (ADR-0023)',
            new.id;
    end if;
    return null;
end;
$$;

create constraint trigger records_require_embedding
    after insert on records
    deferrable initially deferred
    for each row execute function records_require_embedding();
