-- CPR-15: stable Knowledge aggregates, immutable revisions and normalised
-- provenance (ADR-0068 decisions 6 and 7; ADR-0080).
--
-- This migration deliberately does not touch `records`. There is no bridge,
-- trigger, view, backfill or dual write between the two models. For the two
-- bounded cutover packages that follow, old extraction/retrieval continues to
-- read records and no production path writes Knowledge. CPR-16 adds the
-- governed command seam; CPR-17 moves public reads and deletes the replaced
-- record plane according to ADR-0080's checklist.
--
-- Four domain nouns, six tenant tables:
--
--   knowledge_items               stable current aggregate heads
--   knowledge_items_history       closed head states (transaction time)
--   knowledge_revisions           immutable content
--   knowledge_sources             independently governed provenance
--   knowledge_revision_sources    many-to-many provenance links
--   knowledge_relations           explicit item-to-item claims
--
-- Two views:
--
--   knowledge_item_versions       current + historical head state
--   knowledge_current             each head joined to its current revision
--
-- Both are security-invoker, for ADR-0009's reason: a view evaluated as its
-- owner would silently bypass forced RLS.

-- Canonical tags are a database fact as well as a Rust validation. The store
-- lower-cases, sorts and deduplicates first; this function prevents a writer
-- holding a connection from creating two hashes for the same tag set.
create function synveda_knowledge_tags_canonical(value text[]) returns boolean
language sql
immutable
parallel safe
as $$
    select cardinality(value) <= 64
       and not exists (
           select
           from unnest(value) as raw(tag)
           where tag <> lower(btrim(tag))
              or char_length(tag) not between 1 and 64
       )
       and value = coalesce(
           (
               select array_agg(tag order by tag)
               from (
                   select distinct lower(btrim(raw.tag)) as tag
                   from unnest(value) as raw(tag)
               ) canonical
           ),
           '{}'::text[]
       )
$$;

-- ── Stable aggregate head ────────────────────────────────────────────────

create table knowledge_items (
    id                   uuid        not null,
    tenant_id            uuid        not null,
    scope_id             uuid        not null,
    project_id           uuid,
    owner_principal_id   text,
    knowledge_type       text        not null,
    origin               text        not null,
    lifecycle_state      text        not null default 'active',
    current_revision_id  uuid        not null,
    created_by           text,
    updated_by           text,
    created_at           timestamptz not null default now(),
    updated_at           timestamptz not null default now(),
    tx_from              timestamptz not null default now(),
    tx_to                timestamptz,

    constraint knowledge_items_pk primary key (id),
    constraint knowledge_items_tenant_id_unique unique (tenant_id, id),
    constraint knowledge_items_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint knowledge_items_scope_fk
        foreign key (tenant_id, scope_id) references scopes (tenant_id, id),
    constraint knowledge_items_project_fk
        foreign key (tenant_id, project_id) references projects (tenant_id, id),
    constraint knowledge_items_owner_check
        check (owner_principal_id is null
               or (btrim(owner_principal_id) <> ''
                   and char_length(owner_principal_id) <= 255)),
    constraint knowledge_items_type_check
        check (knowledge_type in (
            'fact', 'decision', 'preference', 'procedure', 'entity',
            'episode', 'convention', 'warning', 'reference'
        )),
    constraint knowledge_items_origin_check
        check (origin in ('observed', 'asserted', 'authored', 'imported')),
    constraint knowledge_items_lifecycle_check
        check (lifecycle_state in (
            'active', 'stale', 'superseded', 'archived',
            'erasure_pending', 'erased'
        )),
    constraint knowledge_items_created_by_check
        check (created_by is null
               or (btrim(created_by) <> '' and char_length(created_by) <= 255)),
    constraint knowledge_items_updated_by_check
        check (updated_by is null
               or (btrim(updated_by) <> '' and char_length(updated_by) <= 255)),
    constraint knowledge_items_time_check
        check (updated_at >= created_at and tx_from >= created_at),
    constraint knowledge_items_current_tx_open_check check (tx_to is null)
);

create index knowledge_items_by_scope
    on knowledge_items (tenant_id, scope_id, lifecycle_state, updated_at desc, id);
create index knowledge_items_by_project
    on knowledge_items (tenant_id, project_id, lifecycle_state, updated_at desc, id)
    where project_id is not null;
create index knowledge_items_by_owner
    on knowledge_items (tenant_id, owner_principal_id, lifecycle_state, updated_at desc, id)
    where owner_principal_id is not null;
create index knowledge_items_by_type
    on knowledge_items (tenant_id, knowledge_type, origin, lifecycle_state);

-- Identical head shape, but every row is a closed transaction interval. The
-- archive trigger below is its only ordinary writer.
create table knowledge_items_history (
    id                   uuid        not null,
    tenant_id            uuid        not null,
    scope_id             uuid        not null,
    project_id           uuid,
    owner_principal_id   text,
    knowledge_type       text        not null,
    origin               text        not null,
    lifecycle_state      text        not null,
    current_revision_id  uuid        not null,
    created_by           text,
    updated_by           text,
    created_at           timestamptz not null,
    updated_at           timestamptz not null,
    tx_from              timestamptz not null,
    tx_to                timestamptz not null,

    constraint knowledge_items_history_pk primary key (tenant_id, id, tx_from),
    constraint knowledge_items_history_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint knowledge_items_history_scope_fk
        foreign key (tenant_id, scope_id) references scopes (tenant_id, id),
    constraint knowledge_items_history_project_fk
        foreign key (tenant_id, project_id) references projects (tenant_id, id),
    constraint knowledge_items_history_owner_check
        check (owner_principal_id is null
               or (btrim(owner_principal_id) <> ''
                   and char_length(owner_principal_id) <= 255)),
    constraint knowledge_items_history_type_check
        check (knowledge_type in (
            'fact', 'decision', 'preference', 'procedure', 'entity',
            'episode', 'convention', 'warning', 'reference'
        )),
    constraint knowledge_items_history_origin_check
        check (origin in ('observed', 'asserted', 'authored', 'imported')),
    constraint knowledge_items_history_lifecycle_check
        check (lifecycle_state in (
            'active', 'stale', 'superseded', 'archived',
            'erasure_pending', 'erased'
        )),
    constraint knowledge_items_history_created_by_check
        check (created_by is null
               or (btrim(created_by) <> '' and char_length(created_by) <= 255)),
    constraint knowledge_items_history_updated_by_check
        check (updated_by is null
               or (btrim(updated_by) <> '' and char_length(updated_by) <= 255)),
    constraint knowledge_items_history_time_check
        check (updated_at >= created_at
               and tx_from >= created_at
               and tx_to > tx_from)
);

create index knowledge_items_history_as_known
    on knowledge_items_history (tenant_id, id, tx_from, tx_to);
create index knowledge_items_history_by_scope
    on knowledge_items_history (tenant_id, scope_id, tx_from, tx_to);

-- ── Immutable content revisions ─────────────────────────────────────────

create table knowledge_revisions (
    id                     uuid        not null,
    tenant_id              uuid        not null,
    knowledge_item_id      uuid        not null,
    revision_number        bigint      not null,
    title                  text        not null,
    body_markdown          text        not null,
    summary                text        not null,
    tags                   text[]      not null default '{}'::text[],
    sensitivity            text        not null,
    confidence_permille    integer     not null,
    valid_from             timestamptz not null,
    valid_to               timestamptz,
    stale_after            timestamptz,
    verification_metadata  jsonb       not null default '{}'::jsonb,
    content_hash           text        not null,
    metadata               jsonb       not null default '{}'::jsonb,
    created_by             text,
    transaction_time       timestamptz not null default now(),
    search_document        tsvector generated always as (
        setweight(to_tsvector('simple'::regconfig, title), 'A') ||
        setweight(to_tsvector('simple'::regconfig, summary), 'B') ||
        setweight(to_tsvector('simple'::regconfig, body_markdown), 'C')
    ) stored,

    constraint knowledge_revisions_pk primary key (id),
    constraint knowledge_revisions_tenant_id_unique unique (tenant_id, id),
    constraint knowledge_revisions_item_id_unique
        unique (tenant_id, knowledge_item_id, id),
    constraint knowledge_revisions_number_unique
        unique (tenant_id, knowledge_item_id, revision_number),
    constraint knowledge_revisions_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint knowledge_revisions_item_fk
        foreign key (tenant_id, knowledge_item_id)
        references knowledge_items (tenant_id, id)
        deferrable initially deferred,
    constraint knowledge_revisions_number_check check (revision_number >= 1),
    constraint knowledge_revisions_title_check
        check (btrim(title) <> '' and char_length(title) <= 300),
    constraint knowledge_revisions_body_check
        check (btrim(body_markdown) <> '' and octet_length(body_markdown) <= 131072),
    constraint knowledge_revisions_summary_check
        check (btrim(summary) <> '' and char_length(summary) <= 2000),
    constraint knowledge_revisions_tags_check
        check (synveda_knowledge_tags_canonical(tags)),
    constraint knowledge_revisions_sensitivity_check
        check (sensitivity in ('public', 'internal', 'confidential', 'restricted')),
    constraint knowledge_revisions_confidence_check
        check (confidence_permille between 0 and 1000),
    constraint knowledge_revisions_valid_time_check
        check (valid_to is null or valid_to > valid_from),
    constraint knowledge_revisions_stale_time_check
        check (stale_after is null
               or (stale_after > valid_from
                   and (valid_to is null or stale_after <= valid_to))),
    constraint knowledge_revisions_verification_object_check
        check (jsonb_typeof(verification_metadata) = 'object'),
    constraint knowledge_revisions_verification_size_check
        check (octet_length(verification_metadata::text) <= 16384),
    constraint knowledge_revisions_content_hash_check
        check (content_hash ~ '^[0-9a-f]{64}$'),
    constraint knowledge_revisions_metadata_object_check
        check (jsonb_typeof(metadata) = 'object'),
    constraint knowledge_revisions_metadata_size_check
        check (octet_length(metadata::text) <= 16384),
    constraint knowledge_revisions_created_by_check
        check (created_by is null
               or (btrim(created_by) <> '' and char_length(created_by) <= 255))
);

create index knowledge_revisions_by_item
    on knowledge_revisions (tenant_id, knowledge_item_id, revision_number desc);
create index knowledge_revisions_by_hash
    on knowledge_revisions (tenant_id, content_hash);
create index knowledge_revisions_by_valid_time
    on knowledge_revisions (tenant_id, valid_from, valid_to);
create index knowledge_revisions_stale_queue
    on knowledge_revisions (tenant_id, stale_after)
    where stale_after is not null;
create index knowledge_revisions_lexical
    on knowledge_revisions using gin (search_document);

-- A head must point at one of its own revisions in the same tenant. Deferred
-- so the first head and first revision can be one transaction without a
-- nullable or temporarily invalid pointer.
alter table knowledge_items
    add constraint knowledge_items_current_revision_fk
    foreign key (tenant_id, id, current_revision_id)
    references knowledge_revisions (tenant_id, knowledge_item_id, id)
    deferrable initially deferred;

alter table knowledge_items_history
    add constraint knowledge_items_history_current_revision_fk
    foreign key (tenant_id, id, current_revision_id)
    references knowledge_revisions (tenant_id, knowledge_item_id, id);

-- ── Normalised, independently governed provenance ───────────────────────

create table knowledge_sources (
    id                  uuid        not null,
    tenant_id           uuid        not null,
    scope_id            uuid        not null,
    source_type         text        not null,
    session_event_id    uuid,
    locator             text,
    source_revision     text,
    content_hash        text,
    metadata            jsonb       not null default '{}'::jsonb,
    created_by          text,
    created_at          timestamptz not null default now(),

    constraint knowledge_sources_pk primary key (id),
    constraint knowledge_sources_tenant_id_unique unique (tenant_id, id),
    constraint knowledge_sources_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint knowledge_sources_scope_fk
        foreign key (tenant_id, scope_id) references scopes (tenant_id, id),
    constraint knowledge_sources_event_fk
        foreign key (tenant_id, session_event_id)
        references session_events (tenant_id, id),
    constraint knowledge_sources_type_check
        check (source_type in (
            'session_event', 'manual', 'document', 'repository',
            'url', 'okf', 'system_derived'
        )),
    constraint knowledge_sources_shape_check
        check (
            (source_type = 'session_event'
             and session_event_id is not null and locator is null)
            or
            (source_type = 'manual'
             and session_event_id is null and locator is null
             and source_revision is null)
            or
            (source_type in ('document', 'repository', 'url', 'okf', 'system_derived')
             and session_event_id is null and locator is not null)
        ),
    constraint knowledge_sources_locator_check
        check (locator is null
               or (btrim(locator) <> '' and char_length(locator) <= 2048)),
    constraint knowledge_sources_revision_check
        check (source_revision is null
               or (btrim(source_revision) <> ''
                   and char_length(source_revision) <= 512)),
    constraint knowledge_sources_content_hash_check
        check (content_hash is null or content_hash ~ '^[0-9a-f]{64}$'),
    constraint knowledge_sources_metadata_object_check
        check (jsonb_typeof(metadata) = 'object'),
    constraint knowledge_sources_metadata_size_check
        check (octet_length(metadata::text) <= 16384),
    constraint knowledge_sources_created_by_check
        check (created_by is null
               or (btrim(created_by) <> '' and char_length(created_by) <= 255))
);

create index knowledge_sources_by_scope
    on knowledge_sources (tenant_id, scope_id, source_type, created_at desc);
create index knowledge_sources_by_event
    on knowledge_sources (tenant_id, session_event_id)
    where session_event_id is not null;
create index knowledge_sources_by_hash
    on knowledge_sources (tenant_id, content_hash)
    where content_hash is not null;

-- A session-event source inherits the governed scope derived for its session.
-- Letting a caller label that event with a broader source scope would turn the
-- descriptor into an existence oracle even though the payload remains behind
-- its own authority. The event and session are immutable, so this is a
-- creation-time invariant rather than a synchronisation trigger.
create function synveda_knowledge_source_event_scope() returns trigger
language plpgsql
as $$
begin
    if new.source_type = 'session_event' and not exists (
        select 1
        from session_events event
        join sessions session
          on session.tenant_id = event.tenant_id
         and session.id = event.session_id
        where event.tenant_id = new.tenant_id
          and event.id = new.session_event_id
          and session.scope_id = new.scope_id
    ) then
        raise exception 'session-event Knowledge source scope must match its session'
            using errcode = '23514';
    end if;
    return new;
end
$$;

create trigger knowledge_sources_event_scope
    before insert on knowledge_sources
    for each row execute function synveda_knowledge_source_event_scope();

create table knowledge_revision_sources (
    tenant_id           uuid        not null,
    knowledge_revision_id uuid      not null,
    knowledge_source_id uuid        not null,
    ordinal             integer     not null,
    linked_at           timestamptz not null default now(),

    constraint knowledge_revision_sources_pk
        primary key (tenant_id, knowledge_revision_id, knowledge_source_id),
    constraint knowledge_revision_sources_ordinal_unique
        unique (tenant_id, knowledge_revision_id, ordinal),
    constraint knowledge_revision_sources_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint knowledge_revision_sources_revision_fk
        foreign key (tenant_id, knowledge_revision_id)
        references knowledge_revisions (tenant_id, id),
    constraint knowledge_revision_sources_source_fk
        foreign key (tenant_id, knowledge_source_id)
        references knowledge_sources (tenant_id, id),
    constraint knowledge_revision_sources_ordinal_check check (ordinal >= 1)
);

create index knowledge_revision_sources_by_source
    on knowledge_revision_sources (tenant_id, knowledge_source_id, knowledge_revision_id);

-- A published revision without a source is not representable. Deferred so a
-- revision and its links may be inserted in either order inside one command
-- transaction; the final state, not a statement's intermediate state, is the
-- invariant.
create function synveda_knowledge_revision_has_source() returns trigger
language plpgsql
as $$
begin
    if not exists (
        select 1
        from knowledge_revision_sources link
        where link.tenant_id = new.tenant_id
          and link.knowledge_revision_id = new.id
    ) then
        raise exception 'Knowledge revision % has no provenance source (CPR-15, ADR-0080)',
            new.id
            using errcode = '23514';
    end if;
    return new;
end
$$;

create constraint trigger knowledge_revisions_require_source
    after insert on knowledge_revisions
    deferrable initially deferred
    for each row execute function synveda_knowledge_revision_has_source();

-- ── Explicit relationship claims ────────────────────────────────────────

create table knowledge_relations (
    id                    uuid        not null,
    tenant_id             uuid        not null,
    source_item_id        uuid        not null,
    target_item_id        uuid        not null,
    asserting_revision_id uuid        not null,
    relation_type         text        not null,
    metadata              jsonb       not null default '{}'::jsonb,
    created_by            text,
    created_at            timestamptz not null default now(),

    constraint knowledge_relations_pk primary key (id),
    constraint knowledge_relations_tenant_id_unique unique (tenant_id, id),
    constraint knowledge_relations_claim_unique
        unique (tenant_id, source_item_id, target_item_id,
                asserting_revision_id, relation_type),
    constraint knowledge_relations_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint knowledge_relations_source_fk
        foreign key (tenant_id, source_item_id)
        references knowledge_items (tenant_id, id),
    constraint knowledge_relations_target_fk
        foreign key (tenant_id, target_item_id)
        references knowledge_items (tenant_id, id),
    constraint knowledge_relations_asserting_revision_fk
        foreign key (tenant_id, source_item_id, asserting_revision_id)
        references knowledge_revisions (tenant_id, knowledge_item_id, id),
    constraint knowledge_relations_distinct_items_check
        check (source_item_id <> target_item_id),
    constraint knowledge_relations_type_check
        check (relation_type in (
            'supports', 'duplicates', 'contradicts', 'supersedes',
            'derived_from', 'references', 'related_to', 'transitions_to'
        )),
    constraint knowledge_relations_metadata_object_check
        check (jsonb_typeof(metadata) = 'object'),
    constraint knowledge_relations_metadata_size_check
        check (octet_length(metadata::text) <= 16384),
    constraint knowledge_relations_created_by_check
        check (created_by is null
               or (btrim(created_by) <> '' and char_length(created_by) <= 255))
);

create index knowledge_relations_from
    on knowledge_relations (tenant_id, source_item_id, relation_type, created_at);
create index knowledge_relations_to
    on knowledge_relations (tenant_id, target_item_id, relation_type, created_at);

-- ── Transaction-time head history ───────────────────────────────────────

create function synveda_knowledge_items_archive() returns trigger
language plpgsql
as $$
declare
    changed_at timestamptz;
begin
    if tg_op = 'DELETE' then
        raise exception 'Knowledge items have a governed lifecycle and are never deleted';
    end if;

    if new.id <> old.id or new.tenant_id <> old.tenant_id then
        raise exception 'Knowledge item identity and tenant are immutable';
    end if;
    if new.origin <> old.origin then
        raise exception 'Knowledge origin is a creation fact and is immutable';
    end if;
    if new.created_at <> old.created_at
       or new.created_by is distinct from old.created_by then
        raise exception 'Knowledge item creation provenance is immutable';
    end if;
    if new.tx_from <> old.tx_from or new.tx_to is not null then
        raise exception 'Knowledge transaction time is maintained by the database';
    end if;

    changed_at := clock_timestamp();
    if changed_at <= old.tx_from then
        raise exception 'Knowledge transaction clock did not advance'
            using errcode = '40001';
    end if;

    insert into knowledge_items_history
        (id, tenant_id, scope_id, project_id, owner_principal_id,
         knowledge_type, origin, lifecycle_state, current_revision_id,
         created_by, updated_by, created_at, updated_at, tx_from, tx_to)
    values
        (old.id, old.tenant_id, old.scope_id, old.project_id,
         old.owner_principal_id, old.knowledge_type, old.origin,
         old.lifecycle_state, old.current_revision_id, old.created_by,
         old.updated_by, old.created_at, old.updated_at, old.tx_from,
         changed_at);

    new.updated_at := changed_at;
    new.tx_from := changed_at;
    new.tx_to := null;
    return new;
end
$$;

create trigger knowledge_items_archive
    before update or delete on knowledge_items
    for each row execute function synveda_knowledge_items_archive();

-- Revisions, sources, links, relations and closed head states are append-only
-- facts. Grants carry the primary restriction; triggers make the same rule
-- hold for the migration owner and a future mistakenly widened grant.
create function synveda_knowledge_append_only() returns trigger
language plpgsql
as $$
begin
    raise exception '% is append-only (CPR-15, ADR-0080)', tg_table_name;
end
$$;

create trigger knowledge_items_history_append_only
    before update or delete or truncate on knowledge_items_history
    for each statement execute function synveda_knowledge_append_only();
create trigger knowledge_revisions_append_only
    before update or delete or truncate on knowledge_revisions
    for each statement execute function synveda_knowledge_append_only();
create trigger knowledge_sources_append_only
    before update or delete or truncate on knowledge_sources
    for each statement execute function synveda_knowledge_append_only();
create trigger knowledge_revision_sources_append_only
    before update or delete or truncate on knowledge_revision_sources
    for each statement execute function synveda_knowledge_append_only();
create trigger knowledge_relations_append_only
    before update or delete or truncate on knowledge_relations
    for each statement execute function synveda_knowledge_append_only();

-- ── RLS-safe projections ────────────────────────────────────────────────

create view knowledge_item_versions
with (security_invoker = on)
as
    select id, tenant_id, scope_id, project_id, owner_principal_id,
           knowledge_type, origin, lifecycle_state, current_revision_id,
           created_by, updated_by, created_at, updated_at, tx_from, tx_to
    from knowledge_items
    union all
    select id, tenant_id, scope_id, project_id, owner_principal_id,
           knowledge_type, origin, lifecycle_state, current_revision_id,
           created_by, updated_by, created_at, updated_at, tx_from, tx_to
    from knowledge_items_history;

create view knowledge_current
with (security_invoker = on)
as
    select item.id,
           item.tenant_id,
           item.scope_id,
           item.project_id,
           item.owner_principal_id,
           item.knowledge_type,
           item.origin,
           item.lifecycle_state,
           item.current_revision_id,
           revision.revision_number,
           revision.title,
           revision.body_markdown,
           revision.summary,
           revision.tags,
           revision.sensitivity,
           revision.confidence_permille,
           revision.valid_from,
           revision.valid_to,
           revision.stale_after,
           revision.verification_metadata,
           revision.content_hash,
           revision.metadata,
           revision.created_by as revision_created_by,
           revision.transaction_time,
           revision.search_document,
           item.created_by,
           item.updated_by,
           item.created_at,
           item.updated_at,
           item.tx_from
    from knowledge_items item
    join knowledge_revisions revision
      on revision.tenant_id = item.tenant_id
     and revision.knowledge_item_id = item.id
     and revision.id = item.current_revision_id;

-- ── Tenant isolation and least privilege ────────────────────────────────

grant select, insert on knowledge_items to synveda_app;
grant update (
    scope_id, project_id, owner_principal_id, knowledge_type,
    lifecycle_state, current_revision_id, updated_by
) on knowledge_items to synveda_app;
grant select, insert on knowledge_items_history to synveda_app;
grant select, insert on knowledge_revisions to synveda_app;
grant select, insert on knowledge_sources to synveda_app;
grant select, insert on knowledge_revision_sources to synveda_app;
grant select, insert on knowledge_relations to synveda_app;
grant select on knowledge_item_versions, knowledge_current to synveda_app;

alter table knowledge_items enable row level security;
alter table knowledge_items force row level security;
alter table knowledge_items_history enable row level security;
alter table knowledge_items_history force row level security;
alter table knowledge_revisions enable row level security;
alter table knowledge_revisions force row level security;
alter table knowledge_sources enable row level security;
alter table knowledge_sources force row level security;
alter table knowledge_revision_sources enable row level security;
alter table knowledge_revision_sources force row level security;
alter table knowledge_relations enable row level security;
alter table knowledge_relations force row level security;

create policy knowledge_items_tenant_isolation on knowledge_items
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy knowledge_items_history_tenant_isolation on knowledge_items_history
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy knowledge_revisions_tenant_isolation on knowledge_revisions
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy knowledge_sources_tenant_isolation on knowledge_sources
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy knowledge_revision_sources_tenant_isolation on knowledge_revision_sources
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy knowledge_relations_tenant_isolation on knowledge_relations
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
