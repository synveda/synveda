-- ADR-0160: the three cleanup-authority routines keep their plans. Every
-- restrictive source-eligibility policy calls them for a row whose source is
-- unavailable. As LANGUAGE sql with a SET clause they are neither inlined nor
-- cached before PostgreSQL 18, so each such row re-planned a join of up to
-- eight tables, although the database-owner test that opens each body is
-- false for every runtime role. The bodies below are unchanged; plpgsql keeps
-- one generic plan per session and evaluates that test first.
CREATE OR REPLACE FUNCTION public.synveda_artifact_cleanup_authorized(t uuid,k text,d uuid) RETURNS boolean
 LANGUAGE plpgsql STABLE SET search_path TO 'public','pg_temp' SET plan_cache_mode TO 'force_generic_plan' AS $$
BEGIN
 RETURN (
 SELECT current_user=pg_catalog.pg_get_userbyid((SELECT datdba FROM pg_catalog.pg_database WHERE datname=pg_catalog.current_database()))
 AND t=public.synveda_current_tenant() AND EXISTS (
 SELECT 1 FROM public.artifact_cleanup_targets target
 JOIN public.durable_operations op ON op.tenant_id=target.tenant_id AND op.id=target.operation_id
 JOIN public.operation_attempts attempt ON attempt.tenant_id=op.tenant_id AND attempt.operation_id=op.id AND attempt.attempt_number=op.attempts
 JOIN public.artifact_suppressions suppression ON suppression.tenant_id=target.tenant_id AND suppression.source_key_digest=target.source_key_digest AND suppression.forget_change_id=target.change_id
 JOIN public.knowledge_change_headers change ON change.tenant_id=target.tenant_id AND change.proposal_id=target.change_id
 JOIN public.source_artifact_addresses address ON address.tenant_id=target.tenant_id AND address.artifact_id=target.artifact_id AND address.source_key_digest=target.source_key_digest
 JOIN public.vedaflow_proposals proposal ON proposal.tenant_id=change.tenant_id AND proposal.id=change.proposal_id
 JOIN public.artifact_dependencies dep ON dep.tenant_id=target.tenant_id AND dep.source_key_digest=target.source_key_digest
 WHERE target.tenant_id=t AND dep.dependent_kind=k AND dep.dependent_id=d
 AND target.operation_id::text=current_setting('synveda.artifact_cleanup_operation',true)
 AND op.attempts::text=current_setting('synveda.artifact_cleanup_attempt',true)
 AND op.kind='source_cleanup' AND op.state='running' AND op.lease_expires_at>clock_timestamp()
 AND attempt.state='running' AND attempt.worker_id=op.lease_owner
 AND op.input_hash=target.input_hash AND change.payload_hash=target.input_hash
 AND change.source_artifact_id=target.artifact_id AND change.source_artifact_hash=address.content_hash
 AND change.source_scope_id=address.scope_id AND change.source_mode='forget'
 AND change.command_kind='control_artifact' AND change.operation_id=op.id AND change.applied_at IS NOT NULL
 AND proposal.state='applied' AND proposal.asset_kind='knowledge' AND proposal.target_channel='apply')
 );
END $$;
CREATE OR REPLACE FUNCTION public.synveda_source_cleanup_authorized(t uuid,k text,d uuid) RETURNS boolean
 LANGUAGE plpgsql STABLE SET search_path TO 'public','pg_temp' SET plan_cache_mode TO 'force_generic_plan' AS $$
BEGIN
 RETURN (
 SELECT current_user = pg_catalog.pg_get_userbyid((SELECT datdba FROM pg_catalog.pg_database
    WHERE datname=pg_catalog.current_database())) AND EXISTS (
    SELECT 1 FROM public.source_cleanup_targets target
    JOIN public.durable_operations op ON op.tenant_id=target.tenant_id AND op.id=target.operation_id
    JOIN public.operation_attempts attempt ON attempt.tenant_id=op.tenant_id
       AND attempt.operation_id=op.id AND attempt.attempt_number=op.attempts
    JOIN public.source_suppressions suppression ON suppression.tenant_id=target.tenant_id
       AND suppression.event_id=target.event_id AND suppression.forget_change_id=target.change_id
    JOIN public.knowledge_change_headers change ON change.tenant_id=target.tenant_id AND change.proposal_id=target.change_id
    JOIN public.vedaflow_proposals proposal ON proposal.tenant_id=change.tenant_id AND proposal.id=change.proposal_id
    JOIN public.source_dependencies dep ON dep.tenant_id=target.tenant_id AND dep.source_event_id=target.event_id
    WHERE target.tenant_id=t AND t=public.synveda_current_tenant() AND dep.dependent_kind=k AND dep.dependent_id=d
      AND target.operation_id::text=current_setting('synveda.source_cleanup_operation',true)
      AND op.attempts::text=current_setting('synveda.source_cleanup_attempt',true)
      AND op.kind='source_cleanup' AND op.state='running' AND op.lease_expires_at>clock_timestamp()
      AND attempt.state='running' AND attempt.worker_id=op.lease_owner
      AND op.input_hash=target.input_hash AND change.payload_hash=target.input_hash
      AND change.source_event_id=target.event_id AND change.source_mode='forget'
      AND change.command_kind='control_source' AND change.operation_id=op.id AND change.applied_at IS NOT NULL
      AND proposal.state='applied' AND proposal.asset_kind='knowledge' AND proposal.target_channel='apply')
 );
END $$;
CREATE OR REPLACE FUNCTION public.synveda_item_cleanup_authorized(t uuid,k text,d uuid) RETURNS boolean
 LANGUAGE plpgsql STABLE SET search_path TO 'public','pg_temp' SET plan_cache_mode TO 'force_generic_plan' AS $$
BEGIN
 RETURN (
 SELECT current_user=pg_catalog.pg_get_userbyid((SELECT datdba FROM pg_catalog.pg_database
   WHERE datname=pg_catalog.current_database())) AND EXISTS (
   SELECT 1 FROM public.durable_operations op JOIN public.knowledge_change_headers change
    ON change.tenant_id=op.tenant_id AND change.proposal_id=op.proposal_id
   WHERE op.tenant_id=t AND t=public.synveda_current_tenant()
    AND op.id::text=current_setting('synveda.item_cleanup_operation',true)
    AND op.kind='knowledge_erasure' AND op.state='running' AND op.lease_expires_at>clock_timestamp()
    AND change.command_kind='forget'
    AND (EXISTS(SELECT 1 FROM public.knowledge_items item WHERE item.tenant_id=t AND item.id=op.knowledge_item_id
      AND item.lifecycle_state='erasure_pending') OR EXISTS(SELECT 1 FROM public.knowledge_erasure_tombstones tombstone
      WHERE tombstone.tenant_id=t AND tombstone.knowledge_item_id=op.knowledge_item_id AND tombstone.operation_id=op.id
      AND tombstone.proposal_id=op.proposal_id))
    AND change.payload_hash=op.input_hash AND change.target_item_ids @> ARRAY[op.knowledge_item_id]
    AND (
      (k='knowledge_item' AND d=op.knowledge_item_id)
      OR EXISTS(SELECT 1 FROM public.historical_knowledge_dependencies dep WHERE dep.tenant_id=t
       AND dep.dependent_kind=k AND dep.dependent_id=d AND dep.knowledge_item_id=op.knowledge_item_id)
      OR (k IN ('session_event','context_run') AND EXISTS(SELECT 1 FROM public.knowledge_copy_dependencies copy
       WHERE copy.tenant_id=t AND copy.dependent_kind=k AND copy.dependent_id=d AND copy.knowledge_item_id=op.knowledge_item_id))
      OR (k='knowledge_revision' AND EXISTS(SELECT 1 FROM public.knowledge_revision_headers h
       WHERE h.tenant_id=t AND h.revision_id=d AND h.knowledge_item_id=op.knowledge_item_id))
      OR (k='knowledge_source' AND EXISTS(SELECT 1 FROM public.knowledge_revision_source_headers link
       JOIN public.knowledge_revision_headers h ON h.tenant_id=link.tenant_id AND h.revision_id=link.knowledge_revision_id
       WHERE link.tenant_id=t AND link.knowledge_source_id=d AND h.knowledge_item_id=op.knowledge_item_id))
      OR (k='knowledge_change' AND EXISTS(SELECT 1 FROM public.knowledge_change_headers h
       WHERE h.tenant_id=t AND h.proposal_id=d AND h.target_item_ids @> ARRAY[op.knowledge_item_id]))
      OR (k='context_run' AND (EXISTS (SELECT 1 FROM public.context_selections selection WHERE selection.tenant_id=t AND selection.context_run_id=d AND selection.knowledge_item_id=op.knowledge_item_id) OR EXISTS(SELECT 1 FROM public.context_run_delivery_dependencies m
       WHERE m.tenant_id=t AND m.context_run_id=d AND (m.manifest @> jsonb_build_object('knowledge',
       jsonb_build_array(jsonb_build_object('item_id',op.knowledge_item_id)))
       OR m.manifest @> jsonb_build_object('graph',jsonb_build_array(jsonb_build_object('from_item_id',op.knowledge_item_id)))
       OR m.manifest @> jsonb_build_object('graph',jsonb_build_array(jsonb_build_object('to_item_id',op.knowledge_item_id)))))))
      OR (k='capture_candidate' AND EXISTS(SELECT 1 FROM public.source_dependencies candidate
       JOIN public.source_dependencies revision ON revision.tenant_id=candidate.tenant_id
        AND revision.source_event_id=candidate.source_event_id AND revision.dependent_kind='knowledge_revision'
       JOIN public.knowledge_revision_headers h ON h.tenant_id=revision.tenant_id AND h.revision_id=revision.dependent_id
       WHERE candidate.tenant_id=t AND candidate.dependent_kind=k AND candidate.dependent_id=d AND h.knowledge_item_id=op.knowledge_item_id))
      OR (k='capture_candidate' AND EXISTS(SELECT 1 FROM public.artifact_dependencies candidate JOIN public.artifact_dependencies revision ON revision.tenant_id=candidate.tenant_id AND revision.source_key_digest=candidate.source_key_digest AND revision.dependent_kind='knowledge_revision' JOIN public.knowledge_revision_headers h ON h.tenant_id=revision.tenant_id AND h.revision_id=revision.dependent_id WHERE candidate.tenant_id=t AND candidate.dependent_kind=k AND candidate.dependent_id=d AND h.knowledge_item_id=op.knowledge_item_id))
   ))
 );
END $$;
