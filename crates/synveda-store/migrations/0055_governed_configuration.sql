-- CPR-30 / ADR-0089: immutable governed runtime configuration.
--
-- This is a pre-1.0 hard cut. Mutable policy defaults and assignments are
-- deleted without row translation; the only persisted selector is a
-- revisioned configuration binding. PolicyAssignment remains an in-memory
-- projection handed to Cedar, never a table clients can mutate.

drop table policy_pack_assignments;
drop table policy_pack_defaults;

alter table vedaflow_objects drop constraint vedaflow_objects_kind_check;
alter table vedaflow_objects add constraint vedaflow_objects_kind_check
    check (kind in (
        'memory', 'knowledge', 'prompt', 'skill', 'tool', 'context-pack',
        'policy', 'configuration'
    ));

alter table vedaflow_proposals drop constraint vedaflow_proposals_asset_check;
alter table vedaflow_proposals add constraint vedaflow_proposals_asset_check
    check (asset_kind in (
        'memory', 'knowledge', 'prompt', 'skill', 'tool', 'context-pack',
        'policy', 'configuration'
    ));

alter table vedaflow_proposals drop constraint vedaflow_proposals_apply_asset_check;
alter table vedaflow_proposals add constraint vedaflow_proposals_apply_asset_check
    check (target_channel <> 'apply'
           or asset_kind in ('knowledge', 'skill', 'tool', 'configuration'));

create table configuration_artifacts (
    id                 uuid        not null,
    tenant_id          uuid        not null,
    governing_scope_id uuid        not null,
    name               text        not null,
    current_version_id uuid        not null,
    created_at         timestamptz not null default now(),
    created_by         text        not null,
    updated_at         timestamptz not null default now(),
    updated_by         text        not null,

    constraint configuration_artifacts_pk primary key (tenant_id, id),
    constraint configuration_artifacts_id_unique unique (id),
    constraint configuration_artifacts_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint configuration_artifacts_scope_fk
        foreign key (tenant_id, governing_scope_id) references scopes (tenant_id, id),
    constraint configuration_artifacts_name_unique unique (tenant_id, name),
    constraint configuration_artifacts_name_check
        check (btrim(name) = name and length(name) between 1 and 100),
    constraint configuration_artifacts_actor_check
        check (btrim(created_by) <> '' and length(created_by) <= 255
               and btrim(updated_by) <> '' and length(updated_by) <= 255)
);

create table configuration_versions (
    id              uuid        not null,
    tenant_id       uuid        not null,
    artifact_id     uuid        not null,
    proposal_id     uuid        not null,
    ordinal         bigint      not null,
    document        jsonb       not null,
    content_hash    bytea       not null,
    source_template text,
    created_at      timestamptz not null default now(),
    created_by      text        not null,

    constraint configuration_versions_pk primary key (tenant_id, id),
    constraint configuration_versions_id_unique unique (id),
    constraint configuration_versions_artifact_fk
        foreign key (tenant_id, artifact_id)
        references configuration_artifacts (tenant_id, id),
    constraint configuration_versions_artifact_id_unique
        unique (tenant_id, artifact_id, id),
    constraint configuration_versions_proposal_fk
        foreign key (tenant_id, proposal_id)
        references vedaflow_proposals (tenant_id, id),
    constraint configuration_versions_proposal_unique unique (tenant_id, proposal_id),
    constraint configuration_versions_ordinal_unique
        unique (tenant_id, artifact_id, ordinal),
    constraint configuration_versions_hash_unique
        unique (tenant_id, artifact_id, content_hash),
    constraint configuration_versions_ordinal_check check (ordinal > 0),
    constraint configuration_versions_document_check
        check (jsonb_typeof(document) = 'object' and pg_column_size(document) <= 131072),
    constraint configuration_versions_hash_check check (octet_length(content_hash) = 32),
    constraint configuration_versions_template_check
        check (source_template is null or source_template in ('personal', 'team', 'enterprise')),
    constraint configuration_versions_actor_check
        check (btrim(created_by) <> '' and length(created_by) <= 255)
);

alter table configuration_artifacts
    add constraint configuration_artifacts_current_version_fk
    foreign key (tenant_id, id, current_version_id)
    references configuration_versions (tenant_id, artifact_id, id)
    deferrable initially deferred;

create table configuration_bindings (
    id                uuid        not null,
    tenant_id         uuid        not null,
    scope_id          uuid        not null,
    artifact_id       uuid        not null,
    pinned_version_id uuid,
    enabled           boolean     not null,
    revision          bigint      not null default 1,
    created_at        timestamptz not null default now(),
    created_by        text        not null,
    updated_at        timestamptz not null default now(),
    updated_by        text        not null,

    constraint configuration_bindings_pk primary key (tenant_id, id),
    constraint configuration_bindings_id_unique unique (id),
    constraint configuration_bindings_scope_fk
        foreign key (tenant_id, scope_id) references scopes (tenant_id, id),
    constraint configuration_bindings_artifact_fk
        foreign key (tenant_id, artifact_id)
        references configuration_artifacts (tenant_id, id),
    constraint configuration_bindings_pin_fk
        foreign key (tenant_id, artifact_id, pinned_version_id)
        references configuration_versions (tenant_id, artifact_id, id),
    -- There is one answer at a scope. Changing the selected aggregate is a
    -- revisioned binding transition, not a competing row.
    constraint configuration_bindings_scope_unique unique (tenant_id, scope_id),
    constraint configuration_bindings_revision_check check (revision > 0),
    constraint configuration_bindings_actor_check
        check (btrim(created_by) <> '' and length(created_by) <= 255
               and btrim(updated_by) <> '' and length(updated_by) <= 255)
);

create index configuration_bindings_resolution
    on configuration_bindings (tenant_id, scope_id, enabled)
    where enabled;

create table configuration_changes (
    tenant_id                   uuid        not null,
    proposal_id                 uuid        not null,
    command_kind                text        not null,
    payload                     jsonb       not null,
    payload_hash                text        not null,
    resulting_artifact_id       uuid,
    resulting_version_id        uuid,
    resulting_binding_id        uuid,
    resulting_binding_revision  bigint,
    applied_at                  timestamptz,
    created_at                  timestamptz not null default now(),

    constraint configuration_changes_pk primary key (tenant_id, proposal_id),
    constraint configuration_changes_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint configuration_changes_proposal_fk
        foreign key (tenant_id, proposal_id)
        references vedaflow_proposals (tenant_id, id),
    constraint configuration_changes_kind_check
        check (command_kind in ('create', 'publish', 'bind', 'set_binding')),
    constraint configuration_changes_payload_check
        check (jsonb_typeof(payload) = 'object' and pg_column_size(payload) <= 262144),
    constraint configuration_changes_payload_hash_check
        check (payload_hash ~ '^[0-9a-f]{64}$'),
    constraint configuration_changes_result_shape_check check (
        (applied_at is null and resulting_binding_revision is null)
        or applied_at is not null
    ),
    constraint configuration_changes_binding_revision_check
        check (resulting_binding_revision is null or resulting_binding_revision > 0)
);

-- Exact immutable configuration evidence on runtime work. Existing epoch-2
-- development rows remain null rather than being translated; every new
-- application write supplies the complete pair and Prompt 33's clean baseline
-- removes the migration-era nullable branch.
alter table capture_batches
    add column configuration_version_id uuid,
    add column configuration_hash text,
    add constraint capture_batches_configuration_version_fk
        foreign key (tenant_id, configuration_version_id)
        references configuration_versions (tenant_id, id),
    add constraint capture_batches_configuration_shape_check
        check ((configuration_version_id is null or configuration_hash is not null)
               and (configuration_hash is null
                    or configuration_hash ~ '^[0-9a-f]{64}$'));

alter table session_context_runs
    add column configuration_version_id uuid,
    add column configuration_hash text,
    add constraint context_runs_configuration_version_fk
        foreign key (tenant_id, configuration_version_id)
        references configuration_versions (tenant_id, id),
    add constraint context_runs_configuration_shape_check
        check ((configuration_version_id is null or configuration_hash is not null)
               and (configuration_hash is null
                    or configuration_hash ~ '^[0-9a-f]{64}$'));

-- A governed configuration can explicitly admit pending capture candidates
-- into a context run. They remain a separate, visibly unreviewed channel:
-- no synthetic Knowledge address is invented and feedback stays attached
-- only to immutable published revisions.
alter table context_candidates
    drop constraint context_candidates_address_shape_check,
    add column channel text not null default 'current_knowledge',
    add column capture_candidate_id uuid;

alter table context_candidates
    alter column channel drop default,
    add constraint context_candidates_channel_check
        check (channel in ('current_knowledge', 'unreviewed_candidates')),
    add constraint context_candidates_capture_candidate_fk
        foreign key (tenant_id, capture_candidate_id)
        references capture_candidates (tenant_id, id),
    add constraint context_candidates_address_shape_check check (
        (channel = 'current_knowledge'
         and capture_candidate_id is null
         and ((knowledge_item_id is null
               and knowledge_revision_id is null
               and scope_id is null)
              or (knowledge_item_id is not null
                  and knowledge_revision_id is not null
                  and scope_id is not null)))
        or
        (channel = 'unreviewed_candidates'
         and knowledge_item_id is null
         and knowledge_revision_id is null
         and lifecycle_state is null
         and ((capture_candidate_id is null and scope_id is null)
              or (capture_candidate_id is not null and scope_id is not null)))
    );

create index context_candidates_by_capture_candidate
    on context_candidates (tenant_id, capture_candidate_id, created_at desc)
    where capture_candidate_id is not null;

alter table context_selections
    drop constraint context_selections_address_shape_check,
    add column channel text not null default 'current_knowledge',
    add column capture_candidate_id uuid;

alter table context_selections
    alter column channel drop default,
    add constraint context_selections_channel_check
        check (channel in ('current_knowledge', 'unreviewed_candidates')),
    add constraint context_selections_capture_candidate_fk
        foreign key (tenant_id, capture_candidate_id)
        references capture_candidates (tenant_id, id),
    add constraint context_selections_address_shape_check check (
        (channel = 'current_knowledge'
         and capture_candidate_id is null
         and ((knowledge_item_id is null and knowledge_revision_id is null)
              or (knowledge_item_id is not null
                  and knowledge_revision_id is not null)))
        or
        (channel = 'unreviewed_candidates'
         and knowledge_item_id is null
         and knowledge_revision_id is null)
    );

create index context_selections_by_capture_candidate
    on context_selections (tenant_id, capture_candidate_id, created_at desc)
    where capture_candidate_id is not null;

-- ── Invariants and immutable history ───────────────────────────────────

create function synveda_configuration_version_matches_proposal() returns trigger
language plpgsql
as $$
begin
    if not exists (
        select 1 from vedaflow_proposals proposal
         where proposal.tenant_id = new.tenant_id
           and proposal.id = new.proposal_id
           and proposal.asset_kind = 'configuration'
           and proposal.target_channel = 'apply'
    ) then
        raise exception 'Configuration version must bind a Configuration/apply proposal'
            using errcode = '23514';
    end if;
    return new;
end
$$;

create trigger configuration_versions_proposal_shape
    before insert on configuration_versions
    for each row execute function synveda_configuration_version_matches_proposal();

create function synveda_configuration_aggregate_transition() returns trigger
language plpgsql
as $$
begin
    if new.id <> old.id
        or new.tenant_id <> old.tenant_id
        or new.governing_scope_id <> old.governing_scope_id
        or new.name <> old.name
        or new.created_at <> old.created_at
        or new.created_by <> old.created_by
    then
        raise exception 'a Configuration aggregate identity is immutable (CPR-30)';
    end if;
    if new.current_version_id = old.current_version_id then
        raise exception 'a Configuration publication must advance its immutable version (CPR-30)';
    end if;
    return new;
end
$$;

create trigger configuration_artifacts_transition
    before update on configuration_artifacts
    for each row execute function synveda_configuration_aggregate_transition();

create function synveda_immutable_configuration_row() returns trigger
language plpgsql
as $$
begin
    raise exception '% rows are immutable (CPR-30)', tg_table_name;
end
$$;

create trigger configuration_versions_immutable
    before update or delete on configuration_versions
    for each row execute function synveda_immutable_configuration_row();

create function synveda_configuration_binding_transition() returns trigger
language plpgsql
as $$
begin
    if new.id <> old.id
        or new.tenant_id <> old.tenant_id
        or new.scope_id <> old.scope_id
        or new.created_at <> old.created_at
        or new.created_by <> old.created_by
    then
        raise exception 'a Configuration binding identity is immutable (CPR-30)';
    end if;
    if new.revision <> old.revision + 1 then
        raise exception 'a Configuration binding update must advance revision exactly once (CPR-30)';
    end if;
    if new.artifact_id = old.artifact_id
       and new.enabled = old.enabled
       and new.pinned_version_id is not distinct from old.pinned_version_id
    then
        raise exception 'a Configuration binding update must change selection, pin or state (CPR-30)';
    end if;
    return new;
end
$$;

create trigger configuration_bindings_transition
    before update on configuration_bindings
    for each row execute function synveda_configuration_binding_transition();

create function synveda_configuration_change_transition() returns trigger
language plpgsql
as $$
begin
    if new.tenant_id <> old.tenant_id
        or new.proposal_id <> old.proposal_id
        or new.command_kind <> old.command_kind
        or new.payload <> old.payload
        or new.payload_hash <> old.payload_hash
        or new.created_at <> old.created_at
    then
        raise exception 'a Configuration VedaFlow command is immutable (CPR-30)';
    end if;
    if old.applied_at is not null or new.applied_at is null then
        raise exception 'a Configuration result may be recorded exactly once (CPR-30)';
    end if;
    return new;
end
$$;

create trigger configuration_changes_transition
    before update on configuration_changes
    for each row execute function synveda_configuration_change_transition();

-- ── Least privilege and tenant backstop ────────────────────────────────

grant select, insert on configuration_artifacts to synveda_app;
grant update (current_version_id, updated_at, updated_by)
    on configuration_artifacts to synveda_app;
grant select, insert on configuration_versions to synveda_app;
grant select, insert on configuration_bindings to synveda_app;
grant update (artifact_id, pinned_version_id, enabled, revision, updated_at, updated_by)
    on configuration_bindings to synveda_app;
grant select, insert on configuration_changes to synveda_app;
grant update (resulting_artifact_id, resulting_version_id,
              resulting_binding_id, resulting_binding_revision, applied_at)
    on configuration_changes to synveda_app;

alter table configuration_artifacts enable row level security;
alter table configuration_artifacts force row level security;
alter table configuration_versions enable row level security;
alter table configuration_versions force row level security;
alter table configuration_bindings enable row level security;
alter table configuration_bindings force row level security;
alter table configuration_changes enable row level security;
alter table configuration_changes force row level security;

create policy configuration_artifacts_tenant_isolation on configuration_artifacts
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy configuration_versions_tenant_isolation on configuration_versions
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy configuration_bindings_tenant_isolation on configuration_bindings
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy configuration_changes_tenant_isolation on configuration_changes
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
