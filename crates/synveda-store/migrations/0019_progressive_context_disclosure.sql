-- ADR-0163: a manifest selection and the server-derived expansion evidence.
ALTER TABLE public.session_events DROP CONSTRAINT session_events_type_check;
ALTER TABLE public.session_events ADD CONSTRAINT session_events_type_check CHECK
 (event_type IN ('session.started','session.ended','message.user','message.assistant','tool.invoked','tool.result',
 'file.read','file.changed','command.executed','skill.loaded','context.requested','adapter.warning','memory.asserted',
 'session.compaction_boundary','session.checkpoint','session.task_state','task.outcome','procedure.improvement',
 'context.knowledge_expanded'));

ALTER TABLE public.context_candidates DROP CONSTRAINT context_candidates_reasons_check;
ALTER TABLE public.context_candidates ADD CONSTRAINT context_candidates_reasons_check CHECK
 ((cardinality(reason_codes) >= 1) AND (cardinality(reason_codes) <= 15) AND (array_position(reason_codes, NULL::text) IS NULL)
 AND (reason_codes <@ ARRAY['semantic_match'::text, 'keyword_match'::text, 'project_convention'::text, 'personal_preference'::text, 'freshness_boost'::text, 'explicit_pin'::text, 'superseded'::text, 'stale'::text, 'outside_task_scope'::text, 'token_budget'::text, 'excerpt'::text, 'duplicate'::text, 'graph_expansion'::text, 'contradiction_warning'::text, 'manifest_entry'::text]));

ALTER TABLE public.context_selections DROP CONSTRAINT context_selections_reasons_check;
ALTER TABLE public.context_selections ADD CONSTRAINT context_selections_reasons_check CHECK
 ((cardinality(reason_codes) >= 1) AND (cardinality(reason_codes) <= 15) AND (array_position(reason_codes, NULL::text) IS NULL)
 AND (reason_codes <@ ARRAY['semantic_match'::text, 'keyword_match'::text, 'project_convention'::text, 'personal_preference'::text, 'freshness_boost'::text, 'explicit_pin'::text, 'superseded'::text, 'stale'::text, 'outside_task_scope'::text, 'token_budget'::text, 'excerpt'::text, 'duplicate'::text, 'graph_expansion'::text, 'contradiction_warning'::text, 'manifest_entry'::text]));
