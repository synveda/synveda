-- CPR-23: stable Agent Skill aggregates, immutable versions and governed
-- project/principal bindings (ADR-0085).
--
-- This is the pre-1.0 hard cut. The two mutable draft tables are dropped and
-- recreated as the new model without translating a row. Content objects and
-- VedaFlow object storage remains. Draft-bound checklists and quality
-- overrides leave with the old publication model: the typed proposal's
-- approvals and immutable test evidence are the review record now.

drop table skill_quality_overrides;
drop table skill_reviews;
drop table skill_files;
drop table skills;
drop function synveda_skill_file_transition();
drop function synveda_skill_transition();

alter table vedaflow_proposals
    drop constraint vedaflow_proposals_apply_asset_check;
alter table vedaflow_proposals
    add constraint vedaflow_proposals_apply_asset_check
    check (target_channel <> 'apply' or asset_kind in ('knowledge', 'skill'));

-- Stable identity and current approved version. The composite deferred FK is
-- declared after skill_versions so install can insert both ends atomically.
create table skills (
    id                 uuid        not null,
    tenant_id          uuid        not null,
    governing_scope_id uuid        not null,
    name               text        not null,
    current_version_id uuid        not null,
    created_at         timestamptz not null default now(),
    created_by         uuid        not null,
    updated_at         timestamptz not null default now(),
    updated_by         uuid        not null,

    constraint skills_pk primary key (tenant_id, id),
    constraint skills_id_unique unique (id),
    constraint skills_tenant_fk foreign key (tenant_id) references tenants (id),
    constraint skills_scope_fk foreign key (tenant_id, governing_scope_id)
        references scopes (tenant_id, id),
    constraint skills_name_unique unique (tenant_id, name),
    constraint skills_name_check check (length(name) between 1 and 64)
);

-- An approved bundle. There is no UPDATE or DELETE grant, and the trigger
-- below makes immutability true even to a privileged test connection.
create table skill_versions (
    id                   uuid        not null,
    tenant_id            uuid        not null,
    skill_id             uuid        not null,
    ordinal              bigint      not null,
    bundle_digest        bytea       not null,
    sensitivity          text        not null,
    manifest             jsonb       not null,
    source_kind          text        not null,
    provenance           jsonb       not null,
    scan_report          jsonb       not null,
    scan_ruleset_version integer     not null,
    quality_score        smallint    not null,
    rubric_version       integer     not null,
    created_at           timestamptz not null default now(),
    created_by           uuid        not null,

    constraint skill_versions_pk primary key (tenant_id, id),
    constraint skill_versions_id_unique unique (id),
    constraint skill_versions_skill_fk foreign key (tenant_id, skill_id)
        references skills (tenant_id, id),
    constraint skill_versions_skill_id_unique unique (tenant_id, skill_id, id),
    constraint skill_versions_ordinal_unique unique (tenant_id, skill_id, ordinal),
    constraint skill_versions_digest_unique unique (tenant_id, skill_id, bundle_digest),
    constraint skill_versions_ordinal_check check (ordinal > 0),
    constraint skill_versions_digest_check check (octet_length(bundle_digest) = 32),
    constraint skill_versions_sensitivity_check
        check (sensitivity in ('public', 'internal', 'confidential')),
    constraint skill_versions_manifest_check
        check (jsonb_typeof(manifest) = 'object' and pg_column_size(manifest) <= 32768),
    constraint skill_versions_source_check
        check (source_kind in ('authored', 'directory', 'archive', 'git', 'registry')),
    constraint skill_versions_provenance_check
        check (jsonb_typeof(provenance) = 'object' and pg_column_size(provenance) <= 32768),
    constraint skill_versions_scan_check
        check (jsonb_typeof(scan_report) = 'object' and pg_column_size(scan_report) <= 65536),
    constraint skill_versions_scan_version_check check (scan_ruleset_version > 0),
    constraint skill_versions_quality_check check (quality_score between 0 and 100),
    constraint skill_versions_rubric_check check (rubric_version > 0)
);

alter table skills add constraint skills_current_version_fk
    foreign key (tenant_id, id, current_version_id)
    references skill_versions (tenant_id, skill_id, id)
    deferrable initially deferred;

create table skill_version_files (
    tenant_id   uuid        not null,
    version_id  uuid        not null,
    path        text        not null,
    object_hash bytea       not null,
    chars       integer     not null,
    created_at  timestamptz not null default now(),

    constraint skill_version_files_pk primary key (tenant_id, version_id, path),
    constraint skill_version_files_version_fk foreign key (tenant_id, version_id)
        references skill_versions (tenant_id, id),
    constraint skill_version_files_object_fk foreign key (tenant_id, object_hash)
        references vedaflow_objects (tenant_id, hash),
    constraint skill_version_files_path_check check (length(path) between 1 and 128),
    constraint skill_version_files_hash_check check (octet_length(object_hash) = 32),
    constraint skill_version_files_chars_check check (chars between 0 and 65536)
);

create index skill_version_files_order
    on skill_version_files (tenant_id, version_id, path);

-- The only active distribution switch. Target scopes are checked by the
-- trigger below because shape lives on the referenced scope row.
create table skill_bindings (
    id                uuid        not null,
    tenant_id         uuid        not null,
    scope_id          uuid        not null,
    skill_id          uuid        not null,
    pinned_version_id uuid,
    enabled           boolean     not null,
    revision          bigint      not null default 1,
    created_at        timestamptz not null default now(),
    created_by        uuid        not null,
    updated_at        timestamptz not null default now(),
    updated_by        uuid        not null,

    constraint skill_bindings_pk primary key (tenant_id, id),
    constraint skill_bindings_id_unique unique (id),
    constraint skill_bindings_scope_fk foreign key (tenant_id, scope_id)
        references scopes (tenant_id, id),
    constraint skill_bindings_skill_fk foreign key (tenant_id, skill_id)
        references skills (tenant_id, id),
    constraint skill_bindings_pin_fk foreign key (tenant_id, skill_id, pinned_version_id)
        references skill_versions (tenant_id, skill_id, id),
    constraint skill_bindings_target_unique unique (tenant_id, scope_id, skill_id),
    constraint skill_bindings_revision_check check (revision > 0)
);

create index skill_bindings_available
    on skill_bindings (tenant_id, scope_id, enabled, skill_id)
    where enabled;

-- Typed projection for VedaFlow Skill/apply changes. It carries no workflow
-- status of its own; proposal state is the only status.
create table skill_changes (
    tenant_id                 uuid        not null,
    proposal_id               uuid        not null,
    command_kind              text        not null,
    payload                   jsonb       not null,
    payload_hash              text        not null,
    resulting_skill_id        uuid,
    resulting_version_id      uuid,
    resulting_binding_id      uuid,
    resulting_binding_revision bigint,
    applied_at                timestamptz,
    created_at                timestamptz not null default now(),

    constraint skill_changes_pk primary key (tenant_id, proposal_id),
    constraint skill_changes_tenant_fk foreign key (tenant_id) references tenants (id),
    constraint skill_changes_proposal_fk foreign key (tenant_id, proposal_id)
        references vedaflow_proposals (tenant_id, id),
    constraint skill_changes_kind_check
        check (command_kind in ('install', 'update', 'bind', 'set_binding')),
    constraint skill_changes_payload_check
        check (jsonb_typeof(payload) = 'object' and pg_column_size(payload) <= 1048576),
    constraint skill_changes_payload_hash_check check (length(payload_hash) = 64),
    constraint skill_changes_result_shape_check check (
        (applied_at is null and resulting_binding_revision is null)
        or applied_at is not null
    ),
    constraint skill_changes_binding_revision_check
        check (resulting_binding_revision is null or resulting_binding_revision > 0)
);

-- Append-only evidence. `client_event_id` is scoped to a binding so retries
-- are idempotent without conflating two installations of one version.
create table skill_usage_events (
    id              uuid        not null,
    tenant_id       uuid        not null,
    binding_id      uuid        not null,
    version_id      uuid        not null,
    session_id      uuid,
    principal_id    uuid        not null,
    client_event_id text        not null,
    stage           text        not null,
    evidence        text        not null,
    resource_path   text,
    metadata        jsonb       not null default '{}'::jsonb,
    occurred_at     timestamptz not null,
    received_at     timestamptz not null default now(),

    constraint skill_usage_events_pk primary key (tenant_id, id),
    constraint skill_usage_events_id_unique unique (id),
    constraint skill_usage_events_binding_fk foreign key (tenant_id, binding_id)
        references skill_bindings (tenant_id, id),
    constraint skill_usage_events_version_fk foreign key (tenant_id, version_id)
        references skill_versions (tenant_id, id),
    constraint skill_usage_events_session_fk foreign key (tenant_id, session_id)
        references sessions (tenant_id, id),
    constraint skill_usage_events_client_unique
        unique (tenant_id, binding_id, client_event_id),
    constraint skill_usage_events_client_check
        check (length(client_event_id) between 1 and 200),
    constraint skill_usage_events_stage_check check (stage in (
        'advertised', 'discovered', 'activated', 'instructions_loaded',
        'resource_loaded', 'script_requested', 'executed', 'outcome_reported'
    )),
    constraint skill_usage_events_evidence_check
        check (evidence in ('host_observed', 'model_reported')),
    constraint skill_usage_events_resource_check
        check (resource_path is null or length(resource_path) between 1 and 128),
    constraint skill_usage_events_metadata_check
        check (jsonb_typeof(metadata) = 'object' and pg_column_size(metadata) <= 16384)
);

create index skill_usage_events_by_version
    on skill_usage_events (tenant_id, version_id, received_at desc, id desc);
create index skill_usage_events_by_binding
    on skill_usage_events (tenant_id, binding_id, received_at desc, id desc);

create table skill_test_runs (
    id                   uuid        not null,
    tenant_id            uuid        not null,
    version_id           uuid        not null,
    harness              text        not null,
    harness_version      text        not null,
    outcome              text        not null,
    scan_ruleset_version integer     not null,
    rubric_version       integer     not null,
    evidence             jsonb       not null,
    created_at           timestamptz not null default now(),
    created_by           uuid        not null,

    constraint skill_test_runs_pk primary key (tenant_id, id),
    constraint skill_test_runs_id_unique unique (id),
    constraint skill_test_runs_version_fk foreign key (tenant_id, version_id)
        references skill_versions (tenant_id, id),
    constraint skill_test_runs_harness_check
        check (harness in ('validation_sandbox', 'controlled_client')),
    constraint skill_test_runs_harness_version_check
        check (length(harness_version) between 1 and 100),
    constraint skill_test_runs_outcome_check check (outcome in ('passed', 'failed', 'error')),
    constraint skill_test_runs_ruleset_check check (scan_ruleset_version > 0),
    constraint skill_test_runs_rubric_check check (rubric_version > 0),
    constraint skill_test_runs_evidence_check
        check (jsonb_typeof(evidence) = 'object' and pg_column_size(evidence) <= 32768)
);

create index skill_test_runs_by_version
    on skill_test_runs (tenant_id, version_id, created_at desc, id desc);

-- ── Transition guards ───────────────────────────────────────────────────

create function synveda_skill_aggregate_transition() returns trigger
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
        raise exception 'a Skill aggregate identity is immutable (CPR-23)';
    end if;
    if new.current_version_id = old.current_version_id
        or new.updated_at <= old.updated_at
        or new.updated_by = old.updated_by and new.updated_at = old.updated_at
    then
        raise exception 'a Skill update must advance its current immutable version (CPR-23)';
    end if;
    return new;
end
$$;

create trigger skills_transition
    before update on skills
    for each row execute function synveda_skill_aggregate_transition();

create function synveda_immutable_skill_row() returns trigger
language plpgsql
as $$
begin
    raise exception '% rows are immutable (CPR-23)', tg_table_name;
end
$$;

create trigger skill_versions_immutable
    before update or delete on skill_versions
    for each row execute function synveda_immutable_skill_row();
create trigger skill_version_files_immutable
    before update or delete on skill_version_files
    for each row execute function synveda_immutable_skill_row();
create trigger skill_usage_events_immutable
    before update or delete on skill_usage_events
    for each row execute function synveda_immutable_skill_row();
create trigger skill_test_runs_immutable
    before update or delete on skill_test_runs
    for each row execute function synveda_immutable_skill_row();

create function synveda_skill_binding_transition() returns trigger
language plpgsql
as $$
begin
    if new.id <> old.id
        or new.tenant_id <> old.tenant_id
        or new.scope_id <> old.scope_id
        or new.skill_id <> old.skill_id
        or new.created_at <> old.created_at
        or new.created_by <> old.created_by
    then
        raise exception 'a Skill binding identity is immutable (CPR-23)';
    end if;
    if new.revision <> old.revision + 1 or new.updated_at <= old.updated_at then
        raise exception 'a Skill binding update must advance revision exactly once (CPR-23)';
    end if;
    if new.enabled = old.enabled and new.pinned_version_id is not distinct from old.pinned_version_id then
        raise exception 'a Skill binding update must change enabled or pinned version (CPR-23)';
    end if;
    return new;
end
$$;

create trigger skill_bindings_transition
    before update on skill_bindings
    for each row execute function synveda_skill_binding_transition();

create function synveda_skill_binding_shape() returns trigger
language plpgsql
as $$
declare
    target_kind text;
begin
    select kind into target_kind
      from scopes
     where tenant_id = new.tenant_id and id = new.scope_id;
    if target_kind is null then
        raise exception 'Skill binding target scope does not exist (CPR-23)';
    end if;
    if target_kind not in ('project', 'principal') then
        raise exception 'Skill bindings target project or principal scopes, got % (CPR-23)', target_kind;
    end if;
    return new;
end
$$;

create trigger skill_bindings_shape
    before insert or update on skill_bindings
    for each row execute function synveda_skill_binding_shape();

create function synveda_skill_change_transition() returns trigger
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
        raise exception 'a Skill VedaFlow command is immutable (CPR-23)';
    end if;
    if old.applied_at is not null or new.applied_at is null then
        raise exception 'a Skill change result may be recorded exactly once (CPR-23)';
    end if;
    return new;
end
$$;

create trigger skill_changes_transition
    before update on skill_changes
    for each row execute function synveda_skill_change_transition();

-- ── Least privilege and forced RLS ─────────────────────────────────────

grant select, insert on skills to synveda_app;
grant update (current_version_id, updated_at, updated_by) on skills to synveda_app;
grant select, insert on skill_versions, skill_version_files to synveda_app;
grant select, insert on skill_bindings to synveda_app;
grant update (pinned_version_id, enabled, revision, updated_at, updated_by)
    on skill_bindings to synveda_app;
grant select, insert on skill_changes to synveda_app;
grant update (resulting_skill_id, resulting_version_id, resulting_binding_id,
              resulting_binding_revision, applied_at)
    on skill_changes to synveda_app;
grant select, insert on skill_usage_events, skill_test_runs to synveda_app;

alter table skills enable row level security;
alter table skills force row level security;
alter table skill_versions enable row level security;
alter table skill_versions force row level security;
alter table skill_version_files enable row level security;
alter table skill_version_files force row level security;
alter table skill_bindings enable row level security;
alter table skill_bindings force row level security;
alter table skill_changes enable row level security;
alter table skill_changes force row level security;
alter table skill_usage_events enable row level security;
alter table skill_usage_events force row level security;
alter table skill_test_runs enable row level security;
alter table skill_test_runs force row level security;

create policy skills_tenant_isolation on skills
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy skill_versions_tenant_isolation on skill_versions
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy skill_version_files_tenant_isolation on skill_version_files
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy skill_bindings_tenant_isolation on skill_bindings
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy skill_changes_tenant_isolation on skill_changes
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy skill_usage_events_tenant_isolation on skill_usage_events
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy skill_test_runs_tenant_isolation on skill_test_runs
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
