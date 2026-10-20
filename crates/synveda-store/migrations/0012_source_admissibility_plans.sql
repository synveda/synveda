-- MEM-1: source admissibility keeps its plans.
--
-- Both functions run once per appended Session event: the source projection
-- trigger asks synveda_source_admissible, whose read of session_events passes
-- the restrictive eligibility policy and so asks synveda_source_available.
-- As LANGUAGE sql with a SET clause, PostgreSQL before 18 neither inlines nor
-- keeps a plan for either body, so every row re-planned both. The bodies below
-- are 0010's, unchanged; plpgsql keeps their plans for the session, and the
-- generic plan stops the CASE dispatch from re-planning every call.
CREATE OR REPLACE FUNCTION public.synveda_source_available(t uuid,k text,d uuid) RETURNS boolean
 LANGUAGE plpgsql STABLE SET search_path TO 'public','pg_temp' SET plan_cache_mode TO 'force_generic_plan' AS $$
BEGIN
 RETURN (
 SELECT t=public.synveda_current_tenant() AND (
 (NOT EXISTS(SELECT 1 FROM public.source_dependencies dep JOIN public.source_suppressions s ON s.tenant_id=dep.tenant_id AND s.event_id=dep.source_event_id WHERE dep.tenant_id=t AND dep.dependent_kind=k AND dep.dependent_id=d)
 AND NOT EXISTS(SELECT 1 FROM public.artifact_dependencies dep JOIN public.artifact_suppressions s ON s.tenant_id=dep.tenant_id AND s.source_key_digest=dep.source_key_digest WHERE dep.tenant_id=t AND dep.dependent_kind=k AND dep.dependent_id=d)
 AND NOT EXISTS(SELECT 1 FROM public.unproven_artifact_dependencies WHERE tenant_id=t AND dependent_kind=k AND dependent_id=d) AND NOT EXISTS(SELECT 1 FROM public.knowledge_copy_dependencies copy
 LEFT JOIN public.knowledge_items item ON item.tenant_id=copy.tenant_id AND item.id=copy.knowledge_item_id
 LEFT JOIN public.knowledge_revision_headers header ON header.tenant_id=copy.tenant_id AND header.revision_id=copy.revision_id
 WHERE copy.tenant_id=t AND copy.dependent_kind=k AND copy.dependent_id=d
 AND (item.id IS NULL OR item.lifecycle_state<>'active' OR item.current_revision_id<>copy.revision_id
 OR header.content_hash IS DISTINCT FROM copy.content_hash)) AND NOT EXISTS(SELECT 1 FROM public.historical_knowledge_dependencies dep
 LEFT JOIN public.knowledge_items item ON item.tenant_id=dep.tenant_id AND item.id=dep.knowledge_item_id
 LEFT JOIN public.knowledge_revision_headers header ON header.tenant_id=dep.tenant_id AND header.revision_id=dep.revision_id
 WHERE dep.tenant_id=t AND dep.dependent_kind=k AND dep.dependent_id=d
 AND (item.id IS NULL OR item.lifecycle_state='erasure_pending'
 OR header.knowledge_item_id IS DISTINCT FROM dep.knowledge_item_id
 OR header.content_hash IS DISTINCT FROM dep.content_hash)))
 OR public.synveda_source_cleanup_authorized(t,k,d) OR public.synveda_artifact_cleanup_authorized(t,k,d) OR public.synveda_item_cleanup_authorized(t,k,d))
 );
END $$;
CREATE OR REPLACE FUNCTION public.synveda_source_admissible(t uuid,k text,d uuid) RETURNS boolean
 LANGUAGE plpgsql STABLE SET search_path TO 'public','pg_temp' SET plan_cache_mode TO 'force_generic_plan' AS $$
BEGIN
 RETURN (
 SELECT t=public.synveda_current_tenant() AND CASE k
 WHEN 'session_event' THEN EXISTS(SELECT 1 FROM public.session_events WHERE tenant_id=t AND id=d AND NOT content_erased)
 WHEN 'import_artifact' THEN EXISTS(SELECT 1 FROM public.import_artifacts WHERE tenant_id=t AND id=d AND NOT content_erased)
 WHEN 'import_mapping' THEN EXISTS(SELECT 1 FROM public.import_mappings WHERE tenant_id=t AND id=d AND NOT content_erased)
 WHEN 'knowledge_revision' THEN EXISTS(SELECT 1 FROM public.knowledge_revisions WHERE tenant_id=t AND id=d AND NOT content_erased)
 WHEN 'knowledge_source' THEN EXISTS(SELECT 1 FROM public.knowledge_sources WHERE tenant_id=t AND id=d AND NOT content_erased)
 WHEN 'capture_candidate' THEN EXISTS(SELECT 1 FROM public.capture_candidates WHERE tenant_id=t AND id=d AND NOT content_erased)
 WHEN 'knowledge_change' THEN EXISTS(SELECT 1 FROM public.knowledge_changes WHERE tenant_id=t AND proposal_id=d AND payload IS NOT NULL)
 WHEN 'context_run' THEN EXISTS(SELECT 1 FROM public.session_context_runs WHERE tenant_id=t AND id=d)
 ELSE false END
 AND NOT EXISTS(SELECT 1 FROM public.source_dependencies dep JOIN public.source_suppressions s ON s.tenant_id=dep.tenant_id AND s.event_id=dep.source_event_id WHERE dep.tenant_id=t AND dep.dependent_kind=k AND dep.dependent_id=d)
 AND NOT EXISTS(SELECT 1 FROM public.artifact_dependencies dep JOIN public.artifact_suppressions s ON s.tenant_id=dep.tenant_id AND s.source_key_digest=dep.source_key_digest WHERE dep.tenant_id=t AND dep.dependent_kind=k AND dep.dependent_id=d)
 AND NOT EXISTS(SELECT 1 FROM public.unproven_artifact_dependencies WHERE tenant_id=t AND dependent_kind=k AND dependent_id=d) AND NOT EXISTS(SELECT 1 FROM public.knowledge_copy_dependencies copy
 LEFT JOIN public.knowledge_items item ON item.tenant_id=copy.tenant_id AND item.id=copy.knowledge_item_id
 LEFT JOIN public.knowledge_revision_headers header ON header.tenant_id=copy.tenant_id AND header.revision_id=copy.revision_id
 WHERE copy.tenant_id=t AND copy.dependent_kind=k AND copy.dependent_id=d
 AND (item.id IS NULL OR item.lifecycle_state<>'active' OR item.current_revision_id<>copy.revision_id
 OR header.content_hash IS DISTINCT FROM copy.content_hash)) AND NOT EXISTS(SELECT 1 FROM public.historical_knowledge_dependencies dep
 LEFT JOIN public.knowledge_items item ON item.tenant_id=dep.tenant_id AND item.id=dep.knowledge_item_id
 LEFT JOIN public.knowledge_revision_headers header ON header.tenant_id=dep.tenant_id AND header.revision_id=dep.revision_id
 WHERE dep.tenant_id=t AND dep.dependent_kind=k AND dep.dependent_id=d
 AND (item.id IS NULL OR item.lifecycle_state='erasure_pending'
 OR header.knowledge_item_id IS DISTINCT FROM dep.knowledge_item_id
 OR header.content_hash IS DISTINCT FROM dep.content_hash))
 );
END $$;
