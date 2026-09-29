-- OPS-6 / ADR-0121: additive CTX-6 and CTX-8 schema after the exact v0.4.3
-- epoch-3 baseline. Apply with application writers stopped; this migration is
-- transactional, and an incompatible old binary must not serve the new head.

ALTER TABLE public.session_events
    DROP CONSTRAINT session_events_type_check;
ALTER TABLE public.session_events
    ADD CONSTRAINT session_events_type_check CHECK ((event_type = ANY (ARRAY[
        'session.started'::text, 'session.ended'::text,
        'session.compaction_boundary'::text, 'session.checkpoint'::text,
        'message.user'::text, 'message.assistant'::text,
        'tool.invoked'::text, 'tool.result'::text,
        'file.read'::text, 'file.changed'::text,
        'command.executed'::text, 'skill.loaded'::text,
        'context.requested'::text, 'adapter.warning'::text,
        'memory.asserted'::text
    ])));

ALTER TABLE public.session_context_runs
    ADD COLUMN checkpoint_event_id uuid,
    ADD COLUMN restart_event_ids uuid[] DEFAULT '{}'::uuid[] NOT NULL,
    ADD CONSTRAINT session_context_runs_restart_events_check
        CHECK ((cardinality(restart_event_ids) <= 16));

ALTER TABLE public.context_candidates
    DROP CONSTRAINT context_candidates_exclusion_check,
    DROP CONSTRAINT context_candidates_reasons_check;
ALTER TABLE public.context_candidates
    ADD CONSTRAINT context_candidates_exclusion_check CHECK (((exclusion_reason IS NULL) OR (exclusion_reason = ANY (ARRAY['semantic_match'::text, 'keyword_match'::text, 'project_convention'::text, 'personal_preference'::text, 'freshness_boost'::text, 'explicit_pin'::text, 'superseded'::text, 'stale'::text, 'outside_task_scope'::text, 'token_budget'::text, 'excerpt'::text, 'duplicate'::text, 'graph_expansion'::text, 'contradiction_warning'::text])))),
    ADD CONSTRAINT context_candidates_reasons_check CHECK ((((cardinality(reason_codes) >= 1) AND (cardinality(reason_codes) <= 14)) AND (array_position(reason_codes, NULL::text) IS NULL) AND (reason_codes <@ ARRAY['semantic_match'::text, 'keyword_match'::text, 'project_convention'::text, 'personal_preference'::text, 'freshness_boost'::text, 'explicit_pin'::text, 'superseded'::text, 'stale'::text, 'outside_task_scope'::text, 'token_budget'::text, 'excerpt'::text, 'duplicate'::text, 'graph_expansion'::text, 'contradiction_warning'::text])));

ALTER TABLE public.context_selections
    DROP CONSTRAINT context_selections_reasons_check;
ALTER TABLE public.context_selections
    ADD CONSTRAINT context_selections_reasons_check CHECK ((((cardinality(reason_codes) >= 1) AND (cardinality(reason_codes) <= 14)) AND (array_position(reason_codes, NULL::text) IS NULL) AND (reason_codes <@ ARRAY['semantic_match'::text, 'keyword_match'::text, 'project_convention'::text, 'personal_preference'::text, 'freshness_boost'::text, 'explicit_pin'::text, 'superseded'::text, 'stale'::text, 'outside_task_scope'::text, 'token_budget'::text, 'excerpt'::text, 'duplicate'::text, 'graph_expansion'::text, 'contradiction_warning'::text])));

CREATE FUNCTION public.synveda_context_run_restart_sources() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.checkpoint_event_id is not null and not exists (
        select 1 from session_events event
        where event.tenant_id = new.tenant_id
          and event.session_id = new.session_id
          and event.id = new.checkpoint_event_id
          and event.event_type = 'session.checkpoint'
    ) then
        raise exception 'context run checkpoint must be from its own session'
            using errcode = '23514';
    end if;
    if (select count(*) from unnest(new.restart_event_ids) as source(id)) <>
       (select count(distinct id) from unnest(new.restart_event_ids) as source(id))
       or exists (
           select 1 from unnest(new.restart_event_ids) as source(id)
           where not exists (
               select 1 from session_events event
               where event.tenant_id = new.tenant_id
                 and event.session_id = new.session_id
                 and event.id = source.id
                 and event.event_type = 'message.user'
           )
       ) then
        raise exception 'restart event sources must be unique user events in this session'
            using errcode = '23514';
    end if;
    return new;
end
$$;

CREATE INDEX session_events_checkpoints_latest
    ON public.session_events USING btree (tenant_id, session_id, sequence DESC)
    WHERE (event_type = 'session.checkpoint'::text);

CREATE TRIGGER session_context_runs_restart_sources
    BEFORE INSERT ON public.session_context_runs FOR EACH ROW
    EXECUTE FUNCTION public.synveda_context_run_restart_sources();
