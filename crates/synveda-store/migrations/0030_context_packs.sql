-- PRMT-2: the context-pack registry (ADR-0050).
--
-- Three tables, and only the third is a new *idea*. The first two are
-- migration 0029's draft row, split in two because a pack has documents:
-- `context_packs` is the bundle's identity and `context_pack_documents` is
-- what is in it. Everything else about a pack is already expressible —
-- `vedaflow_objects` addresses each document's bytes, `vedaflow_proposals`
-- reviews them, `vedaflow_refs` publishes them, and the channel's
-- first-parent line is its version history (ADR-0050 decision 1).
--
-- The third, `context_pack_chunks`, is the one thing the read half needs:
-- **the mapping from a `records` row to the document it was cut from.**
-- ADR-0050 decision 2 makes a pack's published content ordinary pinned
-- records, so retrieval, tiering, recall, the retention exemption and the
-- supersession exemption are inherited rather than rebuilt. What `records`
-- cannot say is which document a row is a chunk of, and at which address —
-- and that is exactly what decides whether it composes as published
-- (decision 3).
--
-- Why the drafts are not channels (decision 1, on ADR-0049 decision 2's
-- reasoning): ADR-0032 decision 2 kept `staged` unwritten because "a set
-- channel cannot express withdrawal", and an author replacing a document is
-- exactly that withdrawal. So there is deliberately no `context-pack/staged`
-- ref and nothing writes one.

create table context_packs (
    tenant_id     uuid        not null,
    -- The scope that stands behind it. Part of the key, because the *same*
    -- pack name at a nearer scope is how a team overrides the org's bundle
    -- — two rows, two packs, one name.
    scope_id      uuid        not null,
    -- One segment, lower-case, ≤64 characters: synveda_types::
    -- ContextPackName. This column's bound is the schema's half of that
    -- vocabulary; the type refuses the shapes a CHECK cannot describe, and
    -- the unit tests pin the two together.
    name          text        not null,
    description   text        not null,
    created_at    timestamptz not null default now(),
    created_by    uuid        not null,
    updated_at    timestamptz not null default now(),
    -- Who last authored into it. Deliberately *not* in any document's
    -- object address: a handover is not an edit, and demoting a published
    -- bundle for one would be a surprise nobody could act on (migration
    -- 0029's rule, unchanged).
    updated_by    uuid        not null,

    constraint context_packs_pk primary key (tenant_id, scope_id, name),
    constraint context_packs_tenant_fk foreign key (tenant_id) references tenants (id),
    constraint context_packs_name_check check (length(name) between 1 and 64),
    constraint context_packs_description_check check (length(description) between 0 and 512)
);

-- One document of one pack: what an author uploads, what a reviewer reads,
-- and the unit the published channel names.
create table context_pack_documents (
    tenant_id     uuid        not null,
    scope_id      uuid        not null,
    pack_name     text        not null,
    -- Path-shaped, ≤3 segments, ≤128 characters: synveda_types::
    -- DocumentName. With the pack's 64 that bounds the tree entry name
    -- `pack/document` at 193 — inside vedaflow_tree_entries.name (255) and
    -- inside vedaflow_refs.name (200), which is what a curator glob
    -- matches (ADR-0032).
    document_name text        not null,
    title         text        not null,
    -- Never 'restricted' (decision 12), for migration 0029's reason: the
    -- only mechanism in the product that mints that tier is a
    -- classification proposal over *records*, and this feature ships no
    -- classify effect for authored assets — so a restricted document would
    -- be a row nothing could have created and nothing could read back.
    --
    -- Declared per document rather than per pack, because a glossary of
    -- public terms and an internal runbook are plausibly the same bundle.
    -- Every chunk cut from this document inherits this tier, which is what
    -- the per-scope tier check then applies per entry (ADR-0038).
    sensitivity   text        not null,
    -- The address of exactly these bytes. The FK is the point: a document
    -- whose content is not in the object store is unrepresentable, so "the
    -- bytes a proposal will bind are already stored" is a property of the
    -- schema rather than of a handler.
    object_hash   bytea       not null,
    -- How many chunks this document cut into. Denormalised from
    -- `context_pack_chunks` on purpose: a listing answers "how much of my
    -- session would this cost" without counting rows, and the authoring
    -- transaction that wrote the chunks is the only thing that writes it.
    chunks        integer     not null,
    created_at    timestamptz not null default now(),
    created_by    uuid        not null,
    updated_at    timestamptz not null default now(),
    updated_by    uuid        not null,

    constraint context_pack_documents_pk
        primary key (tenant_id, scope_id, pack_name, document_name),
    constraint context_pack_documents_pack_fk
        foreign key (tenant_id, scope_id, pack_name)
        references context_packs (tenant_id, scope_id, name),
    constraint context_pack_documents_object_fk
        foreign key (tenant_id, object_hash) references vedaflow_objects (tenant_id, hash),
    constraint context_pack_documents_name_check
        check (length(document_name) between 1 and 128),
    constraint context_pack_documents_title_check check (length(title) between 0 and 160),
    constraint context_pack_documents_chunks_check check (chunks between 0 and 512),
    constraint context_pack_documents_sensitivity_check
        check (sensitivity in ('public', 'internal', 'confidential'))
);

-- ── The chunk mapping ───────────────────────────────────────────────────────

-- One row per chunk: which record it is, and which document address it was
-- cut from.
--
-- `document_hash` is the load-bearing column and the reason this table
-- exists. Composition admits a chunk as published only when the scope's
-- `context-pack/published` tree names `pack/document` at **exactly this
-- address** (ADR-0050 decision 3) — ADR-0031 decision 5's rule reaching
-- chunks through their document. Editing a published document moves its
-- address, so every chunk of the old one stops matching and falls off the
-- published set, rather than the edit being laundered through chunks the
-- tree still appears to name.
--
-- Rows are never rewritten in place on a re-author. A new version is new
-- chunk rows at a new document address; the old ones stay addressable,
-- which is what makes a FLOW-7 rewind a ref move with no re-embedding at
-- all (decision 6). The cost is chunk rows no live commit names —
-- ADR-0030's open GC question, not worsened in kind.
create table context_pack_chunks (
    tenant_id     uuid        not null,
    -- The `records` row. The FK is safe and is meant to be: the pinned
    -- exemption is a `kind = 'derived'` predicate in the retention sweep's
    -- own SQL (ADR-0040 decision 8, migration 0025), so nothing in the
    -- product can destroy a record this table points at. If something ever
    -- could, this constraint is the loud failure rather than the silent
    -- orphan.
    record_id     uuid        not null,
    -- Where the chunk composes from, and what the index tier renders:
    -- `pack/document § heading — title` (decision 10). Denormalised from
    -- the document row because that row holds only the *current* draft, and
    -- a chunk of a previous published version has to keep rendering — and
    -- because reading it back out of the document object would be one
    -- object read per composed chunk to render a line that is supposed to
    -- be the cheap alternative to a body.
    scope_id      uuid        not null,
    pack_name     text        not null,
    document_name text        not null,
    title         text        not null,
    -- The address the chunk was cut from. Not a FK to the document row —
    -- that row moves with the draft — but a FK to the object, which is
    -- append-only and therefore always resolvable.
    document_hash bytea       not null,
    -- Its position in the document, from zero.
    ordinal       integer     not null,
    -- The nearest enclosing heading, when the document had one.
    heading       text,

    constraint context_pack_chunks_pk primary key (tenant_id, record_id),
    constraint context_pack_chunks_tenant_fk foreign key (tenant_id) references tenants (id),
    constraint context_pack_chunks_record_fk foreign key (record_id) references records (id),
    constraint context_pack_chunks_object_fk
        foreign key (tenant_id, document_hash) references vedaflow_objects (tenant_id, hash),
    -- One record per (document version, position). The document address
    -- already covers the scope, the pack, the name, the tier, the title and
    -- the content, and the chunker is deterministic — so this pair is the
    -- chunk's identity, and re-authoring identical bytes finds the row
    -- rather than writing a second one.
    constraint context_pack_chunks_unique unique (tenant_id, document_hash, ordinal),
    constraint context_pack_chunks_ordinal_check check (ordinal between 0 and 511),
    constraint context_pack_chunks_title_check check (length(title) between 0 and 160),
    constraint context_pack_chunks_heading_check check (heading is null or length(heading) <= 512)
);

-- The composition read: given the document addresses a scope's channel
-- names, which records are their chunks. One indexed lookup per compose,
-- beside the one `read_memory_members` already does.
create index context_pack_chunks_by_document
    on context_pack_chunks (tenant_id, document_hash, ordinal);

-- No hierarchy FK on any of the three, on migration 0019's rule: recorded
-- governance must neither block a scope deletion nor be destroyed by one. A
-- draft at a deleted scope is on nobody's chain, so it resolves to nothing
-- at the only place it is read, and TEN-5's erasure disposes of it with the
-- rest.

-- ── What a draft may and may not become ─────────────────────────────────────

-- A draft is content that changes; that is the whole point of it. Its
-- *identity* does not. Migration 0029's trigger, applied to both draft
-- tables: a moved scope_id would relocate authored material past the
-- ContextPackWrite decision that admitted it; a renamed pack or document
-- would keep a published entry pointing at a name nobody reviewed it under;
-- and a rewritten created_at/created_by would erase who started it.
create function synveda_context_pack_transition() returns trigger
language plpgsql
as $$
begin
    if new.tenant_id  <> old.tenant_id
        or new.scope_id   <> old.scope_id
        or new.name       <> old.name
        or new.created_at <> old.created_at
        or new.created_by <> old.created_by
    then
        raise exception
            'context pack %/% is identified by its tenant, scope and name (PRMT-2); '
            'renaming or moving one is a new pack, not an edit',
            old.scope_id, old.name;
    end if;
    return new;
end
$$;

create trigger context_packs_transition
    before update on context_packs
    for each row execute function synveda_context_pack_transition();

create function synveda_context_pack_document_transition() returns trigger
language plpgsql
as $$
begin
    if new.tenant_id     <> old.tenant_id
        or new.scope_id      <> old.scope_id
        or new.pack_name     <> old.pack_name
        or new.document_name <> old.document_name
        or new.created_at    <> old.created_at
        or new.created_by    <> old.created_by
    then
        raise exception
            'context pack document %/%/% is identified by its tenant, scope, pack and '
            'name (PRMT-2); renaming or moving one is a new document, not an edit',
            old.scope_id, old.pack_name, old.document_name;
    end if;
    return new;
end
$$;

create trigger context_pack_documents_transition
    before update on context_pack_documents
    for each row execute function synveda_context_pack_document_transition();

-- A chunk row is a fact about an immutable pair — a record and the document
-- address it was cut from — so it has no edit at all. Nothing is granted
-- UPDATE below, and this trigger is the backstop for a principal who has
-- one anyway (the migration 0029 discipline, one step stricter because
-- there is nothing here that could legitimately change).
create function synveda_context_pack_chunk_immutable() returns trigger
language plpgsql
as $$
begin
    raise exception
        'context pack chunk % is a fact about a record and a document address (PRMT-2); '
        're-authoring a document writes new chunks rather than rewriting these',
        old.record_id;
end
$$;

create trigger context_pack_chunks_immutable
    before update on context_pack_chunks
    for each row execute function synveda_context_pack_chunk_immutable();

-- ── RLS + least-privilege grants ────────────────────────────────────────────

-- Tenant-scoped tables ⇒ forced RLS + policy + least-privilege grants in the
-- same migration (ADR-0009's structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it).
--
-- No DELETE grant anywhere, and that is a decision rather than an omission
-- (ADR-0050 decision 14, on ADR-0049's): retracting a *published* pack is
-- FLOW-7's rewind, which works for `context-pack/published` the moment
-- ContextPackRead exists, and replacing a draft is an overwrite. No UPDATE
-- on the chunk table, for the reason its trigger gives.
grant select, insert, update on context_packs to synveda_app;
grant select, insert, update on context_pack_documents to synveda_app;
grant select, insert on context_pack_chunks to synveda_app;

alter table context_packs enable row level security;
alter table context_packs force row level security;
alter table context_pack_documents enable row level security;
alter table context_pack_documents force row level security;
alter table context_pack_chunks enable row level security;
alter table context_pack_chunks force row level security;

create policy context_packs_tenant_isolation on context_packs
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy context_pack_documents_tenant_isolation on context_pack_documents
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy context_pack_chunks_tenant_isolation on context_pack_chunks
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
