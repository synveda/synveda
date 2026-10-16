-- MEM-8/MEM-9, ADR-0147. Event roots and their published migration stay intact.
ALTER TABLE public.import_jobs ADD COLUMN repository_id uuid;
ALTER TABLE public.import_jobs ADD COLUMN repository_identity_digest text;
ALTER TABLE public.import_jobs ADD CONSTRAINT import_job_repository_shape CHECK
 ((repository_id IS NULL AND repository_identity_digest IS NULL) OR (repository_id IS NOT NULL AND repository_identity_digest ~ '^[0-9a-f]{64}$'));
ALTER TABLE public.import_artifacts ADD COLUMN content_erased boolean NOT NULL DEFAULT false;
ALTER TABLE public.import_artifacts DROP CONSTRAINT import_artifacts_path_check;
ALTER TABLE public.import_artifacts ADD CONSTRAINT import_artifacts_path_check CHECK (
 (content_erased AND logical_path='erased/'||id::text AND body_markdown='' AND frontmatter='{}') OR
 (NOT content_erased AND btrim(logical_path)<>'' AND char_length(logical_path)<=1000
  AND left(logical_path,1)<>'/' AND position('\' IN logical_path)=0 AND logical_path !~ '(^|/)\.\.?(/|$)'));
ALTER TABLE public.knowledge_sources ADD COLUMN import_artifact_id uuid;
ALTER TABLE public.knowledge_sources ADD CONSTRAINT knowledge_source_artifact_fk
 FOREIGN KEY (tenant_id,import_artifact_id) REFERENCES public.import_artifacts(tenant_id,id);
ALTER TABLE public.knowledge_sources ADD CONSTRAINT knowledge_source_artifact_shape CHECK
 (import_artifact_id IS NULL OR (source_type='okf' AND session_event_id IS NULL AND content_hash IS NOT NULL));
ALTER TABLE public.knowledge_sources DROP CONSTRAINT knowledge_sources_shape_check;
ALTER TABLE public.knowledge_sources ADD CONSTRAINT knowledge_sources_shape_check CHECK (
 (content_erased AND locator IS NULL AND source_revision IS NULL AND metadata='{}') OR
 (NOT content_erased AND ((source_type='session_event' AND session_event_id IS NOT NULL AND locator IS NULL)
 OR (source_type='manual' AND session_event_id IS NULL AND locator IS NULL AND source_revision IS NULL)
 OR (source_type IN ('document','repository','url','okf','system_derived') AND session_event_id IS NULL AND locator IS NOT NULL))));
ALTER TABLE public.knowledge_changes DROP CONSTRAINT knowledge_changes_command_check;
ALTER TABLE public.knowledge_changes ADD CONSTRAINT knowledge_changes_command_check CHECK
 (command_kind IN ('create','edit','verify','supersede','merge','archive','restore','forget','resolve_conflict','control_source','control_artifact'));
ALTER TABLE public.knowledge_change_headers ADD COLUMN source_artifact_id uuid;
ALTER TABLE public.knowledge_change_headers ADD COLUMN source_artifact_hash text;
ALTER TABLE public.knowledge_change_headers ADD CONSTRAINT artifact_change_shape CHECK
 (command_kind<>'control_artifact' OR (source_artifact_id IS NOT NULL AND source_scope_id IS NOT NULL
  AND source_artifact_hash ~ '^[0-9a-f]{64}$' AND source_mode IN ('withdraw','forget')));

CREATE TABLE public.source_artifact_addresses (
 tenant_id uuid NOT NULL, artifact_id uuid NOT NULL, job_id uuid NOT NULL,
 project_id uuid NOT NULL, scope_id uuid NOT NULL, workspace_id uuid NOT NULL,
 source_identity_digest text NOT NULL, path_digest text NOT NULL,
 content_hash text NOT NULL, source_key_digest text NOT NULL,
 PRIMARY KEY (tenant_id,artifact_id),
 CHECK (source_identity_digest ~ '^[0-9a-f]{64}$'), CHECK (path_digest ~ '^[0-9a-f]{64}$'),
 CHECK (content_hash ~ '^[0-9a-f]{64}$'), CHECK (source_key_digest ~ '^[0-9a-f]{64}$')
);
CREATE INDEX artifact_address_alias ON public.source_artifact_addresses (tenant_id,source_key_digest,artifact_id);
CREATE INDEX artifact_address_job ON public.source_artifact_addresses (tenant_id,job_id,artifact_id);
CREATE TABLE public.artifact_dependencies (
 tenant_id uuid NOT NULL, dependent_kind text NOT NULL, dependent_id uuid NOT NULL, source_key_digest text NOT NULL,
 PRIMARY KEY (tenant_id,dependent_kind,dependent_id,source_key_digest),
 CHECK (dependent_kind IN ('import_artifact','import_mapping','knowledge_source','knowledge_revision','capture_candidate','knowledge_change','context_run','session_event')),
 CHECK (source_key_digest ~ '^[0-9a-f]{64}$')
);
CREATE INDEX artifact_dependency_root ON public.artifact_dependencies (tenant_id,source_key_digest,dependent_kind,dependent_id);
CREATE TABLE public.unproven_artifact_dependencies (
 tenant_id uuid NOT NULL, dependent_kind text NOT NULL, dependent_id uuid NOT NULL,
 PRIMARY KEY (tenant_id,dependent_kind,dependent_id),
 CHECK (dependent_kind IN ('knowledge_source','knowledge_revision','knowledge_change','context_run'))
);
CREATE TABLE public.artifact_suppressions (
 tenant_id uuid NOT NULL, source_key_digest text NOT NULL, artifact_id uuid NOT NULL,
 change_id uuid NOT NULL, forget_change_id uuid, actor_hash text NOT NULL, reason_hash text NOT NULL,
 removed_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY (tenant_id,source_key_digest),
 FOREIGN KEY (tenant_id,artifact_id) REFERENCES public.source_artifact_addresses(tenant_id,artifact_id),
 FOREIGN KEY (tenant_id,change_id) REFERENCES public.knowledge_changes(tenant_id,proposal_id),
 FOREIGN KEY (tenant_id,forget_change_id) REFERENCES public.knowledge_changes(tenant_id,proposal_id),
 CHECK (source_key_digest ~ '^[0-9a-f]{64}$'), CHECK (actor_hash ~ '^[0-9a-f]{64}$'), CHECK (reason_hash ~ '^[0-9a-f]{64}$')
);
CREATE TABLE public.artifact_cleanup_targets (
 tenant_id uuid NOT NULL, operation_id uuid NOT NULL, artifact_id uuid NOT NULL, source_key_digest text NOT NULL,
 change_id uuid NOT NULL, input_hash text NOT NULL, phase smallint NOT NULL DEFAULT 0,
 cursor_id uuid, processed_targets bigint NOT NULL DEFAULT 0,
 PRIMARY KEY (tenant_id,operation_id),
 FOREIGN KEY (tenant_id,operation_id) REFERENCES public.durable_operations(tenant_id,id),
 FOREIGN KEY (tenant_id,artifact_id) REFERENCES public.source_artifact_addresses(tenant_id,artifact_id),
 FOREIGN KEY (tenant_id,source_key_digest) REFERENCES public.artifact_suppressions(tenant_id,source_key_digest),
 FOREIGN KEY (tenant_id,change_id) REFERENCES public.knowledge_changes(tenant_id,proposal_id),
 CHECK (input_hash ~ '^[0-9a-f]{64}$'), CHECK (phase BETWEEN 0 AND 9), CHECK (processed_targets>=0)
);
ALTER TABLE public.source_artifact_addresses ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.source_artifact_addresses FORCE ROW LEVEL SECURITY;
CREATE POLICY artifact_address_tenant ON public.source_artifact_addresses USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
ALTER TABLE public.artifact_dependencies ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.artifact_dependencies FORCE ROW LEVEL SECURITY;
CREATE POLICY artifact_dependency_tenant ON public.artifact_dependencies USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
ALTER TABLE public.unproven_artifact_dependencies ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.unproven_artifact_dependencies FORCE ROW LEVEL SECURITY;
CREATE POLICY unproven_artifact_tenant ON public.unproven_artifact_dependencies USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
ALTER TABLE public.artifact_suppressions ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.artifact_suppressions FORCE ROW LEVEL SECURITY;
CREATE POLICY artifact_suppression_tenant ON public.artifact_suppressions USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
ALTER TABLE public.artifact_cleanup_targets ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.artifact_cleanup_targets FORCE ROW LEVEL SECURITY;
CREATE POLICY artifact_cleanup_tenant ON public.artifact_cleanup_targets USING (tenant_id=public.synveda_current_tenant()) WITH CHECK (tenant_id=public.synveda_current_tenant());
GRANT SELECT ON public.source_artifact_addresses,public.artifact_dependencies,public.unproven_artifact_dependencies,public.artifact_cleanup_targets TO synveda_app;
GRANT SELECT,INSERT ON public.artifact_suppressions TO synveda_app;
GRANT UPDATE (forget_change_id) ON public.artifact_suppressions TO synveda_app;
GRANT INSERT (tenant_id,operation_id,artifact_id,source_key_digest,change_id,input_hash) ON public.artifact_cleanup_targets TO synveda_app;

-- A job/revision/bundle identifier cannot reset an admitted source alias.
CREATE FUNCTION public.synveda_artifact_source_key(s uuid,r text,k text,l text,p text,h text) RETURNS text
 LANGUAGE sql IMMUTABLE SET search_path TO 'public','pg_temp' AS $$
 SELECT encode(sha256(convert_to(jsonb_build_array('synveda.artifact.source.v1',s,
   CASE WHEN r IS NULL THEN jsonb_build_array('declared',k,l) ELSE jsonb_build_array('repository',r) END,
   p,h)::text,'UTF8')),'hex')
$$;

CREATE FUNCTION public.synveda_artifact_cleanup_authorized(t uuid,k text,d uuid) RETURNS boolean
 LANGUAGE sql STABLE SET search_path TO 'public','pg_temp' AS $$
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
$$;
REVOKE ALL ON FUNCTION public.synveda_artifact_cleanup_authorized(uuid,text,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.synveda_artifact_cleanup_authorized(uuid,text,uuid) TO synveda_app;

CREATE FUNCTION public.synveda_artifact_projection() RETURNS trigger
 LANGUAGE plpgsql SECURITY DEFINER SET search_path TO 'public','pg_temp' AS $$
DECLARE k text; d uuid; root_id uuid; root_key text; entry jsonb; job public.import_jobs%ROWTYPE;
BEGIN
 IF NEW.tenant_id IS DISTINCT FROM public.synveda_current_tenant() THEN
  RAISE EXCEPTION 'artifact projection tenant mismatch' USING ERRCODE='23514'; END IF;
 IF TG_TABLE_NAME='import_artifacts' THEN
  SELECT * INTO job FROM public.import_jobs WHERE tenant_id=NEW.tenant_id AND id=NEW.job_id;
  root_key=public.synveda_artifact_source_key(job.scope_id,job.repository_identity_digest,job.source_kind,job.source_locator,
    encode(sha256(convert_to(NEW.logical_path,'UTF8')),'hex'),NEW.content_hash);
  IF EXISTS(SELECT 1 FROM public.artifact_suppressions WHERE tenant_id=NEW.tenant_id AND source_key_digest=root_key) THEN
   RAISE EXCEPTION 'artifact source suppressed' USING ERRCODE='23514'; END IF;
  INSERT INTO public.source_artifact_addresses VALUES (NEW.tenant_id,NEW.id,NEW.job_id,job.project_id,job.scope_id,job.workspace_id,
   encode(sha256(convert_to(CASE WHEN job.repository_id IS NULL THEN jsonb_build_array('declared',job.source_kind,job.source_locator)
    ELSE jsonb_build_array('repository',job.repository_identity_digest) END::text,'UTF8')),'hex'),
   encode(sha256(convert_to(NEW.logical_path,'UTF8')),'hex'),NEW.content_hash,root_key);
  INSERT INTO public.artifact_dependencies VALUES (NEW.tenant_id,'import_artifact',NEW.id,root_key);
  RETURN NEW;
 ELSIF TG_TABLE_NAME='import_mappings' THEN k='import_mapping'; d=NEW.id; root_id=NEW.artifact_id;
 ELSIF TG_TABLE_NAME='capture_candidate_import_artifacts' THEN
  k='capture_candidate'; d=NEW.candidate_id; root_id=NEW.artifact_id;
  IF NOT EXISTS(SELECT 1 FROM public.source_artifact_addresses WHERE tenant_id=NEW.tenant_id AND artifact_id=root_id AND job_id=NEW.import_job_id) THEN
   RAISE EXCEPTION 'candidate artifact job mismatch' USING ERRCODE='23514'; END IF;
 ELSIF TG_TABLE_NAME='knowledge_sources' THEN
  k='knowledge_source'; d=NEW.id; root_id=NEW.import_artifact_id;
  IF root_id IS NULL THEN
   IF NEW.source_type='okf' AND (NEW.metadata ? 'artifact_id' OR NEW.metadata ? 'import_job_id') THEN
    RAISE EXCEPTION 'managed artifact provenance requires a typed pointer' USING ERRCODE='23514'; END IF;
  ELSE
   IF NOT EXISTS(SELECT 1 FROM public.import_artifacts artifact JOIN public.import_jobs j
    ON j.tenant_id=artifact.tenant_id AND j.id=artifact.job_id
    WHERE artifact.tenant_id=NEW.tenant_id AND artifact.id=root_id AND NEW.scope_id=j.scope_id
    AND NEW.source_type='okf' AND NEW.content_hash=artifact.content_hash
    AND NEW.locator=j.source_locator||'#'||artifact.logical_path
    AND NEW.source_revision IS NOT DISTINCT FROM j.source_revision) THEN
     RAISE EXCEPTION 'artifact descriptor differs from its admitted source' USING ERRCODE='23514'; END IF;
  END IF;
 ELSIF TG_TABLE_NAME='knowledge_revision_sources' THEN
  k='knowledge_revision'; d=NEW.knowledge_revision_id;
  INSERT INTO public.artifact_dependencies SELECT NEW.tenant_id,k,d,source_key_digest FROM public.artifact_dependencies
   WHERE tenant_id=NEW.tenant_id AND dependent_kind='knowledge_source' AND dependent_id=NEW.knowledge_source_id ON CONFLICT DO NOTHING;
 ELSIF TG_TABLE_NAME='knowledge_changes' THEN
  IF TG_OP='UPDATE' THEN RETURN NEW; END IF;
  IF NEW.command_kind='control_source' THEN RETURN NEW; END IF;
  k='knowledge_change'; d=NEW.proposal_id;
  IF NEW.command_kind='control_artifact' THEN
   root_id=(NEW.payload->>'artifact_id')::uuid;
   INSERT INTO public.artifact_dependencies SELECT NEW.tenant_id,k,d,source_key_digest FROM public.source_artifact_addresses
    WHERE tenant_id=NEW.tenant_id AND artifact_id=root_id;
   RETURN NEW;
  END IF;
  FOR entry IN SELECT x FROM jsonb_array_elements(coalesce(NEW.payload->'sources','[]')) x LOOP
   root_id=(entry->>'import_artifact_id')::uuid;
   IF root_id IS NULL THEN
    IF entry->>'source_type'='okf' AND (entry->'metadata' ? 'artifact_id' OR entry->'metadata' ? 'import_job_id') THEN
     RAISE EXCEPTION 'managed artifact provenance requires a typed pointer' USING ERRCODE='23514'; END IF;
   ELSE
    IF NOT public.synveda_source_admissible(NEW.tenant_id,'import_artifact',root_id) THEN
     RAISE EXCEPTION 'artifact source unavailable' USING ERRCODE='23514'; END IF;
    IF NOT EXISTS(SELECT 1 FROM public.import_artifacts artifact JOIN public.import_jobs j ON j.tenant_id=artifact.tenant_id AND j.id=artifact.job_id
      WHERE artifact.tenant_id=NEW.tenant_id AND artifact.id=root_id AND j.scope_id=(entry->>'scope_id')::uuid
      AND entry->>'source_type'='okf' AND entry->>'content_hash'=artifact.content_hash
      AND entry->>'locator'=j.source_locator||'#'||artifact.logical_path
      AND entry->>'source_revision' IS NOT DISTINCT FROM j.source_revision) THEN
     RAISE EXCEPTION 'artifact command provenance mismatch' USING ERRCODE='23514'; END IF;
    INSERT INTO public.artifact_dependencies SELECT NEW.tenant_id,k,d,source_key_digest FROM public.source_artifact_addresses
     WHERE tenant_id=NEW.tenant_id AND artifact_id=root_id ON CONFLICT DO NOTHING;
   END IF;
  END LOOP;
  root_id=NULL;
  IF NEW.command_kind='verify' THEN
   INSERT INTO public.artifact_dependencies SELECT NEW.tenant_id,k,d,source_key_digest FROM public.artifact_dependencies
    WHERE tenant_id=NEW.tenant_id AND dependent_kind='knowledge_revision' AND dependent_id=(NEW.payload->>'expected_revision_id')::uuid ON CONFLICT DO NOTHING;
  ELSIF NEW.command_kind='merge' THEN
   INSERT INTO public.artifact_dependencies SELECT NEW.tenant_id,k,d,dep.source_key_digest
    FROM jsonb_array_elements(NEW.payload->'inputs') x JOIN public.artifact_dependencies dep
    ON dep.tenant_id=NEW.tenant_id AND dep.dependent_kind='knowledge_revision' AND dep.dependent_id=(x->>'revision_id')::uuid ON CONFLICT DO NOTHING;
  END IF;
 ELSIF TG_TABLE_NAME='context_run_delivery_dependencies' THEN
  k='context_run'; d=NEW.context_run_id;
  INSERT INTO public.artifact_dependencies SELECT NEW.tenant_id,k,d,dep.source_key_digest
   FROM jsonb_array_elements(NEW.manifest->'knowledge') x JOIN public.artifact_dependencies dep
   ON dep.tenant_id=NEW.tenant_id AND dep.dependent_kind='knowledge_revision' AND dep.dependent_id=(x->>'revision_id')::uuid ON CONFLICT DO NOTHING;
  INSERT INTO public.artifact_dependencies SELECT NEW.tenant_id,k,d,dep.source_key_digest
   FROM jsonb_array_elements(NEW.manifest->'candidates') x JOIN public.artifact_dependencies dep
   ON dep.tenant_id=NEW.tenant_id AND dep.dependent_kind='capture_candidate' AND dep.dependent_id=(x->>'candidate_id')::uuid ON CONFLICT DO NOTHING;
  INSERT INTO public.artifact_dependencies SELECT NEW.tenant_id,k,d,dep.source_key_digest
   FROM jsonb_array_elements(NEW.manifest->'graph') x JOIN public.artifact_dependencies dep
   ON dep.tenant_id=NEW.tenant_id AND dep.dependent_kind='knowledge_revision'
    AND dep.dependent_id=ANY(ARRAY[(x->>'asserting_revision_id')::uuid,(x->>'from_revision_id')::uuid,(x->>'to_revision_id')::uuid]) ON CONFLICT DO NOTHING;
 END IF;
 IF root_id IS NOT NULL THEN
  IF NOT public.synveda_source_admissible(NEW.tenant_id,'import_artifact',root_id) THEN
   RAISE EXCEPTION 'artifact source unavailable' USING ERRCODE='23514'; END IF;
  INSERT INTO public.artifact_dependencies SELECT NEW.tenant_id,k,d,source_key_digest FROM public.source_artifact_addresses
   WHERE tenant_id=NEW.tenant_id AND artifact_id=root_id ON CONFLICT DO NOTHING;
 END IF;
 IF NOT public.synveda_source_admissible(NEW.tenant_id,k,d) THEN
  RAISE EXCEPTION 'artifact source unavailable' USING ERRCODE='23514'; END IF;
 IF (SELECT count(*) FROM (SELECT 1 FROM public.artifact_dependencies WHERE tenant_id=NEW.tenant_id AND dependent_kind=k AND dependent_id=d LIMIT 4097) roots)
   +(SELECT count(*) FROM (SELECT 1 FROM public.source_dependencies WHERE tenant_id=NEW.tenant_id AND dependent_kind=k AND dependent_id=d LIMIT 4097) roots)>4096 THEN
  RAISE EXCEPTION 'source roots bound exhausted' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION public.synveda_artifact_projection() FROM PUBLIC;
-- Run after the event projection; final admission checks both root families.
CREATE TRIGGER zz_artifact_address AFTER INSERT ON public.import_artifacts FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_projection();
CREATE TRIGGER zz_artifact_mapping AFTER INSERT ON public.import_mappings FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_projection();
CREATE TRIGGER zz_artifact_candidate AFTER INSERT ON public.capture_candidate_import_artifacts FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_projection();
CREATE TRIGGER zz_artifact_descriptor AFTER INSERT ON public.knowledge_sources FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_projection();
CREATE TRIGGER zz_artifact_revision AFTER INSERT ON public.knowledge_revision_sources FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_projection();
CREATE TRIGGER zz_artifact_change AFTER INSERT ON public.knowledge_changes FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_projection();
CREATE TRIGGER zz_artifact_delivery AFTER INSERT ON public.context_run_delivery_dependencies FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_projection();

CREATE FUNCTION public.synveda_import_source_artifact(t uuid,s jsonb) RETURNS uuid
 LANGUAGE sql STABLE SET search_path TO 'public','pg_temp' AS $$
 SELECT artifact.id FROM public.import_artifacts artifact JOIN public.import_jobs job
 ON job.tenant_id=artifact.tenant_id AND job.id=artifact.job_id
 WHERE artifact.tenant_id=t AND t=public.synveda_current_tenant() AND s->>'source_type'='okf'
 AND job.scope_id::text=s->>'scope_id' AND artifact.content_hash=s->>'content_hash'
 AND s->>'locator'=job.source_locator||'#'||artifact.logical_path
 AND s->>'source_revision' IS NOT DISTINCT FROM job.source_revision
 AND (artifact.id::text=s->>'import_artifact_id' OR
 (s->>'import_artifact_id' IS NULL AND artifact.id::text=s->'metadata'->>'artifact_id'
 AND job.id::text=s->'metadata'->>'import_job_id' AND job.bundle_digest=s->'metadata'->>'bundle_digest'
 AND artifact.logical_path=s->'metadata'->>'logical_path' AND artifact.artifact_kind=s->'metadata'->>'artifact_kind'
 AND EXISTS(SELECT 1 FROM public.capture_candidate_import_artifacts link JOIN public.capture_candidates candidate
 ON candidate.tenant_id=link.tenant_id AND candidate.id=link.candidate_id
 WHERE link.tenant_id=t AND link.artifact_id=artifact.id AND link.import_job_id=job.id
 AND candidate.import_job_id=job.id AND candidate.proposed_scope_id=job.scope_id
 AND candidate.id::text=s->'metadata'->>'capture_candidate_id'
 AND candidate.batch_id::text=s->'metadata'->>'capture_batch_id')))
 LIMIT 1
$$;
REVOKE ALL ON FUNCTION public.synveda_import_source_artifact(uuid,jsonb) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.synveda_import_source_artifact(uuid,jsonb) TO synveda_app;

-- Migration-only backfill retains FORCE RLS and each exact tenant context.
-- Existing event suppression must not make an old mixed derivative invisible
-- while reconstructing its other root. Recreate these policies before commit.
DROP POLICY revision_source_eligibility ON public.knowledge_revisions;
DROP POLICY descriptor_source_eligibility ON public.knowledge_sources;
DROP POLICY candidate_source_eligibility ON public.capture_candidates;
DROP POLICY decision_source_eligibility ON public.capture_candidate_decisions;
DROP POLICY change_source_eligibility ON public.knowledge_changes;
DROP POLICY context_source_eligibility ON public.session_context_runs;
ALTER TABLE public.knowledge_sources DISABLE TRIGGER knowledge_sources_source_erasure;
DO $$
DECLARE t uuid; previous_tenant text=current_setting('synveda.tenant_id',true);
BEGIN
 FOR t IN SELECT id FROM public.tenants LOOP
 PERFORM set_config('synveda.tenant_id',t::text,true);
 INSERT INTO public.source_artifact_addresses
 SELECT artifact.tenant_id,artifact.id,job.id,job.project_id,job.scope_id,job.workspace_id,
 encode(sha256(convert_to(jsonb_build_array('declared',job.source_kind,job.source_locator)::text,'UTF8')),'hex'),
 encode(sha256(convert_to(artifact.logical_path,'UTF8')),'hex'),artifact.content_hash,
 public.synveda_artifact_source_key(job.scope_id,NULL,job.source_kind,job.source_locator,
  encode(sha256(convert_to(artifact.logical_path,'UTF8')),'hex'),artifact.content_hash)
 FROM public.import_artifacts artifact JOIN public.import_jobs job ON job.tenant_id=artifact.tenant_id AND job.id=artifact.job_id;
 INSERT INTO public.artifact_dependencies SELECT tenant_id,'import_artifact',artifact_id,source_key_digest FROM public.source_artifact_addresses;
 INSERT INTO public.artifact_dependencies SELECT mapping.tenant_id,'import_mapping',mapping.id,address.source_key_digest
 FROM public.import_mappings mapping JOIN public.source_artifact_addresses address ON address.tenant_id=mapping.tenant_id AND address.artifact_id=mapping.artifact_id;
 INSERT INTO public.artifact_dependencies SELECT link.tenant_id,'capture_candidate',link.candidate_id,address.source_key_digest
 FROM public.capture_candidate_import_artifacts link JOIN public.source_artifact_addresses address ON address.tenant_id=link.tenant_id AND address.artifact_id=link.artifact_id ON CONFLICT DO NOTHING;
 UPDATE public.knowledge_sources source SET import_artifact_id=public.synveda_import_source_artifact(t,to_jsonb(source))
 WHERE source.tenant_id=t AND source.source_type='okf' AND NOT source.content_erased;
 INSERT INTO public.unproven_artifact_dependencies SELECT tenant_id,'knowledge_source',id FROM public.knowledge_sources
 WHERE tenant_id=t AND source_type='okf' AND NOT content_erased AND import_artifact_id IS NULL AND (metadata ? 'artifact_id' OR metadata ? 'import_job_id');
 INSERT INTO public.artifact_dependencies SELECT source.tenant_id,'knowledge_source',source.id,address.source_key_digest
 FROM public.knowledge_sources source JOIN public.source_artifact_addresses address ON address.tenant_id=source.tenant_id AND address.artifact_id=source.import_artifact_id ON CONFLICT DO NOTHING;
 INSERT INTO public.artifact_dependencies SELECT link.tenant_id,'knowledge_revision',link.knowledge_revision_id,dep.source_key_digest
 FROM public.knowledge_revision_source_headers link JOIN public.artifact_dependencies dep ON dep.tenant_id=link.tenant_id AND dep.dependent_kind='knowledge_source' AND dep.dependent_id=link.knowledge_source_id ON CONFLICT DO NOTHING;
 INSERT INTO public.unproven_artifact_dependencies SELECT link.tenant_id,'knowledge_revision',link.knowledge_revision_id
 FROM public.knowledge_revision_source_headers link JOIN public.unproven_artifact_dependencies dep ON dep.tenant_id=link.tenant_id AND dep.dependent_kind='knowledge_source' AND dep.dependent_id=link.knowledge_source_id ON CONFLICT DO NOTHING;
 INSERT INTO public.artifact_dependencies SELECT change.tenant_id,'knowledge_change',change.proposal_id,address.source_key_digest
 FROM public.knowledge_changes change CROSS JOIN LATERAL jsonb_array_elements(coalesce(change.payload->'sources','[]')) source
 JOIN public.source_artifact_addresses address ON address.tenant_id=change.tenant_id AND address.artifact_id=public.synveda_import_source_artifact(t,source) ON CONFLICT DO NOTHING;
 INSERT INTO public.unproven_artifact_dependencies SELECT change.tenant_id,'knowledge_change',change.proposal_id
 FROM public.knowledge_changes change CROSS JOIN LATERAL jsonb_array_elements(coalesce(change.payload->'sources','[]')) source
 WHERE change.tenant_id=t AND source->>'source_type'='okf' AND (source->'metadata' ? 'artifact_id' OR source->'metadata' ? 'import_job_id') AND public.synveda_import_source_artifact(t,source) IS NULL ON CONFLICT DO NOTHING;
 INSERT INTO public.artifact_dependencies SELECT change.tenant_id,'knowledge_change',change.proposal_id,dep.source_key_digest
 FROM public.knowledge_changes change JOIN public.artifact_dependencies dep ON dep.tenant_id=change.tenant_id AND dep.dependent_kind='knowledge_revision' AND dep.dependent_id=(change.payload->>'expected_revision_id')::uuid WHERE change.command_kind='verify' ON CONFLICT DO NOTHING;
 INSERT INTO public.artifact_dependencies SELECT change.tenant_id,'knowledge_change',change.proposal_id,dep.source_key_digest
 FROM public.knowledge_changes change CROSS JOIN LATERAL jsonb_array_elements(coalesce(change.payload->'inputs','[]')) input
 JOIN public.artifact_dependencies dep ON dep.tenant_id=change.tenant_id AND dep.dependent_kind='knowledge_revision' AND dep.dependent_id=(input->>'revision_id')::uuid WHERE change.command_kind='merge' ON CONFLICT DO NOTHING;
 INSERT INTO public.unproven_artifact_dependencies SELECT change.tenant_id,'knowledge_change',change.proposal_id
 FROM public.knowledge_changes change JOIN public.unproven_artifact_dependencies dep ON dep.tenant_id=change.tenant_id AND dep.dependent_kind='knowledge_revision' AND dep.dependent_id=(change.payload->>'expected_revision_id')::uuid WHERE change.command_kind='verify' ON CONFLICT DO NOTHING;
 INSERT INTO public.unproven_artifact_dependencies SELECT change.tenant_id,'knowledge_change',change.proposal_id
 FROM public.knowledge_changes change CROSS JOIN LATERAL jsonb_array_elements(coalesce(change.payload->'inputs','[]')) input
 JOIN public.unproven_artifact_dependencies dep ON dep.tenant_id=change.tenant_id AND dep.dependent_kind='knowledge_revision' AND dep.dependent_id=(input->>'revision_id')::uuid WHERE change.command_kind='merge' ON CONFLICT DO NOTHING;
 INSERT INTO public.artifact_dependencies SELECT m.tenant_id,'context_run',m.context_run_id,dep.source_key_digest
 FROM public.context_run_delivery_dependencies m CROSS JOIN LATERAL
 (SELECT 'knowledge_revision'::text k,(x->>'revision_id')::uuid d FROM jsonb_array_elements(m.manifest->'knowledge') x
  UNION SELECT 'capture_candidate',(x->>'candidate_id')::uuid FROM jsonb_array_elements(m.manifest->'candidates') x
  UNION SELECT 'knowledge_revision',v::uuid FROM jsonb_array_elements(m.manifest->'graph') x
   CROSS JOIN LATERAL (VALUES(x->>'asserting_revision_id'),(x->>'from_revision_id'),(x->>'to_revision_id')) revisions(v) WHERE v IS NOT NULL) root
 JOIN public.artifact_dependencies dep ON dep.tenant_id=m.tenant_id AND dep.dependent_kind=root.k AND dep.dependent_id=root.d ON CONFLICT DO NOTHING;
 INSERT INTO public.unproven_artifact_dependencies SELECT m.tenant_id,'context_run',m.context_run_id
 FROM public.context_run_delivery_dependencies m CROSS JOIN LATERAL
 (SELECT 'knowledge_revision'::text k,(x->>'revision_id')::uuid d FROM jsonb_array_elements(m.manifest->'knowledge') x
  UNION SELECT 'knowledge_revision',v::uuid FROM jsonb_array_elements(m.manifest->'graph') x
   CROSS JOIN LATERAL (VALUES(x->>'asserting_revision_id'),(x->>'from_revision_id'),(x->>'to_revision_id')) revisions(v) WHERE v IS NOT NULL) root
 JOIN public.unproven_artifact_dependencies dep ON dep.tenant_id=m.tenant_id AND dep.dependent_kind=root.k AND dep.dependent_id=root.d ON CONFLICT DO NOTHING;
 END LOOP;
 PERFORM set_config('synveda.tenant_id',coalesce(previous_tenant,''),true);
END $$;
ALTER TABLE public.knowledge_sources ENABLE TRIGGER knowledge_sources_source_erasure;
CREATE POLICY revision_source_eligibility ON public.knowledge_revisions AS RESTRICTIVE FOR SELECT
 USING (NOT content_erased AND public.synveda_source_available(tenant_id,'knowledge_revision',id)
 OR public.synveda_source_cleanup_authorized(tenant_id,'knowledge_revision',id)
 OR public.synveda_artifact_cleanup_authorized(tenant_id,'knowledge_revision',id)
 OR public.synveda_item_cleanup_authorized(tenant_id,'knowledge_revision',id));
CREATE POLICY descriptor_source_eligibility ON public.knowledge_sources AS RESTRICTIVE FOR SELECT
 USING (NOT content_erased AND public.synveda_source_available(tenant_id,'knowledge_source',id)
 OR public.synveda_source_cleanup_authorized(tenant_id,'knowledge_source',id)
 OR public.synveda_artifact_cleanup_authorized(tenant_id,'knowledge_source',id)
 OR public.synveda_item_cleanup_authorized(tenant_id,'knowledge_source',id));
CREATE POLICY candidate_source_eligibility ON public.capture_candidates AS RESTRICTIVE FOR SELECT USING (public.synveda_source_available(tenant_id,'capture_candidate',id));
CREATE POLICY decision_source_eligibility ON public.capture_candidate_decisions AS RESTRICTIVE FOR SELECT USING (public.synveda_source_available(tenant_id,'capture_candidate',candidate_id));
CREATE POLICY change_source_eligibility ON public.knowledge_changes AS RESTRICTIVE FOR SELECT USING (command_kind IN ('control_source','control_artifact') OR public.synveda_source_available(tenant_id,'knowledge_change',proposal_id));
CREATE POLICY context_source_eligibility ON public.session_context_runs AS RESTRICTIVE FOR SELECT
 USING ((NOT EXISTS(SELECT 1 FROM public.unproven_context_deliveries unproven
  WHERE unproven.tenant_id=session_context_runs.tenant_id AND unproven.context_run_id=session_context_runs.id)
 OR rendered='' OR public.synveda_item_cleanup_authorized(tenant_id,'context_run',id))
 AND public.synveda_source_available(tenant_id,'context_run',id));
CREATE POLICY artifact_source_eligibility ON public.import_artifacts AS RESTRICTIVE FOR SELECT
 USING (NOT content_erased AND public.synveda_source_available(tenant_id,'import_artifact',id)
 OR public.synveda_artifact_cleanup_authorized(tenant_id,'import_artifact',id));
CREATE POLICY mapping_source_eligibility ON public.import_mappings AS RESTRICTIVE FOR SELECT
 USING (public.synveda_source_available(tenant_id,'import_mapping',id)
 OR public.synveda_item_cleanup_authorized(tenant_id,'knowledge_item',matched_item_id)
 OR public.synveda_item_cleanup_authorized(tenant_id,'capture_candidate',candidate_id));

CREATE FUNCTION public.synveda_artifact_control_binding() RETURNS trigger
 LANGUAGE plpgsql SECURITY DEFINER SET search_path TO 'public','pg_temp' AS $$
DECLARE h public.knowledge_change_headers%ROWTYPE; addr public.source_artifact_addresses%ROWTYPE;
BEGIN
 IF NEW.tenant_id IS DISTINCT FROM public.synveda_current_tenant() THEN
  RAISE EXCEPTION 'artifact binding tenant mismatch' USING ERRCODE='23514'; END IF;
 SELECT * INTO h FROM public.knowledge_change_headers WHERE tenant_id=NEW.tenant_id AND proposal_id=NEW.change_id;
 SELECT * INTO addr FROM public.source_artifact_addresses WHERE tenant_id=NEW.tenant_id AND artifact_id=NEW.artifact_id;
 IF h.command_kind IS DISTINCT FROM 'control_artifact' OR h.source_artifact_id IS DISTINCT FROM addr.artifact_id
 OR h.source_scope_id IS DISTINCT FROM addr.scope_id OR h.source_artifact_hash IS DISTINCT FROM addr.content_hash
 OR NEW.source_key_digest IS DISTINCT FROM addr.source_key_digest THEN
  RAISE EXCEPTION 'artifact control differs from reviewed intent' USING ERRCODE='23514'; END IF;
 IF TG_TABLE_NAME='artifact_suppressions' THEN
  IF TG_OP='UPDATE' THEN
   IF OLD.forget_change_id IS NOT NULL OR NEW.forget_change_id IS NULL
   OR (to_jsonb(NEW)-'forget_change_id') IS DISTINCT FROM (to_jsonb(OLD)-'forget_change_id') THEN
    RAISE EXCEPTION 'artifact suppression is permanent' USING ERRCODE='23514'; END IF;
   SELECT * INTO h FROM public.knowledge_change_headers WHERE tenant_id=NEW.tenant_id AND proposal_id=NEW.forget_change_id;
   IF h.command_kind IS DISTINCT FROM 'control_artifact' OR h.source_mode IS DISTINCT FROM 'forget'
   OR NOT EXISTS(SELECT 1 FROM public.source_artifact_addresses a WHERE a.tenant_id=NEW.tenant_id
    AND a.artifact_id=h.source_artifact_id AND a.source_key_digest=NEW.source_key_digest
    AND a.scope_id=h.source_scope_id AND a.content_hash=h.source_artifact_hash) THEN
    RAISE EXCEPTION 'artifact forgetting differs from reviewed intent' USING ERRCODE='23514'; END IF;
  ELSIF (h.source_mode='forget') IS DISTINCT FROM (NEW.forget_change_id IS NOT NULL AND NEW.forget_change_id=NEW.change_id) THEN
   RAISE EXCEPTION 'artifact suppression mode mismatch' USING ERRCODE='23514'; END IF;
 ELSE
  IF NEW.input_hash IS DISTINCT FROM h.payload_hash OR NEW.phase<>0 OR NEW.cursor_id IS NOT NULL OR NEW.processed_targets<>0
  OR NOT EXISTS(SELECT 1 FROM public.durable_operations WHERE tenant_id=NEW.tenant_id AND id=NEW.operation_id
   AND kind='source_cleanup' AND proposal_id=NEW.change_id AND input_hash=NEW.input_hash)
  OR EXISTS(SELECT 1 FROM public.source_cleanup_targets WHERE tenant_id=NEW.tenant_id AND operation_id=NEW.operation_id) THEN
   RAISE EXCEPTION 'artifact cleanup differs from reviewed intent' USING ERRCODE='23514'; END IF;
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION public.synveda_artifact_control_binding() FROM PUBLIC;
CREATE TRIGGER artifact_suppression_binding BEFORE INSERT OR UPDATE ON public.artifact_suppressions FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_control_binding();
CREATE TRIGGER artifact_cleanup_binding BEFORE INSERT ON public.artifact_cleanup_targets FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_control_binding();
CREATE FUNCTION public.synveda_artifact_applied_binding() RETURNS trigger
 LANGUAGE plpgsql SECURITY DEFINER SET search_path TO 'public','pg_temp' AS $$
BEGIN
 IF NOT EXISTS(SELECT 1 FROM public.knowledge_change_headers h JOIN public.vedaflow_proposals p ON p.tenant_id=h.tenant_id AND p.id=h.proposal_id
 JOIN public.artifact_cleanup_targets target ON target.tenant_id=h.tenant_id AND target.change_id=h.proposal_id
 WHERE h.tenant_id=NEW.tenant_id AND h.proposal_id=NEW.change_id AND h.applied_at IS NOT NULL
 AND h.operation_id=target.operation_id AND target.source_key_digest=NEW.source_key_digest AND p.state='applied')
 OR (NEW.forget_change_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM public.knowledge_change_headers h
 JOIN public.vedaflow_proposals p ON p.tenant_id=h.tenant_id AND p.id=h.proposal_id
 JOIN public.artifact_cleanup_targets target ON target.tenant_id=h.tenant_id AND target.change_id=h.proposal_id
 WHERE h.tenant_id=NEW.tenant_id AND h.proposal_id=NEW.forget_change_id AND h.applied_at IS NOT NULL
 AND h.operation_id=target.operation_id AND target.source_key_digest=NEW.source_key_digest AND p.state='applied')) THEN
  RAISE EXCEPTION 'artifact suppression requires an applied effect' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION public.synveda_artifact_applied_binding() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER artifact_suppression_applied AFTER INSERT OR UPDATE ON public.artifact_suppressions
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_applied_binding();
CREATE FUNCTION public.synveda_artifact_cleanup_progress() RETURNS trigger
 LANGUAGE plpgsql SET search_path TO 'public','pg_temp' AS $$
BEGIN
 IF NOT public.synveda_artifact_cleanup_authorized(OLD.tenant_id,'import_artifact',OLD.artifact_id)
 OR (to_jsonb(NEW)-ARRAY['phase','cursor_id','processed_targets']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['phase','cursor_id','processed_targets'])
 OR NEW.phase<OLD.phase OR NEW.processed_targets<OLD.processed_targets THEN
  RAISE EXCEPTION 'artifact cleanup progress needs its live attempt' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION public.synveda_artifact_cleanup_progress() FROM PUBLIC;
CREATE TRIGGER artifact_cleanup_progress BEFORE UPDATE ON public.artifact_cleanup_targets FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_cleanup_progress();
CREATE FUNCTION public.synveda_artifact_content_erasure() RETURNS trigger
 LANGUAGE plpgsql SET search_path TO 'public','pg_temp' AS $$
BEGIN
 IF OLD.content_erased OR NOT NEW.content_erased OR NEW.logical_path<>'erased/'||OLD.id::text OR NEW.body_markdown<>'' OR NEW.frontmatter<>'{}'
 OR NOT public.synveda_artifact_cleanup_authorized(OLD.tenant_id,'import_artifact',OLD.id)
 OR (to_jsonb(NEW)-ARRAY['logical_path','frontmatter','body_markdown','content_erased']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['logical_path','frontmatter','body_markdown','content_erased']) THEN
  RAISE EXCEPTION 'artifact erasure requires exact one-way approved cleanup' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION public.synveda_artifact_content_erasure() FROM PUBLIC;
DROP TRIGGER import_artifacts_append_only ON public.import_artifacts;
CREATE TRIGGER import_artifacts_append_only BEFORE DELETE OR TRUNCATE ON public.import_artifacts FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_import_append_only();
CREATE TRIGGER import_artifact_source_erasure BEFORE UPDATE ON public.import_artifacts FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_content_erasure();
CREATE TRIGGER artifact_addresses_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON public.source_artifact_addresses FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();
CREATE TRIGGER artifact_dependencies_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON public.artifact_dependencies FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();
CREATE TRIGGER artifact_suppressions_permanent BEFORE DELETE OR TRUNCATE ON public.artifact_suppressions FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();
REVOKE ALL ON FUNCTION public.synveda_artifact_source_key(uuid,text,text,text,text,text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.synveda_artifact_source_key(uuid,text,text,text,text,text) TO synveda_app;



CREATE OR REPLACE FUNCTION public.synveda_source_available(t uuid,k text,d uuid) RETURNS boolean
 LANGUAGE sql STABLE SET search_path TO 'public','pg_temp' AS $$
 SELECT t=public.synveda_current_tenant() AND (
 (NOT EXISTS(SELECT 1 FROM public.source_dependencies dep JOIN public.source_suppressions s ON s.tenant_id=dep.tenant_id AND s.event_id=dep.source_event_id WHERE dep.tenant_id=t AND dep.dependent_kind=k AND dep.dependent_id=d)
 AND NOT EXISTS(SELECT 1 FROM public.artifact_dependencies dep JOIN public.artifact_suppressions s ON s.tenant_id=dep.tenant_id AND s.source_key_digest=dep.source_key_digest WHERE dep.tenant_id=t AND dep.dependent_kind=k AND dep.dependent_id=d)
 AND NOT EXISTS(SELECT 1 FROM public.unproven_artifact_dependencies WHERE tenant_id=t AND dependent_kind=k AND dependent_id=d))
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
 AND NOT EXISTS(SELECT 1 FROM public.unproven_artifact_dependencies WHERE tenant_id=t AND dependent_kind=k AND dependent_id=d)
$$;

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

CREATE OR REPLACE FUNCTION public.synveda_source_content_erasure() RETURNS trigger
LANGUAGE plpgsql SET search_path TO 'public','pg_temp' AS $$
DECLARE k text;
BEGIN
 k=CASE TG_TABLE_NAME WHEN 'knowledge_revisions' THEN 'knowledge_revision'
      WHEN 'session_events' THEN 'session_event' WHEN 'knowledge_relations' THEN 'knowledge_revision' ELSE 'knowledge_source' END;
 IF NOT (public.synveda_source_cleanup_authorized(OLD.tenant_id,k,CASE WHEN TG_TABLE_NAME='knowledge_relations' THEN (to_jsonb(OLD)->>'asserting_revision_id')::uuid ELSE OLD.id END) OR public.synveda_artifact_cleanup_authorized(OLD.tenant_id,k,CASE WHEN TG_TABLE_NAME='knowledge_relations' THEN (to_jsonb(OLD)->>'asserting_revision_id')::uuid ELSE OLD.id END))
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

CREATE OR REPLACE FUNCTION public.synveda_source_cleanup_chunk(t uuid,o uuid,w text,a integer,n integer)
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
   IF EXISTS (SELECT 1 FROM public.unproven_context_deliveries WHERE tenant_id=t) OR EXISTS(SELECT 1 FROM public.unproven_artifact_dependencies WHERE tenant_id=t) THEN
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

CREATE FUNCTION public.synveda_artifact_cleanup_chunk(t uuid,o uuid,w text,a integer,n integer)
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
      WHEN 4 THEN 'knowledge_revision' WHEN 5 THEN 'knowledge_revision' WHEN 6 THEN 'knowledge_revision' WHEN 7 THEN 'knowledge_source' WHEN 8 THEN 'import_mapping' ELSE 'import_artifact' END;
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
 ELSE
   UPDATE public.import_jobs job SET notices='[]' WHERE job.tenant_id=t AND job.notices<>'[]'
    AND EXISTS(SELECT 1 FROM public.source_artifact_addresses address WHERE address.tenant_id=t AND address.job_id=job.id AND address.artifact_id=ANY(ids));
   UPDATE public.import_artifacts SET content_erased=true,logical_path='erased/'||id::text,frontmatter='{}',body_markdown=''
    WHERE tenant_id=t AND id=ANY(ids) AND NOT content_erased;
 END IF;
 affected=CASE WHEN target.phase=0 THEN 0 ELSE coalesce(cardinality(ids),0) END;
 IF coalesce(cardinality(ids),0)<n THEN
   IF target.phase<9 THEN target.phase=target.phase+1; target.cursor_id=NULL;
   ELSE target.phase=9; target.cursor_id=last_id; END IF;
 ELSE target.cursor_id=last_id; END IF;
 UPDATE public.artifact_cleanup_targets SET phase=target.phase,cursor_id=target.cursor_id,processed_targets=processed_targets+affected
    WHERE tenant_id=t AND operation_id=o;
 done=(kind='import_artifact' AND coalesce(cardinality(ids),0)<n);
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
REVOKE ALL ON FUNCTION public.synveda_artifact_cleanup_chunk(uuid,uuid,text,integer,integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.synveda_artifact_cleanup_chunk(uuid,uuid,text,integer,integer) TO synveda_app;

CREATE OR REPLACE FUNCTION public.synveda_import_mapping_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if current_setting('synveda.knowledge_erasure', true) = 'on'
       and current_user = pg_catalog.pg_get_userbyid((
           select database.datdba from pg_catalog.pg_database as database
           where database.datname = pg_catalog.current_database()
       )) then
        if old.content_erased and new.okf_type='[erased]' and old.okf_type<>'[erased]'
           and (to_jsonb(new)-'okf_type') is not distinct from (to_jsonb(old)-'okf_type')
           and (public.synveda_artifact_cleanup_authorized(old.tenant_id,'import_mapping',old.id)
             or public.synveda_item_cleanup_authorized(old.tenant_id,'knowledge_item',old.matched_item_id)
             or public.synveda_item_cleanup_authorized(old.tenant_id,'capture_candidate',old.candidate_id)) then
            return new;
        end if;
        if not (public.synveda_artifact_cleanup_authorized(old.tenant_id,'import_mapping',old.id) or public.synveda_item_cleanup_authorized(old.tenant_id,'knowledge_item',old.matched_item_id) or public.synveda_item_cleanup_authorized(old.tenant_id,'capture_candidate',old.candidate_id)) or not new.content_erased or old.content_erased
           or new.title <> '' or new.body_markdown <> '' or new.summary <> ''
           or new.tags <> '{}'::text[]
           or new.verification_metadata <> '{}'::jsonb
           or new.metadata <> '{}'::jsonb
           or new.proposed_relations <> '[]'::jsonb
           or (new.okf_type <> old.okf_type and new.okf_type <> '[erased]')
           or new.materializable
           or new.matched_item_id is not null
           or new.matched_revision_id is not null
           or (to_jsonb(new) - array[
                   'title', 'body_markdown', 'summary', 'tags',
                   'verification_metadata', 'metadata', 'proposed_relations',
                   'matched_item_id', 'matched_revision_id', 'materializable',
                   'content_erased', 'okf_type'
               ])
               <> (to_jsonb(old) - array[
                   'title', 'body_markdown', 'summary', 'tags',
                   'verification_metadata', 'metadata', 'proposed_relations',
                   'matched_item_id', 'matched_revision_id', 'materializable',
                   'content_erased', 'okf_type'
               ]) then
            raise exception 'import mapping has an invalid Knowledge erasure scrub'
                using errcode = '23514';
        end if;
        return new;
    end if;
    if new.id <> old.id or new.tenant_id <> old.tenant_id
       or new.job_id <> old.job_id or new.artifact_id <> old.artifact_id
       or new.ordinal <> old.ordinal or new.okf_type <> old.okf_type
       or new.knowledge_type <> old.knowledge_type or new.title <> old.title
       or new.body_markdown <> old.body_markdown or new.summary <> old.summary
       or new.tags <> old.tags or new.sensitivity <> old.sensitivity
       or new.confidence_permille <> old.confidence_permille
       or new.valid_from <> old.valid_from or new.valid_to is distinct from old.valid_to
       or new.stale_after is distinct from old.stale_after
       or new.verification_metadata <> old.verification_metadata
       or new.metadata <> old.metadata or new.content_hash <> old.content_hash
       or new.classification <> old.classification
       or new.matched_item_id is distinct from old.matched_item_id
       or new.matched_revision_id is distinct from old.matched_revision_id
       or new.proposed_relations <> old.proposed_relations
       or new.materializable <> old.materializable or new.created_at <> old.created_at then
        raise exception 'import mapping is immutable';
    end if;
    if old.candidate_id is not null or new.candidate_id is null then
        raise exception 'import mapping candidate may be assigned exactly once';
    end if;
    return new;
end
$$;

CREATE OR REPLACE FUNCTION public.synveda_import_job_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.notices='[]'::jsonb and old.notices<>'[]'::jsonb
      and (to_jsonb(new)-'notices') is not distinct from (to_jsonb(old)-'notices')
      and exists(select 1 from public.source_artifact_addresses address
        where address.tenant_id=old.tenant_id and address.job_id=old.id
        and public.synveda_artifact_cleanup_authorized(old.tenant_id,'import_artifact',address.artifact_id)) then
      return new;
    end if;
    if new.repository_id is distinct from old.repository_id or new.repository_identity_digest is distinct from old.repository_identity_digest then
      raise exception 'import repository identity is immutable';
    end if;
    if new.id <> old.id or new.tenant_id <> old.tenant_id
       or new.project_id <> old.project_id or new.scope_id <> old.scope_id
       or new.workspace_id <> old.workspace_id or new.principal_id <> old.principal_id
       or new.format <> old.format or new.format_version <> old.format_version
       or new.specification_commit <> old.specification_commit
       or new.source_kind <> old.source_kind or new.source_locator <> old.source_locator
       or new.source_revision is distinct from old.source_revision
       or new.bundle_digest <> old.bundle_digest or new.artifact_count <> old.artifact_count
       or new.mapping_count <> old.mapping_count or new.notices <> old.notices
       or new.created_at <> old.created_at then
        raise exception 'import job plan is immutable';
    end if;
    if old.state <> 'planned' and new is distinct from old then
        raise exception 'import job result is terminal';
    end if;
    if old.state = 'planned' and new.state not in ('planned', 'materialized', 'failed') then
        raise exception 'invalid import job transition: % -> %', old.state, new.state;
    end if;
    return new;
end
$$;

CREATE FUNCTION public.synveda_import_repository_identity() RETURNS trigger
 LANGUAGE plpgsql SECURITY DEFINER SET search_path TO 'public','pg_temp' AS $$
BEGIN
 IF NEW.tenant_id IS DISTINCT FROM public.synveda_current_tenant() THEN
  RAISE EXCEPTION 'import repository tenant mismatch' USING ERRCODE='23514'; END IF;
 IF NEW.repository_id IS NULL THEN NEW.repository_identity_digest=NULL;
 ELSE
  SELECT encode(sha256(convert_to(jsonb_build_array('synveda.repository.source.v1',r.tenant_id,r.project_id,lower(r.canonical_uri))::text,'UTF8')),'hex')
   INTO NEW.repository_identity_digest FROM public.project_repositories r
   WHERE r.tenant_id=NEW.tenant_id AND r.project_id=NEW.project_id AND r.id=NEW.repository_id;
  IF NEW.repository_identity_digest IS NULL THEN
   RAISE EXCEPTION 'import repository differs from project' USING ERRCODE='23514'; END IF;
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION public.synveda_import_repository_identity() FROM PUBLIC;
CREATE TRIGGER import_repository_identity BEFORE INSERT ON public.import_jobs FOR EACH ROW EXECUTE FUNCTION public.synveda_import_repository_identity();

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
