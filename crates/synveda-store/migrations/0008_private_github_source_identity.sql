-- MEM-9 / ADR-0149: immutable provider identity adopts existing permanent roots.
CREATE TABLE public.source_repository_roots (
 tenant_id uuid NOT NULL, project_id uuid NOT NULL, github_repository_id bigint NOT NULL,
 root_digest text NOT NULL,
 PRIMARY KEY (tenant_id,project_id,github_repository_id),
 UNIQUE (tenant_id,project_id,root_digest),
 FOREIGN KEY (tenant_id,project_id) REFERENCES public.projects(tenant_id,id),
 CHECK (github_repository_id>0), CHECK (root_digest ~ '^[0-9a-f]{64}$')
);
CREATE TABLE public.source_repository_attachments (
 tenant_id uuid NOT NULL, project_id uuid NOT NULL, repository_id uuid NOT NULL,
 github_repository_id bigint NOT NULL, uri_digest text NOT NULL,
 PRIMARY KEY (tenant_id,project_id,repository_id),
 FOREIGN KEY (tenant_id,project_id,github_repository_id)
  REFERENCES public.source_repository_roots(tenant_id,project_id,github_repository_id),
 CHECK (uri_digest ~ '^[0-9a-f]{64}$')
);
CREATE INDEX source_repository_uri ON public.source_repository_attachments(tenant_id,project_id,uri_digest);
CREATE INDEX source_import_repository_root ON public.import_jobs(tenant_id,project_id,repository_identity_digest)
 WHERE repository_id IS NOT NULL;
CREATE INDEX source_import_repository_attachment ON public.import_jobs(tenant_id,project_id,repository_id)
 WHERE repository_id IS NOT NULL;
CREATE INDEX source_head_repository_root ON public.source_reconcile_heads(tenant_id,project_id,repository_identity_digest);
CREATE INDEX source_head_repository_attachment ON public.source_reconcile_heads(tenant_id,project_id,repository_id);
CREATE INDEX source_managed_review_lookup ON public.import_jobs
 (tenant_id,project_id,repository_identity_digest,managed_source_id,bundle_digest,managed_review_digest,created_at,id)
 WHERE managed_source_id IS NOT NULL;
ALTER TABLE public.source_repository_roots ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.source_repository_roots FORCE ROW LEVEL SECURITY;
CREATE POLICY source_repository_root_tenant ON public.source_repository_roots
 USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
ALTER TABLE public.source_repository_attachments ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.source_repository_attachments FORCE ROW LEVEL SECURITY;
CREATE POLICY source_repository_attachment_tenant ON public.source_repository_attachments
 USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
GRANT SELECT ON public.source_repository_roots,public.source_repository_attachments TO synveda_app;

CREATE FUNCTION public.synveda_source_repository_immutable() RETURNS trigger
 LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
BEGIN RAISE EXCEPTION 'source repository identity is immutable' USING ERRCODE='23514'; END $$;
REVOKE ALL ON FUNCTION public.synveda_source_repository_immutable() FROM PUBLIC;
CREATE TRIGGER source_repository_root_immutable BEFORE UPDATE OR DELETE ON public.source_repository_roots
 FOR EACH ROW EXECUTE FUNCTION public.synveda_source_repository_immutable();
CREATE TRIGGER source_repository_attachment_immutable BEFORE UPDATE OR DELETE ON public.source_repository_attachments
 FOR EACH ROW EXECUTE FUNCTION public.synveda_source_repository_immutable();

CREATE FUNCTION public.synveda_repository_source_identity(wanted_tenant uuid,wanted_project uuid,wanted_repository uuid)
 RETURNS text LANGUAGE sql STABLE SET search_path=pg_catalog,public AS $$
 SELECT coalesce(root.root_digest,
  encode(sha256(convert_to(jsonb_build_array('synveda.repository.source.v1',r.tenant_id,r.project_id,lower(r.canonical_uri))::text,'UTF8')),'hex'))
 FROM public.project_repositories r
 LEFT JOIN public.source_repository_attachments alias
  ON alias.tenant_id=r.tenant_id AND alias.project_id=r.project_id AND alias.repository_id=r.id
 LEFT JOIN public.source_repository_roots root
  ON root.tenant_id=alias.tenant_id AND root.project_id=alias.project_id AND root.github_repository_id=alias.github_repository_id
 WHERE r.tenant_id=wanted_tenant AND r.project_id=wanted_project AND r.id=wanted_repository
  AND wanted_tenant=public.synveda_current_tenant()
$$;
REVOKE ALL ON FUNCTION public.synveda_repository_source_identity(uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.synveda_repository_source_identity(uuid,uuid,uuid) TO synveda_app;

-- Only this checked function inserts identity assertions; callers cannot choose
-- root bytes or rewrite historical aliases. Current Cedar precedes this seam.
CREATE FUNCTION public.synveda_bind_github_source_identity(wanted_tenant uuid,wanted_project uuid,wanted_scope uuid,
 wanted_repository uuid,wanted_github_id bigint) RETURNS text
 LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE uri_root text; permanent text;
BEGIN
 IF wanted_tenant IS DISTINCT FROM public.synveda_current_tenant() OR wanted_github_id<=0 THEN
  RAISE EXCEPTION 'source repository binding unavailable' USING ERRCODE='23514'; END IF;
 SELECT encode(sha256(convert_to(jsonb_build_array('synveda.repository.source.v1',r.tenant_id,r.project_id,lower(r.canonical_uri))::text,'UTF8')),'hex')
  INTO uri_root FROM public.project_repositories r JOIN public.projects p ON p.tenant_id=r.tenant_id AND p.id=r.project_id
  WHERE r.tenant_id=wanted_tenant AND r.project_id=wanted_project AND r.id=wanted_repository
   AND p.scope_id=wanted_scope AND r.provider='github' AND r.canonical_uri LIKE 'https://github.com/%';
 IF uri_root IS NULL THEN RAISE EXCEPTION 'source repository binding unavailable' USING ERRCODE='23514'; END IF;
 SELECT root_digest INTO permanent FROM public.source_repository_roots
  WHERE tenant_id=wanted_tenant AND project_id=wanted_project AND github_repository_id=wanted_github_id;
 permanent=coalesce(permanent,uri_root);
 IF EXISTS (SELECT 1 FROM public.source_repository_attachments
  WHERE tenant_id=wanted_tenant AND project_id=wanted_project AND repository_id=wanted_repository
   AND github_repository_id=wanted_github_id AND uri_digest=uri_root) THEN RETURN permanent; END IF;
 IF EXISTS (SELECT 1 FROM public.source_repository_attachments
  WHERE tenant_id=wanted_tenant AND project_id=wanted_project AND uri_digest=uri_root
   AND github_repository_id<>wanted_github_id)
 OR EXISTS (SELECT 1 FROM public.source_repository_roots
  WHERE tenant_id=wanted_tenant AND project_id=wanted_project AND root_digest=uri_root
   AND github_repository_id<>wanted_github_id)
 OR (uri_root<>permanent AND EXISTS (SELECT 1 FROM public.source_reconcile_heads
  WHERE tenant_id=wanted_tenant AND project_id=wanted_project
   AND (repository_id=wanted_repository OR repository_identity_digest=uri_root)
   AND repository_identity_digest<>permanent))
 OR (uri_root<>permanent AND EXISTS (SELECT 1 FROM public.import_jobs
  WHERE tenant_id=wanted_tenant AND project_id=wanted_project
   AND (repository_id=wanted_repository OR repository_identity_digest=uri_root)
   AND repository_identity_digest<>permanent)) THEN
  RAISE EXCEPTION 'source repository identity conflicts with retained roots' USING ERRCODE='23514'; END IF;
 INSERT INTO public.source_repository_roots(tenant_id,project_id,github_repository_id,root_digest)
  VALUES(wanted_tenant,wanted_project,wanted_github_id,permanent)
  ON CONFLICT(tenant_id,project_id,github_repository_id) DO NOTHING;
 INSERT INTO public.source_repository_attachments(tenant_id,project_id,repository_id,github_repository_id,uri_digest)
  VALUES(wanted_tenant,wanted_project,wanted_repository,wanted_github_id,uri_root)
  ON CONFLICT(tenant_id,project_id,repository_id) DO NOTHING;
 IF NOT EXISTS (SELECT 1 FROM public.source_repository_attachments
  WHERE tenant_id=wanted_tenant AND project_id=wanted_project AND repository_id=wanted_repository
   AND github_repository_id=wanted_github_id AND uri_digest=uri_root) THEN
  RAISE EXCEPTION 'source repository identity conflicts with retained attachment' USING ERRCODE='23514'; END IF;
 RETURN permanent;
END $$;
REVOKE ALL ON FUNCTION public.synveda_bind_github_source_identity(uuid,uuid,uuid,uuid,bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.synveda_bind_github_source_identity(uuid,uuid,uuid,uuid,bigint) TO synveda_app;

CREATE OR REPLACE FUNCTION public.synveda_import_repository_identity() RETURNS trigger
 LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
BEGIN
 IF NEW.tenant_id IS DISTINCT FROM public.synveda_current_tenant() THEN
  RAISE EXCEPTION 'import repository tenant mismatch' USING ERRCODE='23514'; END IF;
 IF NEW.repository_id IS NULL THEN NEW.repository_identity_digest=NULL;
 ELSE
  NEW.repository_identity_digest=public.synveda_repository_source_identity(NEW.tenant_id,NEW.project_id,NEW.repository_id);
  IF NEW.repository_identity_digest IS NULL THEN
   RAISE EXCEPTION 'import repository differs from project' USING ERRCODE='23514'; END IF;
 END IF;
 RETURN NEW;
END $$;


CREATE OR REPLACE FUNCTION public.synveda_source_reconcile_guard() RETURNS trigger
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
  SELECT public.synveda_repository_source_identity(r.tenant_id,r.project_id,r.id) INTO digest
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
