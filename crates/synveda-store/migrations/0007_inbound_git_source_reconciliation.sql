-- MEM-9 / ADR-0148: bounded operational observations; plaintext stays in OKF.
ALTER TABLE public.tenant_secrets DROP CONSTRAINT tenant_secrets_kind_check;
ALTER TABLE public.tenant_secrets ADD CONSTRAINT tenant_secrets_kind_check
 CHECK (kind IN ('directory','tool_server','model_provider','import_export','source_read'));
ALTER TABLE public.durable_operations DROP CONSTRAINT durable_operations_kind_check;
ALTER TABLE public.durable_operations ADD CONSTRAINT durable_operations_kind_check
 CHECK (kind IN ('knowledge_erasure','skill_validation','source_cleanup','source_reconcile'));
ALTER TABLE public.durable_operations DROP CONSTRAINT durable_operations_target_check;
ALTER TABLE public.durable_operations ADD CONSTRAINT durable_operations_target_check CHECK (
 (kind='knowledge_erasure' AND proposal_id IS NOT NULL AND knowledge_item_id IS NOT NULL
  AND skill_version_id IS NULL AND requested_by_identity_id IS NULL AND cancel_requested_at IS NULL)
 OR (kind='skill_validation' AND proposal_id IS NULL AND knowledge_item_id IS NULL
  AND skill_version_id IS NOT NULL AND requested_by_identity_id IS NOT NULL)
 OR (kind='source_cleanup' AND proposal_id IS NOT NULL AND knowledge_item_id IS NULL
  AND skill_version_id IS NULL AND requested_by_identity_id IS NULL AND cancel_requested_at IS NULL)
 OR (kind='source_reconcile' AND proposal_id IS NULL AND knowledge_item_id IS NULL
  AND skill_version_id IS NULL AND requested_by_identity_id IS NOT NULL));

CREATE TABLE public.source_reconcile_heads (
 tenant_id uuid NOT NULL, project_id uuid NOT NULL, scope_id uuid NOT NULL,
 source_id text NOT NULL, repository_id uuid NOT NULL,
 repository_identity_digest text NOT NULL,
 observation_id uuid, active_operation_id uuid, last_operation_id uuid,
 PRIMARY KEY (tenant_id,project_id,source_id),
 FOREIGN KEY (tenant_id,project_id) REFERENCES public.projects(tenant_id,id),
 FOREIGN KEY (tenant_id,scope_id) REFERENCES public.scopes(tenant_id,id),
 FOREIGN KEY (tenant_id,active_operation_id) REFERENCES public.durable_operations(tenant_id,id),
 FOREIGN KEY (tenant_id,last_operation_id) REFERENCES public.durable_operations(tenant_id,id),
 CHECK (source_id ~ '^[a-z0-9][a-z0-9-]{0,47}$'),
 CHECK (repository_identity_digest ~ '^[0-9a-f]{64}$')
);
CREATE TABLE public.source_reconcile_requests (
 tenant_id uuid NOT NULL, operation_id uuid NOT NULL, project_id uuid NOT NULL,
 source_id text NOT NULL, repository_id uuid NOT NULL,
 configuration_version_id uuid NOT NULL, descriptor_hash text NOT NULL,
 custody_digest text NOT NULL, credential_revision bigint,
 previous_observation_id uuid, path_count integer NOT NULL,
 commit_oid text, tree_oid text,
 PRIMARY KEY (tenant_id,operation_id),
 FOREIGN KEY (tenant_id,operation_id) REFERENCES public.durable_operations(tenant_id,id),
 FOREIGN KEY (tenant_id,project_id,source_id) REFERENCES public.source_reconcile_heads(tenant_id,project_id,source_id),
 FOREIGN KEY (tenant_id,configuration_version_id) REFERENCES public.configuration_versions(tenant_id,id),
 CHECK (descriptor_hash ~ '^[0-9a-f]{64}$'), CHECK (custody_digest ~ '^[0-9a-f]{64}$'),
 CHECK (credential_revision IS NULL OR credential_revision>0), CHECK (path_count BETWEEN 1 AND 32),
 CHECK ((commit_oid IS NULL AND tree_oid IS NULL) OR
  (commit_oid IS NOT NULL AND tree_oid IS NOT NULL AND commit_oid ~ '^[0-9a-f]{40}$' AND tree_oid ~ '^[0-9a-f]{40}$'))
);
CREATE TABLE public.source_observations (
 tenant_id uuid NOT NULL, id uuid NOT NULL, project_id uuid NOT NULL, source_id text NOT NULL,
 operation_id uuid NOT NULL, previous_id uuid, descriptor_hash text NOT NULL,
 commit_oid text NOT NULL, tree_oid text NOT NULL, observed_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 import_job_id uuid,
 PRIMARY KEY (tenant_id,id), UNIQUE (tenant_id,operation_id),
 FOREIGN KEY (tenant_id,project_id,source_id) REFERENCES public.source_reconcile_heads(tenant_id,project_id,source_id),
 FOREIGN KEY (tenant_id,operation_id) REFERENCES public.source_reconcile_requests(tenant_id,operation_id),
 FOREIGN KEY (tenant_id,previous_id) REFERENCES public.source_observations(tenant_id,id),
 FOREIGN KEY (tenant_id,import_job_id) REFERENCES public.import_jobs(tenant_id,id),
 CHECK (descriptor_hash ~ '^[0-9a-f]{64}$'), CHECK (commit_oid ~ '^[0-9a-f]{40}$'), CHECK (tree_oid ~ '^[0-9a-f]{40}$')
);
CREATE TABLE public.source_observation_paths (
 tenant_id uuid NOT NULL, observation_id uuid NOT NULL, path_digest text NOT NULL,
 state text NOT NULL, blob_oid text, content_hash text, artifact_id uuid,
 PRIMARY KEY (tenant_id,observation_id,path_digest),
 FOREIGN KEY (tenant_id,observation_id) REFERENCES public.source_observations(tenant_id,id),
 FOREIGN KEY (tenant_id,artifact_id) REFERENCES public.source_artifact_addresses(tenant_id,artifact_id),
 CHECK (path_digest ~ '^[0-9a-f]{64}$'), CHECK (state IN ('present','missing','suppressed')),
 CHECK ((state='missing' AND blob_oid IS NULL AND content_hash IS NULL AND artifact_id IS NULL)
  OR (state IN ('present','suppressed') AND blob_oid IS NOT NULL AND content_hash IS NOT NULL
    AND blob_oid ~ '^[0-9a-f]{40}$' AND content_hash ~ '^[0-9a-f]{64}$'))
);
CREATE INDEX source_observation_artifact ON public.source_observation_paths(tenant_id,artifact_id,observation_id)
 WHERE artifact_id IS NOT NULL;
ALTER TABLE public.source_reconcile_heads ADD FOREIGN KEY (tenant_id,observation_id) REFERENCES public.source_observations(tenant_id,id);
ALTER TABLE public.source_reconcile_requests ADD FOREIGN KEY (tenant_id,previous_observation_id) REFERENCES public.source_observations(tenant_id,id);
ALTER TABLE public.import_jobs ADD COLUMN managed_source_id text;
ALTER TABLE public.import_jobs ADD COLUMN managed_review_digest text;
ALTER TABLE public.import_jobs ADD CONSTRAINT import_managed_source_shape CHECK
 ((managed_source_id IS NULL AND managed_review_digest IS NULL) OR
  (managed_source_id IS NOT NULL AND managed_source_id ~ '^[a-z0-9][a-z0-9-]{0,47}$' AND managed_review_digest IS NOT NULL
   AND managed_review_digest ~ '^[0-9a-f]{64}$'
   AND repository_id IS NOT NULL AND repository_identity_digest IS NOT NULL));
ALTER TABLE public.import_jobs ADD FOREIGN KEY (tenant_id,project_id,managed_source_id)
 REFERENCES public.source_reconcile_heads(tenant_id,project_id,source_id);
ALTER TABLE public.import_jobs DROP CONSTRAINT import_jobs_source_digest_unique;
ALTER TABLE public.import_jobs ADD CONSTRAINT import_jobs_source_digest_unique UNIQUE NULLS NOT DISTINCT
 (tenant_id,project_id,source_kind,source_locator,bundle_digest,repository_identity_digest,managed_source_id,managed_review_digest);

ALTER TABLE public.source_reconcile_heads ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.source_reconcile_heads FORCE ROW LEVEL SECURITY;
CREATE POLICY source_reconcile_head_tenant ON public.source_reconcile_heads
 USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
ALTER TABLE public.source_reconcile_requests ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.source_reconcile_requests FORCE ROW LEVEL SECURITY;
CREATE POLICY source_reconcile_request_tenant ON public.source_reconcile_requests
 USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
ALTER TABLE public.source_observations ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.source_observations FORCE ROW LEVEL SECURITY;
CREATE POLICY source_observation_tenant ON public.source_observations
 USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
ALTER TABLE public.source_observation_paths ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.source_observation_paths FORCE ROW LEVEL SECURITY;
CREATE POLICY source_observation_path_tenant ON public.source_observation_paths
 USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
GRANT SELECT,INSERT,UPDATE ON public.source_reconcile_heads,public.source_reconcile_requests TO synveda_app;
GRANT SELECT,INSERT ON public.source_observations,public.source_observation_paths TO synveda_app;

CREATE FUNCTION public.synveda_source_reconcile_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
DECLARE digest text; operation public.durable_operations%ROWTYPE; request public.source_reconcile_requests%ROWTYPE;
 head public.source_reconcile_heads%ROWTYPE;
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'source observation identity is retained' USING ERRCODE='23514'; END IF;
 IF TG_TABLE_NAME='import_jobs' THEN
  IF TG_OP='UPDATE' THEN
   IF (NEW.managed_source_id,NEW.managed_review_digest) IS DISTINCT FROM (OLD.managed_source_id,OLD.managed_review_digest) THEN
    RAISE EXCEPTION 'managed import identity is immutable' USING ERRCODE='23514'; END IF;
   RETURN NEW;
  END IF;
  IF NEW.managed_source_id IS NULL THEN RETURN NEW; END IF;
  IF NOT EXISTS (SELECT 1 FROM public.source_reconcile_requests r
   JOIN public.source_reconcile_heads h ON h.tenant_id=r.tenant_id AND h.project_id=r.project_id AND h.source_id=r.source_id
   JOIN public.durable_operations op ON op.tenant_id=r.tenant_id AND op.id=r.operation_id
   JOIN public.operation_attempts a ON a.tenant_id=op.tenant_id AND a.operation_id=op.id AND a.attempt_number=op.attempts
   WHERE r.tenant_id=NEW.tenant_id AND r.project_id=NEW.project_id AND r.repository_id=NEW.repository_id
    AND r.source_id=NEW.managed_source_id AND h.scope_id=NEW.scope_id AND r.commit_oid=NEW.source_revision
    AND op.kind='source_reconcile' AND op.state='running' AND op.cancel_requested_at IS NULL
    AND op.lease_expires_at>clock_timestamp() AND a.state='running' AND a.worker_id=op.lease_owner
    AND h.active_operation_id=op.id AND h.observation_id IS NOT DISTINCT FROM r.previous_observation_id
    AND op.id::text=current_setting('synveda.source_reconcile_operation',true)
    AND op.attempts::text=current_setting('synveda.source_reconcile_attempt',true)
    AND op.lease_owner=current_setting('synveda.source_reconcile_owner',true)) THEN
   RAISE EXCEPTION 'managed import lacks its live pinned source request' USING ERRCODE='23514'; END IF;
  RETURN NEW;
 END IF;
 IF TG_TABLE_NAME='source_reconcile_heads' THEN
  SELECT encode(sha256(convert_to(jsonb_build_array('synveda.repository.source.v1',r.tenant_id,r.project_id,lower(r.canonical_uri))::text,'UTF8')),'hex') INTO digest
   FROM public.project_repositories r JOIN public.projects p ON p.tenant_id=r.tenant_id AND p.id=r.project_id
   WHERE r.tenant_id=NEW.tenant_id AND r.project_id=NEW.project_id AND r.id=NEW.repository_id AND p.scope_id=NEW.scope_id;
  IF TG_OP='UPDATE' AND NEW.repository_id IS NOT DISTINCT FROM OLD.repository_id THEN
   digest=OLD.repository_identity_digest;
  END IF;
  IF digest IS NULL THEN RAISE EXCEPTION 'source repository unavailable' USING ERRCODE='23514'; END IF;
  IF TG_OP='INSERT' THEN
   NEW.repository_identity_digest=digest;
   IF NEW.observation_id IS NOT NULL OR NEW.active_operation_id IS NOT NULL OR NEW.last_operation_id IS NOT NULL THEN
    RAISE EXCEPTION 'new source head has no observation' USING ERRCODE='23514'; END IF;
  ELSE
   IF (NEW.tenant_id,NEW.project_id,NEW.scope_id,NEW.source_id,NEW.repository_identity_digest)
     IS DISTINCT FROM (OLD.tenant_id,OLD.project_id,OLD.scope_id,OLD.source_id,OLD.repository_identity_digest)
     OR digest IS DISTINCT FROM OLD.repository_identity_digest THEN
    RAISE EXCEPTION 'source id cannot retarget a repository' USING ERRCODE='23514'; END IF;
   IF NEW.observation_id IS DISTINCT FROM OLD.observation_id AND NOT EXISTS (
    SELECT 1 FROM public.source_observations o JOIN public.durable_operations op ON op.tenant_id=o.tenant_id AND op.id=o.operation_id
    WHERE o.tenant_id=NEW.tenant_id AND o.id=NEW.observation_id AND o.project_id=NEW.project_id AND o.source_id=NEW.source_id
     AND o.previous_id IS NOT DISTINCT FROM OLD.observation_id AND o.operation_id=OLD.active_operation_id
     AND op.state='running' AND op.lease_expires_at>clock_timestamp()
     AND op.id::text=current_setting('synveda.source_reconcile_operation',true)
     AND op.attempts::text=current_setting('synveda.source_reconcile_attempt',true)
     AND op.lease_owner=current_setting('synveda.source_reconcile_owner',true)) THEN
    RAISE EXCEPTION 'source cursor lacks a live observation' USING ERRCODE='23514'; END IF;
   IF NEW.active_operation_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM public.source_reconcile_requests r JOIN public.durable_operations op ON op.tenant_id=r.tenant_id AND op.id=r.operation_id
    WHERE r.tenant_id=NEW.tenant_id AND r.operation_id=NEW.active_operation_id AND r.project_id=NEW.project_id AND r.source_id=NEW.source_id
     AND op.kind='source_reconcile' AND op.state IN ('pending','running','failed')) THEN
    RAISE EXCEPTION 'source head lacks its refresh' USING ERRCODE='23514'; END IF;
  END IF;
  RETURN NEW;
 END IF;
 IF TG_TABLE_NAME='source_reconcile_requests' THEN
  SELECT * INTO operation FROM public.durable_operations WHERE tenant_id=NEW.tenant_id AND id=NEW.operation_id;
  IF operation.kind IS DISTINCT FROM 'source_reconcile' THEN RAISE EXCEPTION 'source request kind mismatch' USING ERRCODE='23514'; END IF;
  IF TG_OP='INSERT' AND (NEW.commit_oid IS NOT NULL OR NEW.tree_oid IS NOT NULL) THEN
   RAISE EXCEPTION 'source request starts without a transport pin' USING ERRCODE='23514'; END IF;
  IF TG_OP='INSERT' AND NOT EXISTS (
   SELECT 1 FROM public.configuration_versions v,
    LATERAL jsonb_array_elements(coalesce(v.document->'git_sources','[]'::jsonb)) s
   WHERE v.tenant_id=NEW.tenant_id AND v.id=NEW.configuration_version_id
    AND s->>'source_id'=NEW.source_id AND s->>'repository_id'=NEW.repository_id::text
    AND s->'enabled'='true'::jsonb AND jsonb_array_length(s->'allowed_paths')=NEW.path_count) THEN
   RAISE EXCEPTION 'source request lacks its enabled frozen descriptor' USING ERRCODE='23514'; END IF;
  IF TG_OP='UPDATE' THEN
   IF (to_jsonb(NEW)-ARRAY['commit_oid','tree_oid']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['commit_oid','tree_oid'])
    OR OLD.commit_oid IS NOT NULL OR operation.state<>'running' OR operation.lease_expires_at<=clock_timestamp()
    OR operation.id::text IS DISTINCT FROM current_setting('synveda.source_reconcile_operation',true)
    OR operation.attempts::text IS DISTINCT FROM current_setting('synveda.source_reconcile_attempt',true)
    OR operation.lease_owner IS DISTINCT FROM current_setting('synveda.source_reconcile_owner',true) THEN
    RAISE EXCEPTION 'source request pin is immutable or lost its lease' USING ERRCODE='23514'; END IF;
  END IF;
  RETURN NEW;
 END IF;
 IF TG_OP<>'INSERT' THEN RAISE EXCEPTION 'source observations are immutable' USING ERRCODE='23514'; END IF;
 IF TG_TABLE_NAME='source_observation_paths' THEN
  SELECT r.* INTO request FROM public.source_observations o JOIN public.source_reconcile_requests r ON r.tenant_id=o.tenant_id AND r.operation_id=o.operation_id
   WHERE o.tenant_id=NEW.tenant_id AND o.id=NEW.observation_id;
  IF NOT EXISTS (SELECT 1 FROM public.configuration_versions v,
   LATERAL jsonb_array_elements(coalesce(v.document->'git_sources','[]'::jsonb)) s,
   LATERAL jsonb_array_elements_text(s->'allowed_paths') p
   WHERE v.tenant_id=request.tenant_id AND v.id=request.configuration_version_id
    AND s->>'source_id'=request.source_id AND encode(sha256(convert_to(p.value,'UTF8')),'hex')=NEW.path_digest) THEN
   RAISE EXCEPTION 'observation path is outside frozen coverage' USING ERRCODE='23514'; END IF;
 ELSE
  SELECT * INTO request FROM public.source_reconcile_requests WHERE tenant_id=NEW.tenant_id AND operation_id=NEW.operation_id;
  IF (NEW.project_id,NEW.source_id,NEW.descriptor_hash,NEW.commit_oid,NEW.tree_oid,NEW.previous_id)
   IS DISTINCT FROM (request.project_id,request.source_id,request.descriptor_hash,request.commit_oid,request.tree_oid,request.previous_observation_id)
   THEN RAISE EXCEPTION 'source observation differs from its pinned request' USING ERRCODE='23514'; END IF;
  IF NEW.import_job_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM public.import_jobs job
   WHERE job.tenant_id=NEW.tenant_id AND job.id=NEW.import_job_id AND job.project_id=NEW.project_id AND job.managed_source_id=NEW.source_id) THEN
   RAISE EXCEPTION 'observation import belongs to a different source' USING ERRCODE='23514'; END IF;
 END IF;
 SELECT * INTO operation FROM public.durable_operations WHERE tenant_id=request.tenant_id AND id=request.operation_id;
 SELECT * INTO head FROM public.source_reconcile_heads WHERE tenant_id=request.tenant_id AND project_id=request.project_id AND source_id=request.source_id;
 IF operation.kind IS DISTINCT FROM 'source_reconcile' OR operation.state IS DISTINCT FROM 'running'
  OR operation.cancel_requested_at IS NOT NULL OR operation.lease_expires_at<=clock_timestamp()
  OR head.active_operation_id IS DISTINCT FROM request.operation_id
  OR head.observation_id IS DISTINCT FROM request.previous_observation_id
  OR operation.id::text IS DISTINCT FROM current_setting('synveda.source_reconcile_operation',true)
  OR operation.attempts::text IS DISTINCT FROM current_setting('synveda.source_reconcile_attempt',true)
  OR operation.lease_owner IS DISTINCT FROM current_setting('synveda.source_reconcile_owner',true)
  OR NOT EXISTS (SELECT 1 FROM public.operation_attempts a WHERE a.tenant_id=operation.tenant_id AND a.operation_id=operation.id
    AND a.attempt_number=operation.attempts AND a.worker_id=operation.lease_owner AND a.state='running') THEN
  RAISE EXCEPTION 'source observation lost its current attempt' USING ERRCODE='23514'; END IF;
 IF TG_TABLE_NAME='source_observation_paths' THEN
 IF (NEW.state='suppressed') IS DISTINCT FROM EXISTS (SELECT 1 FROM public.artifact_suppressions s
   WHERE s.tenant_id=NEW.tenant_id AND s.source_key_digest=public.synveda_artifact_source_key(
    head.scope_id,head.repository_identity_digest,'git','managed',NEW.path_digest,NEW.content_hash))
   AND NEW.state<>'missing' THEN
  RAISE EXCEPTION 'observation disagrees with permanent source suppression' USING ERRCODE='23514'; END IF;
 IF NEW.artifact_id IS NOT NULL THEN
  IF NOT EXISTS (SELECT 1 FROM public.source_artifact_addresses a JOIN public.import_jobs job ON job.tenant_id=a.tenant_id AND job.id=a.job_id
   WHERE a.tenant_id=NEW.tenant_id AND a.artifact_id=NEW.artifact_id AND job.managed_source_id=request.source_id
   AND a.project_id=head.project_id AND a.scope_id=head.scope_id
   AND a.source_identity_digest=encode(sha256(convert_to(jsonb_build_array('repository',head.repository_identity_digest)::text,'UTF8')),'hex')
   AND a.path_digest=NEW.path_digest AND a.content_hash=NEW.content_hash)
   OR (NEW.state='present' AND NOT public.synveda_source_admissible(NEW.tenant_id,'import_artifact',NEW.artifact_id)) THEN
   RAISE EXCEPTION 'source observation artifact provenance mismatch' USING ERRCODE='23514'; END IF;
 END IF;
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION public.synveda_source_reconcile_guard() FROM PUBLIC;
CREATE TRIGGER source_reconcile_head_guard BEFORE INSERT OR UPDATE OR DELETE ON public.source_reconcile_heads FOR EACH ROW EXECUTE FUNCTION public.synveda_source_reconcile_guard();
CREATE TRIGGER source_reconcile_request_guard BEFORE INSERT OR UPDATE OR DELETE ON public.source_reconcile_requests FOR EACH ROW EXECUTE FUNCTION public.synveda_source_reconcile_guard();
CREATE TRIGGER source_observation_guard BEFORE INSERT OR UPDATE OR DELETE ON public.source_observations FOR EACH ROW EXECUTE FUNCTION public.synveda_source_reconcile_guard();
CREATE TRIGGER source_observation_path_guard BEFORE INSERT OR UPDATE OR DELETE ON public.source_observation_paths FOR EACH ROW EXECUTE FUNCTION public.synveda_source_reconcile_guard();
CREATE TRIGGER source_managed_import_guard BEFORE INSERT OR UPDATE ON public.import_jobs FOR EACH ROW EXECUTE FUNCTION public.synveda_source_reconcile_guard();

CREATE FUNCTION public.synveda_source_observation_complete() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
BEGIN
 IF NOT EXISTS (SELECT 1 FROM public.source_reconcile_requests r
  JOIN public.durable_operations op ON op.tenant_id=r.tenant_id AND op.id=r.operation_id
  JOIN public.source_reconcile_heads h ON h.tenant_id=r.tenant_id AND h.project_id=r.project_id AND h.source_id=r.source_id
  WHERE r.tenant_id=NEW.tenant_id AND r.operation_id=NEW.operation_id AND op.state='succeeded'
   AND h.observation_id=NEW.id AND h.active_operation_id IS NULL
   AND (SELECT count(*) FROM public.source_observation_paths p WHERE p.tenant_id=NEW.tenant_id AND p.observation_id=NEW.id)=r.path_count) THEN
  RAISE EXCEPTION 'incomplete source observation cannot advance its cursor' USING ERRCODE='23514'; END IF;
 RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION public.synveda_source_observation_complete() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER source_observation_complete AFTER INSERT ON public.source_observations
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.synveda_source_observation_complete();

CREATE FUNCTION public.synveda_managed_import_complete() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
BEGIN
 IF NOT EXISTS (SELECT 1 FROM public.source_observations o JOIN public.durable_operations op ON op.tenant_id=o.tenant_id AND op.id=o.operation_id
  WHERE o.tenant_id=NEW.tenant_id AND o.import_job_id=NEW.id AND o.source_id=NEW.managed_source_id AND op.state='succeeded') THEN
  RAISE EXCEPTION 'managed source copies require their atomic complete observation' USING ERRCODE='23514'; END IF;
 RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION public.synveda_managed_import_complete() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER managed_import_complete AFTER INSERT ON public.import_jobs
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW WHEN (NEW.managed_source_id IS NOT NULL)
 EXECUTE FUNCTION public.synveda_managed_import_complete();
