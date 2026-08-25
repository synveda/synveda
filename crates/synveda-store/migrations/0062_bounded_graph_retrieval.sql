-- CPR-38 / ADR-0097: anchor-first bounded retrieval over KnowledgeRelation.
--
-- The predecessor graph was a second, Record-backed domain. This pre-1.0
-- cut refuses to translate either it or pre-native context traces: reset is
-- the only supported path into the Knowledge graph epoch.

do $$
begin
    if exists (select 1 from graph_vertices limit 1)
       or exists (select 1 from graph_edges limit 1)
       or exists (select 1 from graph_edges_history limit 1)
       or exists (select 1 from session_context_runs limit 1)
       or exists (select 1 from context_candidates limit 1)
       or exists (select 1 from context_selections limit 1) then
        raise exception using
            errcode = '55000',
            message = 'CPR-38 refuses Record-graph or pre-native context data; run `synveda reset --database --force`';
    end if;
end
$$;

drop view graph_edges_versions;
drop table graph_edges_history;
drop table graph_edges;
drop table graph_vertices;
drop function graph_edges_tx_insert();
drop function graph_edges_tx_update();
drop function graph_edges_tx_delete();
drop function graph_edges_history_append_only();
drop function graph_edges_block_truncate();

alter table context_candidates
    drop constraint context_candidates_scores_check,
    drop constraint context_candidates_reasons_check,
    drop constraint context_candidates_exclusion_check,
    add column anchor_score_micros integer not null default 0,
    add column edge_weight_micros integer not null default 0,
    add column hop_penalty_micros integer not null default 0;

alter table context_candidates
    alter column anchor_score_micros drop default,
    alter column edge_weight_micros drop default,
    alter column hop_penalty_micros drop default,
    add constraint context_candidates_tenant_run_id_unique
        unique (tenant_id, context_run_id, id),
    add constraint context_candidates_scores_check check (
        keyword_score_micros between 0 and 1000000
        and semantic_score_micros between 0 and 1000000
        and anchor_score_micros between 0 and 5000000
        and edge_weight_micros between 0 and 2000000
        and hop_penalty_micros between 0 and 1000000
        and freshness_score_micros between 0 and 1000000
        and pin_score_micros between 0 and 1000000
        and current_state_score_micros between 0 and 1000000
        and final_score_micros between 0 and 5000000
    ),
    add constraint context_candidates_reasons_check check (
        cardinality(reason_codes) between 1 and 13
        and array_position(reason_codes, null) is null
        and reason_codes <@ array[
            'semantic_match', 'keyword_match', 'project_convention',
            'personal_preference', 'freshness_boost', 'explicit_pin',
            'superseded', 'stale', 'outside_task_scope',
            'token_budget', 'duplicate', 'graph_expansion',
            'contradiction_warning'
        ]::text[]
    ),
    add constraint context_candidates_exclusion_check check (
        exclusion_reason is null or exclusion_reason in (
            'semantic_match', 'keyword_match', 'project_convention',
            'personal_preference', 'freshness_boost', 'explicit_pin',
            'superseded', 'stale', 'outside_task_scope',
            'token_budget', 'duplicate', 'graph_expansion',
            'contradiction_warning'
        )
    );

alter table context_selections
    drop constraint context_selections_reasons_check,
    add column context_candidate_id uuid not null,
    add constraint context_selections_candidate_fk
        foreign key (tenant_id, context_run_id, context_candidate_id)
        references context_candidates (tenant_id, context_run_id, id),
    add constraint context_selections_reasons_check check (
        cardinality(reason_codes) between 1 and 13
        and array_position(reason_codes, null) is null
        and reason_codes <@ array[
            'semantic_match', 'keyword_match', 'project_convention',
            'personal_preference', 'freshness_boost', 'explicit_pin',
            'superseded', 'stale', 'outside_task_scope',
            'token_budget', 'duplicate', 'graph_expansion',
            'contradiction_warning'
        ]::text[]
    );

create table context_graph_steps (
    tenant_id             uuid        not null,
    context_run_id        uuid        not null,
    context_candidate_id  uuid        not null,
    ordinal               integer     not null,
    hop                   smallint    not null,
    relation_id           uuid,
    relation_hash         text        not null,
    relation_type         text        not null,
    direction             text        not null,
    from_item_id          uuid,
    from_revision_id      uuid,
    to_item_id            uuid,
    to_revision_id        uuid,
    asserting_revision_id uuid,
    from_content_hash     text        not null,
    to_content_hash       text        not null,
    edge_weight_micros    integer     not null,
    supporting            boolean     not null,
    created_at            timestamptz not null default now(),

    constraint context_graph_steps_pk
        primary key (tenant_id, context_candidate_id, ordinal),
    constraint context_graph_steps_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint context_graph_steps_candidate_fk
        foreign key (tenant_id, context_run_id, context_candidate_id)
        references context_candidates (tenant_id, context_run_id, id),
    constraint context_graph_steps_relation_fk
        foreign key (tenant_id, relation_id)
        references knowledge_relations (tenant_id, id),
    constraint context_graph_steps_from_revision_fk
        foreign key (tenant_id, from_item_id, from_revision_id)
        references knowledge_revisions (tenant_id, knowledge_item_id, id),
    constraint context_graph_steps_to_revision_fk
        foreign key (tenant_id, to_item_id, to_revision_id)
        references knowledge_revisions (tenant_id, knowledge_item_id, id),
    constraint context_graph_steps_asserting_revision_fk
        foreign key (tenant_id, asserting_revision_id)
        references knowledge_revisions (tenant_id, id),
    constraint context_graph_steps_address_shape_check check (
        (relation_id is null and from_item_id is null
         and from_revision_id is null and to_item_id is null
         and to_revision_id is null and asserting_revision_id is null)
        or
        (relation_id is not null and from_item_id is not null
         and from_revision_id is not null and to_item_id is not null
         and to_revision_id is not null and asserting_revision_id is not null)
    ),
    constraint context_graph_steps_position_check
        check (ordinal between 0 and 1 and hop between 1 and 2
               and ordinal = hop - 1),
    constraint context_graph_steps_hash_check check (
        relation_hash ~ '^[0-9a-f]{64}$'
        and from_content_hash ~ '^[0-9a-f]{64}$'
        and to_content_hash ~ '^[0-9a-f]{64}$'
    ),
    constraint context_graph_steps_type_check check (relation_type in (
        'supports', 'contradicts', 'supersedes', 'derived_from',
        'references', 'related_to', 'transitions_to'
    )),
    constraint context_graph_steps_direction_check
        check (direction in ('outbound', 'inbound')),
    constraint context_graph_steps_evidence_check check (
        (supporting
         and relation_type in (
             'supports', 'supersedes', 'derived_from', 'references',
             'related_to', 'transitions_to'
         )
         and edge_weight_micros between 1 and 1000000)
        or
        (not supporting and relation_type = 'contradicts'
         and edge_weight_micros = 0)
    )
);

create index context_graph_steps_by_run
    on context_graph_steps (tenant_id, context_run_id, context_candidate_id, ordinal);
create index context_graph_steps_by_relation
    on context_graph_steps (tenant_id, relation_id, created_at)
    where relation_id is not null;

create trigger context_graph_steps_immutable
    before update or delete on context_graph_steps
    for each row execute function synveda_context_trace_immutable();

grant select, insert on context_graph_steps to synveda_app;
alter table context_graph_steps enable row level security;
alter table context_graph_steps force row level security;
create policy context_graph_steps_tenant_isolation on context_graph_steps
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
