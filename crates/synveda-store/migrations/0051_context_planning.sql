-- CPR-20 / ADR-0084: explainable Knowledge-backed context planning.
--
-- `session_context_runs` remains the aggregate head and delivery record. The
-- three new append-only tables retain only visible candidate/selection
-- addresses and content-free feedback. Knowledge body text is never copied
-- into them, and a candidate denied by Cedar never gets a row.

-- The denormalised workspace/project on a run is derived from its session.
-- These two unique targets let the foreign keys below prove that derivation
-- structurally instead of trusting the inserting service.
alter table sessions
    add constraint sessions_tenant_id_workspace_unique
        unique (tenant_id, id, workspace_id);
alter table sessions
    add constraint sessions_tenant_id_project_unique
        unique (tenant_id, id, project_id);

alter table session_context_runs
    add column workspace_id uuid,
    add column project_id uuid,
    add column query_hash text,
    add column requested_budget_tokens integer,
    add column candidate_count integer not null default 0,
    add column selection_count integer not null default 0,
    add column as_of timestamptz,
    add column retrieval_version text,
    add column embedding_model text,
    add column index_version text,
    add column graph_version text,
    add column trace_retention_mode text,
    add column completion_status text,
    add column policy_exclusion boolean not null default false;

-- No old row is translated. A planner-native row supplies the complete
-- non-null shape below; an opaque pre-cut row keeps every marker null and is
-- excluded by all application reads. Prompt 33 deletes that migration-era
-- shape when it replaces this chain with the clean baseline.

alter table session_context_runs
    add constraint session_context_runs_workspace_session_fk
        foreign key (tenant_id, session_id, workspace_id)
        references sessions (tenant_id, id, workspace_id),
    add constraint session_context_runs_project_session_fk
        foreign key (tenant_id, session_id, project_id)
        references sessions (tenant_id, id, project_id),
    add constraint session_context_runs_planner_shape_check
        check ((workspace_id is null
                and as_of is null
                and retrieval_version is null
                and index_version is null
                and trace_retention_mode is null
                and completion_status is null)
               or
               (workspace_id is not null
                and as_of is not null
                and retrieval_version is not null
                and index_version is not null
                and trace_retention_mode is not null
                and completion_status is not null)),
    add constraint session_context_runs_query_hash_check
        check (as_of is null
               or ((query is null) = (query_hash is null)
                   and (query_hash is null or query_hash ~ '^[0-9a-f]{64}$'))),
    add constraint session_context_runs_requested_budget_check
        check (requested_budget_tokens is null or requested_budget_tokens > 0),
    add constraint session_context_runs_trace_counts_check
        check (candidate_count >= 0 and selection_count >= 0),
    add constraint session_context_runs_retrieval_version_check
        check (btrim(retrieval_version) <> ''
               and char_length(retrieval_version) <= 200),
    add constraint session_context_runs_embedding_model_check
        check (embedding_model is null
               or (btrim(embedding_model) <> ''
                   and char_length(embedding_model) <= 300)),
    add constraint session_context_runs_index_version_check
        check (btrim(index_version) <> '' and char_length(index_version) <= 200),
    add constraint session_context_runs_graph_version_check
        check (graph_version is null
               or (btrim(graph_version) <> '' and char_length(graph_version) <= 200)),
    add constraint session_context_runs_trace_retention_check
        check (trace_retention_mode in ('full', 'redacted', 'hashes_only', 'disabled')),
    add constraint session_context_runs_completion_check
        check (completion_status in ('pending', 'completed', 'failed')),
    add constraint session_context_runs_tenant_id_unique
        unique (tenant_id, id);

create index session_context_runs_by_tenant
    on session_context_runs (tenant_id, created_at desc, id desc);
create index session_context_runs_by_project
    on session_context_runs (tenant_id, project_id, created_at desc, id desc)
    where project_id is not null;

create table context_candidates (
    id                         uuid        not null,
    tenant_id                  uuid        not null,
    context_run_id             uuid        not null,
    ordinal                    integer     not null,
    knowledge_item_id          uuid,
    knowledge_revision_id      uuid,
    content_hash               text        not null,
    scope_id                   uuid,
    lifecycle_state            text,
    keyword_score_micros       integer     not null default 0,
    semantic_score_micros      integer     not null default 0,
    freshness_score_micros     integer     not null default 0,
    pin_score_micros           integer     not null default 0,
    current_state_score_micros integer     not null default 0,
    final_score_micros         integer     not null,
    reason_codes               text[]      not null,
    exclusion_reason           text,
    created_at                 timestamptz not null default now(),

    constraint context_candidates_pk primary key (id),
    constraint context_candidates_tenant_id_unique unique (tenant_id, id),
    constraint context_candidates_run_ordinal_unique
        unique (tenant_id, context_run_id, ordinal),
    constraint context_candidates_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint context_candidates_run_fk
        foreign key (tenant_id, context_run_id)
        references session_context_runs (tenant_id, id),
    constraint context_candidates_item_fk
        foreign key (tenant_id, knowledge_item_id)
        references knowledge_items (tenant_id, id),
    constraint context_candidates_revision_fk
        foreign key (tenant_id, knowledge_item_id, knowledge_revision_id)
        references knowledge_revisions (tenant_id, knowledge_item_id, id),
    constraint context_candidates_scope_fk
        foreign key (tenant_id, scope_id) references scopes (tenant_id, id),
    constraint context_candidates_address_shape_check
        check ((knowledge_item_id is null) = (knowledge_revision_id is null)
               and (knowledge_item_id is null) = (scope_id is null)),
    constraint context_candidates_ordinal_check check (ordinal >= 0),
    constraint context_candidates_hash_check check (content_hash ~ '^[0-9a-f]{64}$'),
    constraint context_candidates_lifecycle_check
        check (lifecycle_state is null or lifecycle_state in (
            'active', 'stale', 'superseded', 'archived',
            'erasure_pending', 'erased'
        )),
    constraint context_candidates_scores_check
        check (keyword_score_micros between 0 and 1000000
               and semantic_score_micros between 0 and 1000000
               and freshness_score_micros between 0 and 1000000
               and pin_score_micros between 0 and 1000000
               and current_state_score_micros between 0 and 1000000
               and final_score_micros between 0 and 5000000),
    constraint context_candidates_reasons_check
        check (cardinality(reason_codes) between 1 and 11
               and array_position(reason_codes, null) is null
               and reason_codes <@ array[
                   'semantic_match', 'keyword_match', 'project_convention',
                   'personal_preference', 'freshness_boost', 'explicit_pin',
                   'superseded', 'stale', 'outside_task_scope',
                   'token_budget', 'duplicate'
               ]::text[]),
    constraint context_candidates_exclusion_check
        check (exclusion_reason is null or exclusion_reason in (
            'semantic_match', 'keyword_match', 'project_convention',
            'personal_preference', 'freshness_boost', 'explicit_pin',
            'superseded', 'stale', 'outside_task_scope',
            'token_budget', 'duplicate'
        ))
);

create index context_candidates_by_run
    on context_candidates (tenant_id, context_run_id, ordinal);
create index context_candidates_by_revision
    on context_candidates (tenant_id, knowledge_revision_id, created_at desc)
    where knowledge_revision_id is not null;

create table context_selections (
    id                    uuid        not null,
    tenant_id             uuid        not null,
    context_run_id        uuid        not null,
    rank                  integer     not null,
    knowledge_item_id     uuid,
    knowledge_revision_id uuid,
    content_hash          text        not null,
    token_count           integer     not null,
    reason_codes          text[]      not null,
    created_at            timestamptz not null default now(),

    constraint context_selections_pk primary key (id),
    constraint context_selections_tenant_id_unique unique (tenant_id, id),
    constraint context_selections_feedback_target_unique
        unique (tenant_id, id, context_run_id, knowledge_revision_id),
    constraint context_selections_run_rank_unique
        unique (tenant_id, context_run_id, rank),
    constraint context_selections_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint context_selections_run_fk
        foreign key (tenant_id, context_run_id)
        references session_context_runs (tenant_id, id),
    constraint context_selections_revision_fk
        foreign key (tenant_id, knowledge_item_id, knowledge_revision_id)
        references knowledge_revisions (tenant_id, knowledge_item_id, id),
    constraint context_selections_address_shape_check
        check ((knowledge_item_id is null) = (knowledge_revision_id is null)),
    constraint context_selections_rank_check check (rank >= 1),
    constraint context_selections_hash_check check (content_hash ~ '^[0-9a-f]{64}$'),
    constraint context_selections_token_check check (token_count >= 0),
    constraint context_selections_reasons_check
        check (cardinality(reason_codes) between 1 and 11
               and array_position(reason_codes, null) is null
               and reason_codes <@ array[
                   'semantic_match', 'keyword_match', 'project_convention',
                   'personal_preference', 'freshness_boost', 'explicit_pin',
                   'superseded', 'stale', 'outside_task_scope',
                   'token_budget', 'duplicate'
               ]::text[])
);

create index context_selections_by_run
    on context_selections (tenant_id, context_run_id, rank);
create index context_selections_by_knowledge
    on context_selections
        (tenant_id, knowledge_item_id, created_at desc, id desc)
    where knowledge_item_id is not null;

create table context_feedback (
    id                    uuid        not null,
    tenant_id             uuid        not null,
    context_run_id        uuid        not null,
    context_selection_id  uuid        not null,
    knowledge_revision_id uuid        not null,
    feedback_type         text        not null,
    principal_id          text        not null,
    idempotency_key       text        not null,
    created_at            timestamptz not null default now(),

    constraint context_feedback_pk primary key (id),
    constraint context_feedback_tenant_id_unique unique (tenant_id, id),
    constraint context_feedback_idempotency_unique
        unique (tenant_id, context_run_id, idempotency_key),
    constraint context_feedback_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint context_feedback_selection_fk
        foreign key (
            tenant_id, context_selection_id, context_run_id,
            knowledge_revision_id
        ) references context_selections (
            tenant_id, id, context_run_id, knowledge_revision_id
        ),
    constraint context_feedback_type_check check (feedback_type in (
        'referenced_by_agent', 'accepted_by_user', 'helpful',
        'unhelpful', 'caused_correction'
    )),
    constraint context_feedback_principal_check
        check (btrim(principal_id) <> '' and char_length(principal_id) <= 255),
    constraint context_feedback_idempotency_check
        check (btrim(idempotency_key) <> '' and char_length(idempotency_key) <= 200)
);

create index context_feedback_by_run
    on context_feedback (tenant_id, context_run_id, created_at, id);

-- Historical planner rows never change, including under the owner role used
-- by migrations, break-glass inspection and restore. Erasure removes content
-- from Knowledge and leaves these content-free hashes/ids as evidence.
create function synveda_context_trace_immutable() returns trigger
language plpgsql
as $$
begin
    raise exception '% is immutable (CPR-20, ADR-0084)', tg_table_name;
end
$$;

create trigger session_context_runs_immutable
    before update or delete on session_context_runs
    for each row execute function synveda_context_trace_immutable();
create trigger context_candidates_immutable
    before update or delete on context_candidates
    for each row execute function synveda_context_trace_immutable();
create trigger context_selections_immutable
    before update or delete on context_selections
    for each row execute function synveda_context_trace_immutable();
create trigger context_feedback_immutable
    before update or delete on context_feedback
    for each row execute function synveda_context_trace_immutable();

grant select, insert on context_candidates to synveda_app;
grant select, insert on context_selections to synveda_app;
grant select, insert on context_feedback to synveda_app;

alter table context_candidates enable row level security;
alter table context_candidates force row level security;
alter table context_selections enable row level security;
alter table context_selections force row level security;
alter table context_feedback enable row level security;
alter table context_feedback force row level security;

create policy context_candidates_tenant_isolation on context_candidates
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy context_selections_tenant_isolation on context_selections
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy context_feedback_tenant_isolation on context_feedback
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
