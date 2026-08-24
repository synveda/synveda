-- CPR-17: immutable-revision semantic search (ADR-0082 decision 3).
--
-- Lexical search already lives on `knowledge_revisions.search_document`.
-- This sidecar contains only a reproducible derivative of one immutable
-- revision under one explicit model. It is deliberately not a column on the
-- aggregate head: moving the current pointer never changes history, and a
-- model change can converge beside the old model without mutating either the
-- Knowledge revision or its earlier vector.
--
-- There is no record bridge. The old `record_embeddings` table remains only
-- for CPR-18's controlled context-composer cutover and no row moves between
-- the two tables.

create table knowledge_revision_embeddings (
    tenant_id            uuid        not null,
    knowledge_revision_id uuid       not null,
    model                text        not null,
    dim                  integer     not null,
    embedding            vector      not null,
    embedded_at          timestamptz not null default now(),

    constraint knowledge_revision_embeddings_pk
        primary key (tenant_id, knowledge_revision_id, model),
    constraint knowledge_revision_embeddings_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint knowledge_revision_embeddings_revision_fk
        foreign key (tenant_id, knowledge_revision_id)
        references knowledge_revisions (tenant_id, id) on delete cascade,
    constraint knowledge_revision_embeddings_model_check
        check (btrim(model) <> '' and char_length(model) <= 512),
    constraint knowledge_revision_embeddings_dim_check
        check (dim > 0 and dim = vector_dims(embedding))
);

create index knowledge_revision_embeddings_by_model
    on knowledge_revision_embeddings (tenant_id, model, embedded_at, knowledge_revision_id);

-- pgvector indexes a fixed-dimension expression over the typmod-less column.
-- These are the two dimensions the repository has actually tested: the
-- deterministic development checksum and BGE-M3 through TEI. The API never
-- calls the former semantic (ADR-0082).
create index knowledge_revision_embeddings_hnsw_16
    on knowledge_revision_embeddings
    using hnsw ((embedding::vector(16)) vector_cosine_ops)
    where dim = 16;

create index knowledge_revision_embeddings_hnsw_1024
    on knowledge_revision_embeddings
    using hnsw ((embedding::vector(1024)) vector_cosine_ops)
    where dim = 1024;

-- Append-only in the ordinary application path. A vector leaves through the
-- revision FK cascade during an authorised forget; FK actions do not need a
-- DELETE grant and the erasure transaction already owns the guarded revision
-- deletion.
grant select, insert on knowledge_revision_embeddings to synveda_app;

alter table knowledge_revision_embeddings enable row level security;
alter table knowledge_revision_embeddings force row level security;

create policy knowledge_revision_embeddings_tenant_isolation
    on knowledge_revision_embeddings
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
