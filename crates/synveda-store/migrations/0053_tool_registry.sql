-- CPR-25: trusted MCP server catalogue, immutable versions and exact project
-- bindings (ADR-0086).
--
-- A version's trust state is deliberately not a mutable column. Its
-- `proposal_id` points at the one Tool/apply VedaFlow change that staged it:
-- open is quarantined, applied is approved, and rejected/withdrawn is
-- rejected. The current pointer and an exact project binding can name only a
-- version whose proposal reaches applied before the transaction commits.

alter table vedaflow_objects drop constraint vedaflow_objects_kind_check;
alter table vedaflow_objects add constraint vedaflow_objects_kind_check
    check (kind in (
        'memory', 'knowledge', 'prompt', 'skill', 'tool', 'context-pack', 'policy'
    ));

alter table vedaflow_proposals drop constraint vedaflow_proposals_asset_check;
alter table vedaflow_proposals add constraint vedaflow_proposals_asset_check
    check (asset_kind in (
        'memory', 'knowledge', 'prompt', 'skill', 'tool', 'context-pack', 'policy'
    ));

alter table vedaflow_proposals drop constraint vedaflow_proposals_apply_asset_check;
alter table vedaflow_proposals add constraint vedaflow_proposals_apply_asset_check
    check (target_channel <> 'apply' or asset_kind in ('knowledge', 'skill', 'tool'));

create table tool_servers (
    id                 uuid        not null,
    tenant_id          uuid        not null,
    governing_scope_id uuid        not null,
    name               text        not null,
    current_version_id uuid,
    created_at         timestamptz not null default now(),
    created_by         uuid        not null,
    updated_at         timestamptz not null default now(),
    updated_by         uuid        not null,

    constraint tool_servers_pk primary key (tenant_id, id),
    constraint tool_servers_id_unique unique (id),
    constraint tool_servers_tenant_fk foreign key (tenant_id) references tenants (id),
    constraint tool_servers_scope_fk foreign key (tenant_id, governing_scope_id)
        references scopes (tenant_id, id),
    constraint tool_servers_name_unique unique (tenant_id, name),
    constraint tool_servers_name_check
        check (btrim(name) <> '' and length(name) <= 200)
);

create table tool_server_versions (
    id              uuid        not null,
    tenant_id       uuid        not null,
    server_id       uuid        not null,
    proposal_id     uuid        not null,
    ordinal         bigint      not null,
    digest          bytea       not null,
    protocol_version text       not null,
    descriptor      jsonb       not null,
    created_at      timestamptz not null default now(),
    created_by      uuid        not null,

    constraint tool_server_versions_pk primary key (tenant_id, id),
    constraint tool_server_versions_id_unique unique (id),
    constraint tool_server_versions_server_fk foreign key (tenant_id, server_id)
        references tool_servers (tenant_id, id),
    constraint tool_server_versions_server_id_unique unique (tenant_id, server_id, id),
    constraint tool_server_versions_proposal_fk foreign key (tenant_id, proposal_id)
        references vedaflow_proposals (tenant_id, id),
    constraint tool_server_versions_proposal_unique unique (tenant_id, proposal_id),
    constraint tool_server_versions_ordinal_unique unique (tenant_id, server_id, ordinal),
    constraint tool_server_versions_digest_unique unique (tenant_id, server_id, digest),
    constraint tool_server_versions_ordinal_check check (ordinal > 0),
    constraint tool_server_versions_digest_check check (octet_length(digest) = 32),
    constraint tool_server_versions_protocol_check check (protocol_version = '2026-07-28'),
    constraint tool_server_versions_descriptor_check check (
        jsonb_typeof(descriptor) = 'object' and pg_column_size(descriptor) <= 131072
    )
);

alter table tool_servers add constraint tool_servers_current_version_fk
    foreign key (tenant_id, id, current_version_id)
    references tool_server_versions (tenant_id, server_id, id)
    deferrable initially deferred;

create table capability_snapshots (
    id          uuid        not null,
    tenant_id   uuid        not null,
    version_id  uuid        not null,
    raw         jsonb       not null,
    normalized  jsonb       not null,
    digest      bytea       not null,
    discovered_at timestamptz not null,
    discovered_by uuid      not null,
    created_at  timestamptz not null default now(),

    constraint capability_snapshots_pk primary key (tenant_id, id),
    constraint capability_snapshots_id_unique unique (id),
    constraint capability_snapshots_version_fk foreign key (tenant_id, version_id)
        references tool_server_versions (tenant_id, id),
    constraint capability_snapshots_version_unique unique (tenant_id, version_id),
    constraint capability_snapshots_digest_check check (octet_length(digest) = 32),
    constraint capability_snapshots_raw_check check (
        jsonb_typeof(raw) = 'object' and pg_column_size(raw) <= 524288
    ),
    constraint capability_snapshots_normalized_check check (
        jsonb_typeof(normalized) = 'object' and pg_column_size(normalized) <= 524288
    )
);

create table tool_bindings (
    id          uuid        not null,
    tenant_id   uuid        not null,
    project_id  uuid        not null,
    server_id   uuid        not null,
    version_id  uuid        not null,
    state       text        not null,
    revision    bigint      not null default 1,
    created_at  timestamptz not null default now(),
    created_by  uuid        not null,
    updated_at  timestamptz not null default now(),
    updated_by  uuid        not null,

    constraint tool_bindings_pk primary key (tenant_id, id),
    constraint tool_bindings_id_unique unique (id),
    constraint tool_bindings_project_fk foreign key (tenant_id, project_id)
        references projects (tenant_id, id),
    constraint tool_bindings_server_fk foreign key (tenant_id, server_id)
        references tool_servers (tenant_id, id),
    constraint tool_bindings_version_fk foreign key (tenant_id, server_id, version_id)
        references tool_server_versions (tenant_id, server_id, id),
    constraint tool_bindings_target_unique unique (tenant_id, project_id, server_id),
    constraint tool_bindings_state_check check (state in ('enabled', 'disabled', 'removed')),
    constraint tool_bindings_revision_check check (revision > 0)
);

create index tool_bindings_active
    on tool_bindings (tenant_id, project_id, server_id)
    where state = 'enabled';

create table tool_changes (
    tenant_id                 uuid        not null,
    proposal_id               uuid        not null,
    command_kind              text        not null,
    payload                   jsonb       not null,
    payload_hash              text        not null,
    resulting_server_id       uuid,
    resulting_version_id      uuid,
    resulting_binding_id      uuid,
    resulting_binding_revision bigint,
    applied_at                timestamptz,
    created_at                timestamptz not null default now(),

    constraint tool_changes_pk primary key (tenant_id, proposal_id),
    constraint tool_changes_tenant_fk foreign key (tenant_id) references tenants (id),
    constraint tool_changes_proposal_fk foreign key (tenant_id, proposal_id)
        references vedaflow_proposals (tenant_id, id),
    constraint tool_changes_kind_check
        check (command_kind in ('register', 'stage_version', 'bind', 'set_binding')),
    constraint tool_changes_payload_check check (
        jsonb_typeof(payload) = 'object' and pg_column_size(payload) <= 2097152
    ),
    constraint tool_changes_payload_hash_check check (payload_hash ~ '^[0-9a-f]{64}$'),
    constraint tool_changes_result_shape_check check (
        (applied_at is null and resulting_binding_revision is null)
        or applied_at is not null
    ),
    constraint tool_changes_binding_revision_check
        check (resulting_binding_revision is null or resulting_binding_revision > 0)
);

create table tool_test_runs (
    id              uuid        not null,
    tenant_id       uuid        not null,
    version_id      uuid        not null,
    harness         text        not null,
    harness_version text        not null,
    outcome         text        not null,
    methods         text[]      not null,
    latency_ms      bigint,
    evidence        jsonb       not null,
    created_at      timestamptz not null default now(),
    created_by      uuid        not null,

    constraint tool_test_runs_pk primary key (tenant_id, id),
    constraint tool_test_runs_id_unique unique (id),
    constraint tool_test_runs_version_fk foreign key (tenant_id, version_id)
        references tool_server_versions (tenant_id, id),
    constraint tool_test_runs_harness_check
        check (harness in ('trusted_local_adapter', 'remote_http_adapter')),
    constraint tool_test_runs_harness_version_check
        check (btrim(harness_version) <> '' and length(harness_version) <= 200),
    constraint tool_test_runs_outcome_check check (outcome in ('passed', 'failed', 'error')),
    constraint tool_test_runs_methods_check
        check (cardinality(methods) between 1 and 10),
    constraint tool_test_runs_latency_check check (latency_ms is null or latency_ms >= 0),
    constraint tool_test_runs_evidence_check check (
        jsonb_typeof(evidence) = 'object' and pg_column_size(evidence) <= 65536
    )
);

create index tool_test_runs_by_version
    on tool_test_runs (tenant_id, version_id, created_at desc, id desc);

-- ── Workflow and immutability guards ───────────────────────────────────

create function synveda_tool_version_matches_proposal() returns trigger
language plpgsql
as $$
begin
    if not exists (
        select 1 from vedaflow_proposals proposal
         where proposal.tenant_id = new.tenant_id
           and proposal.id = new.proposal_id
           and proposal.asset_kind = 'tool'
           and proposal.target_channel = 'apply'
    ) then
        raise exception 'Tool version must bind a Tool/apply proposal'
            using errcode = '23514';
    end if;
    return new;
end
$$;

create constraint trigger tool_versions_match_proposal
    after insert on tool_server_versions
    deferrable initially deferred
    for each row execute function synveda_tool_version_matches_proposal();

create function synveda_tool_server_current_is_approved() returns trigger
language plpgsql
as $$
declare
    version_proposal uuid;
begin
    version_proposal := new.current_version_id;
    if version_proposal is null then
        return new;
    end if;
    if not exists (
        select 1
          from tool_server_versions version
          join vedaflow_proposals proposal
            on proposal.tenant_id = version.tenant_id
           and proposal.id = version.proposal_id
         where version.tenant_id = new.tenant_id
           and version.id = version_proposal
           and proposal.state = 'applied'
    ) then
        raise exception 'Tool current pointers and bindings require an approved version'
            using errcode = '23514';
    end if;
    return new;
end
$$;

create function synveda_tool_binding_version_is_approved() returns trigger
language plpgsql
as $$
begin
    if not exists (
        select 1
          from tool_server_versions version
          join vedaflow_proposals proposal
            on proposal.tenant_id = version.tenant_id
           and proposal.id = version.proposal_id
         where version.tenant_id = new.tenant_id
           and version.id = new.version_id
           and proposal.state = 'applied'
    ) then
        raise exception 'Tool current pointers and bindings require an approved version'
            using errcode = '23514';
    end if;
    return new;
end
$$;

create constraint trigger tool_server_current_approved
    after insert or update on tool_servers
    deferrable initially deferred
    for each row execute function synveda_tool_server_current_is_approved();
create constraint trigger tool_binding_version_approved
    after insert or update on tool_bindings
    deferrable initially deferred
    for each row execute function synveda_tool_binding_version_is_approved();

create function synveda_tool_server_transition() returns trigger
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
        raise exception 'a ToolServer identity is immutable (CPR-25)';
    end if;
    if new.current_version_id is not distinct from old.current_version_id
        or new.updated_at <= old.updated_at
    then
        raise exception 'a ToolServer update must advance its approved version (CPR-25)';
    end if;
    return new;
end
$$;
create trigger tool_servers_transition
    before update on tool_servers
    for each row execute function synveda_tool_server_transition();

create function synveda_immutable_tool_row() returns trigger
language plpgsql
as $$
begin
    raise exception '% rows are immutable (CPR-25)', tg_table_name;
end
$$;
create trigger tool_server_versions_immutable
    before update or delete on tool_server_versions
    for each row execute function synveda_immutable_tool_row();
create trigger capability_snapshots_immutable
    before update or delete on capability_snapshots
    for each row execute function synveda_immutable_tool_row();
create trigger tool_test_runs_immutable
    before update or delete on tool_test_runs
    for each row execute function synveda_immutable_tool_row();

create function synveda_tool_binding_transition() returns trigger
language plpgsql
as $$
begin
    if new.id <> old.id
        or new.tenant_id <> old.tenant_id
        or new.project_id <> old.project_id
        or new.server_id <> old.server_id
        or new.created_at <> old.created_at
        or new.created_by <> old.created_by
    then
        raise exception 'a ToolBinding identity is immutable (CPR-25)';
    end if;
    if new.revision <> old.revision + 1 or new.updated_at <= old.updated_at then
        raise exception 'a ToolBinding update must advance revision exactly once (CPR-25)';
    end if;
    if new.version_id = old.version_id and new.state = old.state then
        raise exception 'a ToolBinding update must change version or state (CPR-25)';
    end if;
    return new;
end
$$;
create trigger tool_bindings_transition
    before update on tool_bindings
    for each row execute function synveda_tool_binding_transition();

create function synveda_tool_change_transition() returns trigger
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
        raise exception 'a Tool VedaFlow command is immutable (CPR-25)';
    end if;
    if old.applied_at is not null or new.applied_at is null then
        raise exception 'a Tool change result may be recorded exactly once (CPR-25)';
    end if;
    return new;
end
$$;
create trigger tool_changes_transition
    before update on tool_changes
    for each row execute function synveda_tool_change_transition();

-- ── Least privilege and forced RLS ─────────────────────────────────────

grant select, insert on tool_servers to synveda_app;
grant update (current_version_id, updated_at, updated_by) on tool_servers to synveda_app;
grant select, insert on tool_server_versions, capability_snapshots to synveda_app;
grant select, insert on tool_bindings to synveda_app;
grant update (version_id, state, revision, updated_at, updated_by)
    on tool_bindings to synveda_app;
grant select, insert on tool_changes to synveda_app;
grant update (resulting_server_id, resulting_version_id, resulting_binding_id,
              resulting_binding_revision, applied_at)
    on tool_changes to synveda_app;
grant select, insert on tool_test_runs to synveda_app;

alter table tool_servers enable row level security;
alter table tool_servers force row level security;
alter table tool_server_versions enable row level security;
alter table tool_server_versions force row level security;
alter table capability_snapshots enable row level security;
alter table capability_snapshots force row level security;
alter table tool_bindings enable row level security;
alter table tool_bindings force row level security;
alter table tool_changes enable row level security;
alter table tool_changes force row level security;
alter table tool_test_runs enable row level security;
alter table tool_test_runs force row level security;

create policy tool_servers_tenant_isolation on tool_servers
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy tool_server_versions_tenant_isolation on tool_server_versions
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy capability_snapshots_tenant_isolation on capability_snapshots
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy tool_bindings_tenant_isolation on tool_bindings
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy tool_changes_tenant_isolation on tool_changes
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy tool_test_runs_tenant_isolation on tool_test_runs
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
