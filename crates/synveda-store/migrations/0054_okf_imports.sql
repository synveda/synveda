-- CPR-27: bounded OKF v0.2 import plans and candidate-only materialisation
-- (ADR-0087).
--
-- An import is not Knowledge. Immutable artifacts and mappings form a dry-run
-- plan; materialisation creates ordinary CaptureCandidates. The capture source
-- becomes a closed union so file provenance never masquerades as a session.

create table import_jobs (
    id                    uuid        not null,
    tenant_id             uuid        not null,
    project_id            uuid        not null,
    scope_id              uuid        not null,
    workspace_id          uuid        not null,
    principal_id          text        not null,
    format                text        not null,
    format_version        text        not null,
    specification_commit  text        not null,
    source_kind           text        not null,
    source_locator        text        not null,
    source_revision       text,
    bundle_digest         text        not null,
    state                 text        not null default 'planned',
    artifact_count        integer     not null,
    mapping_count         integer     not null,
    candidate_count       integer     not null default 0,
    capture_batch_id      uuid,
    error_code            text,
    notices               jsonb       not null default '[]'::jsonb,
    created_at            timestamptz not null default now(),
    completed_at          timestamptz,
    updated_at            timestamptz not null default now(),

    constraint import_jobs_pk primary key (id),
    constraint import_jobs_tenant_id_unique unique (tenant_id, id),
    constraint import_jobs_source_digest_unique
        unique (tenant_id, project_id, source_kind, source_locator, bundle_digest),
    constraint import_jobs_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint import_jobs_project_fk
        foreign key (tenant_id, project_id) references projects (tenant_id, id),
    constraint import_jobs_scope_fk
        foreign key (tenant_id, scope_id) references scopes (tenant_id, id),
    constraint import_jobs_workspace_fk
        foreign key (tenant_id, workspace_id) references workspaces (tenant_id, id),
    constraint import_jobs_principal_check
        check (btrim(principal_id) <> '' and char_length(principal_id) <= 255),
    constraint import_jobs_format_check check (format = 'okf'),
    constraint import_jobs_version_check check (format_version = '0.2'),
    constraint import_jobs_specification_check
        check (specification_commit = 'ad30107c31c06aec8a7d5636e0d1058118604e6f'),
    constraint import_jobs_source_kind_check
        check (source_kind in ('directory', 'zip', 'tar', 'git')),
    constraint import_jobs_source_locator_check
        check (btrim(source_locator) <> '' and char_length(source_locator) <= 1000),
    constraint import_jobs_source_revision_check
        check ((source_kind = 'git' and source_revision is not null)
               or source_kind <> 'git'),
    constraint import_jobs_source_revision_value_check
        check (source_revision is null
               or (btrim(source_revision) <> '' and char_length(source_revision) <= 255)),
    constraint import_jobs_digest_check check (bundle_digest ~ '^[0-9a-f]{64}$'),
    constraint import_jobs_state_check
        check (state in ('planned', 'materialized', 'failed')),
    constraint import_jobs_count_check
        check (artifact_count between 1 and 2000
               and mapping_count between 1 and artifact_count
               and candidate_count between 0 and mapping_count),
    constraint import_jobs_error_check
        check (error_code is null
               or (btrim(error_code) <> '' and char_length(error_code) <= 100)),
    constraint import_jobs_notices_check
        check (jsonb_typeof(notices) = 'array'
               and octet_length(notices::text) <= 32768),
    constraint import_jobs_state_shape_check
        check ((state = 'planned' and capture_batch_id is null
                and candidate_count = 0 and completed_at is null and error_code is null)
               or (state = 'materialized' and capture_batch_id is not null
                   and completed_at is not null and error_code is null)
               or (state = 'failed' and capture_batch_id is null
                   and candidate_count = 0 and completed_at is not null
                   and error_code is not null)),
    constraint import_jobs_time_check
        check (updated_at >= created_at
               and (completed_at is null or completed_at >= created_at))
);

create function synveda_import_job_project_identity() returns trigger
language plpgsql
as $$
begin
    if not exists (
        select 1 from projects project
        where project.tenant_id = new.tenant_id
          and project.id = new.project_id
          and project.scope_id = new.scope_id
          and project.workspace_id = new.workspace_id
    ) then
        raise exception 'import job placement must match its project'
            using errcode = '23514';
    end if;
    return new;
end
$$;

create trigger import_jobs_project_identity
    before insert on import_jobs
    for each row execute function synveda_import_job_project_identity();

create index import_jobs_by_project
    on import_jobs (tenant_id, project_id, created_at desc, id desc);
create index import_jobs_by_state
    on import_jobs (tenant_id, state, created_at, id);

create table import_artifacts (
    id                uuid        not null,
    tenant_id         uuid        not null,
    job_id            uuid        not null,
    ordinal           integer     not null,
    logical_path      text        not null,
    artifact_kind     text        not null,
    content_hash      text        not null,
    frontmatter       jsonb       not null,
    body_markdown     text        not null,
    created_at        timestamptz not null default now(),

    constraint import_artifacts_pk primary key (id),
    constraint import_artifacts_tenant_id_unique unique (tenant_id, id),
    constraint import_artifacts_job_id_unique unique (tenant_id, job_id, id),
    constraint import_artifacts_path_unique unique (tenant_id, job_id, logical_path),
    constraint import_artifacts_ordinal_unique unique (tenant_id, job_id, ordinal),
    constraint import_artifacts_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint import_artifacts_job_fk
        foreign key (tenant_id, job_id) references import_jobs (tenant_id, id),
    constraint import_artifacts_ordinal_check check (ordinal between 1 and 2000),
    constraint import_artifacts_path_check
        check (btrim(logical_path) <> '' and char_length(logical_path) <= 1000
               and left(logical_path, 1) <> '/'
               and position(E'\\' in logical_path) = 0
               and logical_path !~ '(^|/)\.\.?(/|$)'),
    constraint import_artifacts_kind_check
        check (artifact_kind in ('concept', 'index', 'log')),
    constraint import_artifacts_hash_check check (content_hash ~ '^[0-9a-f]{64}$'),
    constraint import_artifacts_frontmatter_check
        check (jsonb_typeof(frontmatter) = 'object'
               and octet_length(frontmatter::text) <= 32768),
    constraint import_artifacts_body_check
        check (octet_length(body_markdown) <= 262144)
);

create table import_mappings (
    id                          uuid        not null,
    tenant_id                   uuid        not null,
    job_id                      uuid        not null,
    artifact_id                 uuid        not null,
    ordinal                     integer     not null,
    okf_type                    text        not null,
    knowledge_type              text        not null,
    title                       text        not null,
    body_markdown               text        not null,
    summary                     text        not null,
    tags                        text[]      not null default '{}'::text[],
    sensitivity                 text        not null,
    confidence_permille         integer     not null,
    valid_from                  timestamptz not null,
    valid_to                    timestamptz,
    stale_after                 timestamptz,
    verification_metadata       jsonb       not null,
    metadata                    jsonb       not null,
    content_hash                text        not null,
    classification              text        not null,
    matched_item_id             uuid,
    matched_revision_id         uuid,
    proposed_relations          jsonb       not null default '[]'::jsonb,
    materializable              boolean     not null,
    candidate_id                uuid,
    created_at                  timestamptz not null default now(),

    constraint import_mappings_pk primary key (id),
    constraint import_mappings_tenant_id_unique unique (tenant_id, id),
    constraint import_mappings_job_id_unique unique (tenant_id, job_id, id),
    constraint import_mappings_artifact_unique unique (tenant_id, job_id, artifact_id),
    constraint import_mappings_ordinal_unique unique (tenant_id, job_id, ordinal),
    constraint import_mappings_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint import_mappings_job_fk
        foreign key (tenant_id, job_id) references import_jobs (tenant_id, id),
    constraint import_mappings_artifact_fk
        foreign key (tenant_id, job_id, artifact_id)
        references import_artifacts (tenant_id, job_id, id),
    constraint import_mappings_ordinal_check check (ordinal between 1 and 2000),
    constraint import_mappings_okf_type_check
        check (btrim(okf_type) <> '' and char_length(okf_type) <= 200),
    constraint import_mappings_knowledge_type_check
        check (knowledge_type in (
            'fact', 'decision', 'preference', 'procedure', 'entity',
            'episode', 'convention', 'warning', 'reference'
        )),
    constraint import_mappings_title_check
        check (btrim(title) <> '' and char_length(title) <= 300),
    constraint import_mappings_body_check
        check (btrim(body_markdown) <> '' and octet_length(body_markdown) <= 131072),
    constraint import_mappings_summary_check
        check (btrim(summary) <> '' and char_length(summary) <= 2000),
    constraint import_mappings_tags_check check (synveda_knowledge_tags_canonical(tags)),
    constraint import_mappings_sensitivity_check
        check (sensitivity in ('public', 'internal', 'confidential', 'restricted')),
    constraint import_mappings_confidence_check check (confidence_permille between 0 and 1000),
    constraint import_mappings_valid_time_check check (valid_to is null or valid_to > valid_from),
    constraint import_mappings_stale_time_check
        check (stale_after is null
               or (stale_after > valid_from and (valid_to is null or stale_after <= valid_to))),
    constraint import_mappings_verification_check
        check (jsonb_typeof(verification_metadata) = 'object'
               and octet_length(verification_metadata::text) <= 16384),
    constraint import_mappings_metadata_check
        check (jsonb_typeof(metadata) = 'object'
               and octet_length(metadata::text) <= 16384),
    constraint import_mappings_hash_check check (content_hash ~ '^[0-9a-f]{64}$'),
    constraint import_mappings_classification_check
        check (classification in ('addition', 'update', 'duplicate', 'conflict')),
    constraint import_mappings_match_shape_check
        check ((classification = 'addition'
                and matched_item_id is null and matched_revision_id is null)
               or (classification <> 'addition'
                   and matched_item_id is not null and matched_revision_id is not null)),
    constraint import_mappings_matched_revision_fk
        foreign key (tenant_id, matched_item_id, matched_revision_id)
        references knowledge_revisions (tenant_id, knowledge_item_id, id),
    constraint import_mappings_relations_check
        check (jsonb_typeof(proposed_relations) = 'array'
               and octet_length(proposed_relations::text) <= 65536)
);

create index import_mappings_by_classification
    on import_mappings (tenant_id, job_id, classification, ordinal);
create index import_mappings_by_match
    on import_mappings (tenant_id, matched_item_id)
    where matched_item_id is not null;

-- Capture provenance becomes `session XOR okf_import`. Existing rows are all
-- session-sourced and receive that explicit discriminator.
drop trigger capture_batches_session_identity on capture_batches;
drop function synveda_capture_batch_session_identity();
drop trigger capture_batches_transition on capture_batches;
drop function synveda_capture_batch_transition();
drop trigger capture_candidates_transition on capture_candidates;
drop function synveda_capture_candidate_transition();

alter table capture_batches drop constraint capture_batches_snapshot_unique;
alter table capture_batches drop constraint capture_batches_session_fk;
alter table capture_batches alter column session_id drop not null;
alter table capture_batches add column source_kind text not null default 'session';
alter table capture_batches add column import_job_id uuid;
alter table capture_batches add constraint capture_batches_source_kind_check
    check (source_kind in ('session', 'okf_import'));
alter table capture_batches add constraint capture_batches_source_shape_check
    check ((source_kind = 'session' and session_id is not null and import_job_id is null)
           or (source_kind = 'okf_import' and session_id is null
               and import_job_id is not null and project_id is not null
               and event_count = 0));
alter table capture_batches add constraint capture_batches_session_fk
    foreign key (tenant_id, session_id) references sessions (tenant_id, id);
alter table capture_batches add constraint capture_batches_import_job_fk
    foreign key (tenant_id, import_job_id) references import_jobs (tenant_id, id);
alter table capture_batches add constraint capture_batches_tenant_import_id_unique
    unique (tenant_id, import_job_id, id);

create unique index capture_batches_session_snapshot_unique
    on capture_batches (tenant_id, session_id, input_hash)
    where source_kind = 'session';
create unique index capture_batches_import_unique
    on capture_batches (tenant_id, import_job_id)
    where source_kind = 'okf_import';

create function synveda_capture_batch_source_identity() returns trigger
language plpgsql
as $$
begin
    if new.source_kind = 'session' then
        if not exists (
            select 1 from sessions session
            where session.tenant_id = new.tenant_id
              and session.id = new.session_id
              and session.scope_id = new.scope_id
              and session.workspace_id = new.workspace_id
              and session.project_id is not distinct from new.project_id
              and session.principal_id = new.principal_id
        ) then
            raise exception 'capture batch identity must match its session'
                using errcode = '23514';
        end if;
    elsif not exists (
        select 1 from import_jobs job
        where job.tenant_id = new.tenant_id
          and job.id = new.import_job_id
          and job.scope_id = new.scope_id
          and job.workspace_id = new.workspace_id
          and job.project_id = new.project_id
    ) then
        raise exception 'capture batch identity must match its import job'
            using errcode = '23514';
    end if;
    return new;
end
$$;

create trigger capture_batches_source_identity
    before insert on capture_batches
    for each row execute function synveda_capture_batch_source_identity();

alter table capture_candidates drop constraint capture_candidates_batch_fk;
alter table capture_candidates alter column session_id drop not null;
alter table capture_candidates add column source_kind text not null default 'session';
alter table capture_candidates add column import_job_id uuid;
alter table capture_candidates add constraint capture_candidates_source_kind_check
    check (source_kind in ('session', 'okf_import'));
alter table capture_candidates add constraint capture_candidates_source_shape_check
    check ((source_kind = 'session' and session_id is not null and import_job_id is null)
           or (source_kind = 'okf_import' and session_id is null and import_job_id is not null));
alter table capture_candidates add constraint capture_candidates_batch_fk
    foreign key (tenant_id, batch_id) references capture_batches (tenant_id, id);
alter table capture_candidates add constraint capture_candidates_import_job_fk
    foreign key (tenant_id, import_job_id) references import_jobs (tenant_id, id);
alter table capture_candidates add constraint capture_candidates_tenant_import_id_unique
    unique (tenant_id, import_job_id, id);

create function synveda_capture_candidate_source_identity() returns trigger
language plpgsql
as $$
begin
    if not exists (
        select 1 from capture_batches batch
        where batch.tenant_id = new.tenant_id
          and batch.id = new.batch_id
          and batch.source_kind = new.source_kind
          and batch.session_id is not distinct from new.session_id
          and batch.import_job_id is not distinct from new.import_job_id
    ) then
        raise exception 'capture candidate source must match its batch'
            using errcode = '23514';
    end if;
    return new;
end
$$;

create trigger capture_candidates_source_identity
    before insert on capture_candidates
    for each row execute function synveda_capture_candidate_source_identity();

create table capture_candidate_import_artifacts (
    tenant_id      uuid        not null,
    candidate_id   uuid        not null,
    import_job_id  uuid        not null,
    artifact_id    uuid        not null,
    ordinal        integer     not null,
    linked_at      timestamptz not null default now(),

    constraint capture_candidate_import_artifacts_pk
        primary key (tenant_id, candidate_id, artifact_id),
    constraint capture_candidate_import_artifacts_ordinal_unique
        unique (tenant_id, candidate_id, ordinal),
    constraint capture_candidate_import_artifacts_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint capture_candidate_import_artifacts_candidate_fk
        foreign key (tenant_id, import_job_id, candidate_id)
        references capture_candidates (tenant_id, import_job_id, id),
    constraint capture_candidate_import_artifacts_artifact_fk
        foreign key (tenant_id, import_job_id, artifact_id)
        references import_artifacts (tenant_id, job_id, id),
    constraint capture_candidate_import_artifacts_ordinal_check check (ordinal between 1 and 200)
);

alter table import_mappings add constraint import_mappings_candidate_fk
    foreign key (tenant_id, candidate_id) references capture_candidates (tenant_id, id);

alter table import_jobs add constraint import_jobs_capture_batch_fk
    foreign key (tenant_id, capture_batch_id) references capture_batches (tenant_id, id);

create function synveda_capture_batch_transition() returns trigger
language plpgsql
as $$
begin
    if new.id <> old.id or new.tenant_id <> old.tenant_id
       or new.source_kind <> old.source_kind
       or new.session_id is distinct from old.session_id
       or new.import_job_id is distinct from old.import_job_id
       or new.scope_id <> old.scope_id or new.workspace_id <> old.workspace_id
       or new.project_id is distinct from old.project_id
       or new.principal_id <> old.principal_id or new.input_hash <> old.input_hash
       or new.event_count <> old.event_count or new.created_at <> old.created_at then
        raise exception 'capture batch evidence is immutable';
    end if;
    if not (
        (old.state = 'pending' and new.state = 'running')
        or (old.state = 'running' and new.state in ('pending', 'completed', 'failed'))
        or (old.state = new.state)
    ) then
        raise exception 'invalid capture batch transition: % -> %', old.state, new.state;
    end if;
    return new;
end
$$;

create trigger capture_batches_transition
    before update on capture_batches
    for each row execute function synveda_capture_batch_transition();

create function synveda_capture_candidate_transition() returns trigger
language plpgsql
as $$
begin
    if new.id <> old.id or new.tenant_id <> old.tenant_id
       or new.batch_id <> old.batch_id or new.source_kind <> old.source_kind
       or new.session_id is distinct from old.session_id
       or new.import_job_id is distinct from old.import_job_id
       or new.ordinal <> old.ordinal or new.proposed_scope_id <> old.proposed_scope_id
       or new.proposed_project_id is distinct from old.proposed_project_id
       or new.proposed_owner_principal_id is distinct from old.proposed_owner_principal_id
       or new.knowledge_type <> old.knowledge_type or new.origin <> old.origin
       or new.content_hash <> old.content_hash or new.created_at <> old.created_at then
        raise exception 'capture candidate identity and proposal are immutable';
    end if;
    if current_setting('synveda.knowledge_erasure', true) = 'on' then
        if not new.content_erased or old.content_erased then
            raise exception 'capture candidate erasure is one-way';
        end if;
        return new;
    end if;
    if new.title <> old.title or new.body_markdown <> old.body_markdown
       or new.summary <> old.summary or new.tags <> old.tags
       or new.sensitivity <> old.sensitivity
       or new.confidence_permille <> old.confidence_permille
       or new.valid_from <> old.valid_from or new.valid_to is distinct from old.valid_to
       or new.stale_after is distinct from old.stale_after
       or new.verification_metadata <> old.verification_metadata
       or new.metadata <> old.metadata or new.content_erased <> old.content_erased then
        raise exception 'capture candidate content is immutable';
    end if;
    if old.state <> 'pending' and new is distinct from old then
        raise exception 'capture candidate decision is terminal';
    end if;
    return new;
end
$$;

create trigger capture_candidates_transition
    before update on capture_candidates
    for each row execute function synveda_capture_candidate_transition();

create function synveda_import_job_transition() returns trigger
language plpgsql
as $$
begin
    if new.id <> old.id or new.tenant_id <> old.tenant_id
       or new.project_id <> old.project_id or new.scope_id <> old.scope_id
       or new.workspace_id <> old.workspace_id or new.principal_id <> old.principal_id
       or new.format <> old.format or new.format_version <> old.format_version
       or new.specification_commit <> old.specification_commit
       or new.source_kind <> old.source_kind or new.source_locator <> old.source_locator
       or new.source_revision is distinct from old.source_revision
       or new.bundle_digest <> old.bundle_digest or new.artifact_count <> old.artifact_count
       or new.mapping_count <> old.mapping_count or new.notices <> old.notices
       or new.created_at <> old.created_at then
        raise exception 'import job plan is immutable';
    end if;
    if old.state <> 'planned' and new is distinct from old then
        raise exception 'import job result is terminal';
    end if;
    if old.state = 'planned' and new.state not in ('planned', 'materialized', 'failed') then
        raise exception 'invalid import job transition: % -> %', old.state, new.state;
    end if;
    return new;
end
$$;

create trigger import_jobs_transition
    before update on import_jobs
    for each row execute function synveda_import_job_transition();

create function synveda_import_mapping_transition() returns trigger
language plpgsql
as $$
begin
    if new.id <> old.id or new.tenant_id <> old.tenant_id
       or new.job_id <> old.job_id or new.artifact_id <> old.artifact_id
       or new.ordinal <> old.ordinal or new.okf_type <> old.okf_type
       or new.knowledge_type <> old.knowledge_type or new.title <> old.title
       or new.body_markdown <> old.body_markdown or new.summary <> old.summary
       or new.tags <> old.tags or new.sensitivity <> old.sensitivity
       or new.confidence_permille <> old.confidence_permille
       or new.valid_from <> old.valid_from or new.valid_to is distinct from old.valid_to
       or new.stale_after is distinct from old.stale_after
       or new.verification_metadata <> old.verification_metadata
       or new.metadata <> old.metadata or new.content_hash <> old.content_hash
       or new.classification <> old.classification
       or new.matched_item_id is distinct from old.matched_item_id
       or new.matched_revision_id is distinct from old.matched_revision_id
       or new.proposed_relations <> old.proposed_relations
       or new.materializable <> old.materializable or new.created_at <> old.created_at then
        raise exception 'import mapping is immutable';
    end if;
    if old.candidate_id is not null or new.candidate_id is null then
        raise exception 'import mapping candidate may be assigned exactly once';
    end if;
    return new;
end
$$;

create trigger import_mappings_transition
    before update on import_mappings
    for each row execute function synveda_import_mapping_transition();

create function synveda_import_append_only() returns trigger
language plpgsql
as $$
begin
    raise exception '% is append-only (CPR-27, ADR-0087)', tg_table_name;
end
$$;

create trigger import_artifacts_append_only
    before update or delete or truncate on import_artifacts
    for each statement execute function synveda_import_append_only();
create trigger capture_candidate_import_artifacts_append_only
    before update or delete or truncate on capture_candidate_import_artifacts
    for each statement execute function synveda_import_append_only();

grant select, insert on import_jobs to synveda_app;
grant update (state, candidate_count, capture_batch_id, error_code, completed_at, updated_at)
    on import_jobs to synveda_app;
grant select, insert on import_artifacts to synveda_app;
grant select, insert on import_mappings to synveda_app;
grant update (candidate_id) on import_mappings to synveda_app;
grant select, insert on capture_candidate_import_artifacts to synveda_app;

alter table import_jobs enable row level security;
alter table import_jobs force row level security;
alter table import_artifacts enable row level security;
alter table import_artifacts force row level security;
alter table import_mappings enable row level security;
alter table import_mappings force row level security;
alter table capture_candidate_import_artifacts enable row level security;
alter table capture_candidate_import_artifacts force row level security;

create policy import_jobs_tenant_isolation on import_jobs
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy import_artifacts_tenant_isolation on import_artifacts
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy import_mappings_tenant_isolation on import_mappings
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy capture_candidate_import_artifacts_tenant_isolation
    on capture_candidate_import_artifacts
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
