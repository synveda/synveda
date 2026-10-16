-- SKIL-6 / SKIL-5 / MEM-8, ADR-0151. Prior migration bytes remain immutable.
ALTER TABLE public.session_events DROP CONSTRAINT session_events_type_check;
ALTER TABLE public.session_events ADD CONSTRAINT session_events_type_check CHECK
 (event_type IN ('session.started','session.ended','message.user','message.assistant','tool.invoked','tool.result',
 'file.read','file.changed','command.executed','skill.loaded','context.requested','adapter.warning','memory.asserted',
 'session.compaction_boundary','session.checkpoint','session.task_state','task.outcome','procedure.improvement'));

CREATE TABLE public.procedure_outcome_headers (
 tenant_id uuid NOT NULL, event_id uuid NOT NULL, session_id uuid NOT NULL,
 procedure_item_id uuid NOT NULL, procedure_revision_id uuid NOT NULL, procedure_hash text NOT NULL,
 request_hash text NOT NULL, reporter_id uuid NOT NULL, evidence_class text NOT NULL,
 outcome text NOT NULL, environment_digest text NOT NULL, skill_versions uuid[] NOT NULL,
 context_run_id uuid,context_block_hash text,received_at timestamptz NOT NULL,
 PRIMARY KEY(tenant_id,event_id),
 FOREIGN KEY(tenant_id,event_id) REFERENCES public.source_event_addresses(tenant_id,event_id),
 FOREIGN KEY(tenant_id,procedure_revision_id) REFERENCES public.knowledge_revision_headers(tenant_id,revision_id),
 CHECK(evidence_class IN ('client_reported','authenticated_runner')),
 CHECK(outcome IN ('succeeded','failed','unknown')),
 CHECK(cardinality(skill_versions)<=8 AND array_position(skill_versions,NULL) IS NULL),
 CHECK((context_run_id IS NULL)=(context_block_hash IS NULL) AND (context_block_hash IS NULL OR context_block_hash ~ '^[0-9a-f]{64}$')),
 CHECK(procedure_hash ~ '^[0-9a-f]{64}$' AND request_hash ~ '^[0-9a-f]{64}$' AND environment_digest ~ '^[0-9a-f]{64}$')
);
CREATE INDEX procedure_outcome_context ON public.procedure_outcome_headers(tenant_id,context_run_id,received_at DESC,event_id) WHERE context_run_id IS NOT NULL;
CREATE TABLE public.procedure_improvement_headers (
 tenant_id uuid NOT NULL, event_id uuid NOT NULL, session_id uuid NOT NULL,
 identity_hash text NOT NULL, request_hash text NOT NULL,
 procedure_item_id uuid NOT NULL, procedure_revision_id uuid NOT NULL, procedure_hash text NOT NULL,
 content_hash text NOT NULL,
 PRIMARY KEY(tenant_id,event_id), UNIQUE(tenant_id,identity_hash),
 FOREIGN KEY(tenant_id,event_id) REFERENCES public.source_event_addresses(tenant_id,event_id),
 FOREIGN KEY(tenant_id,procedure_revision_id) REFERENCES public.knowledge_revision_headers(tenant_id,revision_id),
 CHECK(identity_hash ~ '^[0-9a-f]{64}$' AND request_hash ~ '^[0-9a-f]{64}$' AND procedure_hash ~ '^[0-9a-f]{64}$' AND content_hash ~ '^[0-9a-f]{64}$')
);
CREATE TABLE public.procedure_candidate_headers (
 tenant_id uuid NOT NULL, candidate_id uuid NOT NULL, request_event_id uuid NOT NULL,
 content_hash text NOT NULL,
 PRIMARY KEY(tenant_id,candidate_id), UNIQUE(tenant_id,request_event_id),
 FOREIGN KEY(tenant_id,request_event_id) REFERENCES public.procedure_improvement_headers(tenant_id,event_id),
 CHECK(content_hash ~ '^[0-9a-f]{64}$')
);
CREATE TABLE public.procedure_evaluation_attempts (
 tenant_id uuid NOT NULL, id uuid NOT NULL, candidate_id uuid NOT NULL,
 candidate_hash text NOT NULL, request_hash text NOT NULL, suite_id text NOT NULL,
 evaluator_id uuid NOT NULL, configuration_hash text NOT NULL, specification_hash text NOT NULL,
 specification jsonb NOT NULL, created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,id), UNIQUE(tenant_id,candidate_id,candidate_hash,configuration_hash,specification_hash),
 FOREIGN KEY(tenant_id,candidate_id) REFERENCES public.procedure_candidate_headers(tenant_id,candidate_id),
 CHECK(candidate_hash ~ '^[0-9a-f]{64}$' AND request_hash ~ '^[0-9a-f]{64}$' AND configuration_hash ~ '^[0-9a-f]{64}$' AND specification_hash ~ '^[0-9a-f]{64}$'),
 CHECK(jsonb_typeof(specification)='object' AND octet_length(specification::text)<=16384),
 CHECK(length(suite_id) BETWEEN 1 AND 128)
);
CREATE TABLE public.procedure_evaluation_results (
 tenant_id uuid NOT NULL, attempt_id uuid NOT NULL, reporter_id uuid NOT NULL,
 request_hash text NOT NULL, qualified boolean NOT NULL, results jsonb NOT NULL,
 observed_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,attempt_id),
 FOREIGN KEY(tenant_id,attempt_id) REFERENCES public.procedure_evaluation_attempts(tenant_id,id),
 CHECK(request_hash ~ '^[0-9a-f]{64}$'),
 CHECK(jsonb_typeof(results)='array' AND jsonb_array_length(results) BETWEEN 1 AND 32 AND octet_length(results::text)<=16384)
);
-- Historical derivation differs from an exact-current context copy. Item
-- tombstones and inherited source roots revoke it; head advance alone does not.
CREATE TABLE public.historical_knowledge_dependencies (
 tenant_id uuid NOT NULL, dependent_kind text NOT NULL, dependent_id uuid NOT NULL,
 knowledge_item_id uuid NOT NULL, revision_id uuid NOT NULL, content_hash text NOT NULL,
 PRIMARY KEY(tenant_id,dependent_kind,dependent_id,revision_id),
 FOREIGN KEY(tenant_id,revision_id) REFERENCES public.knowledge_revision_headers(tenant_id,revision_id),
 CHECK(dependent_kind IN ('session_event','capture_candidate','knowledge_source','knowledge_revision','knowledge_change','context_run')),
 CHECK(content_hash ~ '^[0-9a-f]{64}$')
);
CREATE INDEX historical_knowledge_item ON public.historical_knowledge_dependencies(tenant_id,knowledge_item_id,dependent_kind,dependent_id);

ALTER TABLE public.procedure_outcome_headers ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.procedure_outcome_headers FORCE ROW LEVEL SECURITY;
CREATE POLICY procedure_outcome_tenant ON public.procedure_outcome_headers USING(tenant_id=public.synveda_current_tenant()) WITH CHECK(tenant_id=public.synveda_current_tenant());
ALTER TABLE public.procedure_improvement_headers ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.procedure_improvement_headers FORCE ROW LEVEL SECURITY;
CREATE POLICY procedure_improvement_tenant ON public.procedure_improvement_headers USING(tenant_id=public.synveda_current_tenant()) WITH CHECK(tenant_id=public.synveda_current_tenant());
ALTER TABLE public.procedure_candidate_headers ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.procedure_candidate_headers FORCE ROW LEVEL SECURITY;
CREATE POLICY procedure_candidate_tenant ON public.procedure_candidate_headers USING(tenant_id=public.synveda_current_tenant()) WITH CHECK(tenant_id=public.synveda_current_tenant());
ALTER TABLE public.procedure_evaluation_attempts ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.procedure_evaluation_attempts FORCE ROW LEVEL SECURITY;
CREATE POLICY procedure_attempt_tenant ON public.procedure_evaluation_attempts USING(tenant_id=public.synveda_current_tenant()) WITH CHECK(tenant_id=public.synveda_current_tenant());
ALTER TABLE public.procedure_evaluation_results ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.procedure_evaluation_results FORCE ROW LEVEL SECURITY;
CREATE POLICY procedure_result_tenant ON public.procedure_evaluation_results USING(tenant_id=public.synveda_current_tenant()) WITH CHECK(tenant_id=public.synveda_current_tenant());
ALTER TABLE public.historical_knowledge_dependencies ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.historical_knowledge_dependencies FORCE ROW LEVEL SECURITY;
CREATE POLICY historical_knowledge_tenant ON public.historical_knowledge_dependencies USING(tenant_id=public.synveda_current_tenant()) WITH CHECK(tenant_id=public.synveda_current_tenant());
GRANT SELECT ON public.procedure_outcome_headers,public.procedure_improvement_headers,public.procedure_candidate_headers,public.historical_knowledge_dependencies TO synveda_app;
GRANT SELECT,INSERT ON public.procedure_evaluation_attempts,public.procedure_evaluation_results TO synveda_app;
CREATE TRIGGER procedure_outcome_immutable BEFORE UPDATE OR DELETE ON public.procedure_outcome_headers FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_capture_append_only();
CREATE TRIGGER procedure_improvement_immutable BEFORE UPDATE OR DELETE ON public.procedure_improvement_headers FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_capture_append_only();
CREATE TRIGGER procedure_candidate_immutable BEFORE UPDATE OR DELETE ON public.procedure_candidate_headers FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_capture_append_only();
CREATE TRIGGER procedure_attempt_immutable BEFORE UPDATE OR DELETE ON public.procedure_evaluation_attempts FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_capture_append_only();
CREATE TRIGGER procedure_result_immutable BEFORE UPDATE OR DELETE ON public.procedure_evaluation_results FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_capture_append_only();
CREATE TRIGGER historical_knowledge_immutable BEFORE UPDATE OR DELETE ON public.historical_knowledge_dependencies FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_capture_append_only();

CREATE FUNCTION public.synveda_inherit_procedure_evidence(t uuid,k text,d uuid,pk text,p uuid) RETURNS void
 LANGUAGE plpgsql SECURITY DEFINER SET search_path TO 'public','pg_temp' AS $$
BEGIN
 IF t IS DISTINCT FROM public.synveda_current_tenant()
 OR k NOT IN ('session_event','capture_candidate','knowledge_source','knowledge_revision','knowledge_change','context_run')
 OR pk NOT IN ('session_event','capture_candidate','knowledge_source','knowledge_revision','knowledge_change','context_run')
 OR NOT public.synveda_source_admissible(t,pk,p) THEN
  RAISE EXCEPTION 'procedure evidence unavailable' USING ERRCODE='23514'; END IF;
 INSERT INTO public.source_dependencies SELECT t,k,d,source_event_id FROM public.source_dependencies
  WHERE tenant_id=t AND dependent_kind=pk AND dependent_id=p ON CONFLICT DO NOTHING;
 INSERT INTO public.artifact_dependencies SELECT t,k,d,source_key_digest FROM public.artifact_dependencies
  WHERE tenant_id=t AND dependent_kind=pk AND dependent_id=p ON CONFLICT DO NOTHING;
 INSERT INTO public.historical_knowledge_dependencies SELECT t,k,d,knowledge_item_id,revision_id,content_hash
  FROM public.historical_knowledge_dependencies WHERE tenant_id=t AND dependent_kind=pk AND dependent_id=p ON CONFLICT DO NOTHING;
 INSERT INTO public.historical_knowledge_dependencies SELECT t,k,d,knowledge_item_id,revision_id,content_hash
  FROM public.knowledge_copy_dependencies WHERE tenant_id=t AND dependent_kind=pk AND dependent_id=p ON CONFLICT DO NOTHING;
 IF pk='knowledge_revision' THEN
  INSERT INTO public.historical_knowledge_dependencies SELECT t,k,d,knowledge_item_id,revision_id,content_hash
   FROM public.knowledge_revision_headers WHERE tenant_id=t AND revision_id=p ON CONFLICT DO NOTHING;
 ELSIF pk='context_run' THEN
  INSERT INTO public.historical_knowledge_dependencies SELECT DISTINCT t,k,d,h.knowledge_item_id,h.revision_id,h.content_hash
   FROM public.context_run_delivery_dependencies m CROSS JOIN LATERAL (
    SELECT (x->>'revision_id')::uuid id FROM jsonb_array_elements(m.manifest->'knowledge') x
    UNION SELECT v::uuid FROM jsonb_array_elements(m.manifest->'graph') x
     CROSS JOIN LATERAL (VALUES(x->>'asserting_revision_id'),(x->>'from_revision_id'),(x->>'to_revision_id')) refs(v) WHERE v IS NOT NULL
   ) refs JOIN public.knowledge_revision_headers h ON h.tenant_id=t AND h.revision_id=refs.id
   WHERE m.tenant_id=t AND m.context_run_id=p ON CONFLICT DO NOTHING;
 END IF;
END $$;
REVOKE ALL ON FUNCTION public.synveda_inherit_procedure_evidence(uuid,text,uuid,text,uuid) FROM PUBLIC;

CREATE FUNCTION public.synveda_lock_procedure_request(t uuid,e uuid) RETURNS boolean
 LANGUAGE plpgsql SECURITY DEFINER SET search_path TO 'public','pg_temp' AS $$
BEGIN
 IF t IS DISTINCT FROM public.synveda_current_tenant() THEN RETURN false; END IF;
 PERFORM 1 FROM public.procedure_improvement_headers WHERE tenant_id=t AND event_id=e FOR UPDATE;
 RETURN FOUND;
END $$;
REVOKE ALL ON FUNCTION public.synveda_lock_procedure_request(uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.synveda_lock_procedure_request(uuid,uuid) TO synveda_app;

CREATE FUNCTION public.synveda_inherit_historical_evidence(t uuid,k text,d uuid,pk text,p uuid) RETURNS void
 LANGUAGE plpgsql SECURITY DEFINER SET search_path TO 'public','pg_temp' AS $$
BEGIN
 IF t IS DISTINCT FROM public.synveda_current_tenant() THEN
  RAISE EXCEPTION 'historical evidence tenant mismatch' USING ERRCODE='23514'; END IF;
 INSERT INTO public.historical_knowledge_dependencies SELECT t,k,d,knowledge_item_id,revision_id,content_hash
 FROM public.historical_knowledge_dependencies WHERE tenant_id=t AND dependent_kind=pk AND dependent_id=p ON CONFLICT DO NOTHING;
END $$;
REVOKE ALL ON FUNCTION public.synveda_inherit_historical_evidence(uuid,text,uuid,text,uuid) FROM PUBLIC;
CREATE OR REPLACE FUNCTION public.synveda_inherit_task_evidence(t uuid,k text,d uuid,pk text,p uuid) RETURNS void
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
 PERFORM public.synveda_inherit_historical_evidence(t,k,d,pk,p);
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
CREATE FUNCTION public.synveda_procedure_projection() RETURNS trigger
 LANGUAGE plpgsql SECURITY DEFINER SET search_path TO 'public','pg_temp' AS $$
DECLARE k text;d uuid;proof jsonb;entry jsonb;source_id uuid;item uuid;version uuid;identity text;
 h public.knowledge_revision_headers%ROWTYPE;request public.procedure_improvement_headers%ROWTYPE;
 event public.session_events%ROWTYPE;scope uuid;project uuid; reporter public.identities%ROWTYPE;
BEGIN
 IF NEW.tenant_id IS DISTINCT FROM public.synveda_current_tenant() THEN
  RAISE EXCEPTION 'procedure projection tenant mismatch' USING ERRCODE='23514'; END IF;
 IF TG_TABLE_NAME='session_events' THEN
  k='session_event';d=NEW.id;
  IF NEW.event_type NOT IN ('task.outcome','procedure.improvement') THEN
   IF NEW.event_type NOT IN ('session.task_state','session.checkpoint') THEN RETURN NEW; END IF;
  ELSE
  proof=NEW.payload->'procedure';item=(proof->>'item_id')::uuid;version=(proof->>'revision_id')::uuid;
  SELECT * INTO h FROM public.knowledge_revision_headers WHERE tenant_id=NEW.tenant_id AND revision_id=version;
  SELECT s.scope_id,s.project_id INTO scope,project FROM public.sessions s WHERE s.tenant_id=NEW.tenant_id AND s.id=NEW.session_id;
  IF NEW.payload->>'schema_version' IS DISTINCT FROM '1' OR h.knowledge_item_id IS DISTINCT FROM item
    OR h.content_hash IS DISTINCT FROM proof->>'content_hash'
    OR NOT EXISTS(SELECT 1 FROM public.knowledge_items i WHERE i.tenant_id=NEW.tenant_id AND i.id=item
      AND i.current_revision_id=version AND i.lifecycle_state='active' AND i.knowledge_type='procedure'
      AND i.scope_id=scope AND i.project_id IS NOT DISTINCT FROM project)
    OR NOT coalesce(NEW.payload->>'request_hash' ~ '^[0-9a-f]{64}$',false) THEN
   RAISE EXCEPTION 'procedure baseline proof mismatch' USING ERRCODE='23514'; END IF;
  PERFORM public.synveda_inherit_procedure_evidence(NEW.tenant_id,k,d,'knowledge_revision',version);
  IF NEW.event_type='task.outcome' THEN
   SELECT * INTO reporter FROM public.identities WHERE tenant_id=NEW.tenant_id AND id=(NEW.payload->>'reporter_id')::uuid;
   IF reporter.id IS NULL OR NEW.payload->>'evidence_class' NOT IN ('client_reported','authenticated_runner')
    OR (NEW.payload->>'evidence_class'='authenticated_runner' AND reporter.kind<>'service')
    OR NEW.payload->>'outcome' NOT IN ('succeeded','failed','unknown')
    OR NOT coalesce(NEW.payload#>>'{environment,environment_digest}' ~ '^[0-9a-f]{64}$',false)
    OR jsonb_typeof(NEW.payload->'supporting_events') IS DISTINCT FROM 'array'
    OR jsonb_array_length(NEW.payload->'supporting_events')>8
    OR jsonb_typeof(NEW.payload->'skill_versions') IS DISTINCT FROM 'array'
    OR jsonb_array_length(NEW.payload->'skill_versions')>8 THEN
    RAISE EXCEPTION 'invalid outcome custody' USING ERRCODE='23514'; END IF;
   INSERT INTO public.procedure_outcome_headers VALUES(NEW.tenant_id,NEW.id,NEW.session_id,item,version,h.content_hash,
     NEW.payload->>'request_hash',reporter.id,NEW.payload->>'evidence_class',NEW.payload->>'outcome',NEW.payload#>>'{environment,environment_digest}',ARRAY(SELECT v::uuid FROM jsonb_array_elements_text(NEW.payload->'skill_versions') v),(NEW.payload#>>'{context,context_run_id}')::uuid,NEW.payload#>>'{context,block_hash}',NEW.received_at);
   FOR entry IN SELECT x FROM jsonb_array_elements(NEW.payload->'supporting_events') x LOOP
    source_id=(entry->>'event_id')::uuid;
    SELECT * INTO event FROM public.session_events WHERE tenant_id=NEW.tenant_id AND id=source_id AND session_id=NEW.session_id;
    IF event.id IS NULL OR event.sequence>=NEW.sequence OR event.payload_hash IS DISTINCT FROM entry->>'payload_hash'
      OR event.event_type NOT IN ('message.user','message.assistant','tool.result','command.executed','file.changed') THEN
      RAISE EXCEPTION 'outcome source proof mismatch' USING ERRCODE='23514'; END IF;
    PERFORM public.synveda_inherit_procedure_evidence(NEW.tenant_id,k,d,'session_event',source_id);
   END LOOP;
   proof=NEW.payload->'context';
   IF proof IS NOT NULL AND proof<>'null' THEN
    source_id=(proof->>'context_run_id')::uuid;
    IF NOT EXISTS(SELECT 1 FROM public.session_context_runs r JOIN public.context_run_delivery_dependencies m
      ON m.tenant_id=r.tenant_id AND m.context_run_id=r.id WHERE r.tenant_id=NEW.tenant_id AND r.id=source_id
      AND r.session_id=NEW.session_id AND r.block_hash=proof->>'block_hash' AND m.dependency_version IN (2,3)) THEN
      RAISE EXCEPTION 'outcome context proof mismatch' USING ERRCODE='23514'; END IF;
    PERFORM public.synveda_inherit_procedure_evidence(NEW.tenant_id,k,d,'context_run',source_id);
   END IF;
  ELSE
   IF NOT coalesce(NEW.payload->>'identity_hash' ~ '^[0-9a-f]{64}$',false)
    OR NOT coalesce(NEW.payload->>'content_hash' ~ '^[0-9a-f]{64}$',false)
    OR octet_length(NEW.payload->>'proposed_append') NOT BETWEEN 1 AND 1024
    OR jsonb_typeof(NEW.payload->'outcomes') IS DISTINCT FROM 'array'
    OR jsonb_array_length(NEW.payload->'outcomes') NOT BETWEEN 1 AND 8 THEN
    RAISE EXCEPTION 'invalid improvement custody' USING ERRCODE='23514'; END IF;
   INSERT INTO public.procedure_improvement_headers VALUES(NEW.tenant_id,NEW.id,NEW.session_id,NEW.payload->>'identity_hash',
     NEW.payload->>'request_hash',item,version,h.content_hash,NEW.payload->>'content_hash');
   FOR entry IN SELECT x FROM jsonb_array_elements(NEW.payload->'outcomes') x LOOP
    source_id=(entry->>'event_id')::uuid;
    SELECT * INTO event FROM public.session_events WHERE tenant_id=NEW.tenant_id AND id=source_id AND session_id=NEW.session_id;
    IF event.id IS NULL OR event.sequence>=NEW.sequence OR event.payload_hash IS DISTINCT FROM entry->>'payload_hash'
      OR NOT EXISTS(SELECT 1 FROM public.procedure_outcome_headers o WHERE o.tenant_id=NEW.tenant_id AND o.event_id=source_id
        AND o.evidence_class='authenticated_runner' AND o.procedure_item_id=item AND o.procedure_revision_id=version) THEN
      RAISE EXCEPTION 'improvement outcome proof mismatch' USING ERRCODE='23514'; END IF;
    PERFORM public.synveda_inherit_procedure_evidence(NEW.tenant_id,k,d,'session_event',source_id);
   END LOOP;
  END IF;
  END IF;
 ELSIF TG_TABLE_NAME='capture_candidates' THEN
  k='capture_candidate';d=NEW.id;identity=NEW.metadata->>'synveda_procedure_improvement';
  IF identity IS NULL THEN RETURN NEW; END IF;
  SELECT * INTO request FROM public.procedure_improvement_headers WHERE tenant_id=NEW.tenant_id AND identity_hash=identity FOR UPDATE;
  IF request.event_id IS NULL OR request.content_hash IS DISTINCT FROM NEW.content_hash OR NEW.knowledge_type<>'procedure'
   OR NOT EXISTS(SELECT 1 FROM public.capture_batches b JOIN public.capture_batch_events e
    ON e.tenant_id=b.tenant_id AND e.batch_id=b.id WHERE b.tenant_id=NEW.tenant_id AND b.id=NEW.batch_id
     AND b.session_id=request.session_id AND e.event_id=request.event_id
     AND b.scope_id=NEW.proposed_scope_id AND b.project_id IS NOT DISTINCT FROM NEW.proposed_project_id) THEN
    RAISE EXCEPTION 'learning candidate proof mismatch' USING ERRCODE='23514'; END IF;
  INSERT INTO public.procedure_candidate_headers VALUES(NEW.tenant_id,NEW.id,request.event_id,request.content_hash);
  PERFORM public.synveda_inherit_procedure_evidence(NEW.tenant_id,k,d,'session_event',request.event_id);
 ELSIF TG_TABLE_NAME='capture_candidate_events' THEN
  k='capture_candidate';d=NEW.candidate_id;
  PERFORM public.synveda_inherit_procedure_evidence(NEW.tenant_id,k,d,'session_event',NEW.event_id);
 ELSIF TG_TABLE_NAME='knowledge_sources' THEN
  k='knowledge_source';d=NEW.id;
  IF NEW.session_event_id IS NOT NULL THEN
   PERFORM public.synveda_inherit_procedure_evidence(NEW.tenant_id,k,d,'session_event',NEW.session_event_id);
  END IF;
 ELSIF TG_TABLE_NAME='knowledge_revision_sources' THEN
  k='knowledge_revision';d=NEW.knowledge_revision_id;
  PERFORM public.synveda_inherit_procedure_evidence(NEW.tenant_id,k,d,'knowledge_source',NEW.knowledge_source_id);
 ELSIF TG_TABLE_NAME='knowledge_changes' THEN
  k='knowledge_change';d=NEW.proposal_id;
  IF NEW.payload IS NULL THEN RETURN NEW; END IF;
  IF NEW.command_kind IN ('control_source','control_artifact','forget') THEN RETURN NEW; END IF;
  IF NEW.command_kind='verify' THEN
   PERFORM public.synveda_inherit_historical_evidence(NEW.tenant_id,k,d,'knowledge_revision',(NEW.payload->>'expected_revision_id')::uuid);
  ELSIF NEW.command_kind='merge' THEN
   FOR source_id IN SELECT (x->>'revision_id')::uuid FROM jsonb_array_elements(NEW.payload->'inputs') x LOOP
    PERFORM public.synveda_inherit_historical_evidence(NEW.tenant_id,k,d,'knowledge_revision',source_id);
   END LOOP;
  END IF;
  FOR entry IN SELECT x FROM jsonb_array_elements(coalesce(NEW.payload->'sources','[]')) x LOOP
   source_id=(entry->>'session_event_id')::uuid;
   IF source_id IS NOT NULL THEN PERFORM public.synveda_inherit_procedure_evidence(NEW.tenant_id,k,d,'session_event',source_id); END IF;
  END LOOP;
 ELSIF TG_TABLE_NAME='context_run_delivery_dependencies' THEN
  k='context_run';d=NEW.context_run_id;
  FOR source_id IN SELECT (x->>'revision_id')::uuid FROM jsonb_array_elements(NEW.manifest->'knowledge') x
   UNION SELECT v::uuid FROM jsonb_array_elements(NEW.manifest->'graph') x
    CROSS JOIN LATERAL (VALUES(x->>'asserting_revision_id'),(x->>'from_revision_id'),(x->>'to_revision_id')) refs(v) WHERE v IS NOT NULL LOOP
   PERFORM public.synveda_inherit_historical_evidence(NEW.tenant_id,k,d,'knowledge_revision',source_id);
  END LOOP;
  FOR source_id IN SELECT (x->>'candidate_id')::uuid FROM jsonb_array_elements(NEW.manifest->'candidates') x LOOP
   PERFORM public.synveda_inherit_historical_evidence(NEW.tenant_id,k,d,'capture_candidate',source_id);
  END LOOP;
  FOR source_id IN SELECT x::uuid FROM jsonb_array_elements_text(NEW.manifest->'restart_source_event_ids') x LOOP
   PERFORM public.synveda_inherit_procedure_evidence(NEW.tenant_id,k,d,'session_event',source_id);
  END LOOP;
 ELSE RAISE EXCEPTION 'unsupported procedure projection' USING ERRCODE='23514'; END IF;
 IF (SELECT count(*) FROM(SELECT 1 FROM public.historical_knowledge_dependencies WHERE tenant_id=NEW.tenant_id AND dependent_kind=k AND dependent_id=d LIMIT 65) bounded)>64 THEN
  RAISE EXCEPTION 'historical procedure fan-in exhausted' USING ERRCODE='23514'; END IF;
 FOR item IN SELECT DISTINCT knowledge_item_id FROM public.historical_knowledge_dependencies
   WHERE tenant_id=NEW.tenant_id AND dependent_kind=k AND dependent_id=d ORDER BY knowledge_item_id LOOP
  PERFORM 1 FROM public.knowledge_items WHERE tenant_id=NEW.tenant_id AND id=item FOR UPDATE;
  IF (SELECT count(*) FROM(SELECT 1 FROM public.historical_knowledge_dependencies WHERE tenant_id=NEW.tenant_id AND knowledge_item_id=item LIMIT 4097) bounded)>4096 THEN
   RAISE EXCEPTION 'historical procedure copy bound exhausted' USING ERRCODE='23514'; END IF;
 END LOOP;
 IF (SELECT count(*) FROM(SELECT 1 FROM public.source_dependencies WHERE tenant_id=NEW.tenant_id AND dependent_kind=k AND dependent_id=d LIMIT 4097) bounded)>4096
 OR (SELECT count(*) FROM(SELECT 1 FROM public.artifact_dependencies WHERE tenant_id=NEW.tenant_id AND dependent_kind=k AND dependent_id=d LIMIT 4097) bounded)>4096 THEN
  RAISE EXCEPTION 'procedure source roots bound exhausted' USING ERRCODE='23514'; END IF;
 IF (SELECT count(*) FROM(SELECT DISTINCT skill.skill_version_id FROM public.source_dependencies dep
 JOIN public.procedure_outcome_headers header ON header.tenant_id=dep.tenant_id AND header.event_id=dep.source_event_id
 CROSS JOIN LATERAL unnest(header.skill_versions) skill(skill_version_id)
 WHERE dep.tenant_id=NEW.tenant_id AND dep.dependent_kind=k AND dep.dependent_id=d LIMIT 65) bounded)>64 THEN
  RAISE EXCEPTION 'procedure Skill authority bound exhausted' USING ERRCODE='23514'; END IF;
 IF NOT public.synveda_source_admissible(NEW.tenant_id,k,d) THEN
  RAISE EXCEPTION 'procedure evidence became unavailable' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION public.synveda_procedure_projection() FROM PUBLIC;
CREATE TRIGGER zzzzzz_procedure_projection AFTER INSERT ON public.session_events FOR EACH ROW EXECUTE FUNCTION public.synveda_procedure_projection();
CREATE TRIGGER zzzzzz_procedure_projection AFTER INSERT ON public.capture_candidates FOR EACH ROW EXECUTE FUNCTION public.synveda_procedure_projection();
CREATE TRIGGER zzzzzz_procedure_projection AFTER INSERT ON public.capture_candidate_events FOR EACH ROW EXECUTE FUNCTION public.synveda_procedure_projection();
CREATE TRIGGER zzzzzz_procedure_projection AFTER INSERT ON public.knowledge_sources FOR EACH ROW EXECUTE FUNCTION public.synveda_procedure_projection();
CREATE TRIGGER zzzzzz_procedure_projection AFTER INSERT ON public.knowledge_revision_sources FOR EACH ROW EXECUTE FUNCTION public.synveda_procedure_projection();
CREATE TRIGGER zzzzzz_procedure_projection AFTER INSERT ON public.knowledge_changes FOR EACH ROW EXECUTE FUNCTION public.synveda_procedure_projection();
CREATE TRIGGER zzzzzz_procedure_projection AFTER INSERT ON public.context_run_delivery_dependencies FOR EACH ROW EXECUTE FUNCTION public.synveda_procedure_projection();

-- Historical eligibility preserves superseded evidence but never item erasure.
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
 OR header.content_hash IS DISTINCT FROM copy.content_hash)) AND NOT EXISTS(SELECT 1 FROM public.historical_knowledge_dependencies dep
 LEFT JOIN public.knowledge_items item ON item.tenant_id=dep.tenant_id AND item.id=dep.knowledge_item_id
 LEFT JOIN public.knowledge_revision_headers header ON header.tenant_id=dep.tenant_id AND header.revision_id=dep.revision_id
 WHERE dep.tenant_id=t AND dep.dependent_kind=k AND dep.dependent_id=d
 AND (item.id IS NULL OR item.lifecycle_state='erasure_pending'
 OR header.knowledge_item_id IS DISTINCT FROM dep.knowledge_item_id
 OR header.content_hash IS DISTINCT FROM dep.content_hash)))
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
 OR header.content_hash IS DISTINCT FROM copy.content_hash)) AND NOT EXISTS(SELECT 1 FROM public.historical_knowledge_dependencies dep
 LEFT JOIN public.knowledge_items item ON item.tenant_id=dep.tenant_id AND item.id=dep.knowledge_item_id
 LEFT JOIN public.knowledge_revision_headers header ON header.tenant_id=dep.tenant_id AND header.revision_id=dep.revision_id
 WHERE dep.tenant_id=t AND dep.dependent_kind=k AND dep.dependent_id=d
 AND (item.id IS NULL OR item.lifecycle_state='erasure_pending'
 OR header.knowledge_item_id IS DISTINCT FROM dep.knowledge_item_id
 OR header.content_hash IS DISTINCT FROM dep.content_hash))
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
$$;
CREATE OR REPLACE FUNCTION public.synveda_source_content_erasure() RETURNS trigger
LANGUAGE plpgsql SET search_path TO 'public','pg_temp' AS $$
DECLARE k text;
BEGIN
 k=CASE TG_TABLE_NAME WHEN 'knowledge_revisions' THEN 'knowledge_revision'
      WHEN 'session_events' THEN 'session_event' WHEN 'knowledge_relations' THEN 'knowledge_revision' ELSE 'knowledge_source' END;
 IF NOT (public.synveda_source_cleanup_authorized(OLD.tenant_id,k,CASE WHEN TG_TABLE_NAME='knowledge_relations' THEN (to_jsonb(OLD)->>'asserting_revision_id')::uuid ELSE OLD.id END) OR public.synveda_artifact_cleanup_authorized(OLD.tenant_id,k,CASE WHEN TG_TABLE_NAME='knowledge_relations' THEN (to_jsonb(OLD)->>'asserting_revision_id')::uuid ELSE OLD.id END) OR public.synveda_item_cleanup_authorized(OLD.tenant_id,k,CASE WHEN TG_TABLE_NAME='knowledge_relations' THEN (to_jsonb(OLD)->>'asserting_revision_id')::uuid ELSE OLD.id END))
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

    if exists(select 1 from public.knowledge_revision_headers h where h.tenant_id=wanted_tenant
      and (h.knowledge_item_id=wanted_item or exists(select 1 from public.historical_knowledge_dependencies dep
       where dep.tenant_id=h.tenant_id and dep.knowledge_item_id=wanted_item
        and dep.dependent_kind='knowledge_revision' and dep.dependent_id=h.revision_id))
      and (h.legal_hold or h.retention_hold)) then
      raise exception 'Knowledge erasure is held by retained evidence' using errcode='23514'; end if;
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
    -- Historical copies have an independent bounded index. A clean later
    -- head survives; only exact revisions derived from this item are cleared.
    update public.session_events event set content_erased=true,payload='{}',redactions=null
     where event.tenant_id=wanted_tenant and not event.content_erased and exists(select 1
      from public.historical_knowledge_dependencies dep where dep.tenant_id=event.tenant_id
       and dep.dependent_kind='session_event' and dep.dependent_id=event.id and dep.knowledge_item_id=wanted_item);
    update public.session_context_runs run set rendered='' where run.tenant_id=wanted_tenant
     and run.rendered<>'' and exists(select 1 from public.historical_knowledge_dependencies dep
      where dep.tenant_id=run.tenant_id and dep.dependent_kind='context_run' and dep.dependent_id=run.id and dep.knowledge_item_id=wanted_item);
    update public.capture_candidate_decisions decision set payload=null where decision.tenant_id=wanted_tenant
     and decision.payload is not null and exists(select 1 from public.historical_knowledge_dependencies dep
      where dep.tenant_id=decision.tenant_id and dep.dependent_kind='capture_candidate' and dep.dependent_id=decision.candidate_id and dep.knowledge_item_id=wanted_item);
    update public.capture_candidates candidate set content_erased=true,title='',body_markdown='',summary='',decision_reason=null,tags='{}',metadata='{}',verification_metadata='{}'
     where candidate.tenant_id=wanted_tenant and not candidate.content_erased and exists(select 1 from public.historical_knowledge_dependencies dep
      where dep.tenant_id=candidate.tenant_id and dep.dependent_kind='capture_candidate' and dep.dependent_id=candidate.id and dep.knowledge_item_id=wanted_item);
    update public.vedaflow_proposals proposal set state='rejected',closed_at=clock_timestamp(),updated_at=clock_timestamp(),close_reason='source_unavailable',
     closed_by=(select closed_by from public.vedaflow_proposals where tenant_id=wanted_tenant and id=wanted_proposal)
     where proposal.tenant_id=wanted_tenant and proposal.state='open' and exists(select 1 from public.historical_knowledge_dependencies dep
      where dep.tenant_id=proposal.tenant_id and dep.dependent_kind='knowledge_change' and dep.dependent_id=proposal.id and dep.knowledge_item_id=wanted_item);
    update public.knowledge_changes change set payload=null where change.tenant_id=wanted_tenant and change.payload is not null
     and exists(select 1 from public.historical_knowledge_dependencies dep where dep.tenant_id=change.tenant_id
      and dep.dependent_kind='knowledge_change' and dep.dependent_id=change.proposal_id and dep.knowledge_item_id=wanted_item);
    insert into public.knowledge_index_invalidations(tenant_id,operation_id,revision_id,content_hash)
     select h.tenant_id,wanted_operation,h.revision_id,h.content_hash from public.knowledge_revision_headers h
      join public.historical_knowledge_dependencies dep on dep.tenant_id=h.tenant_id and dep.dependent_kind='knowledge_revision' and dep.dependent_id=h.revision_id
      where dep.tenant_id=wanted_tenant and dep.knowledge_item_id=wanted_item on conflict do nothing;
    delete from public.knowledge_revision_embeddings embedding using public.historical_knowledge_dependencies dep
     where dep.tenant_id=wanted_tenant and dep.knowledge_item_id=wanted_item and dep.dependent_kind='knowledge_revision'
      and embedding.tenant_id=dep.tenant_id and embedding.knowledge_revision_id=dep.dependent_id;
    update public.knowledge_relations relation set content_erased=true,metadata='{}' where relation.tenant_id=wanted_tenant
     and not relation.content_erased and exists(select 1 from public.historical_knowledge_dependencies dep where dep.tenant_id=relation.tenant_id
      and dep.dependent_kind='knowledge_revision' and dep.dependent_id=relation.asserting_revision_id and dep.knowledge_item_id=wanted_item);
    update public.knowledge_revisions revision set content_erased=true,title='[erased]',body_markdown='[erased]',summary='[erased]',tags='{}',metadata='{}',verification_metadata='{}'
     where revision.tenant_id=wanted_tenant and not revision.content_erased and exists(select 1 from public.historical_knowledge_dependencies dep where dep.tenant_id=revision.tenant_id
      and dep.dependent_kind='knowledge_revision' and dep.dependent_id=revision.id and dep.knowledge_item_id=wanted_item);
    update public.knowledge_sources source set content_erased=true,locator=null,source_revision=null,metadata='{}'
     where source.tenant_id=wanted_tenant and not source.content_erased and exists(select 1 from public.historical_knowledge_dependencies dep where dep.tenant_id=source.tenant_id
      and dep.dependent_kind='knowledge_source' and dep.dependent_id=source.id and dep.knowledge_item_id=wanted_item);
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
      AND event_type IN ('session.task_state','session.checkpoint','task.outcome')) THEN
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
       WHERE tenant_id=NEW.tenant_id AND id=source_id AND event_type IN ('session.task_state','session.checkpoint','task.outcome')) THEN
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

-- The immutable result may qualify only the fixed case set and evaluator that
-- were frozen before observation. Application approval cannot change this gate.
CREATE FUNCTION public.synveda_procedure_evaluation_guard() RETURNS trigger
 LANGUAGE plpgsql SECURITY DEFINER SET search_path TO 'public','pg_temp' AS $$
DECLARE attempt public.procedure_evaluation_attempts%ROWTYPE;
 request_id uuid; candidate_hash text; entry jsonb; result jsonb; qualified boolean=true;
BEGIN
 IF NEW.tenant_id IS DISTINCT FROM public.synveda_current_tenant() THEN
  RAISE EXCEPTION 'evaluation tenant mismatch' USING ERRCODE='23514'; END IF;
 IF TG_TABLE_NAME='procedure_evaluation_attempts' THEN
  SELECT request_event_id,content_hash INTO request_id,candidate_hash FROM public.procedure_candidate_headers
   WHERE tenant_id=NEW.tenant_id AND candidate_id=NEW.candidate_id;
  PERFORM 1 FROM public.procedure_improvement_headers WHERE tenant_id=NEW.tenant_id AND event_id=request_id FOR UPDATE;
  IF request_id IS NULL OR candidate_hash IS DISTINCT FROM NEW.candidate_hash
    OR NOT public.synveda_source_admissible(NEW.tenant_id,'capture_candidate',NEW.candidate_id)
    OR (SELECT count(*) FROM public.procedure_evaluation_attempts WHERE tenant_id=NEW.tenant_id AND candidate_id=NEW.candidate_id)>=8
    OR jsonb_typeof(NEW.specification) IS DISTINCT FROM 'object'
    OR NEW.specification-ARRAY['suite_id','evaluator_id','harness','harness_version','configuration_digest','cases']<>'{}'
    OR NOT coalesce(NEW.specification->>'suite_id' ~ '^[A-Za-z0-9_.-]{1,128}$',false)
    OR NOT coalesce(NEW.specification->>'harness' ~ '^[A-Za-z0-9_.-]{1,128}$',false)
    OR NOT coalesce(NEW.specification->>'harness_version' ~ '^[A-Za-z0-9_.-]{1,128}$',false)
    OR NOT coalesce(NEW.specification->>'configuration_digest' ~ '^[0-9a-f]{64}$',false)
    OR NEW.specification->>'suite_id' IS DISTINCT FROM NEW.suite_id
    OR NEW.specification->>'evaluator_id' IS DISTINCT FROM NEW.evaluator_id::text
    OR jsonb_typeof(NEW.specification->'cases') IS DISTINCT FROM 'array'
    OR jsonb_array_length(NEW.specification->'cases') NOT BETWEEN 1 AND 32
    OR NOT EXISTS(SELECT 1 FROM jsonb_array_elements(NEW.specification->'cases') x WHERE x->'required'='true'::jsonb) THEN
   RAISE EXCEPTION 'invalid frozen evaluation' USING ERRCODE='23514'; END IF;
  IF (SELECT count(DISTINCT x->>'case_id') FROM jsonb_array_elements(NEW.specification->'cases') x)<>jsonb_array_length(NEW.specification->'cases') THEN
   RAISE EXCEPTION 'ambiguous fixed cases' USING ERRCODE='23514'; END IF;
  FOR entry IN SELECT x FROM jsonb_array_elements(NEW.specification->'cases') x LOOP
   IF jsonb_typeof(entry) IS DISTINCT FROM 'object' OR entry-ARRAY['case_id','digest','required']<>'{}'
    OR jsonb_typeof(entry->'required') IS DISTINCT FROM 'boolean'
    OR NOT coalesce(entry->>'case_id' ~ '^[A-Za-z0-9_.-]{1,128}$',false)
    OR NOT coalesce(entry->>'digest' ~ '^[0-9a-f]{64}$',false) THEN
    RAISE EXCEPTION 'invalid fixed case shape' USING ERRCODE='23514'; END IF;
  END LOOP;
 ELSE
  SELECT * INTO attempt FROM public.procedure_evaluation_attempts WHERE tenant_id=NEW.tenant_id AND id=NEW.attempt_id;
  IF attempt.id IS NULL OR attempt.evaluator_id IS DISTINCT FROM NEW.reporter_id
   OR NOT public.synveda_source_admissible(NEW.tenant_id,'capture_candidate',attempt.candidate_id)
   OR jsonb_array_length(NEW.results)<>jsonb_array_length(attempt.specification->'cases')
   OR (SELECT count(DISTINCT x->>'case_id') FROM jsonb_array_elements(NEW.results) x)<>jsonb_array_length(NEW.results) THEN
   RAISE EXCEPTION 'invalid controlled result custody' USING ERRCODE='23514'; END IF;
  FOR entry IN SELECT x FROM jsonb_array_elements(attempt.specification->'cases') x LOOP
   SELECT x INTO result FROM jsonb_array_elements(NEW.results) x WHERE x->>'case_id'=entry->>'case_id';
   IF result IS NULL OR jsonb_typeof(result) IS DISTINCT FROM 'object'
     OR result-ARRAY['case_id','digest','baseline','candidate','result_digest']<>'{}'
     OR result->>'digest' IS DISTINCT FROM entry->>'digest'
     OR NOT coalesce(result->>'result_digest' ~ '^[0-9a-f]{64}$',false)
     OR NOT coalesce(result->>'baseline' IN ('passed','failed','unknown'),false)
     OR NOT coalesce(result->>'candidate' IN ('passed','failed','unknown'),false) THEN
    RAISE EXCEPTION 'fixed case identity mismatch' USING ERRCODE='23514'; END IF;
   IF (entry->>'required'='true' AND result->>'candidate'<>'passed')
    OR (result->>'baseline'='passed' AND result->>'candidate'<>'passed') OR result->>'baseline'='unknown' THEN qualified=false; END IF;
  END LOOP;
  IF NEW.qualified IS DISTINCT FROM qualified THEN
   RAISE EXCEPTION 'fixed quality gate mismatch' USING ERRCODE='23514'; END IF;
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION public.synveda_procedure_evaluation_guard() FROM PUBLIC;
CREATE TRIGGER procedure_evaluation_guard BEFORE INSERT ON public.procedure_evaluation_attempts FOR EACH ROW EXECUTE FUNCTION public.synveda_procedure_evaluation_guard();
CREATE TRIGGER procedure_evaluation_guard BEFORE INSERT ON public.procedure_evaluation_results FOR EACH ROW EXECUTE FUNCTION public.synveda_procedure_evaluation_guard();
