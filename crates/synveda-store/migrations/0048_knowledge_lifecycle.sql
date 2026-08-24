-- CPR-16: governed Knowledge lifecycle through VedaFlow (ADR-0081).
--
-- `vedaflow_proposals` remains the one workflow. `knowledge_changes` is the
-- typed effect projection: it binds an erasable command payload to the exact
-- content-free manifest hash reviewed by VedaFlow, and records the result of
-- applying it. It has no competing status or approval vocabulary.

-- ── Extend the existing VedaFlow vocabulary ─────────────────────────────

alter table vedaflow_objects drop constraint vedaflow_objects_kind_check;
alter table vedaflow_objects add constraint vedaflow_objects_kind_check
    check (kind in (
        'memory', 'knowledge', 'prompt', 'skill', 'context-pack', 'policy'
    ));

alter table vedaflow_proposals drop constraint vedaflow_proposals_asset_check;
alter table vedaflow_proposals add constraint vedaflow_proposals_asset_check
    check (asset_kind in (
        'memory', 'knowledge', 'prompt', 'skill', 'context-pack', 'policy'
    ));

alter table vedaflow_proposals drop constraint vedaflow_proposals_channel_check;
alter table vedaflow_proposals add constraint vedaflow_proposals_channel_check
    check (target_channel in ('published', 'lapse', 'classify', 'apply'));

alter table vedaflow_proposals add constraint vedaflow_proposals_apply_asset_check
    check (target_channel <> 'apply' or asset_kind = 'knowledge');

alter table vedaflow_proposals drop constraint vedaflow_proposals_state_check;
alter table vedaflow_proposals add constraint vedaflow_proposals_state_check
    check (state in ('open', 'rejected', 'withdrawn', 'published', 'applied'));

-- ── Typed effect projection, not a second workflow ──────────────────────

create table knowledge_changes (
    tenant_id             uuid        not null,
    proposal_id           uuid        not null,
    command_kind          text        not null,
    target_item_ids       uuid[]      not null default '{}'::uuid[],
    payload               jsonb,
    payload_hash          text        not null,
    resulting_item_id     uuid,
    resulting_revision_id uuid,
    operation_id          uuid,
    created_at            timestamptz not null default now(),
    applied_at            timestamptz,

    constraint knowledge_changes_pk primary key (tenant_id, proposal_id),
    constraint knowledge_changes_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint knowledge_changes_proposal_fk
        foreign key (tenant_id, proposal_id)
        references vedaflow_proposals (tenant_id, id),
    constraint knowledge_changes_command_check
        check (command_kind in (
            'create', 'edit', 'verify', 'supersede', 'merge',
            'archive', 'restore', 'forget'
        )),
    constraint knowledge_changes_targets_check
        check (cardinality(target_item_ids) <= 200),
    constraint knowledge_changes_payload_object_check
        check (payload is null or jsonb_typeof(payload) = 'object'),
    constraint knowledge_changes_payload_size_check
        check (payload is null or octet_length(payload::text) <= 2097152),
    constraint knowledge_changes_hash_check
        check (payload_hash ~ '^[0-9a-f]{64}$'),
    constraint knowledge_changes_result_shape_check
        check ((applied_at is null)
               = (resulting_item_id is null
                  and resulting_revision_id is null
                  and operation_id is null))
);

create index knowledge_changes_by_target
    on knowledge_changes using gin (target_item_ids);

-- An effect row must bind the existing Knowledge/apply proposal. This is a
-- deferred trigger so proposal and effect projection may be inserted in one
-- transaction in either statement order, while an orphan or wrong-kind row
-- can never commit.
create function synveda_knowledge_change_matches_proposal() returns trigger
language plpgsql
as $$
begin
    if not exists (
        select 1
        from vedaflow_proposals proposal
        where proposal.tenant_id = new.tenant_id
          and proposal.id = new.proposal_id
          and proposal.asset_kind = 'knowledge'
          and proposal.target_channel = 'apply'
    ) then
        raise exception 'Knowledge change must bind a Knowledge/apply proposal'
            using errcode = '23514';
    end if;
    return new;
end
$$;

create constraint trigger knowledge_changes_match_proposal
    after insert on knowledge_changes
    deferrable initially deferred
    for each row execute function synveda_knowledge_change_matches_proposal();

-- A change is immutable except for its one result assignment. Erasure may
-- additionally clear an old payload, but cannot alter the hash reviewers saw.
create function synveda_knowledge_change_transition() returns trigger
language plpgsql
as $$
begin
    if new.tenant_id <> old.tenant_id
       or new.proposal_id <> old.proposal_id
       or new.command_kind <> old.command_kind
       or new.target_item_ids <> old.target_item_ids
       or new.payload_hash <> old.payload_hash
       or new.created_at <> old.created_at then
        raise exception 'Knowledge change identity and reviewed manifest are immutable';
    end if;

    if current_setting('synveda.knowledge_erasure', true) = 'on' then
        if new.payload is not null
           or new.resulting_item_id is distinct from old.resulting_item_id
           or new.resulting_revision_id is distinct from old.resulting_revision_id
           or new.operation_id is distinct from old.operation_id
           or new.applied_at is distinct from old.applied_at then
            raise exception 'Knowledge erasure may only clear a change payload';
        end if;
        return new;
    end if;

    if old.applied_at is not null
       or new.applied_at is null
       or new.payload is distinct from old.payload then
        raise exception 'Knowledge change result may be assigned exactly once';
    end if;
    return new;
end
$$;

create trigger knowledge_changes_transition
    before update on knowledge_changes
    for each row execute function synveda_knowledge_change_transition();
create trigger knowledge_changes_no_delete
    before delete or truncate on knowledge_changes
    for each statement execute function synveda_vedaflow_immutable();

-- ── Reusable durable operation ledger ──────────────────────────────────

create table durable_operations (
    tenant_id        uuid        not null,
    id               uuid        not null,
    kind             text        not null,
    state            text        not null default 'pending',
    proposal_id      uuid        not null,
    knowledge_item_id uuid,
    input_hash       text        not null,
    attempts         integer     not null default 0,
    lease_owner      text,
    lease_expires_at timestamptz,
    last_error_code  text,
    result           jsonb       not null default '{}'::jsonb,
    created_at       timestamptz not null default now(),
    updated_at       timestamptz not null default now(),
    started_at       timestamptz,
    completed_at     timestamptz,

    constraint durable_operations_pk primary key (tenant_id, id),
    constraint durable_operations_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint durable_operations_proposal_fk
        foreign key (tenant_id, proposal_id)
        references vedaflow_proposals (tenant_id, id),
    constraint durable_operations_kind_check
        check (kind in ('knowledge_erasure')),
    constraint durable_operations_state_check
        check (state in ('pending', 'running', 'succeeded', 'failed', 'blocked')),
    constraint durable_operations_attempts_check check (attempts >= 0),
    constraint durable_operations_input_hash_check
        check (input_hash ~ '^[0-9a-f]{64}$'),
    constraint durable_operations_lease_owner_check
        check (lease_owner is null
               or (btrim(lease_owner) <> '' and char_length(lease_owner) <= 255)),
    constraint durable_operations_error_check
        check (last_error_code is null
               or (btrim(last_error_code) <> '' and char_length(last_error_code) <= 128)),
    constraint durable_operations_result_object_check
        check (jsonb_typeof(result) = 'object'),
    constraint durable_operations_result_size_check
        check (octet_length(result::text) <= 16384),
    constraint durable_operations_time_check
        check ((state = 'pending' and started_at is null and completed_at is null
               and lease_owner is null and lease_expires_at is null)
            or (state = 'running' and started_at is not null and completed_at is null
                and lease_owner is not null and lease_expires_at is not null)
            or (state in ('succeeded', 'failed', 'blocked')
                and completed_at is not null
                and lease_owner is null and lease_expires_at is null))
);

create index durable_operations_queue
    on durable_operations (tenant_id, state, created_at, id)
    where state in ('pending', 'failed');

create function synveda_durable_operation_transition() returns trigger
language plpgsql
as $$
begin
    if new.tenant_id <> old.tenant_id
       or new.id <> old.id
       or new.kind <> old.kind
       or new.proposal_id <> old.proposal_id
       or new.knowledge_item_id is distinct from old.knowledge_item_id
       or new.input_hash <> old.input_hash
       or new.created_at <> old.created_at then
        raise exception 'durable operation identity is immutable';
    end if;
    if old.state in ('succeeded', 'blocked') then
        raise exception 'durable operation % is terminal', old.id;
    end if;
    if not (
        (old.state = 'pending' and new.state in ('running', 'blocked'))
        or (old.state = 'running' and new.state in ('succeeded', 'failed', 'blocked'))
        or (old.state = 'failed' and new.state in ('running', 'blocked'))
    ) then
        raise exception 'invalid durable operation transition % -> %', old.state, new.state;
    end if;
    return new;
end
$$;

create trigger durable_operations_transition
    before update on durable_operations
    for each row execute function synveda_durable_operation_transition();
create trigger durable_operations_no_delete
    before delete or truncate on durable_operations
    for each statement execute function synveda_vedaflow_immutable();

-- ── Content-free erasure evidence and index invalidation ────────────────

create table knowledge_erasure_tombstones (
    tenant_id       uuid        not null,
    knowledge_item_id uuid      not null,
    proposal_id     uuid        not null,
    operation_id    uuid        not null,
    revision_hashes jsonb       not null,
    actor_hash      text        not null,
    reason_hash     text        not null,
    erased_at       timestamptz not null default now(),

    constraint knowledge_erasure_tombstones_pk
        primary key (tenant_id, knowledge_item_id),
    constraint knowledge_erasure_tombstones_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint knowledge_erasure_tombstones_proposal_fk
        foreign key (tenant_id, proposal_id)
        references vedaflow_proposals (tenant_id, id),
    constraint knowledge_erasure_tombstones_operation_fk
        foreign key (tenant_id, operation_id)
        references durable_operations (tenant_id, id),
    constraint knowledge_erasure_tombstones_revisions_check
        check (jsonb_typeof(revision_hashes) = 'array'
               and octet_length(revision_hashes::text) <= 65536),
    constraint knowledge_erasure_tombstones_actor_hash_check
        check (actor_hash ~ '^[0-9a-f]{64}$'),
    constraint knowledge_erasure_tombstones_reason_hash_check
        check (reason_hash ~ '^[0-9a-f]{64}$')
);

create table knowledge_index_invalidations (
    tenant_id       uuid        not null,
    operation_id    uuid        not null,
    revision_id     uuid        not null,
    content_hash    text        not null,
    created_at      timestamptz not null default now(),
    processed_at    timestamptz,

    constraint knowledge_index_invalidations_pk
        primary key (tenant_id, operation_id, revision_id),
    constraint knowledge_index_invalidations_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint knowledge_index_invalidations_operation_fk
        foreign key (tenant_id, operation_id)
        references durable_operations (tenant_id, id),
    constraint knowledge_index_invalidations_hash_check
        check (content_hash ~ '^[0-9a-f]{64}$')
);

create trigger knowledge_erasure_tombstones_append_only
    before update or delete or truncate on knowledge_erasure_tombstones
    for each statement execute function synveda_vedaflow_immutable();
create trigger knowledge_index_invalidations_no_delete
    before delete or truncate on knowledge_index_invalidations
    for each statement execute function synveda_vedaflow_immutable();

-- CPR-15 made Knowledge append-only. Erasure is the only deliberate
-- exception, available only inside the security-definer function below;
-- ordinary app SQL still has no DELETE grant on any content table.
create or replace function synveda_knowledge_items_archive() returns trigger
language plpgsql
as $$
declare
    changed_at timestamptz;
begin
    if tg_op = 'DELETE' then
        if current_setting('synveda.knowledge_erasure', true) = 'on' then
            return old;
        end if;
        raise exception 'Knowledge items have a governed lifecycle and are never directly deleted';
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

create or replace function synveda_knowledge_append_only() returns trigger
language plpgsql
as $$
begin
    if tg_op = 'DELETE'
       and current_setting('synveda.knowledge_erasure', true) = 'on' then
        return null;
    end if;
    raise exception '% is append-only (CPR-15/16, ADR-0080/0081)', tg_table_name;
end
$$;

-- The sole plaintext-destruction primitive. It verifies ambient tenancy and
-- operation ownership, records hashes before deleting, clears every pending
-- change payload that named the aggregate, deletes exclusively-owned source
-- descriptors, and leaves only content-free tombstone/index evidence.
create function synveda_erase_knowledge(
    wanted_tenant uuid,
    wanted_item uuid,
    wanted_proposal uuid,
    wanted_operation uuid,
    wanted_actor_hash text,
    wanted_reason_hash text
) returns void
language plpgsql
security definer
set search_path = public, pg_temp
as $$
declare
    revision_evidence jsonb;
    source_ids uuid[];
begin
    if wanted_tenant <> synveda_current_tenant() then
        raise exception 'cross-tenant Knowledge erasure refused'
            using errcode = '42501';
    end if;
    if wanted_actor_hash !~ '^[0-9a-f]{64}$'
       or wanted_reason_hash !~ '^[0-9a-f]{64}$' then
        raise exception 'Knowledge erasure evidence hashes are malformed'
            using errcode = '22023';
    end if;
    if not exists (
        select 1 from durable_operations operation
        where operation.tenant_id = wanted_tenant
          and operation.id = wanted_operation
          and operation.proposal_id = wanted_proposal
          and operation.knowledge_item_id = wanted_item
          and operation.kind = 'knowledge_erasure'
          and operation.state = 'running'
    ) then
        raise exception 'Knowledge erasure operation is not running'
            using errcode = '23514';
    end if;
    if not exists (
        select 1 from knowledge_items item
        where item.tenant_id = wanted_tenant
          and item.id = wanted_item
          and item.lifecycle_state = 'erasure_pending'
    ) then
        raise exception 'Knowledge item is not erasure_pending'
            using errcode = '23514';
    end if;

    select coalesce(
               jsonb_agg(jsonb_build_object('id', id, 'hash', content_hash)
                         order by revision_number),
               '[]'::jsonb
           ),
           coalesce(array_agg(source_id) filter (where source_id is not null), '{}'::uuid[])
    into revision_evidence, source_ids
    from (
        select revision.id, revision.content_hash, revision.revision_number,
               link.knowledge_source_id as source_id
        from knowledge_revisions revision
        left join knowledge_revision_sources link
          on link.tenant_id = revision.tenant_id
         and link.knowledge_revision_id = revision.id
        where revision.tenant_id = wanted_tenant
          and revision.knowledge_item_id = wanted_item
    ) evidence;

    -- Duplicate revision rows introduced by the source join are removed from
    -- the tombstone deterministically.
    select coalesce(
               jsonb_agg(jsonb_build_object('id', id, 'hash', content_hash)
                         order by revision_number),
               '[]'::jsonb
           )
    into revision_evidence
    from (
        select distinct id, content_hash, revision_number
        from knowledge_revisions
        where tenant_id = wanted_tenant and knowledge_item_id = wanted_item
    ) revisions;

    insert into knowledge_erasure_tombstones
        (tenant_id, knowledge_item_id, proposal_id, operation_id,
         revision_hashes, actor_hash, reason_hash)
    values
        (wanted_tenant, wanted_item, wanted_proposal, wanted_operation,
         revision_evidence, wanted_actor_hash, wanted_reason_hash);

    insert into knowledge_index_invalidations
        (tenant_id, operation_id, revision_id, content_hash)
    select tenant_id, wanted_operation, id, content_hash
    from knowledge_revisions
    where tenant_id = wanted_tenant and knowledge_item_id = wanted_item;

    perform set_config('synveda.knowledge_erasure', 'on', true);
    update knowledge_changes
       set payload = null
     where tenant_id = wanted_tenant
       and target_item_ids @> array[wanted_item]::uuid[]
       and payload is not null;
    delete from knowledge_relations
     where tenant_id = wanted_tenant
       and (source_item_id = wanted_item or target_item_id = wanted_item);
    delete from knowledge_revision_sources link
     using knowledge_revisions revision
     where link.tenant_id = wanted_tenant
       and revision.tenant_id = link.tenant_id
       and revision.id = link.knowledge_revision_id
       and revision.knowledge_item_id = wanted_item;
    delete from knowledge_items_history
     where tenant_id = wanted_tenant and id = wanted_item;

    set constraints knowledge_items_current_revision_fk deferred;
    set constraints knowledge_revisions_item_fk deferred;
    delete from knowledge_items
     where tenant_id = wanted_tenant and id = wanted_item;
    delete from knowledge_revisions
     where tenant_id = wanted_tenant and knowledge_item_id = wanted_item;
    delete from knowledge_sources source
     where source.tenant_id = wanted_tenant
       and source.id = any(source_ids)
       and not exists (
           select 1 from knowledge_revision_sources remaining
           where remaining.tenant_id = source.tenant_id
             and remaining.knowledge_source_id = source.id
       );
    perform set_config('synveda.knowledge_erasure', 'off', true);

    update durable_operations
       set state = 'succeeded', completed_at = now(), updated_at = now(),
           lease_owner = null, lease_expires_at = null, last_error_code = null,
           result = jsonb_build_object('erased', true)
     where tenant_id = wanted_tenant and id = wanted_operation;
end
$$;

revoke all on function synveda_erase_knowledge(uuid, uuid, uuid, uuid, text, text)
    from public;
grant execute on function synveda_erase_knowledge(uuid, uuid, uuid, uuid, text, text)
    to synveda_app;

-- ── Tenant isolation and least privilege ────────────────────────────────

grant select, insert on knowledge_changes to synveda_app;
grant update (payload, resulting_item_id, resulting_revision_id, operation_id, applied_at)
    on knowledge_changes to synveda_app;
grant select, insert on durable_operations to synveda_app;
grant update (state, attempts, lease_owner, lease_expires_at, last_error_code,
              result, updated_at, started_at, completed_at)
    on durable_operations to synveda_app;
grant select on knowledge_erasure_tombstones to synveda_app;
grant select, update (processed_at) on knowledge_index_invalidations to synveda_app;

alter table knowledge_changes enable row level security;
alter table knowledge_changes force row level security;
alter table durable_operations enable row level security;
alter table durable_operations force row level security;
alter table knowledge_erasure_tombstones enable row level security;
alter table knowledge_erasure_tombstones force row level security;
alter table knowledge_index_invalidations enable row level security;
alter table knowledge_index_invalidations force row level security;

create policy knowledge_changes_tenant_isolation on knowledge_changes
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy durable_operations_tenant_isolation on durable_operations
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy knowledge_erasure_tombstones_tenant_isolation
    on knowledge_erasure_tombstones
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy knowledge_index_invalidations_tenant_isolation
    on knowledge_index_invalidations
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
