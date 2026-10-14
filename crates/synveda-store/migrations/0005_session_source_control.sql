-- MEM-8 / ADR-0146. Writers are stopped during this additive migration.
-- These projections contain identities and hashes, never source text.
CREATE TABLE public.source_event_addresses (
    tenant_id uuid NOT NULL, event_id uuid NOT NULL, session_id uuid NOT NULL,
    scope_id uuid NOT NULL, workspace_id uuid NOT NULL, project_id uuid, sequence bigint NOT NULL,
    client_event_id_digest text NOT NULL, source_payload_hash text NOT NULL,
    payload_hash text NOT NULL,
    PRIMARY KEY (tenant_id, event_id),
    CHECK (client_event_id_digest ~ '^[0-9a-f]{64}$'),
    CHECK (source_payload_hash ~ '^[0-9a-f]{64}$'),
    CHECK (payload_hash ~ '^[0-9a-f]{64}$'), CHECK (sequence > 0)
);
CREATE INDEX source_event_session_sequence ON public.source_event_addresses
    (tenant_id, session_id, sequence);
CREATE TABLE public.source_dependencies (
    tenant_id uuid NOT NULL, dependent_kind text NOT NULL, dependent_id uuid NOT NULL,
    source_event_id uuid NOT NULL,
    PRIMARY KEY (tenant_id, dependent_kind, dependent_id, source_event_id),
    CHECK (dependent_kind IN ('session_event','knowledge_revision','knowledge_source',
        'capture_candidate','knowledge_change','context_run'))
);
CREATE INDEX source_dependencies_root ON public.source_dependencies
    (tenant_id, source_event_id, dependent_kind, dependent_id);
CREATE TABLE public.knowledge_revision_headers (
    tenant_id uuid NOT NULL, revision_id uuid NOT NULL, knowledge_item_id uuid NOT NULL,
    revision_number bigint NOT NULL, sensitivity text NOT NULL, content_hash text NOT NULL,
    valid_from timestamptz NOT NULL, valid_to timestamptz, transaction_time timestamptz NOT NULL,
    legal_hold boolean NOT NULL, retention_hold boolean NOT NULL,
    PRIMARY KEY (tenant_id, revision_id), CHECK (revision_number > 0),
    CHECK (sensitivity IN ('public','internal','confidential','restricted')),
    CHECK (content_hash ~ '^[0-9a-f]{64}$')
);
-- Retained link identities keep item cleanup scoped after product links are deleted.
CREATE TABLE public.knowledge_revision_source_headers (
 tenant_id uuid NOT NULL, knowledge_revision_id uuid NOT NULL, knowledge_source_id uuid NOT NULL,
 PRIMARY KEY (tenant_id,knowledge_revision_id,knowledge_source_id)
);
ALTER TABLE public.knowledge_revision_source_headers ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.knowledge_revision_source_headers FORCE ROW LEVEL SECURITY;
CREATE POLICY knowledge_revision_source_header_tenant ON public.knowledge_revision_source_headers
 USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
GRANT SELECT ON public.knowledge_revision_source_headers TO synveda_app;
CREATE TABLE public.knowledge_change_headers (
 tenant_id uuid NOT NULL, proposal_id uuid NOT NULL, command_kind text NOT NULL, payload_hash text NOT NULL,
 target_item_ids uuid[] NOT NULL, source_event_id uuid, source_session_id uuid, source_scope_id uuid,
 source_payload_hash text, source_mode text, operation_id uuid, applied_at timestamptz,
 PRIMARY KEY (tenant_id,proposal_id), CHECK (payload_hash ~ '^[0-9a-f]{64}$'),
 CHECK (command_kind<>'control_source' OR (source_event_id IS NOT NULL AND source_session_id IS NOT NULL
  AND source_scope_id IS NOT NULL AND source_payload_hash ~ '^[0-9a-f]{64}$' AND source_mode IN ('withdraw','forget')))
);
ALTER TABLE public.knowledge_change_headers ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.knowledge_change_headers FORCE ROW LEVEL SECURITY;
CREATE POLICY knowledge_change_header_tenant ON public.knowledge_change_headers
 USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
GRANT SELECT ON public.knowledge_change_headers TO synveda_app;
CREATE TABLE public.unproven_context_deliveries (
 tenant_id uuid NOT NULL, context_run_id uuid NOT NULL,
 PRIMARY KEY (tenant_id,context_run_id),
 FOREIGN KEY (tenant_id,context_run_id) REFERENCES public.session_context_runs (tenant_id,id) ON DELETE CASCADE
);
ALTER TABLE public.unproven_context_deliveries ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.unproven_context_deliveries FORCE ROW LEVEL SECURITY;
CREATE POLICY unproven_context_delivery_tenant ON public.unproven_context_deliveries
 USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
GRANT SELECT ON public.unproven_context_deliveries TO synveda_app;
CREATE TABLE public.source_suppressions (
    tenant_id uuid NOT NULL, event_id uuid NOT NULL, session_id uuid NOT NULL,
    scope_id uuid NOT NULL, client_event_id_digest text NOT NULL,
    source_payload_hash text NOT NULL, payload_hash text NOT NULL,
    change_id uuid NOT NULL, forget_change_id uuid, actor_hash text NOT NULL,
    reason_hash text NOT NULL, removed_at timestamptz DEFAULT clock_timestamp() NOT NULL,
    PRIMARY KEY (tenant_id, event_id),
    FOREIGN KEY (tenant_id, event_id) REFERENCES public.source_event_addresses (tenant_id,event_id),
    FOREIGN KEY (tenant_id, change_id) REFERENCES public.knowledge_changes (tenant_id,proposal_id),
    FOREIGN KEY (tenant_id, forget_change_id) REFERENCES public.knowledge_changes (tenant_id,proposal_id),
    CHECK (client_event_id_digest ~ '^[0-9a-f]{64}$'),
    CHECK (source_payload_hash ~ '^[0-9a-f]{64}$'), CHECK (payload_hash ~ '^[0-9a-f]{64}$'),
    CHECK (actor_hash ~ '^[0-9a-f]{64}$'), CHECK (reason_hash ~ '^[0-9a-f]{64}$')
);
CREATE INDEX source_suppression_alias ON public.source_suppressions
    (tenant_id, session_id, client_event_id_digest, source_payload_hash);
CREATE TABLE public.source_cleanup_targets (
    tenant_id uuid NOT NULL, operation_id uuid NOT NULL, event_id uuid NOT NULL,
    change_id uuid NOT NULL, input_hash text NOT NULL,
    phase smallint DEFAULT 0 NOT NULL, cursor_id uuid,
    processed_targets bigint DEFAULT 0 NOT NULL,
    PRIMARY KEY (tenant_id, operation_id),
    FOREIGN KEY (tenant_id, operation_id) REFERENCES public.durable_operations (tenant_id,id),
    FOREIGN KEY (tenant_id,event_id) REFERENCES public.source_suppressions (tenant_id,event_id),
    FOREIGN KEY (tenant_id,change_id) REFERENCES public.knowledge_changes (tenant_id,proposal_id),
    CHECK (input_hash ~ '^[0-9a-f]{64}$'), CHECK (phase BETWEEN 0 AND 8), CHECK (processed_targets >= 0)
);
ALTER TABLE public.durable_operations DROP CONSTRAINT durable_operations_kind_check;
ALTER TABLE public.durable_operations ADD CONSTRAINT durable_operations_kind_check
    CHECK (kind IN ('knowledge_erasure','skill_validation','source_cleanup'));
ALTER TABLE public.durable_operations DROP CONSTRAINT durable_operations_target_check;
ALTER TABLE public.durable_operations ADD CONSTRAINT durable_operations_target_check CHECK (
    (kind='knowledge_erasure' AND proposal_id IS NOT NULL AND knowledge_item_id IS NOT NULL
      AND skill_version_id IS NULL AND requested_by_identity_id IS NULL AND cancel_requested_at IS NULL)
    OR (kind='skill_validation' AND proposal_id IS NULL AND knowledge_item_id IS NULL
      AND skill_version_id IS NOT NULL AND requested_by_identity_id IS NOT NULL)
    OR (kind='source_cleanup' AND proposal_id IS NOT NULL AND knowledge_item_id IS NULL
      AND skill_version_id IS NULL AND requested_by_identity_id IS NULL AND cancel_requested_at IS NULL));
ALTER TABLE public.knowledge_changes DROP CONSTRAINT knowledge_changes_command_check;
ALTER TABLE public.knowledge_changes ADD CONSTRAINT knowledge_changes_command_check
    CHECK (command_kind IN ('create','edit','verify','supersede','merge','archive','restore',
        'forget','resolve_conflict','control_source'));
ALTER TABLE public.knowledge_revisions ADD COLUMN content_erased boolean DEFAULT false NOT NULL;
ALTER TABLE public.session_events ADD COLUMN content_erased boolean DEFAULT false NOT NULL;
ALTER TABLE public.knowledge_sources ADD COLUMN content_erased boolean DEFAULT false NOT NULL;
ALTER TABLE public.knowledge_relations ADD COLUMN content_erased boolean DEFAULT false NOT NULL;
CREATE INDEX source_relation_assertion ON public.knowledge_relations (tenant_id,asserting_revision_id,id);

ALTER TABLE public.source_event_addresses ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.source_event_addresses FORCE ROW LEVEL SECURITY;
CREATE POLICY source_event_address_tenant ON public.source_event_addresses
    USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
ALTER TABLE public.source_dependencies ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.source_dependencies FORCE ROW LEVEL SECURITY;
CREATE POLICY source_dependency_tenant ON public.source_dependencies
    USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
ALTER TABLE public.knowledge_revision_headers ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.knowledge_revision_headers FORCE ROW LEVEL SECURITY;
CREATE POLICY knowledge_revision_header_tenant ON public.knowledge_revision_headers
    USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
ALTER TABLE public.source_suppressions ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.source_suppressions FORCE ROW LEVEL SECURITY;
CREATE POLICY source_suppression_tenant ON public.source_suppressions
    USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
ALTER TABLE public.source_cleanup_targets ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.source_cleanup_targets FORCE ROW LEVEL SECURITY;
CREATE POLICY source_cleanup_target_tenant ON public.source_cleanup_targets
    USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
GRANT SELECT ON public.source_event_addresses, public.source_dependencies,
    public.knowledge_revision_headers TO synveda_app;
GRANT SELECT, INSERT ON public.source_suppressions TO synveda_app;
GRANT UPDATE (forget_change_id) ON public.source_suppressions TO synveda_app;
GRANT SELECT ON public.source_cleanup_targets TO synveda_app;
GRANT INSERT (tenant_id,operation_id,event_id,change_id,input_hash) ON public.source_cleanup_targets TO synveda_app;
CREATE INDEX source_suppression_hash ON public.source_suppressions (tenant_id,session_id,source_payload_hash);
ALTER TABLE public.context_run_delivery_dependencies DROP CONSTRAINT context_delivery_version_check;
ALTER TABLE public.context_run_delivery_dependencies ADD CONSTRAINT context_delivery_version_check CHECK (dependency_version IN (1,2));

-- A cleanup capability is meaningful only under the owner's definer routine,
-- for the exact live attempt, approved change and suppressed root. This helper
-- deliberately remains INVOKER; owner authority cannot be conferred by a read.
CREATE FUNCTION public.synveda_source_cleanup_authorized(t uuid,k text,d uuid) RETURNS boolean
LANGUAGE sql STABLE SET search_path TO 'public','pg_temp' AS $$
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
$$;
CREATE FUNCTION public.synveda_item_cleanup_authorized(t uuid,k text,d uuid) RETURNS boolean
LANGUAGE sql STABLE SET search_path TO 'public','pg_temp' AS $$
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
   ))
$$;
REVOKE ALL ON FUNCTION public.synveda_item_cleanup_authorized(uuid,text,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.synveda_item_cleanup_authorized(uuid,text,uuid) TO synveda_app;
CREATE FUNCTION public.synveda_source_available(t uuid,k text,d uuid) RETURNS boolean
LANGUAGE sql STABLE SET search_path TO 'public','pg_temp' AS $$
 SELECT t=public.synveda_current_tenant() AND (
    NOT EXISTS (SELECT 1 FROM public.source_dependencies dep
      JOIN public.source_suppressions suppression ON suppression.tenant_id=dep.tenant_id
        AND suppression.event_id=dep.source_event_id
      WHERE dep.tenant_id=t AND dep.dependent_kind=k AND dep.dependent_id=d)
    OR public.synveda_source_cleanup_authorized(t,k,d) OR public.synveda_item_cleanup_authorized(t,k,d))
$$;
REVOKE ALL ON FUNCTION public.synveda_source_cleanup_authorized(uuid,text,uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION public.synveda_source_available(uuid,text,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.synveda_source_cleanup_authorized(uuid,text,uuid),
    public.synveda_source_available(uuid,text,uuid) TO synveda_app;

-- Backfill after schemas are created but before eligibility policies are added.
-- Ordinary product roles never insert or rewrite this private projection.
DO $$
DECLARE source_tenant uuid; previous_tenant text=current_setting('synveda.tenant_id',true);
BEGIN
 FOR source_tenant IN SELECT id FROM public.tenants LOOP
  PERFORM set_config('synveda.tenant_id',source_tenant::text,true);
INSERT INTO public.unproven_context_deliveries
 SELECT run.tenant_id,run.id FROM public.session_context_runs run WHERE run.rendered<>'' AND NOT EXISTS (
 SELECT 1 FROM public.context_run_delivery_dependencies proof WHERE proof.tenant_id=run.tenant_id AND proof.context_run_id=run.id);
INSERT INTO public.source_event_addresses
 SELECT e.tenant_id,e.id,e.session_id,s.scope_id,s.workspace_id,s.project_id,e.sequence,
    encode(sha256(convert_to(e.client_event_id,'UTF8')),'hex'),e.source_payload_hash,e.payload_hash
 FROM public.session_events e JOIN public.sessions s ON s.tenant_id=e.tenant_id AND s.id=e.session_id;
INSERT INTO public.knowledge_revision_headers
 SELECT tenant_id,id,knowledge_item_id,revision_number,sensitivity,content_hash,valid_from,valid_to,
    transaction_time, coalesce(metadata->'legal_hold'='true'::jsonb,false), coalesce(metadata->'retention_hold'='true'::jsonb,false)
 FROM public.knowledge_revisions;
INSERT INTO public.knowledge_revision_source_headers SELECT tenant_id,knowledge_revision_id,knowledge_source_id
 FROM public.knowledge_revision_sources;
INSERT INTO public.knowledge_change_headers
 SELECT tenant_id,proposal_id,command_kind,payload_hash,target_item_ids,NULL,NULL,NULL,NULL,NULL,operation_id,applied_at
 FROM public.knowledge_changes;
INSERT INTO public.source_dependencies SELECT tenant_id,'session_event',id,id FROM public.session_events;
INSERT INTO public.source_dependencies
 SELECT e.tenant_id,'session_event',e.id,(x->>'source_event_id')::uuid
 FROM public.session_events e CROSS JOIN LATERAL jsonb_array_elements(coalesce(e.payload->'labelled_excerpts','[]')) x
 WHERE e.event_type='session.checkpoint' AND x->>'source_event_id' IS NOT NULL ON CONFLICT DO NOTHING;
INSERT INTO public.source_dependencies
 SELECT source.tenant_id,'knowledge_source',source.id,dep.source_event_id FROM public.knowledge_sources source
 JOIN public.source_dependencies dep ON dep.tenant_id=source.tenant_id AND dep.dependent_kind='session_event'
 AND dep.dependent_id=source.session_event_id ON CONFLICT DO NOTHING;
INSERT INTO public.source_dependencies
 SELECT link.tenant_id,'knowledge_revision',link.knowledge_revision_id,dep.source_event_id
 FROM public.knowledge_revision_sources link JOIN public.source_dependencies dep
 ON dep.tenant_id=link.tenant_id AND dep.dependent_kind='knowledge_source' AND dep.dependent_id=link.knowledge_source_id
 ON CONFLICT DO NOTHING;
INSERT INTO public.source_dependencies
 SELECT link.tenant_id,'capture_candidate',link.candidate_id,dep.source_event_id
 FROM public.capture_candidate_events link JOIN public.source_dependencies dep
 ON dep.tenant_id=link.tenant_id AND dep.dependent_kind='session_event' AND dep.dependent_id=link.event_id
 ON CONFLICT DO NOTHING;

INSERT INTO public.source_dependencies
 SELECT c.tenant_id,'knowledge_change',c.proposal_id,dep.source_event_id
 FROM public.knowledge_changes c CROSS JOIN LATERAL jsonb_array_elements(coalesce(c.payload->'sources','[]')) x
 JOIN public.source_dependencies dep ON dep.tenant_id=c.tenant_id AND dep.dependent_kind='session_event'
 AND dep.dependent_id=(x->>'session_event_id')::uuid WHERE x->>'session_event_id' IS NOT NULL ON CONFLICT DO NOTHING;
INSERT INTO public.source_dependencies
 SELECT c.tenant_id,'knowledge_change',c.proposal_id,dep.source_event_id
 FROM public.knowledge_changes c JOIN public.source_dependencies dep ON dep.tenant_id=c.tenant_id
 AND dep.dependent_kind='knowledge_revision' AND dep.dependent_id=(c.payload->>'expected_revision_id')::uuid
 WHERE c.command_kind='verify' ON CONFLICT DO NOTHING;
INSERT INTO public.source_dependencies
 SELECT c.tenant_id,'knowledge_change',c.proposal_id,dep.source_event_id
 FROM public.knowledge_changes c CROSS JOIN LATERAL jsonb_array_elements(coalesce(c.payload->'inputs','[]')) x
 JOIN public.source_dependencies dep ON dep.tenant_id=c.tenant_id AND dep.dependent_kind='knowledge_revision'
 AND dep.dependent_id=(x->>'revision_id')::uuid WHERE c.command_kind='merge' ON CONFLICT DO NOTHING;
INSERT INTO public.source_dependencies
 SELECT m.tenant_id,'context_run',m.context_run_id,dep.source_event_id
 FROM public.context_run_delivery_dependencies m CROSS JOIN LATERAL jsonb_array_elements(m.manifest->'knowledge') x
 JOIN public.source_dependencies dep ON dep.tenant_id=m.tenant_id AND dep.dependent_kind='knowledge_revision'
 AND dep.dependent_id=(x->>'revision_id')::uuid ON CONFLICT DO NOTHING;
INSERT INTO public.source_dependencies
 SELECT m.tenant_id,'context_run',m.context_run_id,dep.source_event_id
 FROM public.context_run_delivery_dependencies m CROSS JOIN LATERAL jsonb_array_elements(m.manifest->'candidates') x
 JOIN public.source_dependencies dep ON dep.tenant_id=m.tenant_id AND dep.dependent_kind='capture_candidate'
 AND dep.dependent_id=(x->>'candidate_id')::uuid ON CONFLICT DO NOTHING;
INSERT INTO public.source_dependencies
 SELECT m.tenant_id,'context_run',m.context_run_id,dep.source_event_id
 FROM public.context_run_delivery_dependencies m CROSS JOIN LATERAL jsonb_array_elements(m.manifest->'graph') x
 JOIN public.source_dependencies dep ON dep.tenant_id=m.tenant_id AND dep.dependent_kind='knowledge_revision'
 AND dep.dependent_id=ANY(ARRAY[(x->>'asserting_revision_id')::uuid,(x->>'from_revision_id')::uuid,(x->>'to_revision_id')::uuid]) ON CONFLICT DO NOTHING;
INSERT INTO public.source_dependencies
 SELECT m.tenant_id,'context_run',m.context_run_id,dep.source_event_id
 FROM public.context_run_delivery_dependencies m CROSS JOIN LATERAL jsonb_array_elements(m.manifest->'tail') x
 JOIN public.source_dependencies dep ON dep.tenant_id=m.tenant_id AND dep.dependent_kind='session_event'
 AND dep.dependent_id=(x->>'event_id')::uuid ON CONFLICT DO NOTHING;

INSERT INTO public.source_dependencies
 WITH RECURSIVE checkpoints AS (
   SELECT m.tenant_id,m.context_run_id,(m.manifest->'checkpoint'->>'event_id')::uuid event_id,1 depth
   FROM public.context_run_delivery_dependencies m WHERE m.manifest->'checkpoint'->>'event_id' IS NOT NULL
   UNION ALL
   SELECT c.tenant_id,c.context_run_id,(e.payload->>'previous_checkpoint_event_id')::uuid,c.depth+1
   FROM checkpoints c JOIN public.session_events e ON e.tenant_id=c.tenant_id AND e.id=c.event_id
   WHERE c.depth<4 AND e.payload->>'previous_checkpoint_event_id' IS NOT NULL
 ) SELECT c.tenant_id,'context_run',c.context_run_id,dep.source_event_id FROM checkpoints c
 JOIN public.source_dependencies dep ON dep.tenant_id=c.tenant_id AND dep.dependent_kind='session_event'
 AND dep.dependent_id=c.event_id ON CONFLICT DO NOTHING;

 END LOOP;
 PERFORM set_config('synveda.tenant_id',coalesce(previous_tenant,''),true);
END $$;

CREATE FUNCTION public.synveda_source_admissible(t uuid,k text,d uuid) RETURNS boolean
LANGUAGE sql STABLE SET search_path TO 'public','pg_temp' AS $$
 SELECT t=public.synveda_current_tenant() AND CASE k
  WHEN 'session_event' THEN EXISTS(SELECT 1 FROM public.session_events WHERE tenant_id=t AND id=d AND NOT content_erased)
  WHEN 'knowledge_revision' THEN EXISTS(SELECT 1 FROM public.knowledge_revisions WHERE tenant_id=t AND id=d AND NOT content_erased)
  WHEN 'knowledge_source' THEN EXISTS(SELECT 1 FROM public.knowledge_sources WHERE tenant_id=t AND id=d AND NOT content_erased)
  WHEN 'capture_candidate' THEN EXISTS(SELECT 1 FROM public.capture_candidates WHERE tenant_id=t AND id=d AND NOT content_erased)
  WHEN 'knowledge_change' THEN EXISTS(SELECT 1 FROM public.knowledge_changes WHERE tenant_id=t AND proposal_id=d AND payload IS NOT NULL)
  WHEN 'context_run' THEN EXISTS(SELECT 1 FROM public.session_context_runs WHERE tenant_id=t AND id=d)
  ELSE false END AND NOT EXISTS (
   SELECT 1 FROM public.source_dependencies dep JOIN public.source_suppressions suppression
   ON suppression.tenant_id=dep.tenant_id AND suppression.event_id=dep.source_event_id
   WHERE dep.tenant_id=t AND dep.dependent_kind=k AND dep.dependent_id=d)
$$;
REVOKE ALL ON FUNCTION public.synveda_source_admissible(uuid,text,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.synveda_source_admissible(uuid,text,uuid) TO synveda_app;

CREATE FUNCTION public.synveda_source_projection() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path TO 'public','pg_temp' AS $$
DECLARE source_id uuid; parent_id uuid; kind text; roots jsonb; control_effect boolean=false;
BEGIN
 IF NEW.tenant_id IS DISTINCT FROM public.synveda_current_tenant() THEN
   RAISE EXCEPTION 'source projection tenant mismatch' USING ERRCODE='23514'; END IF;
 IF TG_TABLE_NAME='session_events' THEN
   IF EXISTS (SELECT 1 FROM public.source_suppressions s WHERE s.tenant_id=NEW.tenant_id
      AND s.session_id=NEW.session_id AND (s.client_event_id_digest=encode(sha256(convert_to(NEW.client_event_id,'UTF8')),'hex')
        OR s.source_payload_hash=NEW.source_payload_hash)) THEN
      RAISE EXCEPTION 'source suppressed' USING ERRCODE='23514'; END IF;
   INSERT INTO public.source_event_addresses SELECT NEW.tenant_id,NEW.id,NEW.session_id,s.scope_id,s.workspace_id,s.project_id,NEW.sequence,
       encode(sha256(convert_to(NEW.client_event_id,'UTF8')),'hex'),NEW.source_payload_hash,NEW.payload_hash
     FROM public.sessions s WHERE s.tenant_id=NEW.tenant_id AND s.id=NEW.session_id;
   INSERT INTO public.source_dependencies VALUES (NEW.tenant_id,'session_event',NEW.id,NEW.id);
   roots=CASE WHEN NEW.event_type='session.checkpoint' THEN coalesce(NEW.payload->'labelled_excerpts','[]') ELSE '[]'::jsonb END; kind='session_event'; parent_id=NEW.id;
 ELSIF TG_TABLE_NAME='knowledge_revisions' THEN
   INSERT INTO public.knowledge_revision_headers VALUES (NEW.tenant_id,NEW.id,NEW.knowledge_item_id,
      NEW.revision_number,NEW.sensitivity,NEW.content_hash,NEW.valid_from,NEW.valid_to,NEW.transaction_time,
      coalesce(NEW.metadata->'legal_hold'='true'::jsonb,false), coalesce(NEW.metadata->'retention_hold'='true'::jsonb,false));
   RETURN NEW;
 ELSIF TG_TABLE_NAME='knowledge_sources' THEN
   kind='knowledge_source'; parent_id=NEW.id; source_id=NEW.session_event_id;
 ELSIF TG_TABLE_NAME='knowledge_revision_sources' THEN
   INSERT INTO public.knowledge_revision_source_headers VALUES (NEW.tenant_id,NEW.knowledge_revision_id,NEW.knowledge_source_id);
   IF NOT public.synveda_source_admissible(NEW.tenant_id,'knowledge_source',NEW.knowledge_source_id) THEN
      RAISE EXCEPTION 'source unavailable' USING ERRCODE='23514'; END IF;
   INSERT INTO public.source_dependencies SELECT NEW.tenant_id,'knowledge_revision',NEW.knowledge_revision_id,dep.source_event_id
      FROM public.source_dependencies dep WHERE dep.tenant_id=NEW.tenant_id AND dep.dependent_kind='knowledge_source'
        AND dep.dependent_id=NEW.knowledge_source_id ON CONFLICT DO NOTHING;
   IF (SELECT count(*) FROM (SELECT 1 FROM public.source_dependencies WHERE tenant_id=NEW.tenant_id
     AND dependent_kind='knowledge_revision' AND dependent_id=NEW.knowledge_revision_id LIMIT 4097) bounded)>4096 THEN
     RAISE EXCEPTION 'source roots bound exhausted' USING ERRCODE='23514'; END IF;
   RETURN NEW;
 ELSIF TG_TABLE_NAME='capture_candidate_events' THEN
   kind='capture_candidate'; parent_id=NEW.candidate_id; source_id=NEW.event_id;
 ELSIF TG_TABLE_NAME='knowledge_changes' THEN
   IF TG_OP='UPDATE' THEN
     UPDATE public.knowledge_change_headers SET operation_id=NEW.operation_id,applied_at=NEW.applied_at
       WHERE tenant_id=NEW.tenant_id AND proposal_id=NEW.proposal_id AND applied_at IS NULL AND NEW.applied_at IS NOT NULL;
     RETURN NEW;
   END IF;
   INSERT INTO public.knowledge_change_headers VALUES (NEW.tenant_id,NEW.proposal_id,NEW.command_kind,NEW.payload_hash,
     NEW.target_item_ids,(NEW.payload->>'event_id')::uuid,(NEW.payload->>'session_id')::uuid,(NEW.payload->>'scope_id')::uuid,
     NEW.payload->>'expected_payload_hash',NEW.payload->>'mode',NEW.operation_id,NEW.applied_at);
   kind='knowledge_change'; parent_id=NEW.proposal_id; control_effect=NEW.command_kind='control_source';
   roots=coalesce(NEW.payload->'sources','[]');
   IF NEW.command_kind='control_source' THEN
      source_id=(NEW.payload->>'event_id')::uuid;
   ELSIF NEW.command_kind='verify' THEN
      INSERT INTO public.source_dependencies SELECT NEW.tenant_id,kind,parent_id,dep.source_event_id
        FROM public.source_dependencies dep WHERE dep.tenant_id=NEW.tenant_id AND dep.dependent_kind='knowledge_revision'
         AND dep.dependent_id=(NEW.payload->>'expected_revision_id')::uuid ON CONFLICT DO NOTHING;
   ELSIF NEW.command_kind='merge' THEN
      INSERT INTO public.source_dependencies SELECT NEW.tenant_id,kind,parent_id,dep.source_event_id
        FROM jsonb_array_elements(NEW.payload->'inputs') x JOIN public.source_dependencies dep
         ON dep.tenant_id=NEW.tenant_id AND dep.dependent_kind='knowledge_revision' AND dep.dependent_id=(x->>'revision_id')::uuid
        ON CONFLICT DO NOTHING;
   END IF;
   IF NEW.command_kind<>'control_source' AND NOT public.synveda_source_admissible(NEW.tenant_id,kind,parent_id) THEN
     RAISE EXCEPTION 'source unavailable' USING ERRCODE='23514'; END IF;
 ELSIF TG_TABLE_NAME='context_run_delivery_dependencies' THEN
   kind='context_run'; parent_id=NEW.context_run_id;
   FOR source_id IN SELECT (x->>'revision_id')::uuid FROM jsonb_array_elements(NEW.manifest->'knowledge') x
      UNION SELECT v::uuid FROM jsonb_array_elements(NEW.manifest->'graph') x
       CROSS JOIN LATERAL (VALUES(x->>'asserting_revision_id'),(x->>'from_revision_id'),(x->>'to_revision_id')) revisions(v)
       WHERE v IS NOT NULL LOOP
     IF NOT public.synveda_source_admissible(NEW.tenant_id,'knowledge_revision',source_id) THEN
       RAISE EXCEPTION 'source unavailable' USING ERRCODE='23514'; END IF;
   END LOOP;
   FOR source_id IN SELECT (x->>'candidate_id')::uuid FROM jsonb_array_elements(NEW.manifest->'candidates') x LOOP
     IF NOT public.synveda_source_admissible(NEW.tenant_id,'capture_candidate',source_id) THEN
       RAISE EXCEPTION 'source unavailable' USING ERRCODE='23514'; END IF;
   END LOOP;

   INSERT INTO public.source_dependencies SELECT NEW.tenant_id,kind,parent_id,dep.source_event_id
     FROM jsonb_array_elements(NEW.manifest->'knowledge') x JOIN public.source_dependencies dep
      ON dep.tenant_id=NEW.tenant_id AND dep.dependent_kind='knowledge_revision' AND dep.dependent_id=(x->>'revision_id')::uuid
     ON CONFLICT DO NOTHING;
   INSERT INTO public.source_dependencies SELECT NEW.tenant_id,kind,parent_id,dep.source_event_id
     FROM jsonb_array_elements(NEW.manifest->'candidates') x JOIN public.source_dependencies dep
      ON dep.tenant_id=NEW.tenant_id AND dep.dependent_kind='capture_candidate' AND dep.dependent_id=(x->>'candidate_id')::uuid
     ON CONFLICT DO NOTHING;
   INSERT INTO public.source_dependencies SELECT NEW.tenant_id,kind,parent_id,dep.source_event_id
     FROM jsonb_array_elements(NEW.manifest->'graph') x JOIN public.source_dependencies dep
      ON dep.tenant_id=NEW.tenant_id AND dep.dependent_kind='knowledge_revision'
       AND dep.dependent_id=ANY(ARRAY[(x->>'asserting_revision_id')::uuid,(x->>'from_revision_id')::uuid,(x->>'to_revision_id')::uuid]) ON CONFLICT DO NOTHING;
   source_id=NULL;
   roots=coalesce(NEW.manifest->'restart_source_event_ids','[]');
 END IF;
 IF source_id IS NOT NULL THEN
   -- A control effect may name an already withdrawn source to request forgetting.
   IF TG_TABLE_NAME<>'knowledge_changes' AND NOT public.synveda_source_admissible(NEW.tenant_id,'session_event',source_id) THEN
     RAISE EXCEPTION 'source unavailable' USING ERRCODE='23514'; END IF;
   INSERT INTO public.source_dependencies SELECT NEW.tenant_id,kind,parent_id,dep.source_event_id
     FROM public.source_dependencies dep WHERE dep.tenant_id=NEW.tenant_id AND dep.dependent_kind='session_event'
       AND dep.dependent_id=source_id ON CONFLICT DO NOTHING;
 END IF;
 IF roots IS NOT NULL THEN
   IF jsonb_array_length(roots)>256 THEN RAISE EXCEPTION 'source roots bound exhausted' USING ERRCODE='23514'; END IF;
   FOR source_id IN SELECT coalesce(x->>'source_event_id',x->>'session_event_id',x#>>'{}')::uuid
      FROM jsonb_array_elements(roots) x
      WHERE jsonb_typeof(x)='string' OR x->>'source_event_id' IS NOT NULL OR x->>'session_event_id' IS NOT NULL LOOP
     IF TG_TABLE_NAME='session_events' THEN
       IF NOT EXISTS (SELECT 1 FROM public.source_event_addresses address
         WHERE address.tenant_id=NEW.tenant_id AND address.event_id=source_id AND address.session_id=NEW.session_id) THEN
         RAISE EXCEPTION 'checkpoint source Session mismatch' USING ERRCODE='23514'; END IF;
     END IF;
     IF NOT public.synveda_source_admissible(NEW.tenant_id,'session_event',source_id) THEN
       RAISE EXCEPTION 'source unavailable' USING ERRCODE='23514'; END IF;
     INSERT INTO public.source_dependencies SELECT NEW.tenant_id,kind,parent_id,dep.source_event_id
       FROM public.source_dependencies dep WHERE dep.tenant_id=NEW.tenant_id AND dep.dependent_kind='session_event'
         AND dep.dependent_id=source_id ON CONFLICT DO NOTHING;
   END LOOP;
 END IF;
 IF kind IS NOT NULL THEN
   IF NOT control_effect AND NOT public.synveda_source_admissible(NEW.tenant_id,kind,parent_id) THEN
     RAISE EXCEPTION 'source unavailable' USING ERRCODE='23514'; END IF;
   IF (SELECT count(*) FROM (SELECT 1 FROM public.source_dependencies WHERE tenant_id=NEW.tenant_id
       AND dependent_kind=kind AND dependent_id=parent_id LIMIT 4097) bounded)>4096 THEN
     RAISE EXCEPTION 'source roots bound exhausted' USING ERRCODE='23514'; END IF;
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION public.synveda_source_projection() FROM PUBLIC;
CREATE TRIGGER session_source_projection AFTER INSERT ON public.session_events
 FOR EACH ROW EXECUTE FUNCTION public.synveda_source_projection();
CREATE TRIGGER revision_source_header AFTER INSERT ON public.knowledge_revisions
 FOR EACH ROW EXECUTE FUNCTION public.synveda_source_projection();
CREATE TRIGGER source_descriptor_projection AFTER INSERT ON public.knowledge_sources
 FOR EACH ROW EXECUTE FUNCTION public.synveda_source_projection();
CREATE TRIGGER revision_source_projection AFTER INSERT ON public.knowledge_revision_sources
 FOR EACH ROW EXECUTE FUNCTION public.synveda_source_projection();
CREATE TRIGGER capture_source_projection AFTER INSERT ON public.capture_candidate_events
 FOR EACH ROW EXECUTE FUNCTION public.synveda_source_projection();
CREATE TRIGGER change_source_projection AFTER INSERT OR UPDATE ON public.knowledge_changes
 FOR EACH ROW EXECUTE FUNCTION public.synveda_source_projection();
CREATE TRIGGER delivery_source_projection AFTER INSERT ON public.context_run_delivery_dependencies
 FOR EACH ROW EXECUTE FUNCTION public.synveda_source_projection();

CREATE POLICY session_source_eligibility ON public.session_events AS RESTRICTIVE FOR SELECT
 USING (NOT content_erased AND public.synveda_source_available(tenant_id,'session_event',id)
   OR public.synveda_source_cleanup_authorized(tenant_id,'session_event',id));
CREATE POLICY revision_source_eligibility ON public.knowledge_revisions AS RESTRICTIVE FOR SELECT
 USING (NOT content_erased AND public.synveda_source_available(tenant_id,'knowledge_revision',id)
   OR public.synveda_source_cleanup_authorized(tenant_id,'knowledge_revision',id)
   OR public.synveda_item_cleanup_authorized(tenant_id,'knowledge_revision',id));
CREATE POLICY descriptor_source_eligibility ON public.knowledge_sources AS RESTRICTIVE FOR SELECT
 USING (NOT content_erased AND public.synveda_source_available(tenant_id,'knowledge_source',id)
   OR public.synveda_source_cleanup_authorized(tenant_id,'knowledge_source',id)
   OR public.synveda_item_cleanup_authorized(tenant_id,'knowledge_source',id));
CREATE POLICY candidate_source_eligibility ON public.capture_candidates AS RESTRICTIVE FOR SELECT
 USING (public.synveda_source_available(tenant_id,'capture_candidate',id));
CREATE POLICY decision_source_eligibility ON public.capture_candidate_decisions AS RESTRICTIVE FOR SELECT
 USING (public.synveda_source_available(tenant_id,'capture_candidate',candidate_id));
CREATE POLICY change_source_eligibility ON public.knowledge_changes AS RESTRICTIVE FOR SELECT
 USING (command_kind='control_source' OR public.synveda_source_available(tenant_id,'knowledge_change',proposal_id));
CREATE POLICY context_source_eligibility ON public.session_context_runs AS RESTRICTIVE FOR SELECT
 USING ((NOT EXISTS (SELECT 1 FROM public.unproven_context_deliveries unproven
   WHERE unproven.tenant_id=session_context_runs.tenant_id AND unproven.context_run_id=session_context_runs.id)
   OR rendered='' OR public.synveda_item_cleanup_authorized(tenant_id,'context_run',id))
   AND public.synveda_source_available(tenant_id,'context_run',id));
CREATE POLICY relation_source_eligibility ON public.knowledge_relations AS RESTRICTIVE FOR SELECT
 USING (public.synveda_source_available(tenant_id,'knowledge_revision',asserting_revision_id)
   OR public.synveda_item_cleanup_authorized(tenant_id,'knowledge_item',source_item_id)
   OR public.synveda_item_cleanup_authorized(tenant_id,'knowledge_item',target_item_id));
CREATE POLICY embedding_source_eligibility ON public.knowledge_revision_embeddings AS RESTRICTIVE FOR SELECT
 USING (public.synveda_source_available(tenant_id,'knowledge_revision',knowledge_revision_id));

-- One-way erasure only: original identifiers, hashes, ordering, authority and
-- temporal fields remain immutable. The app receives no UPDATE privilege.
CREATE FUNCTION public.synveda_source_content_erasure() RETURNS trigger
LANGUAGE plpgsql SET search_path TO 'public','pg_temp' AS $$
DECLARE k text;
BEGIN
 k=CASE TG_TABLE_NAME WHEN 'knowledge_revisions' THEN 'knowledge_revision'
      WHEN 'session_events' THEN 'session_event' WHEN 'knowledge_relations' THEN 'knowledge_revision' ELSE 'knowledge_source' END;
 IF NOT public.synveda_source_cleanup_authorized(OLD.tenant_id,k,CASE WHEN TG_TABLE_NAME='knowledge_relations' THEN (to_jsonb(OLD)->>'asserting_revision_id')::uuid ELSE OLD.id END)
    OR OLD.content_erased OR NOT NEW.content_erased THEN
    RAISE EXCEPTION 'source erasure requires a live approved attempt' USING ERRCODE='23514'; END IF;
 IF TG_TABLE_NAME='knowledge_revisions' THEN
   IF NEW.title<>'[erased]' OR NEW.body_markdown<>'[erased]' OR NEW.summary<>'[erased]'
      OR NEW.tags<>'{}' OR NEW.metadata<>'{}' OR NEW.verification_metadata<>'{}'
      OR (to_jsonb(NEW)-ARRAY['title','body_markdown','summary','tags','metadata','verification_metadata','search_document','content_erased'])
       IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['title','body_markdown','summary','tags','metadata','verification_metadata','search_document','content_erased']) THEN
      RAISE EXCEPTION 'revision erasure is one-way content clearing' USING ERRCODE='23514'; END IF;
 ELSIF TG_TABLE_NAME='session_events' THEN
   IF NEW.payload<>'{}' OR NEW.redactions IS NOT NULL
      OR (to_jsonb(NEW)-ARRAY['payload','redactions','content_erased']) IS DISTINCT FROM
         (to_jsonb(OLD)-ARRAY['payload','redactions','content_erased']) THEN
      RAISE EXCEPTION 'event erasure is one-way content clearing' USING ERRCODE='23514'; END IF;
 ELSIF TG_TABLE_NAME='knowledge_relations' THEN
   IF NEW.metadata<>'{}' OR (to_jsonb(NEW)-ARRAY['metadata','content_erased']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['metadata','content_erased']) THEN
    RAISE EXCEPTION 'relation erasure is one-way content clearing' USING ERRCODE='23514'; END IF;
 ELSE
   IF NEW.metadata<>'{}' OR NEW.locator IS NOT NULL OR NEW.source_revision IS NOT NULL
      OR (to_jsonb(NEW)-ARRAY['metadata','locator','source_revision','content_erased']) IS DISTINCT FROM
         (to_jsonb(OLD)-ARRAY['metadata','locator','source_revision','content_erased']) THEN
      RAISE EXCEPTION 'descriptor erasure is one-way content clearing' USING ERRCODE='23514'; END IF;
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION public.synveda_source_content_erasure() FROM PUBLIC;
DROP TRIGGER knowledge_revisions_append_only ON public.knowledge_revisions;
CREATE TRIGGER knowledge_revisions_append_only BEFORE DELETE OR TRUNCATE ON public.knowledge_revisions
 FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();
CREATE TRIGGER knowledge_revisions_source_erasure BEFORE UPDATE ON public.knowledge_revisions
 FOR EACH ROW EXECUTE FUNCTION public.synveda_source_content_erasure();
DROP TRIGGER knowledge_sources_append_only ON public.knowledge_sources;
CREATE TRIGGER knowledge_sources_append_only BEFORE DELETE OR TRUNCATE ON public.knowledge_sources
 FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();
CREATE TRIGGER knowledge_sources_source_erasure BEFORE UPDATE ON public.knowledge_sources
 FOR EACH ROW EXECUTE FUNCTION public.synveda_source_content_erasure();
CREATE TRIGGER session_events_source_erasure BEFORE UPDATE ON public.session_events
 FOR EACH ROW EXECUTE FUNCTION public.synveda_source_content_erasure();
DROP TRIGGER knowledge_relations_append_only ON public.knowledge_relations;
CREATE TRIGGER knowledge_relations_append_only BEFORE DELETE OR TRUNCATE ON public.knowledge_relations
 FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();
CREATE TRIGGER knowledge_relations_source_erasure BEFORE UPDATE ON public.knowledge_relations
 FOR EACH ROW EXECUTE FUNCTION public.synveda_source_content_erasure();

CREATE FUNCTION public.synveda_source_cleanup_chunk(t uuid,o uuid,w text,a integer,n integer)
 RETURNS TABLE(progress smallint,done boolean,erased bigint)
LANGUAGE plpgsql SECURITY DEFINER SET search_path TO 'public','pg_temp' AS $$
DECLARE target public.source_cleanup_targets%ROWTYPE; op public.durable_operations%ROWTYPE;
 kind text; ids uuid[]; last_id uuid; affected bigint=0; changed bigint=0;
BEGIN
 IF t IS DISTINCT FROM public.synveda_current_tenant() OR n IS NULL OR n NOT BETWEEN 1 AND 100 THEN
   RAISE EXCEPTION 'invalid source cleanup boundary' USING ERRCODE='23514'; END IF;
 SELECT * INTO op FROM public.durable_operations WHERE tenant_id=t AND id=o FOR UPDATE;
 SELECT * INTO target FROM public.source_cleanup_targets WHERE tenant_id=t AND operation_id=o FOR UPDATE;
 IF op.kind IS DISTINCT FROM 'source_cleanup' OR op.state IS DISTINCT FROM 'running'
    OR op.lease_owner IS DISTINCT FROM w OR op.attempts IS DISTINCT FROM a
    OR op.lease_expires_at<=clock_timestamp() OR target.operation_id IS NULL
    OR NOT EXISTS (SELECT 1 FROM public.operation_attempts WHERE tenant_id=t AND operation_id=o
       AND attempt_number=a AND worker_id=w AND state='running') THEN
   RAISE EXCEPTION 'source cleanup lost its attempt' USING ERRCODE='23514'; END IF;
 PERFORM set_config('synveda.source_cleanup_operation',o::text,true);
 PERFORM set_config('synveda.source_cleanup_attempt',a::text,true);
 IF NOT public.synveda_source_cleanup_authorized(t,'session_event',target.event_id) THEN
   RAISE EXCEPTION 'source cleanup lacks an applied decision' USING ERRCODE='23514'; END IF;
 UPDATE public.durable_operations SET lease_expires_at=clock_timestamp()+interval '30 seconds' WHERE tenant_id=t AND id=o;
 kind=CASE target.phase WHEN 0 THEN 'knowledge_revision' WHEN 1 THEN 'context_run'
      WHEN 2 THEN 'capture_candidate' WHEN 3 THEN 'knowledge_change'
      WHEN 4 THEN 'knowledge_revision' WHEN 5 THEN 'knowledge_revision' WHEN 6 THEN 'knowledge_revision' WHEN 7 THEN 'knowledge_source' ELSE 'session_event' END;
 SELECT array_agg(dependent_id ORDER BY dependent_id),max(dependent_id::text)::uuid INTO ids,last_id
   FROM (SELECT dependent_id FROM public.source_dependencies WHERE tenant_id=t AND source_event_id=target.event_id
     AND dependent_kind=kind AND (target.cursor_id IS NULL OR dependent_id>target.cursor_id)
     ORDER BY dependent_id LIMIT n) page;
 IF target.phase=0 THEN
   IF EXISTS (SELECT 1 FROM public.unproven_context_deliveries WHERE tenant_id=t) THEN
     UPDATE public.durable_operations SET state='blocked',last_error_code='lineage_unproven',completed_at=clock_timestamp(),
       updated_at=clock_timestamp(),lease_owner=NULL,lease_expires_at=NULL,next_attempt_at=NULL WHERE tenant_id=t AND id=o;
     UPDATE public.operation_attempts SET state='cancelled',safe_error_code='lineage_unproven',completed_at=clock_timestamp()
       WHERE tenant_id=t AND operation_id=o AND attempt_number=a;
     UPDATE public.operation_outbox SET state='completed',lease_owner=NULL,lease_expires_at=NULL,next_dispatch_at=NULL,
       completed_at=clock_timestamp(),updated_at=clock_timestamp() WHERE tenant_id=t AND operation_id=o;
     PERFORM set_config('synveda.source_cleanup_operation','',true);
     PERFORM set_config('synveda.source_cleanup_attempt','',true);
     RETURN QUERY SELECT op.progress_percent,true,0::bigint; RETURN;
   END IF;
   IF EXISTS (SELECT 1 FROM public.knowledge_revision_headers WHERE tenant_id=t AND revision_id=ANY(ids) AND (legal_hold OR retention_hold)) THEN
     UPDATE public.durable_operations SET state='blocked',last_error_code='retention_hold',completed_at=clock_timestamp(),
       updated_at=clock_timestamp(),lease_owner=NULL,lease_expires_at=NULL,next_attempt_at=NULL WHERE tenant_id=t AND id=o;
     UPDATE public.operation_attempts SET state='cancelled',safe_error_code='retention_hold',completed_at=clock_timestamp()
       WHERE tenant_id=t AND operation_id=o AND attempt_number=a;
     UPDATE public.operation_outbox SET state='completed',lease_owner=NULL,lease_expires_at=NULL,next_dispatch_at=NULL,
       completed_at=clock_timestamp(),updated_at=clock_timestamp() WHERE tenant_id=t AND operation_id=o;
     PERFORM set_config('synveda.source_cleanup_operation','',true);
     PERFORM set_config('synveda.source_cleanup_attempt','',true);
     RETURN QUERY SELECT op.progress_percent,true,0::bigint; RETURN;
   END IF;
 ELSIF target.phase=1 THEN
   PERFORM set_config('synveda.knowledge_erasure','on',true);
   UPDATE public.session_context_runs SET rendered='' WHERE tenant_id=t AND id=ANY(ids) AND rendered<>'';
   GET DIAGNOSTICS changed=ROW_COUNT;
   PERFORM set_config('synveda.knowledge_erasure','off',true);
 ELSIF target.phase=2 THEN
   PERFORM set_config('synveda.knowledge_erasure','on',true);
   UPDATE public.capture_candidate_decisions SET payload=NULL WHERE tenant_id=t AND candidate_id=ANY(ids) AND payload IS NOT NULL;
   UPDATE public.capture_candidates SET content_erased=true,title='',body_markdown='',summary='',decision_reason=NULL,tags='{}',
      metadata='{}',verification_metadata='{}' WHERE tenant_id=t AND id=ANY(ids) AND NOT content_erased;
   PERFORM set_config('synveda.knowledge_erasure','off',true);
 ELSIF target.phase=3 THEN
   UPDATE public.vedaflow_proposals p SET state='rejected',closed_at=clock_timestamp(),updated_at=clock_timestamp(),
     closed_by=(SELECT closed_by FROM public.vedaflow_proposals WHERE tenant_id=t AND id=target.change_id),close_reason='source_unavailable'
     WHERE p.tenant_id=t AND p.id=ANY(ids) AND p.state='open';
   PERFORM set_config('synveda.knowledge_erasure','on',true);
   UPDATE public.knowledge_changes SET payload=NULL WHERE tenant_id=t AND proposal_id=ANY(ids) AND payload IS NOT NULL;
   PERFORM set_config('synveda.knowledge_erasure','off',true);
 ELSIF target.phase=4 THEN
   UPDATE public.knowledge_revisions SET content_erased=true,title='[erased]',body_markdown='[erased]',summary='[erased]',
      tags='{}',metadata='{}',verification_metadata='{}' WHERE tenant_id=t AND id=ANY(ids) AND NOT content_erased;
 ELSIF target.phase=5 THEN
   DELETE FROM public.knowledge_revision_embeddings embedding WHERE embedding.ctid IN (
     SELECT e.ctid FROM public.knowledge_revision_embeddings e JOIN public.source_dependencies dep
       ON dep.tenant_id=e.tenant_id AND dep.dependent_kind='knowledge_revision' AND dep.dependent_id=e.knowledge_revision_id
     WHERE e.tenant_id=t AND dep.source_event_id=target.event_id
     ORDER BY e.knowledge_revision_id,e.model LIMIT n FOR UPDATE OF e);
   GET DIAGNOSTICS changed=ROW_COUNT;
   ids=CASE WHEN changed=n THEN array_fill(target.event_id,ARRAY[n]) ELSE '{}'::uuid[] END;
   last_id=NULL;
 ELSIF target.phase=6 THEN
   SELECT array_agg(id ORDER BY id),max(id::text)::uuid INTO ids,last_id FROM (
     SELECT r.id FROM public.knowledge_relations r JOIN public.source_dependencies dep
       ON dep.tenant_id=r.tenant_id AND dep.dependent_kind='knowledge_revision' AND dep.dependent_id=r.asserting_revision_id
     WHERE r.tenant_id=t AND dep.source_event_id=target.event_id AND NOT r.content_erased
       AND (target.cursor_id IS NULL OR r.id>target.cursor_id) ORDER BY r.id LIMIT n) page;
   UPDATE public.knowledge_relations SET content_erased=true,metadata='{}' WHERE tenant_id=t AND id=ANY(ids) AND NOT content_erased;
 ELSIF target.phase=7 THEN
   UPDATE public.knowledge_sources SET content_erased=true,locator=NULL,source_revision=NULL,metadata='{}'
     WHERE tenant_id=t AND id=ANY(ids) AND NOT content_erased;
 ELSE
   UPDATE public.session_events SET content_erased=true,payload='{}',redactions=NULL
     WHERE tenant_id=t AND id=ANY(ids) AND NOT content_erased;
 END IF;
 affected=CASE WHEN target.phase=0 THEN 0 ELSE coalesce(cardinality(ids),0) END;
 IF coalesce(cardinality(ids),0)<n THEN
   IF target.phase<8 THEN target.phase=target.phase+1; target.cursor_id=NULL;
   ELSE target.phase=8; target.cursor_id=last_id; END IF;
 ELSE target.cursor_id=last_id; END IF;
 UPDATE public.source_cleanup_targets SET phase=target.phase,cursor_id=target.cursor_id,processed_targets=processed_targets+affected
    WHERE tenant_id=t AND operation_id=o;
 done=(kind='session_event' AND coalesce(cardinality(ids),0)<n);
 progress=CASE WHEN done THEN 100 ELSE LEAST(99,target.phase*11)::smallint END;
 IF NOT public.synveda_source_cleanup_authorized(t,'session_event',target.event_id) THEN RAISE EXCEPTION 'source cleanup page expired' USING ERRCODE='23514'; END IF;
 UPDATE public.durable_operations SET progress_percent=progress,updated_at=clock_timestamp(),
    state=CASE WHEN done THEN 'succeeded' ELSE 'running' END,
    completed_at=CASE WHEN done THEN clock_timestamp() ELSE NULL END,
    lease_owner=CASE WHEN done THEN NULL ELSE w END,
    lease_expires_at=CASE WHEN done THEN NULL ELSE clock_timestamp()+interval '30 seconds' END
    WHERE tenant_id=t AND id=o;
 IF done THEN
   UPDATE public.operation_attempts SET state='succeeded',completed_at=clock_timestamp()
     WHERE tenant_id=t AND operation_id=o AND attempt_number=a;
   UPDATE public.operation_outbox SET state='completed',lease_owner=NULL,lease_expires_at=NULL,
     completed_at=clock_timestamp(),updated_at=clock_timestamp(),next_dispatch_at=NULL WHERE tenant_id=t AND operation_id=o;
 END IF;
 PERFORM set_config('synveda.source_cleanup_operation','',true);
 PERFORM set_config('synveda.source_cleanup_attempt','',true);
 RETURN QUERY SELECT progress,done,affected;
END $$;
REVOKE ALL ON FUNCTION public.synveda_source_cleanup_chunk(uuid,uuid,text,integer,integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.synveda_source_cleanup_chunk(uuid,uuid,text,integer,integer) TO synveda_app;

CREATE FUNCTION public.synveda_source_control_binding() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path TO 'public','pg_temp' AS $$
DECLARE h public.knowledge_change_headers%ROWTYPE; addr public.source_event_addresses%ROWTYPE;
BEGIN
 IF NEW.tenant_id IS DISTINCT FROM public.synveda_current_tenant() THEN
  RAISE EXCEPTION 'source binding tenant mismatch' USING ERRCODE='23514'; END IF;
 SELECT * INTO h FROM public.knowledge_change_headers WHERE tenant_id=NEW.tenant_id AND proposal_id=NEW.change_id;
 SELECT * INTO addr FROM public.source_event_addresses WHERE tenant_id=NEW.tenant_id AND event_id=NEW.event_id;
 IF h.command_kind IS DISTINCT FROM 'control_source' OR h.source_event_id IS DISTINCT FROM addr.event_id
  OR h.source_session_id IS DISTINCT FROM addr.session_id OR h.source_scope_id IS DISTINCT FROM addr.scope_id
  OR h.source_payload_hash IS DISTINCT FROM addr.payload_hash THEN
  RAISE EXCEPTION 'source binding differs from reviewed intent' USING ERRCODE='23514'; END IF;
 IF TG_TABLE_NAME='source_suppressions' THEN
  IF NEW.session_id<>addr.session_id OR NEW.scope_id<>addr.scope_id OR NEW.client_event_id_digest<>addr.client_event_id_digest
   OR NEW.source_payload_hash<>addr.source_payload_hash OR NEW.payload_hash<>addr.payload_hash THEN
   RAISE EXCEPTION 'suppression address mismatch' USING ERRCODE='23514'; END IF;
  IF TG_OP='UPDATE' THEN
   IF OLD.forget_change_id IS NOT NULL OR NEW.forget_change_id IS NULL OR
     (to_jsonb(NEW)-'forget_change_id') IS DISTINCT FROM (to_jsonb(OLD)-'forget_change_id') THEN
    RAISE EXCEPTION 'suppression is permanent' USING ERRCODE='23514'; END IF;
   SELECT * INTO h FROM public.knowledge_change_headers WHERE tenant_id=NEW.tenant_id AND proposal_id=NEW.forget_change_id;
   IF h.source_event_id IS DISTINCT FROM NEW.event_id OR h.source_session_id IS DISTINCT FROM addr.session_id
    OR h.source_scope_id IS DISTINCT FROM addr.scope_id OR h.source_payload_hash IS DISTINCT FROM addr.payload_hash
    OR h.source_mode IS DISTINCT FROM 'forget' THEN
    RAISE EXCEPTION 'forgetting differs from reviewed intent' USING ERRCODE='23514'; END IF;
  ELSIF (h.source_mode='forget') IS DISTINCT FROM (NEW.forget_change_id IS NOT NULL AND NEW.forget_change_id=NEW.change_id) THEN
   RAISE EXCEPTION 'suppression mode mismatch' USING ERRCODE='23514'; END IF;
 ELSE
  IF NEW.input_hash IS DISTINCT FROM h.payload_hash OR NEW.phase<>0 OR NEW.cursor_id IS NOT NULL OR NEW.processed_targets<>0
   OR NOT EXISTS (SELECT 1 FROM public.durable_operations WHERE tenant_id=NEW.tenant_id AND id=NEW.operation_id
     AND kind='source_cleanup' AND proposal_id=NEW.change_id AND input_hash=NEW.input_hash) THEN
    RAISE EXCEPTION 'cleanup target differs from reviewed intent' USING ERRCODE='23514'; END IF;
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION public.synveda_source_control_binding() FROM PUBLIC;
CREATE TRIGGER source_suppression_binding BEFORE INSERT OR UPDATE ON public.source_suppressions
 FOR EACH ROW EXECUTE FUNCTION public.synveda_source_control_binding();
CREATE TRIGGER source_cleanup_binding BEFORE INSERT ON public.source_cleanup_targets
 FOR EACH ROW EXECUTE FUNCTION public.synveda_source_control_binding();
CREATE FUNCTION public.synveda_source_applied_binding() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path TO 'public','pg_temp' AS $$
BEGIN
 IF NOT EXISTS (SELECT 1 FROM public.knowledge_change_headers h JOIN public.vedaflow_proposals p
   ON p.tenant_id=h.tenant_id AND p.id=h.proposal_id
   JOIN public.source_cleanup_targets target ON target.tenant_id=h.tenant_id AND target.change_id=h.proposal_id
   WHERE h.tenant_id=NEW.tenant_id AND h.proposal_id=NEW.change_id AND h.applied_at IS NOT NULL
    AND h.operation_id=target.operation_id AND target.event_id=NEW.event_id AND p.state='applied') THEN
   RAISE EXCEPTION 'source suppression requires an applied effect' USING ERRCODE='23514'; END IF;
 IF NEW.forget_change_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM public.knowledge_change_headers h
   JOIN public.vedaflow_proposals p ON p.tenant_id=h.tenant_id AND p.id=h.proposal_id
   JOIN public.source_cleanup_targets target ON target.tenant_id=h.tenant_id AND target.change_id=h.proposal_id
   WHERE h.tenant_id=NEW.tenant_id AND h.proposal_id=NEW.forget_change_id AND h.applied_at IS NOT NULL
    AND h.operation_id=target.operation_id AND target.event_id=NEW.event_id AND p.state='applied') THEN
   RAISE EXCEPTION 'source forgetting requires an applied effect' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION public.synveda_source_applied_binding() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER source_suppression_applied AFTER INSERT OR UPDATE ON public.source_suppressions
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.synveda_source_applied_binding();
CREATE FUNCTION public.synveda_source_cleanup_progress() RETURNS trigger
LANGUAGE plpgsql SET search_path TO 'public','pg_temp' AS $$
BEGIN
 IF NOT public.synveda_source_cleanup_authorized(OLD.tenant_id,'session_event',OLD.event_id)
 OR (to_jsonb(NEW)-ARRAY['phase','cursor_id','processed_targets']) IS DISTINCT FROM
    (to_jsonb(OLD)-ARRAY['phase','cursor_id','processed_targets']) OR NEW.phase<OLD.phase
 OR NEW.processed_targets<OLD.processed_targets THEN
  RAISE EXCEPTION 'source cleanup progress needs its live attempt' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION public.synveda_source_cleanup_progress() FROM PUBLIC;
CREATE TRIGGER source_cleanup_progress BEFORE UPDATE ON public.source_cleanup_targets
 FOR EACH ROW EXECUTE FUNCTION public.synveda_source_cleanup_progress();

-- Preserve item Forget's checked in-flight ordering under source eligibility.
CREATE OR REPLACE FUNCTION public.synveda_erase_knowledge(wanted_tenant uuid, wanted_item uuid, wanted_proposal uuid, wanted_operation uuid, wanted_actor_hash text, wanted_reason_hash text) RETURNS void
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'public', 'pg_temp'
    AS $_$
declare
    revision_evidence jsonb;
    revision_ids uuid[];
    source_ids uuid[];
    conflict_ids uuid[];
begin
    if wanted_tenant <> synveda_current_tenant() then
        raise exception 'cross-tenant Knowledge erasure refused'
            using errcode = '42501';
    end if;
    if wanted_actor_hash !~ '^[0-9a-f]{64}$'
       or wanted_reason_hash !~ '^[0-9a-f]{64}$' then
        raise exception 'Knowledge erasure evidence hashes are malformed'
            using errcode = '22023';
    end if;
    if not exists (
        select 1 from durable_operations operation
        where operation.tenant_id = wanted_tenant
          and operation.id = wanted_operation
          and operation.proposal_id = wanted_proposal
          and operation.knowledge_item_id = wanted_item
          and operation.kind = 'knowledge_erasure'
          and operation.state = 'running'
    ) then
        raise exception 'Knowledge erasure operation is not running'
            using errcode = '23514';
    end if;
    if not exists (
        select 1 from knowledge_items item
        where item.tenant_id = wanted_tenant
          and item.id = wanted_item
          and item.lifecycle_state = 'erasure_pending'
    ) then
        raise exception 'Knowledge item is not erasure_pending'
            using errcode = '23514';
    end if;

    perform set_config('synveda.item_cleanup_operation',wanted_operation::text,true);

    select coalesce(
               jsonb_agg(jsonb_build_object('id', id, 'hash', content_hash)
                         order by revision_number),
               '[]'::jsonb
           ),
           coalesce(array_agg(source_id) filter (where source_id is not null), '{}'::uuid[])
    into revision_evidence, source_ids
    from (
        select revision.id, revision.content_hash, revision.revision_number,
               link.knowledge_source_id as source_id
        from knowledge_revisions revision
        left join knowledge_revision_sources link
          on link.tenant_id = revision.tenant_id
         and link.knowledge_revision_id = revision.id
        where revision.tenant_id = wanted_tenant
          and revision.knowledge_item_id = wanted_item
    ) evidence;

    -- Duplicate revision rows introduced by the source join are removed from
    -- the tombstone deterministically.
    select coalesce(
               jsonb_agg(jsonb_build_object('id', id, 'hash', content_hash)
                         order by revision_number),
               '[]'::jsonb
           ),
           coalesce(array_agg(id order by revision_number), '{}'::uuid[])
    into revision_evidence, revision_ids
    from (
        select distinct id, content_hash, revision_number
        from knowledge_revisions
        where tenant_id = wanted_tenant and knowledge_item_id = wanted_item
    ) revisions;

    insert into knowledge_erasure_tombstones
        (tenant_id, knowledge_item_id, proposal_id, operation_id,
         revision_hashes, actor_hash, reason_hash)
    values
        (wanted_tenant, wanted_item, wanted_proposal, wanted_operation,
         revision_evidence, wanted_actor_hash, wanted_reason_hash);

    insert into knowledge_index_invalidations
        (tenant_id, operation_id, revision_id, content_hash)
    select tenant_id, wanted_operation, id, content_hash
    from knowledge_revisions
    where tenant_id = wanted_tenant and knowledge_item_id = wanted_item;

    perform set_config('synveda.knowledge_erasure', 'on', true);
    update knowledge_changes
       set payload = null
     where tenant_id = wanted_tenant
       and target_item_ids @> array[wanted_item]::uuid[]
       and payload is not null;
    update session_context_runs run
       set rendered = ''
      from (
          select distinct selection.tenant_id, selection.context_run_id
          from context_selections selection
          where selection.tenant_id = wanted_tenant
            and selection.knowledge_item_id = wanted_item
      ) affected
     where run.tenant_id = affected.tenant_id
       and run.id = affected.context_run_id
       and run.rendered <> '';
    delete from context_feedback feedback
     using context_selections selection
     where feedback.tenant_id = wanted_tenant
       and selection.tenant_id = feedback.tenant_id
       and selection.id = feedback.context_selection_id
       and selection.context_run_id = feedback.context_run_id
       and selection.knowledge_item_id = wanted_item;
    update context_graph_steps step
       set relation_id = null,
           from_item_id = null,
           from_revision_id = null,
           to_item_id = null,
           to_revision_id = null,
           asserting_revision_id = null
      from (
          select relation.tenant_id, relation.id
          from knowledge_relations relation
          where relation.tenant_id = wanted_tenant
            and relation.source_item_id = wanted_item
          union
          select relation.tenant_id, relation.id
          from knowledge_relations relation
          where relation.tenant_id = wanted_tenant
            and relation.target_item_id = wanted_item
      ) affected
     where step.tenant_id = affected.tenant_id
       and step.relation_id = affected.id;
    update context_selections
       set knowledge_item_id = null, knowledge_revision_id = null
     where tenant_id = wanted_tenant and knowledge_item_id = wanted_item;
    update context_candidates
       set knowledge_item_id = null, knowledge_revision_id = null, scope_id = null
     where tenant_id = wanted_tenant
       and knowledge_revision_id = any(revision_ids);
    update import_mappings
       set title = '', body_markdown = '', summary = '', tags = '{}'::text[],
           verification_metadata = '{}'::jsonb, metadata = '{}'::jsonb,
           proposed_relations = '[]'::jsonb, matched_item_id = null,
           matched_revision_id = null, materializable = false,
           content_erased = true
     where tenant_id = wanted_tenant
       and matched_item_id = wanted_item
       and not content_erased;
    update import_mappings mapping
       set title = '', body_markdown = '', summary = '', tags = '{}'::text[],
           verification_metadata = '{}'::jsonb, metadata = '{}'::jsonb,
           proposed_relations = '[]'::jsonb, matched_item_id = null,
           matched_revision_id = null, materializable = false,
           content_erased = true
      from capture_candidates candidate
     where candidate.tenant_id = wanted_tenant
       and candidate.resulting_knowledge_item_id = wanted_item
       and mapping.tenant_id = candidate.tenant_id
       and mapping.candidate_id = candidate.id
       and not mapping.content_erased;
    select coalesce(array_agg(distinct member.conflict_set_id), '{}'::uuid[])
      into conflict_ids
      from knowledge_conflict_members member
     where member.tenant_id = wanted_tenant
       and member.knowledge_item_id = wanted_item;
    delete from knowledge_conflict_members member
     where member.tenant_id = wanted_tenant
       and member.conflict_set_id = any(conflict_ids);
    delete from knowledge_conflict_sets conflict
     where conflict.tenant_id = wanted_tenant
       and conflict.id = any(conflict_ids);
    delete from knowledge_relations
     where tenant_id = wanted_tenant
       and (source_item_id = wanted_item or target_item_id = wanted_item);
    delete from knowledge_revision_sources link
     using knowledge_revisions revision
     where link.tenant_id = wanted_tenant
       and revision.tenant_id = link.tenant_id
       and revision.id = link.knowledge_revision_id
       and revision.knowledge_item_id = wanted_item;
    delete from knowledge_items_history
     where tenant_id = wanted_tenant and id = wanted_item;

    set constraints knowledge_items_current_revision_fk deferred;
    set constraints knowledge_revisions_item_fk deferred;
    delete from knowledge_items
     where tenant_id = wanted_tenant and id = wanted_item;
    delete from knowledge_revisions
     where tenant_id = wanted_tenant and knowledge_item_id = wanted_item;
    delete from knowledge_sources source
     where source.tenant_id = wanted_tenant
       and source.id = any(source_ids)
       and not exists (
           select 1 from knowledge_revision_sources remaining
           where remaining.tenant_id = source.tenant_id
             and remaining.knowledge_source_id = source.id
       );
    delete from public.unproven_context_deliveries unproven using public.session_context_runs run
     where unproven.tenant_id=wanted_tenant and run.tenant_id=unproven.tenant_id
       and run.id=unproven.context_run_id and run.rendered='';
    perform set_config('synveda.knowledge_erasure', 'off', true);
    perform set_config('synveda.item_cleanup_operation','',true);

    update durable_operations
       set state = 'succeeded', completed_at = now(), updated_at = now(),
           lease_owner = null, lease_expires_at = null, last_error_code = null,
           next_attempt_at = null, progress_percent = 100,
           result = jsonb_build_object('erased', true)
     where tenant_id = wanted_tenant and id = wanted_operation;
end
$_$;


CREATE TRIGGER source_addresses_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON public.source_event_addresses
 FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();
CREATE TRIGGER source_dependencies_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON public.source_dependencies
 FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();
CREATE TRIGGER revision_headers_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON public.knowledge_revision_headers
 FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();

CREATE TRIGGER revision_source_headers_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON public.knowledge_revision_source_headers
 FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();
CREATE TRIGGER source_suppression_permanent BEFORE DELETE OR TRUNCATE ON public.source_suppressions
 FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();
CREATE TRIGGER change_headers_permanent BEFORE DELETE OR TRUNCATE ON public.knowledge_change_headers
 FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();
CREATE FUNCTION public.synveda_change_header_result() RETURNS trigger
LANGUAGE plpgsql SET search_path TO 'public','pg_temp' AS $$
BEGIN
 IF current_user<>pg_catalog.pg_get_userbyid((SELECT datdba FROM pg_catalog.pg_database WHERE datname=current_database()))
   OR OLD.applied_at IS NOT NULL OR NEW.applied_at IS NULL
   OR (to_jsonb(NEW)-ARRAY['operation_id','applied_at']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['operation_id','applied_at']) THEN
   RAISE EXCEPTION 'immutable command header' USING ERRCODE='42501'; END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION public.synveda_change_header_result() FROM PUBLIC;
CREATE TRIGGER change_header_result BEFORE UPDATE ON public.knowledge_change_headers
 FOR EACH ROW EXECUTE FUNCTION public.synveda_change_header_result();

-- INSERT RETURNING happens before the private manifest is written. Require
-- proof at commit, when the same transaction has completed the receipt.
CREATE FUNCTION public.synveda_source_delivery_proof() RETURNS trigger
LANGUAGE plpgsql SET search_path TO 'public','pg_temp' AS $$
BEGIN
 IF NEW.rendered<>'' AND NOT EXISTS(SELECT 1 FROM public.context_run_delivery_dependencies proof
  WHERE proof.tenant_id=NEW.tenant_id AND proof.context_run_id=NEW.id) THEN
  RAISE EXCEPTION 'context delivery requires exact dependency proof' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION public.synveda_source_delivery_proof() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER source_delivery_proof AFTER INSERT ON public.session_context_runs
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.synveda_source_delivery_proof();

-- An embedding provider may finish after the cleanup cursor passed vectors.
-- Late output must not recreate a vector for permanently suppressed content.
CREATE POLICY embedding_source_admission ON public.knowledge_revision_embeddings AS RESTRICTIVE FOR INSERT
 WITH CHECK (public.synveda_source_admissible(tenant_id,'knowledge_revision',knowledge_revision_id));
