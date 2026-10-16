-- CTX-6 / MEM-8, ADR-0150. Prior migration bytes and receiver-local restart stay exact.
ALTER TABLE public.session_events DROP CONSTRAINT session_events_type_check;
ALTER TABLE public.session_events ADD CONSTRAINT session_events_type_check CHECK
 (event_type IN ('session.started','session.ended','message.user','message.assistant','tool.invoked','tool.result',
 'file.read','file.changed','command.executed','skill.loaded','context.requested','adapter.warning','memory.asserted',
 'session.compaction_boundary','session.checkpoint','session.task_state'));
ALTER TABLE public.context_run_delivery_dependencies DROP CONSTRAINT context_delivery_version_check;
ALTER TABLE public.context_run_delivery_dependencies ADD CONSTRAINT context_delivery_version_check CHECK (dependency_version IN (1,2,3));

CREATE TABLE public.session_snapshot_headers (
 tenant_id uuid NOT NULL, id uuid NOT NULL, session_id uuid NOT NULL, sequence bigint NOT NULL,
 event_type text NOT NULL, payload_hash text NOT NULL, client_event_id_digest text NOT NULL, request_hash text,
 PRIMARY KEY (tenant_id,id), UNIQUE (tenant_id,session_id,client_event_id_digest),
 FOREIGN KEY (tenant_id,id) REFERENCES public.source_event_addresses(tenant_id,event_id),
 CHECK (event_type IN ('session.checkpoint','session.task_state')), CHECK (sequence>0),
 CHECK (payload_hash ~ '^[0-9a-f]{64}$'), CHECK (client_event_id_digest ~ '^[0-9a-f]{64}$'),
 CHECK ((event_type='session.task_state' AND request_hash ~ '^[0-9a-f]{64}$') OR (event_type='session.checkpoint' AND request_hash IS NULL))
);
CREATE INDEX session_snapshot_head ON public.session_snapshot_headers(tenant_id,session_id,event_type,sequence DESC);
-- The transactional migration locks the table with old writers stopped.
-- Backfill only exact content-free headers, including unavailable checkpoints.
ALTER TABLE public.session_events DISABLE ROW LEVEL SECURITY;
DO $$ BEGIN
 INSERT INTO public.session_snapshot_headers
  SELECT tenant_id,id,session_id,sequence,event_type,payload_hash,encode(sha256(convert_to(client_event_id,'UTF8')),'hex'),NULL
  FROM public.session_events WHERE event_type='session.checkpoint';
END $$;
ALTER TABLE public.session_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.session_events FORCE ROW LEVEL SECURITY;
ALTER TABLE public.session_snapshot_headers ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.session_snapshot_headers FORCE ROW LEVEL SECURITY;
CREATE POLICY snapshot_header_tenant ON public.session_snapshot_headers
 USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
GRANT SELECT ON public.session_snapshot_headers TO synveda_app;
CREATE TRIGGER snapshot_header_immutable BEFORE UPDATE OR DELETE ON public.session_snapshot_headers
 FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_capture_append_only();

CREATE TABLE public.knowledge_copy_dependencies (
 tenant_id uuid NOT NULL, dependent_kind text NOT NULL, dependent_id uuid NOT NULL,
 knowledge_item_id uuid NOT NULL, revision_id uuid NOT NULL, content_hash text NOT NULL,
 PRIMARY KEY (tenant_id,dependent_kind,dependent_id,knowledge_item_id),
 FOREIGN KEY (tenant_id,revision_id) REFERENCES public.knowledge_revision_headers(tenant_id,revision_id),
 CHECK (dependent_kind IN ('session_event','context_run')), CHECK (content_hash ~ '^[0-9a-f]{64}$')
);
CREATE INDEX knowledge_copy_item ON public.knowledge_copy_dependencies(tenant_id,knowledge_item_id,dependent_kind,dependent_id);
ALTER TABLE public.knowledge_copy_dependencies ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.knowledge_copy_dependencies FORCE ROW LEVEL SECURITY;
CREATE POLICY knowledge_copy_tenant ON public.knowledge_copy_dependencies
 USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
GRANT SELECT ON public.knowledge_copy_dependencies TO synveda_app;
CREATE TRIGGER knowledge_copy_immutable BEFORE UPDATE OR DELETE ON public.knowledge_copy_dependencies
 FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_capture_append_only();

CREATE FUNCTION public.synveda_inherit_task_evidence(t uuid,k text,d uuid,pk text,p uuid) RETURNS void
 LANGUAGE plpgsql SECURITY DEFINER SET search_path TO 'public','pg_temp' AS $$
BEGIN
 IF t IS DISTINCT FROM public.synveda_current_tenant() OR k NOT IN ('session_event','context_run')
 OR pk NOT IN ('session_event','context_run','knowledge_revision','import_artifact')
 OR NOT public.synveda_source_admissible(t,pk,p) THEN
  RAISE EXCEPTION 'task source unavailable' USING ERRCODE='23514'; END IF;
 INSERT INTO public.source_dependencies SELECT t,k,d,source_event_id FROM public.source_dependencies
  WHERE tenant_id=t AND dependent_kind=pk AND dependent_id=p ON CONFLICT DO NOTHING;
 INSERT INTO public.artifact_dependencies SELECT t,k,d,source_key_digest FROM public.artifact_dependencies
  WHERE tenant_id=t AND dependent_kind=pk AND dependent_id=p ON CONFLICT DO NOTHING;
 INSERT INTO public.knowledge_copy_dependencies SELECT t,k,d,knowledge_item_id,revision_id,content_hash
  FROM public.knowledge_copy_dependencies WHERE tenant_id=t AND dependent_kind=pk AND dependent_id=p ON CONFLICT DO NOTHING;
 IF pk='knowledge_revision' THEN
  INSERT INTO public.knowledge_copy_dependencies SELECT t,k,d,knowledge_item_id,revision_id,content_hash
   FROM public.knowledge_revision_headers WHERE tenant_id=t AND revision_id=p ON CONFLICT DO NOTHING;
 ELSIF pk='context_run' THEN
  INSERT INTO public.knowledge_copy_dependencies SELECT DISTINCT t,k,d,h.knowledge_item_id,h.revision_id,h.content_hash
   FROM public.context_run_delivery_dependencies manifest
   CROSS JOIN LATERAL (
    SELECT (x->>'revision_id')::uuid revision_id FROM jsonb_array_elements(manifest.manifest->'knowledge') x
    UNION SELECT v::uuid FROM jsonb_array_elements(manifest.manifest->'graph') x
    CROSS JOIN LATERAL (VALUES(x->>'asserting_revision_id'),(x->>'from_revision_id'),(x->>'to_revision_id')) revisions(v) WHERE v IS NOT NULL
   ) refs JOIN public.knowledge_revision_headers h ON h.tenant_id=t AND h.revision_id=refs.revision_id
   WHERE manifest.tenant_id=t AND manifest.context_run_id=p ON CONFLICT DO NOTHING;
 END IF;
END $$;
REVOKE ALL ON FUNCTION public.synveda_inherit_task_evidence(uuid,text,uuid,text,uuid) FROM PUBLIC;

CREATE FUNCTION public.synveda_task_evidence_projection() RETURNS trigger
 LANGUAGE plpgsql SECURITY DEFINER SET search_path TO 'public','pg_temp' AS $$
DECLARE k text; d uuid; entry jsonb; proof jsonb; source_id uuid; source public.session_events%ROWTYPE;
 body text; text_value text; start_byte integer; end_byte integer; item uuid; version uuid; expected_hash text;
 h public.knowledge_revision_headers%ROWTYPE; snapshot public.session_context_runs%ROWTYPE;
BEGIN
 IF NEW.tenant_id IS DISTINCT FROM public.synveda_current_tenant() THEN
  RAISE EXCEPTION 'task projection tenant mismatch' USING ERRCODE='23514'; END IF;
 IF TG_TABLE_NAME='session_events' THEN
  k='session_event';d=NEW.id;
  IF NEW.event_type IN ('session.task_state','session.checkpoint') THEN
   INSERT INTO public.session_snapshot_headers VALUES (NEW.tenant_id,NEW.id,NEW.session_id,NEW.sequence,NEW.event_type,
    NEW.payload_hash,encode(sha256(convert_to(NEW.client_event_id,'UTF8')),'hex'),
    CASE WHEN NEW.event_type='session.task_state' THEN NEW.payload->>'request_hash' ELSE NULL END);
  END IF;
  IF NEW.event_type='session.task_state' THEN
   IF NEW.payload->>'schema_version' IS DISTINCT FROM '1' OR NEW.payload->>'method' IS DISTINCT FROM 'exact_evidence_task_state_v1'
    OR NOT EXISTS(SELECT 1 FROM public.sessions session WHERE session.tenant_id=NEW.tenant_id AND session.id=NEW.session_id
      AND session.repository_id IS NOT DISTINCT FROM (NEW.payload->>'repository_id')::uuid)
    OR jsonb_typeof(NEW.payload->'statements') IS DISTINCT FROM 'array' OR jsonb_array_length(NEW.payload->'statements') NOT BETWEEN 1 AND 32
    OR NOT coalesce(NEW.payload->>'request_hash' ~ '^[0-9a-f]{64}$',false) THEN
    RAISE EXCEPTION 'invalid task state shape' USING ERRCODE='23514'; END IF;
   FOR entry IN SELECT x FROM jsonb_array_elements(NEW.payload->'statements') x LOOP
    IF entry->>'kind' NOT IN ('objective','constraint','decision','proposal','observation','work','question','next_action','artifact','verification')
      OR (entry->>'declared_outcome' IS NOT NULL AND entry->>'declared_outcome' NOT IN ('planned','attempted','succeeded','failed','unknown')) THEN
      RAISE EXCEPTION 'invalid task declaration' USING ERRCODE='23514'; END IF;
    proof=entry->'evidence';start_byte=(proof#>>'{span,start}')::integer;end_byte=(proof#>>'{span,end}')::integer;
    IF start_byte IS NULL OR end_byte IS NULL OR start_byte<0 OR end_byte<=start_byte OR end_byte-start_byte>1024 THEN
      RAISE EXCEPTION 'invalid task span' USING ERRCODE='23514'; END IF;
    IF proof->>'source_kind'='session_event' THEN
     source_id=(proof->>'event_id')::uuid;
     SELECT * INTO source FROM public.session_events WHERE tenant_id=NEW.tenant_id AND id=source_id AND session_id=NEW.session_id;
     IF source.id IS NULL OR source.sequence>=NEW.sequence OR source.payload_hash IS DISTINCT FROM proof->>'payload_hash'
       OR source.event_type NOT IN ('message.user','message.assistant','tool.result','command.executed','memory.asserted','file.read','file.changed')
       OR entry->>'evidence_class' IS DISTINCT FROM (CASE WHEN source.event_type='message.user' THEN 'user_declared' ELSE 'client_reported' END) THEN
       RAISE EXCEPTION 'task event proof mismatch' USING ERRCODE='23514'; END IF;
     IF proof->>'field'='text' AND proof->>'tool_index' IS NULL THEN body=source.payload->>'text';
     ELSIF proof->>'field'='tool_text' AND (proof->>'tool_index')::integer BETWEEN 0 AND 31 THEN
      body=source.payload#>>ARRAY['tools',proof->>'tool_index','text'];
     ELSE RAISE EXCEPTION 'invalid task selector' USING ERRCODE='23514'; END IF;
     PERFORM public.synveda_inherit_task_evidence(NEW.tenant_id,k,d,'session_event',source_id);
    ELSIF proof->>'source_kind'='knowledge_revision' THEN
     version=(proof->>'revision_id')::uuid;item=(proof->>'item_id')::uuid;
     SELECT * INTO h FROM public.knowledge_revision_headers WHERE tenant_id=NEW.tenant_id AND revision_id=version;
     SELECT r.body_markdown INTO body FROM public.knowledge_items i JOIN public.knowledge_revisions r
       ON r.tenant_id=i.tenant_id AND r.id=i.current_revision_id WHERE i.tenant_id=NEW.tenant_id AND i.id=item
       AND i.current_revision_id=version AND i.lifecycle_state='active' AND NOT r.content_erased;
     IF body IS NULL OR h.knowledge_item_id IS DISTINCT FROM item OR h.content_hash IS DISTINCT FROM proof->>'content_hash'
       OR entry->>'evidence_class' IS DISTINCT FROM 'published_knowledge' THEN
       RAISE EXCEPTION 'task Knowledge proof mismatch' USING ERRCODE='23514'; END IF;
     PERFORM public.synveda_inherit_task_evidence(NEW.tenant_id,k,d,'knowledge_revision',version);
    ELSIF proof->>'source_kind'='import_artifact' THEN
     source_id=(proof->>'artifact_id')::uuid;
     SELECT body_markdown,content_hash INTO body,expected_hash FROM public.import_artifacts
      WHERE tenant_id=NEW.tenant_id AND id=source_id AND NOT content_erased;
     IF body IS NULL OR expected_hash IS DISTINCT FROM proof->>'content_hash' OR entry->>'evidence_class' IS DISTINCT FROM 'imported_source' THEN
       RAISE EXCEPTION 'task artifact proof mismatch' USING ERRCODE='23514'; END IF;
     PERFORM public.synveda_inherit_task_evidence(NEW.tenant_id,k,d,'import_artifact',source_id);
    ELSE RAISE EXCEPTION 'unsupported task evidence' USING ERRCODE='23514'; END IF;
    text_value=convert_from(substring(convert_to(body,'UTF8') FROM start_byte+1 FOR end_byte-start_byte),'UTF8');
    IF end_byte>octet_length(body) OR text_value IS DISTINCT FROM entry->>'text' OR btrim(text_value)='' THEN
     RAISE EXCEPTION 'task excerpt differs from source' USING ERRCODE='23514'; END IF;
   END LOOP;
   proof=NEW.payload->'checkout';
   IF proof IS NOT NULL AND proof<>'null' THEN
    source_id=(proof#>>'{source,event_id}')::uuid;
    SELECT * INTO source FROM public.session_events WHERE tenant_id=NEW.tenant_id AND id=source_id AND session_id=NEW.session_id;
    IF source.id IS NULL OR source.sequence>=NEW.sequence OR source.payload_hash IS DISTINCT FROM proof#>>'{source,payload_hash}'
      OR source.payload#>>'{context,checkout,ref}' IS DISTINCT FROM proof->>'worktree_ref'
      OR source.payload#>>'{context,checkout,branch}' IS DISTINCT FROM proof->>'branch'
      OR source.payload#>>'{context,checkout,commit}' IS DISTINCT FROM proof->>'commit'
      OR source.payload#>>'{context,checkout,tree}' IS DISTINCT FROM proof->>'tree'
      OR coalesce(source.payload#>'{context,checkout,dirty}','null'::jsonb) IS DISTINCT FROM proof->'dirty'
      OR source.payload#>>'{context,checkout,dirty_digest}' IS DISTINCT FROM proof->>'dirty_digest' THEN
      RAISE EXCEPTION 'task checkout proof mismatch' USING ERRCODE='23514'; END IF;
    PERFORM public.synveda_inherit_task_evidence(NEW.tenant_id,k,d,'session_event',source_id);
   END IF;
   proof=NEW.payload->'context_snapshot';
   IF proof IS NOT NULL AND proof<>'null' THEN
    source_id=(proof->>'context_run_id')::uuid;
    SELECT * INTO snapshot FROM public.session_context_runs WHERE tenant_id=NEW.tenant_id AND id=source_id AND session_id=NEW.session_id;
    IF snapshot.id IS NULL OR snapshot.block_hash IS DISTINCT FROM proof->>'block_hash'
      OR NOT EXISTS(SELECT 1 FROM public.context_run_delivery_dependencies m WHERE m.tenant_id=NEW.tenant_id AND m.context_run_id=source_id
        AND m.manifest->>'restart_contributed'='false' AND (m.manifest->'handoff' IS NULL OR m.manifest->'handoff'='null')) THEN
      RAISE EXCEPTION 'task context proof mismatch' USING ERRCODE='23514'; END IF;
    PERFORM public.synveda_inherit_task_evidence(NEW.tenant_id,k,d,'context_run',source_id);
   END IF;
  ELSIF NEW.event_type='session.checkpoint' AND NEW.payload->>'schema_version'='2' THEN
   proof=NEW.payload->'task_state';source_id=(proof->>'event_id')::uuid;
   SELECT * INTO source FROM public.session_events WHERE tenant_id=NEW.tenant_id AND id=source_id AND session_id=NEW.session_id;
   IF NEW.payload->>'method'<>'exact_task_state_and_event_excerpts_v2' OR source.id IS NULL
     OR source.event_type<>'session.task_state' OR source.sequence>=NEW.sequence OR source.payload_hash IS DISTINCT FROM proof->>'payload_hash' THEN
     RAISE EXCEPTION 'checkpoint task proof mismatch' USING ERRCODE='23514'; END IF;
   PERFORM public.synveda_inherit_task_evidence(NEW.tenant_id,k,d,'session_event',source_id);
  ELSE RETURN NEW; END IF;
 ELSIF TG_TABLE_NAME='context_run_delivery_dependencies' THEN
  k='context_run';d=NEW.context_run_id;
  IF NEW.dependency_version=3 THEN
   proof=NEW.manifest->'handoff';
   IF proof IS NOT NULL AND proof<>'null' THEN
    source_id=(proof->>'snapshot_event_id')::uuid;
    SELECT * INTO source FROM public.session_events WHERE tenant_id=NEW.tenant_id AND id=source_id AND session_id=(proof->>'source_session_id')::uuid;
    IF source.id IS NULL OR source.event_type NOT IN ('session.task_state','session.checkpoint') OR source.payload_hash IS DISTINCT FROM proof->>'payload_hash' THEN
     RAISE EXCEPTION 'handoff source proof mismatch' USING ERRCODE='23514'; END IF;
    PERFORM public.synveda_inherit_task_evidence(NEW.tenant_id,k,d,'session_event',source_id);
   END IF;
  END IF;
  IF NEW.manifest->>'restart_contributed'='true' THEN
   FOR source_id IN SELECT x::uuid FROM jsonb_array_elements_text(NEW.manifest->'restart_source_event_ids') x LOOP
    IF NOT EXISTS(SELECT 1 FROM public.source_event_addresses address WHERE address.tenant_id=NEW.tenant_id AND address.event_id=source_id
      AND address.session_id=coalesce((NEW.manifest#>>'{handoff,source_session_id}')::uuid,
       (SELECT session_id FROM public.session_context_runs WHERE tenant_id=NEW.tenant_id AND id=NEW.context_run_id))) THEN
      RAISE EXCEPTION 'delivered task root Session mismatch' USING ERRCODE='23514'; END IF;
    PERFORM public.synveda_inherit_task_evidence(NEW.tenant_id,k,d,'session_event',source_id);
   END LOOP;
  END IF;
  IF NEW.manifest->>'restart_contributed'='true' AND NEW.manifest#>>'{checkpoint,event_id}' IS NOT NULL THEN
   PERFORM public.synveda_inherit_task_evidence(NEW.tenant_id,k,d,'session_event',(NEW.manifest#>>'{checkpoint,event_id}')::uuid);
  END IF;
 ELSE RAISE EXCEPTION 'unsupported task projection' USING ERRCODE='23514'; END IF;
 -- Lock exact item guards in one global order. Two shared admissions cannot
 -- both observe the last free slot and exceed the inline erasure bound.
 FOR item IN SELECT knowledge_item_id FROM public.knowledge_copy_dependencies
   WHERE tenant_id=NEW.tenant_id AND dependent_kind=k AND dependent_id=d ORDER BY knowledge_item_id LOOP
  PERFORM 1 FROM public.knowledge_items WHERE tenant_id=NEW.tenant_id AND id=item FOR UPDATE;
  IF (SELECT count(*) FROM (SELECT 1 FROM public.knowledge_copy_dependencies
    WHERE tenant_id=NEW.tenant_id AND knowledge_item_id=item LIMIT 4097) bounded)>4096 THEN
   RAISE EXCEPTION 'task copy bound exhausted' USING ERRCODE='23514'; END IF;
 END LOOP;
 IF (SELECT count(*) FROM (SELECT 1 FROM public.source_dependencies WHERE tenant_id=NEW.tenant_id AND dependent_kind=k AND dependent_id=d LIMIT 4097) bounded)>4096
 OR (SELECT count(*) FROM (SELECT 1 FROM public.artifact_dependencies WHERE tenant_id=NEW.tenant_id AND dependent_kind=k AND dependent_id=d LIMIT 4097) bounded)>4096 THEN
  RAISE EXCEPTION 'task roots bound exhausted' USING ERRCODE='23514'; END IF;
 IF NOT public.synveda_source_admissible(NEW.tenant_id,k,d) THEN
  RAISE EXCEPTION 'task evidence changed' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION public.synveda_task_evidence_projection() FROM PUBLIC;
CREATE TRIGGER zzzz_task_event_projection AFTER INSERT ON public.session_events
 FOR EACH ROW EXECUTE FUNCTION public.synveda_task_evidence_projection();
CREATE TRIGGER zzzz_task_delivery_projection AFTER INSERT ON public.context_run_delivery_dependencies
 FOR EACH ROW EXECUTE FUNCTION public.synveda_task_evidence_projection();

-- Physical cleanup is still authorized by the original exact governed effect.
ALTER TABLE public.artifact_cleanup_targets DROP CONSTRAINT artifact_cleanup_targets_phase_check;
ALTER TABLE public.artifact_cleanup_targets ADD CONSTRAINT artifact_cleanup_targets_phase_check CHECK (phase BETWEEN 0 AND 10);
DROP POLICY session_source_eligibility ON public.session_events;
CREATE POLICY session_source_eligibility ON public.session_events AS RESTRICTIVE FOR SELECT
 USING ((NOT content_erased AND public.synveda_source_available(tenant_id,'session_event',id))
 OR public.synveda_source_cleanup_authorized(tenant_id,'session_event',id)
 OR public.synveda_artifact_cleanup_authorized(tenant_id,'session_event',id)
 OR public.synveda_item_cleanup_authorized(tenant_id,'session_event',id));

-- Exact E cleanup overrides; qualified prior routines retain every other guard.
CREATE OR REPLACE FUNCTION public.synveda_source_available(t uuid,k text,d uuid) RETURNS boolean
 LANGUAGE sql STABLE SET search_path TO 'public','pg_temp' AS $$
 SELECT t=public.synveda_current_tenant() AND (
 (NOT EXISTS(SELECT 1 FROM public.source_dependencies dep JOIN public.source_suppressions s ON s.tenant_id=dep.tenant_id AND s.event_id=dep.source_event_id WHERE dep.tenant_id=t AND dep.dependent_kind=k AND dep.dependent_id=d)
 AND NOT EXISTS(SELECT 1 FROM public.artifact_dependencies dep JOIN public.artifact_suppressions s ON s.tenant_id=dep.tenant_id AND s.source_key_digest=dep.source_key_digest WHERE dep.tenant_id=t AND dep.dependent_kind=k AND dep.dependent_id=d)
 AND NOT EXISTS(SELECT 1 FROM public.unproven_artifact_dependencies WHERE tenant_id=t AND dependent_kind=k AND dependent_id=d) AND NOT EXISTS(SELECT 1 FROM public.knowledge_copy_dependencies copy
 LEFT JOIN public.knowledge_items item ON item.tenant_id=copy.tenant_id AND item.id=copy.knowledge_item_id
 LEFT JOIN public.knowledge_revision_headers header ON header.tenant_id=copy.tenant_id AND header.revision_id=copy.revision_id
 WHERE copy.tenant_id=t AND copy.dependent_kind=k AND copy.dependent_id=d
 AND (item.id IS NULL OR item.lifecycle_state<>'active' OR item.current_revision_id<>copy.revision_id
 OR header.content_hash IS DISTINCT FROM copy.content_hash)))
 OR public.synveda_source_cleanup_authorized(t,k,d) OR public.synveda_artifact_cleanup_authorized(t,k,d) OR public.synveda_item_cleanup_authorized(t,k,d))
$$;

CREATE OR REPLACE FUNCTION public.synveda_source_admissible(t uuid,k text,d uuid) RETURNS boolean
 LANGUAGE sql STABLE SET search_path TO 'public','pg_temp' AS $$
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
 OR header.content_hash IS DISTINCT FROM copy.content_hash))
$$;

CREATE OR REPLACE FUNCTION public.synveda_item_cleanup_authorized(t uuid,k text,d uuid) RETURNS boolean
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
$$;

CREATE OR REPLACE FUNCTION public.synveda_source_content_erasure() RETURNS trigger
LANGUAGE plpgsql SET search_path TO 'public','pg_temp' AS $$
DECLARE k text;
BEGIN
 k=CASE TG_TABLE_NAME WHEN 'knowledge_revisions' THEN 'knowledge_revision'
      WHEN 'session_events' THEN 'session_event' WHEN 'knowledge_relations' THEN 'knowledge_revision' ELSE 'knowledge_source' END;
 IF NOT (public.synveda_source_cleanup_authorized(OLD.tenant_id,k,CASE WHEN TG_TABLE_NAME='knowledge_relations' THEN (to_jsonb(OLD)->>'asserting_revision_id')::uuid ELSE OLD.id END) OR public.synveda_artifact_cleanup_authorized(OLD.tenant_id,k,CASE WHEN TG_TABLE_NAME='knowledge_relations' THEN (to_jsonb(OLD)->>'asserting_revision_id')::uuid ELSE OLD.id END) OR (TG_TABLE_NAME='session_events' AND public.synveda_item_cleanup_authorized(OLD.tenant_id,k,OLD.id)))
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

CREATE OR REPLACE FUNCTION public.synveda_artifact_cleanup_chunk(t uuid,o uuid,w text,a integer,n integer)
 RETURNS TABLE(progress smallint,done boolean,erased bigint)
LANGUAGE plpgsql SECURITY DEFINER SET search_path TO 'public','pg_temp' AS $$
DECLARE target public.artifact_cleanup_targets%ROWTYPE; op public.durable_operations%ROWTYPE;
 kind text; ids uuid[]; last_id uuid; affected bigint=0; changed bigint=0;
BEGIN
 IF t IS DISTINCT FROM public.synveda_current_tenant() OR n IS NULL OR n NOT BETWEEN 1 AND 100 THEN
   RAISE EXCEPTION 'invalid source cleanup boundary' USING ERRCODE='23514'; END IF;
 SELECT * INTO op FROM public.durable_operations WHERE tenant_id=t AND id=o FOR UPDATE;
 SELECT * INTO target FROM public.artifact_cleanup_targets WHERE tenant_id=t AND operation_id=o FOR UPDATE;
 IF op.kind IS DISTINCT FROM 'source_cleanup' OR op.state IS DISTINCT FROM 'running'
    OR op.lease_owner IS DISTINCT FROM w OR op.attempts IS DISTINCT FROM a
    OR op.lease_expires_at<=clock_timestamp() OR target.operation_id IS NULL
    OR NOT EXISTS (SELECT 1 FROM public.operation_attempts WHERE tenant_id=t AND operation_id=o
       AND attempt_number=a AND worker_id=w AND state='running') THEN
   RAISE EXCEPTION 'source cleanup lost its attempt' USING ERRCODE='23514'; END IF;
 PERFORM set_config('synveda.artifact_cleanup_operation',o::text,true);
 PERFORM set_config('synveda.artifact_cleanup_attempt',a::text,true);
 IF NOT public.synveda_artifact_cleanup_authorized(t,'import_artifact',target.artifact_id) THEN
   RAISE EXCEPTION 'source cleanup lacks an applied decision' USING ERRCODE='23514'; END IF;
 UPDATE public.durable_operations SET lease_expires_at=clock_timestamp()+interval '30 seconds' WHERE tenant_id=t AND id=o;
 kind=CASE target.phase WHEN 0 THEN 'knowledge_revision' WHEN 1 THEN 'context_run'
      WHEN 2 THEN 'capture_candidate' WHEN 3 THEN 'knowledge_change'
      WHEN 4 THEN 'knowledge_revision' WHEN 5 THEN 'knowledge_revision' WHEN 6 THEN 'knowledge_revision' WHEN 7 THEN 'knowledge_source' WHEN 8 THEN 'import_mapping' WHEN 9 THEN 'import_artifact' ELSE 'session_event' END;
 SELECT array_agg(dependent_id ORDER BY dependent_id),max(dependent_id::text)::uuid INTO ids,last_id
   FROM (SELECT dependent_id FROM public.artifact_dependencies WHERE tenant_id=t AND source_key_digest=target.source_key_digest
     AND dependent_kind=kind AND (target.cursor_id IS NULL OR dependent_id>target.cursor_id)
     ORDER BY dependent_id LIMIT n) page;
 IF target.phase=0 THEN
   IF EXISTS (SELECT 1 FROM public.unproven_context_deliveries WHERE tenant_id=t) OR EXISTS(SELECT 1 FROM public.unproven_artifact_dependencies WHERE tenant_id=t) THEN
     UPDATE public.durable_operations SET state='blocked',last_error_code='lineage_unproven',completed_at=clock_timestamp(),
       updated_at=clock_timestamp(),lease_owner=NULL,lease_expires_at=NULL,next_attempt_at=NULL WHERE tenant_id=t AND id=o;
     UPDATE public.operation_attempts SET state='cancelled',safe_error_code='lineage_unproven',completed_at=clock_timestamp()
       WHERE tenant_id=t AND operation_id=o AND attempt_number=a;
     UPDATE public.operation_outbox SET state='completed',lease_owner=NULL,lease_expires_at=NULL,next_dispatch_at=NULL,
       completed_at=clock_timestamp(),updated_at=clock_timestamp() WHERE tenant_id=t AND operation_id=o;
     PERFORM set_config('synveda.artifact_cleanup_operation','',true);
     PERFORM set_config('synveda.artifact_cleanup_attempt','',true);
     RETURN QUERY SELECT op.progress_percent,true,0::bigint; RETURN;
   END IF;
   IF EXISTS (SELECT 1 FROM public.knowledge_revision_headers WHERE tenant_id=t AND revision_id=ANY(ids) AND (legal_hold OR retention_hold)) THEN
     UPDATE public.durable_operations SET state='blocked',last_error_code='retention_hold',completed_at=clock_timestamp(),
       updated_at=clock_timestamp(),lease_owner=NULL,lease_expires_at=NULL,next_attempt_at=NULL WHERE tenant_id=t AND id=o;
     UPDATE public.operation_attempts SET state='cancelled',safe_error_code='retention_hold',completed_at=clock_timestamp()
       WHERE tenant_id=t AND operation_id=o AND attempt_number=a;
     UPDATE public.operation_outbox SET state='completed',lease_owner=NULL,lease_expires_at=NULL,next_dispatch_at=NULL,
       completed_at=clock_timestamp(),updated_at=clock_timestamp() WHERE tenant_id=t AND operation_id=o;
     PERFORM set_config('synveda.artifact_cleanup_operation','',true);
     PERFORM set_config('synveda.artifact_cleanup_attempt','',true);
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
     SELECT e.ctid FROM public.knowledge_revision_embeddings e JOIN public.artifact_dependencies dep
       ON dep.tenant_id=e.tenant_id AND dep.dependent_kind='knowledge_revision' AND dep.dependent_id=e.knowledge_revision_id
     WHERE e.tenant_id=t AND dep.source_key_digest=target.source_key_digest
     ORDER BY e.knowledge_revision_id,e.model LIMIT n FOR UPDATE OF e);
   GET DIAGNOSTICS changed=ROW_COUNT;
   ids=CASE WHEN changed=n THEN array_fill(target.artifact_id,ARRAY[n]) ELSE '{}'::uuid[] END;
   last_id=NULL;
 ELSIF target.phase=6 THEN
   SELECT array_agg(id ORDER BY id),max(id::text)::uuid INTO ids,last_id FROM (
     SELECT r.id FROM public.knowledge_relations r JOIN public.artifact_dependencies dep
       ON dep.tenant_id=r.tenant_id AND dep.dependent_kind='knowledge_revision' AND dep.dependent_id=r.asserting_revision_id
     WHERE r.tenant_id=t AND dep.source_key_digest=target.source_key_digest AND NOT r.content_erased
       AND (target.cursor_id IS NULL OR r.id>target.cursor_id) ORDER BY r.id LIMIT n) page;
   UPDATE public.knowledge_relations SET content_erased=true,metadata='{}' WHERE tenant_id=t AND id=ANY(ids) AND NOT content_erased;
 ELSIF target.phase=7 THEN
   UPDATE public.knowledge_sources SET content_erased=true,locator=NULL,source_revision=NULL,metadata='{}'
     WHERE tenant_id=t AND id=ANY(ids) AND NOT content_erased;
 ELSIF target.phase=8 THEN
   PERFORM set_config('synveda.knowledge_erasure','on',true);
   UPDATE public.import_mappings SET content_erased=true,okf_type='[erased]',title='',body_markdown='',summary='',tags='{}',
    verification_metadata='{}',metadata='{}',proposed_relations='[]',matched_item_id=NULL,matched_revision_id=NULL,materializable=false
    WHERE tenant_id=t AND id=ANY(ids) AND (NOT content_erased OR okf_type<>'[erased]');
   PERFORM set_config('synveda.knowledge_erasure','off',true);
 ELSIF target.phase=9 THEN
   UPDATE public.import_jobs job SET notices='[]' WHERE job.tenant_id=t AND job.notices<>'[]'
    AND EXISTS(SELECT 1 FROM public.source_artifact_addresses address WHERE address.tenant_id=t AND address.job_id=job.id AND address.artifact_id=ANY(ids));
   UPDATE public.import_artifacts SET content_erased=true,logical_path='erased/'||id::text,frontmatter='{}',body_markdown=''
    WHERE tenant_id=t AND id=ANY(ids) AND NOT content_erased;
 ELSE
   UPDATE public.session_events SET content_erased=true,payload='{}',redactions=NULL
    WHERE tenant_id=t AND id=ANY(ids) AND NOT content_erased;
 END IF;
 affected=CASE WHEN target.phase=0 THEN 0 ELSE coalesce(cardinality(ids),0) END;
 IF coalesce(cardinality(ids),0)<n THEN
   IF target.phase<10 THEN target.phase=target.phase+1; target.cursor_id=NULL;
   ELSE target.phase=10; target.cursor_id=last_id; END IF;
 ELSE target.cursor_id=last_id; END IF;
 UPDATE public.artifact_cleanup_targets SET phase=target.phase,cursor_id=target.cursor_id,processed_targets=processed_targets+affected
    WHERE tenant_id=t AND operation_id=o;
 done=(kind='session_event' AND coalesce(cardinality(ids),0)<n);
 progress=CASE WHEN done THEN 100 ELSE LEAST(99,target.phase*10)::smallint END;
 IF NOT public.synveda_artifact_cleanup_authorized(t,'import_artifact',target.artifact_id) THEN RAISE EXCEPTION 'source cleanup page expired' USING ERRCODE='23514'; END IF;
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
 PERFORM set_config('synveda.artifact_cleanup_operation','',true);
 PERFORM set_config('synveda.artifact_cleanup_attempt','',true);
 RETURN QUERY SELECT progress,done,affected;
END $$;

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
    -- Exact E copies are capped at admission and independently indexed. Clear
    -- only these derivatives; original Session evidence remains intact.
    update public.session_events event set content_erased=true,payload='{}',redactions=null
     where event.tenant_id=wanted_tenant and not event.content_erased
      and exists(select 1 from public.knowledge_copy_dependencies copy where copy.tenant_id=event.tenant_id
       and copy.dependent_kind='session_event' and copy.dependent_id=event.id and copy.knowledge_item_id=wanted_item);
    update public.session_context_runs run set rendered=''
     where run.tenant_id=wanted_tenant and run.rendered<>''
      and exists(select 1 from public.knowledge_copy_dependencies copy where copy.tenant_id=run.tenant_id
       and copy.dependent_kind='context_run' and copy.dependent_id=run.id and copy.knowledge_item_id=wanted_item);
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
       set okf_type='[erased]', title = '', body_markdown = '', summary = '', tags = '{}'::text[],
           verification_metadata = '{}'::jsonb, metadata = '{}'::jsonb,
           proposed_relations = '[]'::jsonb, matched_item_id = null,
           matched_revision_id = null, materializable = false,
           content_erased = true
     where tenant_id = wanted_tenant
       and matched_item_id = wanted_item
       and not content_erased;
    update import_mappings mapping
       set okf_type='[erased]', title = '', body_markdown = '', summary = '', tags = '{}'::text[],
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
    if not public.synveda_item_cleanup_authorized(wanted_tenant,'knowledge_item',wanted_item) then
        raise exception 'Knowledge erasure lost its checked capability' using errcode='23514';
    end if;
    -- Retire only ambiguity whose protected content this exact checked item
    -- operation has now deleted or cleared. Caller GUCs grant no such right.
    delete from public.unproven_artifact_dependencies marker
     where marker.tenant_id=wanted_tenant and (
       (marker.dependent_kind='knowledge_revision' and marker.dependent_id=any(revision_ids)
        and not exists(select 1 from public.knowledge_revisions where tenant_id=wanted_tenant and id=marker.dependent_id))
       or (marker.dependent_kind='knowledge_source' and marker.dependent_id=any(source_ids)
        and not exists(select 1 from public.knowledge_sources where tenant_id=wanted_tenant and id=marker.dependent_id))
       or (marker.dependent_kind='knowledge_change' and exists(select 1 from public.knowledge_changes change
        join public.knowledge_change_headers header on header.tenant_id=change.tenant_id and header.proposal_id=change.proposal_id
        where change.tenant_id=wanted_tenant and change.proposal_id=marker.dependent_id and change.payload is null and header.target_item_ids @> array[wanted_item]))
       or (marker.dependent_kind='context_run' and exists(select 1 from public.session_context_runs run
        where run.tenant_id=wanted_tenant and run.id=marker.dependent_id and run.rendered=''
        and public.synveda_item_cleanup_authorized(wanted_tenant,'context_run',run.id)))
     );
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

CREATE OR REPLACE FUNCTION public.synveda_source_projection() RETURNS trigger
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
   IF EXISTS(SELECT 1 FROM public.session_events WHERE tenant_id=NEW.tenant_id AND id=source_id
      AND event_type IN ('session.task_state','session.checkpoint')) THEN
    RAISE EXCEPTION 'derived task state cannot be primary Knowledge evidence' USING ERRCODE='23514'; END IF;
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
   INSERT INTO public.knowledge_change_headers (tenant_id,proposal_id,command_kind,payload_hash,target_item_ids,source_event_id,source_session_id,source_scope_id,source_payload_hash,source_mode,operation_id,applied_at,source_artifact_id,source_artifact_hash) VALUES (NEW.tenant_id,NEW.proposal_id,NEW.command_kind,NEW.payload_hash,
     NEW.target_item_ids,(NEW.payload->>'event_id')::uuid,(NEW.payload->>'session_id')::uuid,(NEW.payload->>'scope_id')::uuid,
     NEW.payload->>'expected_payload_hash',NEW.payload->>'mode',NEW.operation_id,NEW.applied_at,(NEW.payload->>'artifact_id')::uuid,NEW.payload->>'expected_content_hash');
   kind='knowledge_change'; parent_id=NEW.proposal_id; control_effect=NEW.command_kind IN ('control_source','control_artifact');
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
   IF NEW.command_kind NOT IN ('control_source','control_artifact') AND NOT public.synveda_source_admissible(NEW.tenant_id,kind,parent_id) THEN
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
     IF TG_TABLE_NAME='knowledge_changes' AND NOT control_effect AND EXISTS(SELECT 1 FROM public.session_events
       WHERE tenant_id=NEW.tenant_id AND id=source_id AND event_type IN ('session.task_state','session.checkpoint')) THEN
       RAISE EXCEPTION 'derived task state cannot be primary Knowledge evidence' USING ERRCODE='23514'; END IF;
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
