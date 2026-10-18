-- OPS-11 / CPR-45, ADR-0152: portable epoch-3 baseline for fresh installations.
-- It is the published 0001-0011 head, dumped once and reviewed, with no
-- syntax newer than PostgreSQL 14. Published-lineage databases keep their
-- own SQLx ledger and never run this file.
--
-- PostgreSQL database dump
--



SET statement_timeout = 0;
SET lock_timeout = 0;
SET idle_in_transaction_session_timeout = 0;
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;

-- Cluster roles and required extensions are deployment infrastructure, not
-- application schema history (ADR-0069 decision 13; CPR-45). The deployment
-- bootstrap must establish the safe NOLOGIN `synveda_app` role and install
-- `btree_gin` and `vector` before this narrow migration owner runs the
-- baseline. Keeping that authority out of the baseline lets the migrator own
-- domain objects without CREATEROLE or superuser privileges.

--
-- Name: synveda_artifact_applied_binding(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_artifact_applied_binding() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'public', 'pg_temp'
    AS $$
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


--
-- Name: synveda_artifact_cleanup_authorized(uuid, text, uuid); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_artifact_cleanup_authorized(t uuid, k text, d uuid) RETURNS boolean
    LANGUAGE sql STABLE
    SET search_path TO 'public', 'pg_temp'
    AS $$
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


--
-- Name: synveda_artifact_cleanup_chunk(uuid, uuid, text, integer, integer); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_artifact_cleanup_chunk(t uuid, o uuid, w text, a integer, n integer) RETURNS TABLE(progress smallint, done boolean, erased bigint)
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'public', 'pg_temp'
    AS $$
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


--
-- Name: synveda_artifact_cleanup_progress(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_artifact_cleanup_progress() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'public', 'pg_temp'
    AS $$
BEGIN
 IF NOT public.synveda_artifact_cleanup_authorized(OLD.tenant_id,'import_artifact',OLD.artifact_id)
 OR (to_jsonb(NEW)-ARRAY['phase','cursor_id','processed_targets']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['phase','cursor_id','processed_targets'])
 OR NEW.phase<OLD.phase OR NEW.processed_targets<OLD.processed_targets THEN
  RAISE EXCEPTION 'artifact cleanup progress needs its live attempt' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;


--
-- Name: synveda_artifact_content_erasure(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_artifact_content_erasure() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'public', 'pg_temp'
    AS $$
BEGIN
 IF OLD.content_erased OR NOT NEW.content_erased OR NEW.logical_path<>'erased/'||OLD.id::text OR NEW.body_markdown<>'' OR NEW.frontmatter<>'{}'
 OR NOT public.synveda_artifact_cleanup_authorized(OLD.tenant_id,'import_artifact',OLD.id)
 OR (to_jsonb(NEW)-ARRAY['logical_path','frontmatter','body_markdown','content_erased']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['logical_path','frontmatter','body_markdown','content_erased']) THEN
  RAISE EXCEPTION 'artifact erasure requires exact one-way approved cleanup' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;


--
-- Name: synveda_artifact_control_binding(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_artifact_control_binding() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'public', 'pg_temp'
    AS $$
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


--
-- Name: synveda_artifact_projection(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_artifact_projection() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'public', 'pg_temp'
    AS $$
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


--
-- Name: synveda_artifact_source_key(uuid, text, text, text, text, text); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_artifact_source_key(s uuid, r text, k text, l text, p text, h text) RETURNS text
    LANGUAGE sql IMMUTABLE
    SET search_path TO 'public', 'pg_temp'
    AS $$
 SELECT encode(sha256(convert_to(jsonb_build_array('synveda.artifact.source.v1',s,
   CASE WHEN r IS NULL THEN jsonb_build_array('declared',k,l) ELSE jsonb_build_array('repository',r) END,
   p,h)::text,'UTF8')),'hex')
$$;


--
-- Name: synveda_audit_log_immutable(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_audit_log_immutable() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    raise exception 'audit_log is append-only (AUD-1, ADR-0019)';
end
$$;


--
-- Name: synveda_bind_github_source_identity(uuid, uuid, uuid, uuid, bigint); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_bind_github_source_identity(wanted_tenant uuid, wanted_project uuid, wanted_scope uuid, wanted_repository uuid, wanted_github_id bigint) RETURNS text
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'pg_catalog', 'public'
    AS $$
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


--
-- Name: synveda_capture_append_only(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_capture_append_only() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if tg_op = 'DELETE'
       and current_user = pg_catalog.pg_get_userbyid((
           select database.datdba from pg_catalog.pg_database as database
           where database.datname = pg_catalog.current_database()
       ))
       and (current_setting('synveda.knowledge_erasure', true) = 'on'
            or current_setting('synveda.retention_purge', true) = 'on') then
        -- This is a statement trigger: returning NULL allows the statement
        -- to proceed and avoids reading an unassigned OLD record.
        return null;
    end if;
    raise exception '% is append-only (CPR-18, ADR-0083)', tg_table_name;
end
$$;


--
-- Name: synveda_capture_batch_source_identity(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_capture_batch_source_identity() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.source_kind = 'session' then
        if not exists (
            select 1 from sessions session
            where session.tenant_id = new.tenant_id
              and session.id = new.session_id
              and session.scope_id = new.scope_id
              and session.workspace_id = new.workspace_id
              and session.project_id is not distinct from new.project_id
              and session.principal_id = new.principal_id
        ) then
            raise exception 'capture batch identity must match its session'
                using errcode = '23514';
        end if;
    elsif not exists (
        select 1 from import_jobs job
        where job.tenant_id = new.tenant_id
          and job.id = new.import_job_id
          and job.scope_id = new.scope_id
          and job.workspace_id = new.workspace_id
          and job.project_id = new.project_id
    ) then
        raise exception 'capture batch identity must match its import job'
            using errcode = '23514';
    end if;
    return new;
end
$$;


--
-- Name: synveda_capture_batch_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_capture_batch_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.id <> old.id or new.tenant_id <> old.tenant_id
       or new.source_kind <> old.source_kind
       or new.session_id is distinct from old.session_id
       or new.import_job_id is distinct from old.import_job_id
       or new.scope_id <> old.scope_id or new.workspace_id <> old.workspace_id
       or new.project_id is distinct from old.project_id
       or new.principal_id <> old.principal_id or new.input_hash <> old.input_hash
       or new.event_count <> old.event_count or new.created_at <> old.created_at then
        raise exception 'capture batch evidence is immutable';
    end if;
    if not (
        (old.state = 'pending' and new.state = 'running')
        or (old.state = 'running' and new.state in ('pending', 'completed', 'failed'))
        or (old.state = new.state)
    ) then
        raise exception 'invalid capture batch transition: % -> %', old.state, new.state;
    end if;
    return new;
end
$$;


--
-- Name: synveda_capture_candidate_source_identity(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_capture_candidate_source_identity() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if not exists (
        select 1 from capture_batches batch
        where batch.tenant_id = new.tenant_id
          and batch.id = new.batch_id
          and batch.source_kind = new.source_kind
          and batch.session_id is not distinct from new.session_id
          and batch.import_job_id is not distinct from new.import_job_id
    ) then
        raise exception 'capture candidate source must match its batch'
            using errcode = '23514';
    end if;
    return new;
end
$$;


--
-- Name: synveda_capture_candidate_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_capture_candidate_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.id <> old.id or new.tenant_id <> old.tenant_id
       or new.batch_id <> old.batch_id or new.source_kind <> old.source_kind
       or new.session_id is distinct from old.session_id
       or new.import_job_id is distinct from old.import_job_id
       or new.ordinal <> old.ordinal or new.proposed_scope_id <> old.proposed_scope_id
       or new.proposed_project_id is distinct from old.proposed_project_id
       or new.proposed_owner_principal_id is distinct from old.proposed_owner_principal_id
       or new.knowledge_type <> old.knowledge_type or new.origin <> old.origin
       or new.content_hash <> old.content_hash or new.created_at <> old.created_at then
        raise exception 'capture candidate identity and proposal are immutable';
    end if;
    if current_setting('synveda.knowledge_erasure', true) = 'on'
       and current_user = pg_catalog.pg_get_userbyid((
           select database.datdba from pg_catalog.pg_database as database
           where database.datname = pg_catalog.current_database()
       )) then
        if not new.content_erased or old.content_erased then
            raise exception 'capture candidate erasure is one-way';
        end if;
        return new;
    end if;
    if new.title <> old.title or new.body_markdown <> old.body_markdown
       or new.summary <> old.summary or new.tags <> old.tags
       or new.sensitivity <> old.sensitivity
       or new.confidence_permille <> old.confidence_permille
       or new.valid_from <> old.valid_from or new.valid_to is distinct from old.valid_to
       or new.stale_after is distinct from old.stale_after
       or new.verification_metadata <> old.verification_metadata
       or new.metadata <> old.metadata or new.content_erased <> old.content_erased then
        raise exception 'capture candidate content is immutable';
    end if;
    if old.state <> 'pending' and new is distinct from old then
        raise exception 'capture candidate decision is terminal';
    end if;
    return new;
end
$$;


--
-- Name: synveda_capture_decision_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_capture_decision_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.id <> old.id or new.tenant_id <> old.tenant_id
       or new.candidate_id <> old.candidate_id or new.action <> old.action
       or new.actor_subject <> old.actor_subject
       or new.idempotency_key <> old.idempotency_key
       or new.request_hash <> old.request_hash
       or new.payload_hash <> old.payload_hash or new.created_at <> old.created_at then
        raise exception 'capture decision intent is immutable';
    end if;
    if current_setting('synveda.knowledge_erasure', true) = 'on'
       and current_user = pg_catalog.pg_get_userbyid((
           select database.datdba from pg_catalog.pg_database as database
           where database.datname = pg_catalog.current_database()
       )) then
        if new.payload is not null then
            raise exception 'capture decision erasure may only clear payload';
        end if;
        return new;
    end if;
    if new.payload is distinct from old.payload then
        raise exception 'capture decision payload is immutable';
    end if;
    if old.state <> 'running' and new is distinct from old then
        raise exception 'capture decision result is terminal';
    end if;
    return new;
end
$$;


--
-- Name: synveda_capture_scrub_for_knowledge(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_capture_scrub_for_knowledge() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if current_setting('synveda.knowledge_erasure', true) <> 'on'
       or current_user <> pg_catalog.pg_get_userbyid((
           select database.datdba from pg_catalog.pg_database as database
           where database.datname = pg_catalog.current_database()
       )) then
        return old;
    end if;
    update capture_candidate_decisions decision
       set payload = null
      from capture_candidates candidate
     where candidate.tenant_id = old.tenant_id
       and candidate.resulting_knowledge_item_id = old.id
       and decision.tenant_id = candidate.tenant_id
       and decision.candidate_id = candidate.id
       and decision.payload is not null;
    update capture_candidates
       set title = '', body_markdown = '', summary = '', tags = '{}'::text[],
           verification_metadata = '{}'::jsonb, metadata = '{}'::jsonb,
           content_erased = true
     where tenant_id = old.tenant_id
       and resulting_knowledge_item_id = old.id
       and not content_erased;
    return old;
end
$$;


--
-- Name: synveda_change_header_result(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_change_header_result() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'public', 'pg_temp'
    AS $$
BEGIN
 IF current_user<>pg_catalog.pg_get_userbyid((SELECT datdba FROM pg_catalog.pg_database WHERE datname=current_database()))
   OR OLD.applied_at IS NOT NULL OR NEW.applied_at IS NULL
   OR (to_jsonb(NEW)-ARRAY['operation_id','applied_at']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['operation_id','applied_at']) THEN
   RAISE EXCEPTION 'immutable command header' USING ERRCODE='42501'; END IF;
 RETURN NEW;
END $$;


--
-- Name: synveda_configuration_aggregate_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_configuration_aggregate_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.id <> old.id
        or new.tenant_id <> old.tenant_id
        or new.governing_scope_id <> old.governing_scope_id
        or new.name <> old.name
        or new.created_at <> old.created_at
        or new.created_by <> old.created_by
    then
        raise exception 'a Configuration aggregate identity is immutable (CPR-30)';
    end if;
    if new.current_version_id = old.current_version_id then
        raise exception 'a Configuration publication must advance its immutable version (CPR-30)';
    end if;
    return new;
end
$$;


--
-- Name: synveda_configuration_binding_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_configuration_binding_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.id <> old.id
        or new.tenant_id <> old.tenant_id
        or new.scope_id <> old.scope_id
        or new.created_at <> old.created_at
        or new.created_by <> old.created_by
    then
        raise exception 'a Configuration binding identity is immutable (CPR-30)';
    end if;
    if new.revision <> old.revision + 1 then
        raise exception 'a Configuration binding update must advance revision exactly once (CPR-30)';
    end if;
    if new.artifact_id = old.artifact_id
       and new.enabled = old.enabled
       and new.pinned_version_id is not distinct from old.pinned_version_id
    then
        raise exception 'a Configuration binding update must change selection, pin or state (CPR-30)';
    end if;
    return new;
end
$$;


--
-- Name: synveda_configuration_change_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_configuration_change_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.tenant_id <> old.tenant_id
        or new.proposal_id <> old.proposal_id
        or new.command_kind <> old.command_kind
        or new.payload <> old.payload
        or new.payload_hash <> old.payload_hash
        or new.created_at <> old.created_at
    then
        raise exception 'a Configuration VedaFlow command is immutable (CPR-30)';
    end if;
    if old.applied_at is not null or new.applied_at is null then
        raise exception 'a Configuration result may be recorded exactly once (CPR-30)';
    end if;
    return new;
end
$$;


--
-- Name: synveda_configuration_version_matches_proposal(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_configuration_version_matches_proposal() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if not exists (
        select 1 from vedaflow_proposals proposal
         where proposal.tenant_id = new.tenant_id
           and proposal.id = new.proposal_id
           and proposal.asset_kind = 'configuration'
           and proposal.target_channel = 'apply'
    ) then
        raise exception 'Configuration version must bind a Configuration/apply proposal'
            using errcode = '23514';
    end if;
    return new;
end
$$;


--
-- Name: synveda_context_delivery_knowledge_erasure(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_context_delivery_knowledge_erasure() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'public', 'pg_temp'
    AS $$
begin
    if coalesce(current_setting('synveda.knowledge_erasure', true), 'off') <> 'on'
       or old.tenant_id IS DISTINCT FROM public.synveda_current_tenant()
       or not exists (
           select 1 from public.knowledge_erasure_tombstones tombstone
           join public.durable_operations operation
             on operation.tenant_id = tombstone.tenant_id
            and operation.id = tombstone.operation_id
            and operation.proposal_id = tombstone.proposal_id
            and operation.knowledge_item_id = tombstone.knowledge_item_id
           where tombstone.tenant_id = old.tenant_id
             and tombstone.knowledge_item_id = old.knowledge_item_id
             and operation.kind = 'knowledge_erasure'
             and operation.state = 'running'
       ) then
        raise exception 'delivery erasure requires the governed Knowledge operation'
            using errcode = '23514';
    end if;
    update public.session_context_runs run set rendered = ''
    from public.context_run_delivery_dependencies dependency
    where run.tenant_id = old.tenant_id
      and dependency.tenant_id = run.tenant_id
      and dependency.context_run_id = run.id
      and run.rendered <> ''
      and (
          dependency.manifest @> jsonb_build_object('knowledge', jsonb_build_array(
              jsonb_build_object('item_id', old.knowledge_item_id)))
          or dependency.manifest @> jsonb_build_object('graph', jsonb_build_array(
              jsonb_build_object('from_item_id', old.knowledge_item_id)))
          or dependency.manifest @> jsonb_build_object('graph', jsonb_build_array(
              jsonb_build_object('to_item_id', old.knowledge_item_id)))
          or dependency.manifest @> jsonb_build_object('graph', jsonb_build_array(
              jsonb_build_object('asserting_revision_id', old.id)))
      );
    return old;
end
$$;


--
-- Name: synveda_context_pack_chunk_immutable(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_context_pack_chunk_immutable() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    raise exception
        'context pack chunk % is a fact about a record and a document address (PRMT-2); '
        're-authoring a document writes new chunks rather than rewriting these',
        old.record_id;
end
$$;


--
-- Name: synveda_context_pack_document_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_context_pack_document_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.tenant_id     <> old.tenant_id
        or new.scope_id      <> old.scope_id
        or new.pack_name     <> old.pack_name
        or new.document_name <> old.document_name
        or new.created_at    <> old.created_at
        or new.created_by    <> old.created_by
    then
        raise exception
            'context pack document %/%/% is identified by its tenant, scope, pack and '
            'name (PRMT-2); renaming or moving one is a new document, not an edit',
            old.scope_id, old.pack_name, old.document_name;
    end if;
    return new;
end
$$;


--
-- Name: synveda_context_pack_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_context_pack_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.tenant_id  <> old.tenant_id
        or new.scope_id   <> old.scope_id
        or new.name       <> old.name
        or new.created_at <> old.created_at
        or new.created_by <> old.created_by
    then
        raise exception
            'context pack %/% is identified by its tenant, scope and name (PRMT-2); '
            'renaming or moving one is a new pack, not an edit',
            old.scope_id, old.name;
    end if;
    return new;
end
$$;


--
-- Name: synveda_context_run_restart_sources(); Type: FUNCTION; Schema: public; Owner: -
--

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


--
-- Name: synveda_context_trace_immutable(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_context_trace_immutable() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
declare
    new_row jsonb;
    old_row jsonb;
begin
    if current_setting('synveda.knowledge_erasure', true) = 'on'
       and current_user = pg_catalog.pg_get_userbyid((
           select database.datdba from pg_catalog.pg_database as database
           where database.datname = pg_catalog.current_database()
       )) then
        if tg_table_name = 'context_feedback' and tg_op = 'DELETE' then
            return old;
        end if;
        if tg_op = 'UPDATE' then
            new_row := to_jsonb(new);
            old_row := to_jsonb(old);
            if tg_table_name = 'session_context_runs'
               and new_row ->> 'rendered' = ''
               and (new_row - 'rendered') = (old_row - 'rendered') then
                return new;
            end if;
            if tg_table_name = 'context_candidates'
               and new_row -> 'knowledge_item_id' = 'null'::jsonb
               and new_row -> 'knowledge_revision_id' = 'null'::jsonb
               and new_row -> 'scope_id' = 'null'::jsonb
               and (new_row - array['knowledge_item_id', 'knowledge_revision_id', 'scope_id'])
                   = (old_row - array['knowledge_item_id', 'knowledge_revision_id', 'scope_id']) then
                return new;
            end if;
            if tg_table_name = 'context_selections'
               and new_row -> 'knowledge_item_id' = 'null'::jsonb
               and new_row -> 'knowledge_revision_id' = 'null'::jsonb
               and (new_row - array['knowledge_item_id', 'knowledge_revision_id'])
                   = (old_row - array['knowledge_item_id', 'knowledge_revision_id']) then
                return new;
            end if;
            if tg_table_name = 'context_graph_steps'
               and new_row -> 'relation_id' = 'null'::jsonb
               and new_row -> 'from_item_id' = 'null'::jsonb
               and new_row -> 'from_revision_id' = 'null'::jsonb
               and new_row -> 'to_item_id' = 'null'::jsonb
               and new_row -> 'to_revision_id' = 'null'::jsonb
               and new_row -> 'asserting_revision_id' = 'null'::jsonb
               and (new_row - array[
                       'relation_id', 'from_item_id', 'from_revision_id',
                       'to_item_id', 'to_revision_id', 'asserting_revision_id'
                   ])
                   = (old_row - array[
                       'relation_id', 'from_item_id', 'from_revision_id',
                       'to_item_id', 'to_revision_id', 'asserting_revision_id'
                   ]) then
                return new;
            end if;
        end if;
        raise exception '% has an invalid Knowledge erasure scrub', tg_table_name
            using errcode = '23514';
    end if;
    raise exception '% is immutable (CPR-20, ADR-0084)', tg_table_name;
end
$$;


--
-- Name: synveda_current_tenant(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_current_tenant() RETURNS uuid
    LANGUAGE sql STABLE PARALLEL SAFE
    AS $$
    select nullif(current_setting('synveda.tenant_id', true), '')::uuid
$$;


--
-- Name: synveda_durable_operation_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_durable_operation_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.tenant_id <> old.tenant_id
       or new.id <> old.id
       or new.kind <> old.kind
       or new.operation_version <> old.operation_version
       or new.proposal_id is distinct from old.proposal_id
       or new.knowledge_item_id is distinct from old.knowledge_item_id
       or new.skill_version_id is distinct from old.skill_version_id
       or new.requested_by_identity_id is distinct from old.requested_by_identity_id
       or new.authorized_at <> old.authorized_at
       or new.input_hash <> old.input_hash
       or new.created_at <> old.created_at then
        raise exception 'durable operation identity is immutable';
    end if;
    if old.state in ('succeeded', 'blocked', 'cancelled', 'dead_lettered') then
        raise exception 'durable operation % is terminal', old.id;
    end if;
    if new.progress_percent < old.progress_percent then
        raise exception 'durable operation progress cannot move backwards';
    end if;
    if old.cancel_requested_at is not null
       and new.cancel_requested_at is distinct from old.cancel_requested_at then
        raise exception 'durable operation cancellation is immutable';
    end if;
    if old.kind = 'knowledge_erasure'
       and new.cancel_requested_at is not null then
        raise exception 'Knowledge erasure cannot use caller cancellation';
    end if;
    if not (
        (old.state = 'pending' and new.state in ('running', 'blocked', 'cancelled', 'dead_lettered'))
        or (old.state = 'running' and new.state in
            ('running', 'succeeded', 'failed', 'blocked', 'cancelled', 'dead_lettered'))
        or (old.state = 'failed' and new.state in ('running', 'blocked', 'cancelled', 'dead_lettered'))
    ) then
        raise exception 'invalid durable operation transition % -> %', old.state, new.state;
    end if;
    if new.state = 'running' then
        if old.state in ('pending', 'failed') and new.attempts <> old.attempts + 1 then
            raise exception 'a durable operation claim increments attempts exactly once';
        end if;
        if old.state = 'running' then
            if new.attempts = old.attempts then
                if new.lease_owner <> old.lease_owner then
                    raise exception 'a durable operation lease owner cannot change during renewal';
                end if;
            elsif new.attempts = old.attempts + 1 then
                if old.lease_expires_at > statement_timestamp() then
                    raise exception 'a live durable operation lease cannot be reclaimed';
                end if;
            else
                raise exception 'a durable operation reclaim increments attempts exactly once';
            end if;
        end if;
    elsif new.attempts <> old.attempts then
        raise exception 'durable operation attempts change only on claim';
    end if;
    return new;
end
$$;


--
-- Name: synveda_erase_knowledge(uuid, uuid, uuid, uuid, text, text); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_erase_knowledge(wanted_tenant uuid, wanted_item uuid, wanted_proposal uuid, wanted_operation uuid, wanted_actor_hash text, wanted_reason_hash text) RETURNS void
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


--
-- Name: synveda_grants_are_immutable(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_grants_are_immutable() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    raise exception
        'scope_grants rows are never updated; revoke and grant instead (CPR-5, ADR-0072)';
end
$$;


--
-- Name: synveda_groups_immutable_columns(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_groups_immutable_columns() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.id <> old.id then
        raise exception 'groups.id is immutable (CPR-5, ADR-0072)';
    end if;
    if new.tenant_id <> old.tenant_id then
        raise exception
            'group % cannot move across tenants (% to %) (CPR-5, ADR-0072)',
            old.id, old.tenant_id, new.tenant_id;
    end if;
    if new.slug <> old.slug then
        raise exception
            'groups.slug is immutable; an update changes display_name (CPR-5, ADR-0072)';
    end if;
    if new.source <> old.source
        or new.directory_source is distinct from old.directory_source
        or new.directory_resource_id is distinct from old.directory_resource_id then
        raise exception
            'a group does not change hands between the product and a directory (CPR-34, ADR-0093)';
    end if;
    if new.created_at <> old.created_at
        or new.created_by is distinct from old.created_by then
        raise exception 'group provenance is immutable (CPR-5, ADR-0072)';
    end if;
    if new.revision <> old.revision + 1 then
        raise exception
            'groups.revision steps forward by one; % to % (CPR-5, ADR-0072)',
            old.revision, new.revision;
    end if;
    return new;
end
$$;


--
-- Name: synveda_immutable_configuration_row(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_immutable_configuration_row() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    raise exception '% rows are immutable (CPR-30)', tg_table_name;
end
$$;


--
-- Name: synveda_immutable_relaxation_version(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_immutable_relaxation_version() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    raise exception 'Relaxation versions are immutable (CPR-31)';
end
$$;


--
-- Name: synveda_immutable_skill_row(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_immutable_skill_row() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    raise exception '% rows are immutable (CPR-23)', tg_table_name;
end
$$;


--
-- Name: synveda_immutable_tool_row(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_immutable_tool_row() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    raise exception '% rows are immutable (CPR-25)', tg_table_name;
end
$$;


--
-- Name: synveda_import_append_only(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_import_append_only() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    raise exception '% is append-only (CPR-27, ADR-0087)', tg_table_name;
end
$$;


--
-- Name: synveda_import_job_project_identity(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_import_job_project_identity() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if not exists (
        select 1 from projects project
        where project.tenant_id = new.tenant_id
          and project.id = new.project_id
          and project.scope_id = new.scope_id
          and project.workspace_id = new.workspace_id
    ) then
        raise exception 'import job placement must match its project'
            using errcode = '23514';
    end if;
    return new;
end
$$;


--
-- Name: synveda_import_job_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_import_job_transition() RETURNS trigger
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


--
-- Name: synveda_import_mapping_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_import_mapping_transition() RETURNS trigger
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


--
-- Name: synveda_import_repository_identity(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_import_repository_identity() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'pg_catalog', 'public'
    AS $$
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


--
-- Name: synveda_import_source_artifact(uuid, jsonb); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_import_source_artifact(t uuid, s jsonb) RETURNS uuid
    LANGUAGE sql STABLE
    SET search_path TO 'public', 'pg_temp'
    AS $$
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


--
-- Name: synveda_inherit_historical_evidence(uuid, text, uuid, text, uuid); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_inherit_historical_evidence(t uuid, k text, d uuid, pk text, p uuid) RETURNS void
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'public', 'pg_temp'
    AS $$
BEGIN
 IF t IS DISTINCT FROM public.synveda_current_tenant() THEN
  RAISE EXCEPTION 'historical evidence tenant mismatch' USING ERRCODE='23514'; END IF;
 INSERT INTO public.historical_knowledge_dependencies SELECT t,k,d,knowledge_item_id,revision_id,content_hash
 FROM public.historical_knowledge_dependencies WHERE tenant_id=t AND dependent_kind=pk AND dependent_id=p ON CONFLICT DO NOTHING;
END $$;


--
-- Name: synveda_inherit_procedure_evidence(uuid, text, uuid, text, uuid); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_inherit_procedure_evidence(t uuid, k text, d uuid, pk text, p uuid) RETURNS void
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'public', 'pg_temp'
    AS $$
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


--
-- Name: synveda_inherit_task_evidence(uuid, text, uuid, text, uuid); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_inherit_task_evidence(t uuid, k text, d uuid, pk text, p uuid) RETURNS void
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'public', 'pg_temp'
    AS $$
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


--
-- Name: synveda_invites_are_terminal(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_invites_are_terminal() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.id <> old.id or new.tenant_id <> old.tenant_id
        or new.scope_id <> old.scope_id or new.role_key <> old.role_key
        or new.token_hash <> old.token_hash or new.expires_at <> old.expires_at then
        raise exception
            'an invitation''s terms are immutable; revoke it and issue another (CPR-5, ADR-0072)';
    end if;
    if new.created_at <> old.created_at
        or new.created_by is distinct from old.created_by then
        raise exception 'invitation provenance is immutable (CPR-5, ADR-0072)';
    end if;
    if old.status <> 'pending' then
        raise exception
            'invitation % is already %; an invitation is one-time (CPR-5, ADR-0072)',
            old.id, old.status;
    end if;
    return new;
end
$$;


--
-- Name: synveda_item_cleanup_authorized(uuid, text, uuid); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_item_cleanup_authorized(t uuid, k text, d uuid) RETURNS boolean
    LANGUAGE sql STABLE
    SET search_path TO 'public', 'pg_temp'
    AS $$
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


--
-- Name: synveda_knowledge_append_only(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_knowledge_append_only() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if tg_op = 'DELETE'
       and current_setting('synveda.knowledge_erasure', true) = 'on'
       and current_user = pg_catalog.pg_get_userbyid((
           select database.datdba from pg_catalog.pg_database as database
           where database.datname = pg_catalog.current_database()
       )) then
        return null;
    end if;
    raise exception '% is append-only (CPR-15/16, ADR-0080/0081)', tg_table_name;
end
$$;


--
-- Name: synveda_knowledge_change_matches_proposal(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_knowledge_change_matches_proposal() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if not exists (
        select 1
        from vedaflow_proposals proposal
        where proposal.tenant_id = new.tenant_id
          and proposal.id = new.proposal_id
          and proposal.asset_kind = 'knowledge'
          and proposal.target_channel = 'apply'
    ) then
        raise exception 'Knowledge change must bind a Knowledge/apply proposal'
            using errcode = '23514';
    end if;
    return new;
end
$$;


--
-- Name: synveda_knowledge_change_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_knowledge_change_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.tenant_id <> old.tenant_id
       or new.proposal_id <> old.proposal_id
       or new.command_kind <> old.command_kind
       or new.target_item_ids <> old.target_item_ids
       or new.payload_hash <> old.payload_hash
       or new.created_at <> old.created_at then
        raise exception 'Knowledge change identity and reviewed manifest are immutable';
    end if;

    if current_setting('synveda.knowledge_erasure', true) = 'on'
       and current_user = pg_catalog.pg_get_userbyid((
           select database.datdba from pg_catalog.pg_database as database
           where database.datname = pg_catalog.current_database()
       )) then
        if new.payload is not null
           or new.resulting_item_id is distinct from old.resulting_item_id
           or new.resulting_revision_id is distinct from old.resulting_revision_id
           or new.operation_id is distinct from old.operation_id
           or new.applied_at is distinct from old.applied_at then
            raise exception 'Knowledge erasure may only clear a change payload';
        end if;
        return new;
    end if;

    if old.applied_at is not null
       or new.applied_at is null
       or new.payload is distinct from old.payload then
        raise exception 'Knowledge change result may be assigned exactly once';
    end if;
    return new;
end
$$;


--
-- Name: synveda_knowledge_conflict_member_immutable(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_knowledge_conflict_member_immutable() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if tg_op = 'DELETE'
       and current_setting('synveda.knowledge_erasure', true) = 'on'
       and current_user = pg_catalog.pg_get_userbyid((
           select database.datdba from pg_catalog.pg_database as database
           where database.datname = pg_catalog.current_database()
       )) then
        return null;
    end if;
    raise exception 'Knowledge conflict members are immutable'
        using errcode = '23514';
end
$$;


--
-- Name: synveda_knowledge_conflict_set_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_knowledge_conflict_set_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if tg_op = 'DELETE' then
        if current_setting('synveda.knowledge_erasure', true) = 'on'
           and current_user = pg_catalog.pg_get_userbyid((
               select database.datdba from pg_catalog.pg_database as database
               where database.datname = pg_catalog.current_database()
           )) then
            return old;
        end if;
        raise exception 'Knowledge conflict sets are durable evidence'
            using errcode = '23514';
    end if;
    if new.id <> old.id
       or new.tenant_id <> old.tenant_id
       or new.scope_id <> old.scope_id
       or new.project_id is distinct from old.project_id
       or new.classification <> old.classification
       or new.capture_candidate_id is distinct from old.capture_candidate_id
       or new.created_by <> old.created_by
       or new.created_at <> old.created_at then
        raise exception 'Knowledge conflict identity and evidence address are immutable'
            using errcode = '23514';
    end if;
    if new.revision <> old.revision + 1 or new.updated_at <= old.updated_at then
        raise exception 'Knowledge conflict revisions advance exactly once'
            using errcode = '23514';
    end if;
    if old.status not in ('open', 'pending_review')
       or (old.status = 'pending_review' and not (
           (new.status in ('resolved', 'dismissed')
            and new.resolution_change_id = old.resolution_change_id
            and new.resolution = old.resolution
            and new.resolved_by = old.resolved_by)
           or
           (new.status = 'open'
            and new.resolution_change_id is null
            and new.resolution is null
            and new.resolved_by is null
            and new.resolved_at is null)
       )) then
        raise exception 'a Knowledge conflict resolution has an invalid transition'
            using errcode = '23514';
    end if;
    return new;
end
$$;


--
-- Name: synveda_knowledge_items_archive(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_knowledge_items_archive() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
declare
    changed_at timestamptz;
begin
    if tg_op = 'DELETE' then
        if current_setting('synveda.knowledge_erasure', true) = 'on'
           and current_user = pg_catalog.pg_get_userbyid((
               select database.datdba from pg_catalog.pg_database as database
               where database.datname = pg_catalog.current_database()
           )) then
            return old;
        end if;
        raise exception 'Knowledge items have a governed lifecycle and are never directly deleted';
    end if;

    if new.id <> old.id or new.tenant_id <> old.tenant_id then
        raise exception 'Knowledge item identity and tenant are immutable';
    end if;
    if new.origin <> old.origin then
        raise exception 'Knowledge origin is a creation fact and is immutable';
    end if;
    if new.created_at <> old.created_at
       or new.created_by is distinct from old.created_by then
        raise exception 'Knowledge item creation provenance is immutable';
    end if;
    if new.tx_from <> old.tx_from or new.tx_to is not null then
        raise exception 'Knowledge transaction time is maintained by the database';
    end if;

    changed_at := clock_timestamp();
    if changed_at <= old.tx_from then
        raise exception 'Knowledge transaction clock did not advance'
            using errcode = '40001';
    end if;

    insert into knowledge_items_history
        (id, tenant_id, scope_id, project_id, owner_principal_id,
         knowledge_type, origin, lifecycle_state, current_revision_id,
         created_by, updated_by, created_at, updated_at, tx_from, tx_to)
    values
        (old.id, old.tenant_id, old.scope_id, old.project_id,
         old.owner_principal_id, old.knowledge_type, old.origin,
         old.lifecycle_state, old.current_revision_id, old.created_by,
         old.updated_by, old.created_at, old.updated_at, old.tx_from,
         changed_at);

    new.updated_at := changed_at;
    new.tx_from := changed_at;
    new.tx_to := null;
    return new;
end
$$;


--
-- Name: synveda_knowledge_revision_has_source(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_knowledge_revision_has_source() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if not exists (
        select 1
        from knowledge_revision_sources link
        where link.tenant_id = new.tenant_id
          and link.knowledge_revision_id = new.id
    ) then
        raise exception 'Knowledge revision % has no provenance source (CPR-15, ADR-0080)',
            new.id
            using errcode = '23514';
    end if;
    return new;
end
$$;


--
-- Name: synveda_knowledge_source_event_scope(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_knowledge_source_event_scope() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.source_type = 'session_event' and not exists (
        select 1
        from session_events event
        join sessions session
          on session.tenant_id = event.tenant_id
         and session.id = event.session_id
        where event.tenant_id = new.tenant_id
          and event.id = new.session_event_id
          and session.scope_id = new.scope_id
    ) then
        raise exception 'session-event Knowledge source scope must match its session'
            using errcode = '23514';
    end if;
    return new;
end
$$;


--
-- Name: synveda_knowledge_tags_canonical(text[]); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_knowledge_tags_canonical(value text[]) RETURNS boolean
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    AS $$
    select cardinality(value) <= 64
       and not exists (
           select
           from unnest(value) as raw(tag)
           where tag <> lower(btrim(tag))
              or char_length(tag) not between 1 and 64
       )
       and value = coalesce(
           (
               select array_agg(tag order by tag)
               from (
                   select distinct lower(btrim(raw.tag)) as tag
                   from unnest(value) as raw(tag)
               ) canonical
           ),
           '{}'::text[]
       )
$$;


--
-- Name: synveda_lock_procedure_request(uuid, uuid); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_lock_procedure_request(t uuid, e uuid) RETURNS boolean
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'public', 'pg_temp'
    AS $$
BEGIN
 IF t IS DISTINCT FROM public.synveda_current_tenant() THEN RETURN false; END IF;
 PERFORM 1 FROM public.procedure_improvement_headers WHERE tenant_id=t AND event_id=e FOR UPDATE;
 RETURN FOUND;
END $$;


--
-- Name: synveda_managed_import_complete(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_managed_import_complete() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'public'
    AS $$
BEGIN
 IF NOT EXISTS (SELECT 1 FROM public.source_observations o JOIN public.durable_operations op ON op.tenant_id=o.tenant_id AND op.id=o.operation_id
  WHERE o.tenant_id=NEW.tenant_id AND o.import_job_id=NEW.id AND o.source_id=NEW.managed_source_id AND op.state='succeeded') THEN
  RAISE EXCEPTION 'managed source copies require their atomic complete observation' USING ERRCODE='23514'; END IF;
 RETURN NULL;
END $$;


--
-- Name: synveda_operation_attempt_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_operation_attempt_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.tenant_id <> old.tenant_id
       or new.operation_id <> old.operation_id
       or new.attempt_number <> old.attempt_number
       or new.worker_id <> old.worker_id
       or new.started_at <> old.started_at then
        raise exception 'operation attempt identity is immutable';
    end if;
    if old.state <> 'running' then
        raise exception 'operation attempt is terminal';
    end if;
    if not (
        (new.state = 'running' and new.heartbeat_at >= old.heartbeat_at)
        or new.state in ('succeeded', 'retry_scheduled', 'cancelled', 'dead_lettered')
    ) then
        raise exception 'invalid operation attempt transition % -> %', old.state, new.state;
    end if;
    return new;
end
$$;


--
-- Name: synveda_operation_outbox_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_operation_outbox_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.tenant_id <> old.tenant_id
       or new.operation_id <> old.operation_id
       or new.operation_version <> old.operation_version
       or new.created_at <> old.created_at then
        raise exception 'operation outbox identity is immutable';
    end if;
    if old.state = 'completed' then
        raise exception 'operation outbox row is terminal';
    end if;
    if not (
        (old.state = 'pending' and new.state = 'claimed')
        or (old.state = 'claimed' and new.state = 'claimed'
            and old.lease_expires_at <= statement_timestamp())
        or (old.state = 'dispatched' and new.state = 'claimed')
        or (old.state = 'claimed' and new.state in ('pending', 'dispatched'))
        or (old.state in ('pending', 'dispatched') and new.state = 'pending')
        or (old.state in ('pending', 'claimed', 'dispatched') and new.state = 'completed')
    ) then
        raise exception 'invalid operation outbox transition % -> %', old.state, new.state;
    end if;
    if new.state = 'claimed' and new.dispatch_attempts <> old.dispatch_attempts + 1 then
        raise exception 'an outbox claim increments dispatch attempts exactly once';
    end if;
    if new.state <> 'claimed' and new.dispatch_attempts <> old.dispatch_attempts then
        raise exception 'outbox dispatch attempts change only on claim';
    end if;
    if old.state = 'claimed' and new.state = 'pending'
       and new.submission_failures <> old.submission_failures + 1 then
        raise exception 'a failed outbox submission increments consecutive failures exactly once';
    end if;
    if old.state = 'claimed' and new.state = 'dispatched'
       and new.submission_failures <> 0 then
        raise exception 'a successful outbox submission resets consecutive failures';
    end if;
    if not (old.state = 'claimed' and new.state in ('pending', 'dispatched'))
       and new.submission_failures <> old.submission_failures then
        raise exception 'outbox consecutive failures change only after submission';
    end if;
    return new;
end
$$;


--
-- Name: synveda_policy_relaxation_aggregate_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_policy_relaxation_aggregate_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.id <> old.id
       or new.tenant_id <> old.tenant_id
       or new.governing_scope_id <> old.governing_scope_id
       or new.created_at <> old.created_at
       or new.created_by <> old.created_by
    then
        raise exception 'a Relaxation aggregate identity is immutable (CPR-31)';
    end if;
    if old.expiry_recorded_at is null
       and new.expiry_recorded_at is not null
       and new.current_version_id = old.current_version_id
       and new.revision = old.revision
       and new.updated_at = old.updated_at
       and new.updated_by = old.updated_by
       and new.revoked_at is not distinct from old.revoked_at
       and new.revoked_by is not distinct from old.revoked_by
       and new.revocation_proposal_id is not distinct from old.revocation_proposal_id
       and new.revocation_reason is not distinct from old.revocation_reason
    then
        return new;
    end if;
    if new.revision <> old.revision + 1 then
        raise exception 'a Relaxation transition must advance revision exactly once (CPR-31)';
    end if;
    if old.revoked_at is not null then
        raise exception 'a revoked Relaxation is terminal (CPR-31)';
    end if;
    if new.current_version_id = old.current_version_id
       and new.revoked_at is null
       and new.expiry_recorded_at is not distinct from old.expiry_recorded_at
    then
        raise exception 'a Relaxation transition must publish, revoke, or record expiry (CPR-31)';
    end if;
    if old.expiry_recorded_at is not null
       and new.expiry_recorded_at is distinct from old.expiry_recorded_at
    then
        raise exception 'a Relaxation expiry may be recorded once (CPR-31)';
    end if;
    return new;
end
$$;


--
-- Name: synveda_policy_relaxation_change_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_policy_relaxation_change_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.tenant_id <> old.tenant_id
       or new.proposal_id <> old.proposal_id
       or new.command_kind <> old.command_kind
       or new.payload <> old.payload
       or new.payload_hash <> old.payload_hash
       or new.created_at <> old.created_at
    then
        raise exception 'a Relaxation VedaFlow command is immutable (CPR-31)';
    end if;
    if old.applied_at is not null or new.applied_at is null then
        raise exception 'a Relaxation result may be recorded exactly once (CPR-31)';
    end if;
    return new;
end
$$;


--
-- Name: synveda_policy_relaxation_version_matches_proposal(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_policy_relaxation_version_matches_proposal() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if not exists (
        select 1 from vedaflow_proposals proposal
         where proposal.tenant_id = new.tenant_id
           and proposal.id = new.proposal_id
           and proposal.asset_kind = 'policy'
           and proposal.target_channel = 'apply'
    ) then
        raise exception 'Relaxation version must bind a Policy/apply proposal'
            using errcode = '23514';
    end if;
    return new;
end
$$;


--
-- Name: synveda_procedure_evaluation_guard(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_procedure_evaluation_guard() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'public', 'pg_temp'
    AS $_$
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
END $_$;


--
-- Name: synveda_procedure_projection(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_procedure_projection() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'public', 'pg_temp'
    AS $_$
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
END $_$;


--
-- Name: synveda_projects_immutable_workspace(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_projects_immutable_workspace() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.workspace_id <> old.workspace_id
        or new.workspace_scope_id <> old.workspace_scope_id then
        raise exception
            'project % cannot move between workspaces (CPR-4, ADR-0071)', old.id;
    end if;
    return new;
end
$$;


--
-- Name: synveda_prompt_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_prompt_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.tenant_id  <> old.tenant_id
        or new.scope_id   <> old.scope_id
        or new.name       <> old.name
        or new.created_at <> old.created_at
        or new.created_by <> old.created_by
    then
        raise exception
            'prompt %/% is identified by its tenant, scope and name (PRMT-1); '
            'renaming or moving one is a new prompt, not an edit',
            old.scope_id, old.name;
    end if;
    return new;
end
$$;


--
-- Name: synveda_record_initial_administrator(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_record_initial_administrator() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.role_key = 'administrator'
       and exists (
           select 1
             from public.scopes
            where tenant_id = new.tenant_id
              and id = new.scope_id
              and kind = 'tenant'
              and parent_scope_id is null
       ) then
        insert into public.tenant_administrator_bootstraps (tenant_id, grant_id)
        values (new.tenant_id, new.id)
        on conflict (tenant_id) do nothing;
    end if;
    return new;
end
$$;


--
-- Name: synveda_repository_immutable_columns(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_repository_immutable_columns() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.id <> old.id
        or new.tenant_id <> old.tenant_id
        or new.project_id <> old.project_id
        or new.canonical_uri <> old.canonical_uri
        or new.provider <> old.provider then
        raise exception
            'project_repositories identity is immutable; detach and attach instead (CPR-4, ADR-0071)';
    end if;
    if new.created_at <> old.created_at
        or new.created_by is distinct from old.created_by then
        raise exception 'repository provenance is immutable (CPR-4, ADR-0071)';
    end if;
    return new;
end
$$;


--
-- Name: synveda_repository_source_identity(uuid, uuid, uuid); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_repository_source_identity(wanted_tenant uuid, wanted_project uuid, wanted_repository uuid) RETURNS text
    LANGUAGE sql STABLE
    SET search_path TO 'pg_catalog', 'public'
    AS $$
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


--
-- Name: synveda_scopes_immutable_columns(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_scopes_immutable_columns() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.id <> old.id then
        raise exception 'scopes.id is immutable (CPR-3, ADR-0070)';
    end if;
    if new.tenant_id <> old.tenant_id then
        raise exception
            'scope % cannot move across tenants (% to %) (CPR-3, ADR-0070)',
            old.id, old.tenant_id, new.tenant_id;
    end if;
    if new.kind <> old.kind then
        raise exception
            'scopes.kind is immutable; scope % is a % (CPR-3, ADR-0070)',
            old.id, old.kind;
    end if;
    if new.slug <> old.slug then
        raise exception
            'scopes.slug is immutable; rename changes display_name (CPR-3, ADR-0070)';
    end if;
    -- A principal scope is somebody's own, and whose it is cannot be edited:
    -- re-pointing one would hand somebody else's private material to a new
    -- subject without a single grant row changing (CPR-6, ADR-0073).
    if new.principal_id is distinct from old.principal_id then
        raise exception
            'scopes.principal_id is immutable; scope % belongs to one subject (CPR-6, ADR-0073)',
            old.id;
    end if;
    if new.created_at <> old.created_at
        or new.created_by is distinct from old.created_by then
        raise exception 'scope provenance is immutable (CPR-3, ADR-0070)';
    end if;
    return new;
end
$$;


--
-- Name: synveda_session_event_quarantine_immutable(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_session_event_quarantine_immutable() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if tg_op = 'DELETE'
       and coalesce(current_setting('synveda.retention_purge', true), 'off') = 'on'
       and current_user = pg_catalog.pg_get_userbyid((
           select database.datdba from pg_catalog.pg_database as database
           where database.datname = pg_catalog.current_database()
       ))
    then
        return old;
    end if;
    raise exception
        'session_event_quarantine rows are retired by retention disposal (MEM-6), never deleted';
end
$$;


--
-- Name: synveda_session_event_quarantine_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_session_event_quarantine_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if old.state <> 'pending' then
        raise exception 'quarantine review is one-shot: event % is already %',
            old.event_id, old.state;
    end if;
    if new.state = 'pending' then
        raise exception 'a quarantine review cannot return to pending';
    end if;
    if new.event_id <> old.event_id
        or new.tenant_id <> old.tenant_id
        or new.session_id <> old.session_id
        or new.scope_id <> old.scope_id
        or new.findings <> old.findings
        or new.created_at <> old.created_at
    then
        raise exception 'quarantine provenance columns are immutable';
    end if;
    return new;
end
$$;


--
-- Name: synveda_session_events_immutable(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_session_events_immutable() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if tg_op = 'DELETE'
       and coalesce(current_setting('synveda.retention_purge', true), 'off') = 'on'
       and current_user = pg_catalog.pg_get_userbyid((
           select database.datdba from pg_catalog.pg_database as database
           where database.datname = pg_catalog.current_database()
       ))
    then
        return old;
    end if;
    raise exception
        'session_events rows are retired by retention disposal (MEM-6), never deleted';
end
$$;


--
-- Name: synveda_sessions_lifecycle(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_sessions_lifecycle() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.id <> old.id then
        raise exception 'sessions.id is immutable (CPR-10, ADR-0076)';
    end if;
    if new.tenant_id <> old.tenant_id then
        raise exception
            'session % cannot move across tenants (% to %) (CPR-10, ADR-0076)',
            old.id, old.tenant_id, new.tenant_id;
    end if;
    if new.workspace_id <> old.workspace_id
        or new.project_id is distinct from old.project_id
        or new.workspace_scope_id <> old.workspace_scope_id
        or new.project_scope_id is distinct from old.project_scope_id
        or new.scope_id <> old.scope_id then
        raise exception
            'session % cannot move between workspaces, projects or scopes (CPR-10, ADR-0076)',
            old.id;
    end if;
    if new.principal_id <> old.principal_id
        or new.client_name <> old.client_name
        or new.started_at <> old.started_at
        or new.created_at <> old.created_at then
        raise exception 'session % provenance is immutable (CPR-10, ADR-0076)', old.id;
    end if;
    if new.status <> old.status
        and not (
            (old.status = 'active' and new.status = 'ending')
            or (old.status in ('active', 'ending')
                and new.status in ('ended', 'abandoned', 'failed'))
        ) then
        raise exception
            'session % cannot go from % to %; a closed session never reopens (CPR-10, ADR-0076)',
            old.id, old.status, new.status;
    end if;
    -- `last_observed_at` never moves backwards: it is the newest event's
    -- instant, and an out-of-order delivery must not rewind it.
    if new.last_observed_at is not null
        and old.last_observed_at is not null
        and new.last_observed_at < old.last_observed_at then
        raise exception
            'session %.last_observed_at never moves backwards (CPR-10, ADR-0076)', old.id;
    end if;
    return new;
end
$$;


--
-- Name: synveda_skill_aggregate_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_skill_aggregate_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.id <> old.id
        or new.tenant_id <> old.tenant_id
        or new.governing_scope_id <> old.governing_scope_id
        or new.name <> old.name
        or new.created_at <> old.created_at
        or new.created_by <> old.created_by
    then
        raise exception 'a Skill aggregate identity is immutable (CPR-23)';
    end if;
    if new.current_version_id = old.current_version_id
        or new.updated_at <= old.updated_at
        or new.updated_by = old.updated_by and new.updated_at = old.updated_at
    then
        raise exception 'a Skill update must advance its current immutable version (CPR-23)';
    end if;
    return new;
end
$$;


--
-- Name: synveda_skill_binding_shape(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_skill_binding_shape() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
declare
    target_kind text;
begin
    select kind into target_kind
      from scopes
     where tenant_id = new.tenant_id and id = new.scope_id;
    if target_kind is null then
        raise exception 'Skill binding target scope does not exist (CPR-23)';
    end if;
    if target_kind not in ('project', 'principal') then
        raise exception 'Skill bindings target project or principal scopes, got % (CPR-23)', target_kind;
    end if;
    return new;
end
$$;


--
-- Name: synveda_skill_binding_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_skill_binding_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.id <> old.id
        or new.tenant_id <> old.tenant_id
        or new.scope_id <> old.scope_id
        or new.skill_id <> old.skill_id
        or new.created_at <> old.created_at
        or new.created_by <> old.created_by
    then
        raise exception 'a Skill binding identity is immutable (CPR-23)';
    end if;
    if new.revision <> old.revision + 1 or new.updated_at <= old.updated_at then
        raise exception 'a Skill binding update must advance revision exactly once (CPR-23)';
    end if;
    if new.enabled = old.enabled and new.pinned_version_id is not distinct from old.pinned_version_id then
        raise exception 'a Skill binding update must change enabled or pinned version (CPR-23)';
    end if;
    return new;
end
$$;


--
-- Name: synveda_skill_change_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_skill_change_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.tenant_id <> old.tenant_id
        or new.proposal_id <> old.proposal_id
        or new.command_kind <> old.command_kind
        or new.payload <> old.payload
        or new.payload_hash <> old.payload_hash
        or new.created_at <> old.created_at
    then
        raise exception 'a Skill VedaFlow command is immutable (CPR-23)';
    end if;
    if old.applied_at is not null or new.applied_at is null then
        raise exception 'a Skill change result may be recorded exactly once (CPR-23)';
    end if;
    return new;
end
$$;


--
-- Name: synveda_source_admissible(uuid, text, uuid); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_source_admissible(t uuid, k text, d uuid) RETURNS boolean
    LANGUAGE sql STABLE
    SET search_path TO 'public', 'pg_temp'
    AS $$
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


--
-- Name: synveda_source_applied_binding(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_source_applied_binding() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'public', 'pg_temp'
    AS $$
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


--
-- Name: synveda_source_available(uuid, text, uuid); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_source_available(t uuid, k text, d uuid) RETURNS boolean
    LANGUAGE sql STABLE
    SET search_path TO 'public', 'pg_temp'
    AS $$
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


--
-- Name: synveda_source_cleanup_authorized(uuid, text, uuid); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_source_cleanup_authorized(t uuid, k text, d uuid) RETURNS boolean
    LANGUAGE sql STABLE
    SET search_path TO 'public', 'pg_temp'
    AS $$
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


--
-- Name: synveda_source_cleanup_chunk(uuid, uuid, text, integer, integer); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_source_cleanup_chunk(t uuid, o uuid, w text, a integer, n integer) RETURNS TABLE(progress smallint, done boolean, erased bigint)
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'public', 'pg_temp'
    AS $$
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


--
-- Name: synveda_source_cleanup_progress(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_source_cleanup_progress() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'public', 'pg_temp'
    AS $$
BEGIN
 IF NOT public.synveda_source_cleanup_authorized(OLD.tenant_id,'session_event',OLD.event_id)
 OR (to_jsonb(NEW)-ARRAY['phase','cursor_id','processed_targets']) IS DISTINCT FROM
    (to_jsonb(OLD)-ARRAY['phase','cursor_id','processed_targets']) OR NEW.phase<OLD.phase
 OR NEW.processed_targets<OLD.processed_targets THEN
  RAISE EXCEPTION 'source cleanup progress needs its live attempt' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;


--
-- Name: synveda_source_content_erasure(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_source_content_erasure() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'public', 'pg_temp'
    AS $$
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


--
-- Name: synveda_source_control_binding(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_source_control_binding() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'public', 'pg_temp'
    AS $$
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


--
-- Name: synveda_source_delivery_proof(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_source_delivery_proof() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'public', 'pg_temp'
    AS $$
BEGIN
 IF NEW.rendered<>'' AND NOT EXISTS(SELECT 1 FROM public.context_run_delivery_dependencies proof
  WHERE proof.tenant_id=NEW.tenant_id AND proof.context_run_id=NEW.id) THEN
  RAISE EXCEPTION 'context delivery requires exact dependency proof' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;


--
-- Name: synveda_source_observation_complete(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_source_observation_complete() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'public'
    AS $$
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


--
-- Name: synveda_source_projection(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_source_projection() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'public', 'pg_temp'
    AS $$
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


--
-- Name: synveda_source_reconcile_guard(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_source_reconcile_guard() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'public'
    AS $$
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


--
-- Name: synveda_source_repository_immutable(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_source_repository_immutable() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'public'
    AS $$
BEGIN RAISE EXCEPTION 'source repository identity is immutable' USING ERRCODE='23514'; END $$;


--
-- Name: synveda_subtype_immutable_columns(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_subtype_immutable_columns() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.id <> old.id then
        raise exception '%.id is immutable (CPR-4, ADR-0071)', tg_table_name;
    end if;
    if new.tenant_id <> old.tenant_id then
        raise exception
            '% % cannot move across tenants (% to %) (CPR-4, ADR-0071)',
            tg_table_name, old.id, old.tenant_id, new.tenant_id;
    end if;
    if new.scope_id <> old.scope_id then
        raise exception
            '% % cannot change the scope it owns (CPR-4, ADR-0071)',
            tg_table_name, old.id;
    end if;
    if new.slug <> old.slug then
        raise exception
            '%.slug is immutable; an update changes display_name (CPR-4, ADR-0071)',
            tg_table_name;
    end if;
    if new.created_at <> old.created_at
        or new.created_by is distinct from old.created_by then
        raise exception '% provenance is immutable (CPR-4, ADR-0071)', tg_table_name;
    end if;
    if new.revision <> old.revision + 1 then
        raise exception
            '%.revision steps forward by one; % to % (CPR-4, ADR-0071)',
            tg_table_name, old.revision, new.revision;
    end if;
    return new;
end
$$;


--
-- Name: synveda_task_evidence_projection(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_task_evidence_projection() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'public', 'pg_temp'
    AS $_$
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
END $_$;


--
-- Name: synveda_tenant_secret_transition_guard(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_tenant_secret_transition_guard() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.id <> old.id
       or new.tenant_id <> old.tenant_id
       or new.scope_id <> old.scope_id
       or new.kind <> old.kind
       or new.label <> old.label
       or new.provider is distinct from old.provider
       or new.created_at <> old.created_at then
        raise exception 'tenant-secret identity and credential-free ownership metadata are immutable'
            using errcode = '23514';
    end if;

    if new.value_revision = old.value_revision then
        -- A DEK re-encryption changes only envelope/generation/update time.
        if old.state <> 'active' or new.state <> 'active'
           or new.key_version = old.key_version
           or new.sealed = old.sealed
           or new.rotated_at <> old.rotated_at
           or new.revoked_at is distinct from old.revoked_at then
            raise exception 'same-revision tenant-secret updates are DEK re-encryption only'
                using errcode = '23514';
        end if;
    elsif new.value_revision = old.value_revision + 1 then
        -- A logical rotate, revoke or reactivation advances exactly once.
        if new.updated_at <= old.updated_at then
            raise exception 'tenant-secret logical transitions must advance updated_at'
                using errcode = '23514';
        end if;
    else
        raise exception 'tenant-secret value revisions advance exactly once'
            using errcode = '23514';
    end if;
    return new;
end
$$;


--
-- Name: synveda_tool_binding_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_tool_binding_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.id <> old.id
        or new.tenant_id <> old.tenant_id
        or new.project_id <> old.project_id
        or new.server_id <> old.server_id
        or new.created_at <> old.created_at
        or new.created_by <> old.created_by
    then
        raise exception 'a ToolBinding identity is immutable (CPR-25)';
    end if;
    if new.revision <> old.revision + 1 or new.updated_at <= old.updated_at then
        raise exception 'a ToolBinding update must advance revision exactly once (CPR-25)';
    end if;
    if new.version_id = old.version_id and new.state = old.state then
        raise exception 'a ToolBinding update must change version or state (CPR-25)';
    end if;
    return new;
end
$$;


--
-- Name: synveda_tool_binding_version_is_approved(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_tool_binding_version_is_approved() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if not exists (
        select 1
          from tool_server_versions version
          join vedaflow_proposals proposal
            on proposal.tenant_id = version.tenant_id
           and proposal.id = version.proposal_id
         where version.tenant_id = new.tenant_id
           and version.id = new.version_id
           and proposal.state = 'applied'
    ) then
        raise exception 'Tool current pointers and bindings require an approved version'
            using errcode = '23514';
    end if;
    return new;
end
$$;


--
-- Name: synveda_tool_change_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_tool_change_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.tenant_id <> old.tenant_id
        or new.proposal_id <> old.proposal_id
        or new.command_kind <> old.command_kind
        or new.payload <> old.payload
        or new.payload_hash <> old.payload_hash
        or new.created_at <> old.created_at
    then
        raise exception 'a Tool VedaFlow command is immutable (CPR-25)';
    end if;
    if old.applied_at is not null or new.applied_at is null then
        raise exception 'a Tool change result may be recorded exactly once (CPR-25)';
    end if;
    return new;
end
$$;


--
-- Name: synveda_tool_server_current_is_approved(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_tool_server_current_is_approved() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
declare
    version_proposal uuid;
begin
    version_proposal := new.current_version_id;
    if version_proposal is null then
        return new;
    end if;
    if not exists (
        select 1
          from tool_server_versions version
          join vedaflow_proposals proposal
            on proposal.tenant_id = version.tenant_id
           and proposal.id = version.proposal_id
         where version.tenant_id = new.tenant_id
           and version.id = version_proposal
           and proposal.state = 'applied'
    ) then
        raise exception 'Tool current pointers and bindings require an approved version'
            using errcode = '23514';
    end if;
    return new;
end
$$;


--
-- Name: synveda_tool_server_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_tool_server_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if new.id <> old.id
        or new.tenant_id <> old.tenant_id
        or new.governing_scope_id <> old.governing_scope_id
        or new.name <> old.name
        or new.created_at <> old.created_at
        or new.created_by <> old.created_by
    then
        raise exception 'a ToolServer identity is immutable (CPR-25)';
    end if;
    if new.current_version_id is not distinct from old.current_version_id
        or new.updated_at <= old.updated_at
    then
        raise exception 'a ToolServer update must advance its approved version (CPR-25)';
    end if;
    return new;
end
$$;


--
-- Name: synveda_tool_version_matches_proposal(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_tool_version_matches_proposal() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if not exists (
        select 1 from vedaflow_proposals proposal
         where proposal.tenant_id = new.tenant_id
           and proposal.id = new.proposal_id
           and proposal.asset_kind = 'tool'
           and proposal.target_channel = 'apply'
    ) then
        raise exception 'Tool version must bind a Tool/apply proposal'
            using errcode = '23514';
    end if;
    return new;
end
$$;


--
-- Name: synveda_valid_artifact_references(jsonb); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_valid_artifact_references(candidate jsonb) RETURNS boolean
    LANGUAGE sql IMMUTABLE
    AS $$
    select case
      when jsonb_typeof(candidate) <> 'array' then false
      else jsonb_array_length(candidate) between 1 and 200
       and not exists (
            select 1
            from jsonb_array_elements(candidate) as entry(value)
            where jsonb_typeof(value) <> 'object'
               or not (value ?& array['family', 'artifact_id', 'operation', 'version'])
               or value - array['family', 'artifact_id', 'operation', 'version',
                                'expected_revision']::text[] <> '{}'::jsonb
               or jsonb_typeof(value -> 'family') <> 'string'
               or value ->> 'family' not in (
                    'knowledge', 'skill', 'tool_server', 'tool_binding',
                    'configuration', 'policy_relaxation', 'okf_import',
                    'prompt', 'context_pack'
               )
               or jsonb_typeof(value -> 'artifact_id') <> 'string'
               or char_length(value ->> 'artifact_id') not between 1 and 1024
               or value ->> 'artifact_id' ~ '[[:cntrl:]]'
               or jsonb_typeof(value -> 'operation') <> 'string'
               or char_length(value ->> 'operation') not between 1 and 64
               or value ->> 'operation' ~ '[[:cntrl:]]'
               or jsonb_typeof(value -> 'version') <> 'string'
               or char_length(value ->> 'version') not between 1 and 512
               or value ->> 'version' ~ '[[:cntrl:]]'
               or (
                    value ? 'expected_revision'
                    and (
                        jsonb_typeof(value -> 'expected_revision') <> 'string'
                        or char_length(value ->> 'expected_revision') not between 1 and 512
                        or value ->> 'expected_revision' ~ '[[:cntrl:]]'
                    )
               )
       )
       and jsonb_array_length(candidate) = (
            select count(distinct value)
            from jsonb_array_elements(candidate) as entry(value)
       )
      end
$$;


--
-- Name: synveda_vedaflow_immutable(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_vedaflow_immutable() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    raise exception '% is append-only (FLOW-1, ADR-0030)', tg_table_name;
end
$$;


--
-- Name: synveda_vedaflow_proposal_transition(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_vedaflow_proposal_transition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if old.state <> 'open' then
        raise exception 'proposal % is already %; closed proposals are history (FLOW-3)',
            old.id, old.state;
    end if;
    if new.state = 'open' then
        raise exception 'proposal % update changed nothing about its state (FLOW-3)', old.id;
    end if;
    if new.tenant_id             <> old.tenant_id
        or new.id                    <> old.id
        or new.target_scope_id       <> old.target_scope_id
        or new.source_scope_id       <> old.source_scope_id
        or new.asset_kind            <> old.asset_kind
        or new.target_channel        <> old.target_channel
        or new.commit_hash           <> old.commit_hash
        or new.sensitivity           <> old.sensitivity
        or new.title                 <> old.title
        or new.proposer_id           <> old.proposer_id
        or new.proposer_subject      <> old.proposer_subject
        or new.artifact_references   <> old.artifact_references
        or new.created_at            <> old.created_at
    then
        raise exception 'proposal % is immutable except for its closure (FLOW-3)', old.id;
    end if;
    return new;
end
$$;


--
-- Name: synveda_vedaflow_refs_delete_guard(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.synveda_vedaflow_refs_delete_guard() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
    if old.name not like 'pin/%' then
        raise exception
            'vedaflow_refs.% is a channel pointer; only pins (pin/*) may be deleted (FLOW-7, ADR-0036)',
            old.name;
    end if;
    return old;
end
$$;


SET default_tablespace = '';


--
-- Name: artifact_cleanup_targets; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.artifact_cleanup_targets (
    tenant_id uuid NOT NULL,
    operation_id uuid NOT NULL,
    artifact_id uuid NOT NULL,
    source_key_digest text NOT NULL,
    change_id uuid NOT NULL,
    input_hash text NOT NULL,
    phase smallint DEFAULT 0 NOT NULL,
    cursor_id uuid,
    processed_targets bigint DEFAULT 0 NOT NULL,
    CONSTRAINT artifact_cleanup_targets_input_hash_check CHECK ((input_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT artifact_cleanup_targets_phase_check CHECK (((phase >= 0) AND (phase <= 10))),
    CONSTRAINT artifact_cleanup_targets_processed_targets_check CHECK ((processed_targets >= 0))
);

ALTER TABLE ONLY public.artifact_cleanup_targets FORCE ROW LEVEL SECURITY;


--
-- Name: artifact_dependencies; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.artifact_dependencies (
    tenant_id uuid NOT NULL,
    dependent_kind text NOT NULL,
    dependent_id uuid NOT NULL,
    source_key_digest text NOT NULL,
    CONSTRAINT artifact_dependencies_dependent_kind_check CHECK ((dependent_kind = ANY (ARRAY['import_artifact'::text, 'import_mapping'::text, 'knowledge_source'::text, 'knowledge_revision'::text, 'capture_candidate'::text, 'knowledge_change'::text, 'context_run'::text, 'session_event'::text]))),
    CONSTRAINT artifact_dependencies_source_key_digest_check CHECK ((source_key_digest ~ '^[0-9a-f]{64}$'::text))
);

ALTER TABLE ONLY public.artifact_dependencies FORCE ROW LEVEL SECURITY;


--
-- Name: artifact_suppressions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.artifact_suppressions (
    tenant_id uuid NOT NULL,
    source_key_digest text NOT NULL,
    artifact_id uuid NOT NULL,
    change_id uuid NOT NULL,
    forget_change_id uuid,
    actor_hash text NOT NULL,
    reason_hash text NOT NULL,
    removed_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT artifact_suppressions_actor_hash_check CHECK ((actor_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT artifact_suppressions_reason_hash_check CHECK ((reason_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT artifact_suppressions_source_key_digest_check CHECK ((source_key_digest ~ '^[0-9a-f]{64}$'::text))
);

ALTER TABLE ONLY public.artifact_suppressions FORCE ROW LEVEL SECURITY;


--
-- Name: audit_chain_heads; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.audit_chain_heads (
    tenant_id uuid NOT NULL,
    seq bigint NOT NULL,
    head_hash bytea NOT NULL,
    CONSTRAINT audit_chain_heads_hash_check CHECK ((octet_length(head_hash) = 32)),
    CONSTRAINT audit_chain_heads_seq_check CHECK ((seq >= 0))
);

ALTER TABLE ONLY public.audit_chain_heads FORCE ROW LEVEL SECURITY;


--
-- Name: audit_log; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.audit_log (
    tenant_id uuid NOT NULL,
    seq bigint NOT NULL,
    occurred_at timestamp with time zone NOT NULL,
    actor_kind text NOT NULL,
    actor_subject text NOT NULL,
    action text NOT NULL,
    resource text NOT NULL,
    outcome text NOT NULL,
    payload jsonb DEFAULT '{}'::jsonb NOT NULL,
    trace_id text,
    prev_hash bytea NOT NULL,
    hash bytea NOT NULL,
    CONSTRAINT audit_log_action_check CHECK (((length(action) >= 1) AND (length(action) <= 100))),
    CONSTRAINT audit_log_actor_kind_check CHECK ((actor_kind = ANY (ARRAY['subject'::text, 'break_glass'::text, 'system'::text]))),
    CONSTRAINT audit_log_actor_subject_check CHECK (((length(actor_subject) >= 1) AND (length(actor_subject) <= 255))),
    CONSTRAINT audit_log_hash_check CHECK ((octet_length(hash) = 32)),
    CONSTRAINT audit_log_outcome_check CHECK ((outcome = ANY (ARRAY['allow'::text, 'deny'::text, 'success'::text, 'failure'::text]))),
    CONSTRAINT audit_log_prev_hash_check CHECK ((octet_length(prev_hash) = 32)),
    CONSTRAINT audit_log_resource_check CHECK (((length(resource) >= 1) AND (length(resource) <= 512))),
    CONSTRAINT audit_log_seq_check CHECK ((seq >= 1))
);

ALTER TABLE ONLY public.audit_log FORCE ROW LEVEL SECURITY;


--
-- Name: capability_snapshots; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.capability_snapshots (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    version_id uuid NOT NULL,
    raw jsonb NOT NULL,
    normalized jsonb NOT NULL,
    digest bytea NOT NULL,
    discovered_at timestamp with time zone NOT NULL,
    discovered_by uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT capability_snapshots_digest_check CHECK ((octet_length(digest) = 32)),
    CONSTRAINT capability_snapshots_normalized_check CHECK (((jsonb_typeof(normalized) = 'object'::text) AND (pg_column_size(normalized) <= 524288))),
    CONSTRAINT capability_snapshots_raw_check CHECK (((jsonb_typeof(raw) = 'object'::text) AND (pg_column_size(raw) <= 524288)))
);

ALTER TABLE ONLY public.capability_snapshots FORCE ROW LEVEL SECURITY;


--
-- Name: capture_batch_events; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.capture_batch_events (
    tenant_id uuid NOT NULL,
    batch_id uuid NOT NULL,
    session_id uuid NOT NULL,
    event_id uuid NOT NULL,
    ordinal integer NOT NULL,
    linked_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT capture_batch_events_ordinal_check CHECK ((ordinal >= 1))
);

ALTER TABLE ONLY public.capture_batch_events FORCE ROW LEVEL SECURITY;


--
-- Name: capture_batches; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.capture_batches (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    session_id uuid,
    scope_id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    project_id uuid,
    principal_id text NOT NULL,
    input_hash text NOT NULL,
    event_count integer NOT NULL,
    state text DEFAULT 'pending'::text NOT NULL,
    extractor_method text,
    model_version text,
    attempts integer DEFAULT 0 NOT NULL,
    lease_owner text,
    lease_expires_at timestamp with time zone,
    candidate_count integer DEFAULT 0 NOT NULL,
    error_code text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    started_at timestamp with time zone,
    completed_at timestamp with time zone,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    source_kind text DEFAULT 'session'::text NOT NULL,
    import_job_id uuid,
    configuration_version_id uuid,
    configuration_hash text,
    CONSTRAINT capture_batches_attempts_check CHECK (((attempts >= 0) AND (attempts <= 5))),
    CONSTRAINT capture_batches_candidate_count_check CHECK ((candidate_count >= 0)),
    CONSTRAINT capture_batches_configuration_shape_check CHECK ((((configuration_version_id IS NULL) OR (configuration_hash IS NOT NULL)) AND ((configuration_hash IS NULL) OR (configuration_hash ~ '^[0-9a-f]{64}$'::text)))),
    CONSTRAINT capture_batches_error_check CHECK (((error_code IS NULL) OR ((btrim(error_code) <> ''::text) AND (char_length(error_code) <= 100)))),
    CONSTRAINT capture_batches_event_count_check CHECK ((event_count >= 0)),
    CONSTRAINT capture_batches_hash_check CHECK ((input_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT capture_batches_lease_check CHECK ((((lease_owner IS NULL) = (lease_expires_at IS NULL)) AND ((lease_owner IS NULL) OR ((btrim(lease_owner) <> ''::text) AND (char_length(lease_owner) <= 255))))),
    CONSTRAINT capture_batches_method_check CHECK (((extractor_method IS NULL) OR ((btrim(extractor_method) <> ''::text) AND (char_length(extractor_method) <= 100)))),
    CONSTRAINT capture_batches_model_check CHECK (((model_version IS NULL) OR ((btrim(model_version) <> ''::text) AND (char_length(model_version) <= 512)))),
    CONSTRAINT capture_batches_principal_check CHECK (((btrim(principal_id) <> ''::text) AND (char_length(principal_id) <= 255))),
    CONSTRAINT capture_batches_source_kind_check CHECK ((source_kind = ANY (ARRAY['session'::text, 'okf_import'::text]))),
    CONSTRAINT capture_batches_source_shape_check CHECK ((((source_kind = 'session'::text) AND (session_id IS NOT NULL) AND (import_job_id IS NULL)) OR ((source_kind = 'okf_import'::text) AND (session_id IS NULL) AND (import_job_id IS NOT NULL) AND (project_id IS NOT NULL) AND (event_count = 0)))),
    CONSTRAINT capture_batches_state_check CHECK ((state = ANY (ARRAY['pending'::text, 'running'::text, 'completed'::text, 'failed'::text]))),
    CONSTRAINT capture_batches_state_shape_check CHECK ((((state = 'pending'::text) AND (completed_at IS NULL) AND (lease_owner IS NULL) AND (lease_expires_at IS NULL)) OR ((state = 'running'::text) AND (started_at IS NOT NULL) AND (completed_at IS NULL) AND (lease_owner IS NOT NULL)) OR ((state = ANY (ARRAY['completed'::text, 'failed'::text])) AND (started_at IS NOT NULL) AND (completed_at IS NOT NULL) AND (lease_owner IS NULL) AND (lease_expires_at IS NULL)))),
    CONSTRAINT capture_batches_time_check CHECK (((updated_at >= created_at) AND ((started_at IS NULL) OR (started_at >= created_at)) AND ((completed_at IS NULL) OR (completed_at >= started_at))))
);

ALTER TABLE ONLY public.capture_batches FORCE ROW LEVEL SECURITY;


--
-- Name: capture_candidate_decisions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.capture_candidate_decisions (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    candidate_id uuid NOT NULL,
    action text NOT NULL,
    state text DEFAULT 'running'::text NOT NULL,
    actor_subject text NOT NULL,
    idempotency_key text NOT NULL,
    request_hash text NOT NULL,
    payload jsonb,
    payload_hash text NOT NULL,
    resulting_change_id uuid,
    resulting_outcome text,
    resulting_knowledge_item_id uuid,
    resulting_revision_id uuid,
    error_code text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    completed_at timestamp with time zone,
    CONSTRAINT capture_candidate_decisions_action_check CHECK ((action = ANY (ARRAY['accept'::text, 'edit_and_accept'::text, 'merge'::text, 'replace'::text, 'dismiss'::text]))),
    CONSTRAINT capture_candidate_decisions_actor_check CHECK (((btrim(actor_subject) <> ''::text) AND (char_length(actor_subject) <= 255))),
    CONSTRAINT capture_candidate_decisions_error_check CHECK (((error_code IS NULL) OR ((btrim(error_code) <> ''::text) AND (char_length(error_code) <= 100)))),
    CONSTRAINT capture_candidate_decisions_key_check CHECK (((btrim(idempotency_key) <> ''::text) AND (char_length(idempotency_key) <= 255))),
    CONSTRAINT capture_candidate_decisions_outcome_check CHECK (((resulting_outcome IS NULL) OR (resulting_outcome = ANY (ARRAY['applied'::text, 'pending_review'::text, 'rejected'::text])))),
    CONSTRAINT capture_candidate_decisions_payload_check CHECK (((payload IS NULL) OR ((jsonb_typeof(payload) = 'object'::text) AND (octet_length((payload)::text) <= 147456)))),
    CONSTRAINT capture_candidate_decisions_payload_hash_check CHECK ((payload_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT capture_candidate_decisions_request_hash_check CHECK ((request_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT capture_candidate_decisions_state_check CHECK ((state = ANY (ARRAY['running'::text, 'succeeded'::text, 'failed'::text]))),
    CONSTRAINT capture_candidate_decisions_state_shape_check CHECK ((((state = 'running'::text) AND (completed_at IS NULL) AND (resulting_change_id IS NULL) AND (resulting_outcome IS NULL) AND (error_code IS NULL)) OR ((state = 'succeeded'::text) AND (completed_at IS NOT NULL) AND (error_code IS NULL)) OR ((state = 'failed'::text) AND (completed_at IS NOT NULL) AND (error_code IS NOT NULL))))
);

ALTER TABLE ONLY public.capture_candidate_decisions FORCE ROW LEVEL SECURITY;


--
-- Name: capture_candidate_events; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.capture_candidate_events (
    tenant_id uuid NOT NULL,
    candidate_id uuid NOT NULL,
    batch_id uuid NOT NULL,
    event_id uuid NOT NULL,
    ordinal integer NOT NULL,
    linked_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT capture_candidate_events_ordinal_check CHECK ((ordinal >= 1))
);

ALTER TABLE ONLY public.capture_candidate_events FORCE ROW LEVEL SECURITY;


--
-- Name: capture_candidate_import_artifacts; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.capture_candidate_import_artifacts (
    tenant_id uuid NOT NULL,
    candidate_id uuid NOT NULL,
    import_job_id uuid NOT NULL,
    artifact_id uuid NOT NULL,
    ordinal integer NOT NULL,
    linked_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT capture_candidate_import_artifacts_ordinal_check CHECK (((ordinal >= 1) AND (ordinal <= 200)))
);

ALTER TABLE ONLY public.capture_candidate_import_artifacts FORCE ROW LEVEL SECURITY;


--
-- Name: capture_candidate_matches; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.capture_candidate_matches (
    tenant_id uuid NOT NULL,
    candidate_id uuid NOT NULL,
    knowledge_item_id uuid NOT NULL,
    knowledge_revision_id uuid NOT NULL,
    match_kind text NOT NULL,
    similarity_permille integer NOT NULL,
    reason_code text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT capture_candidate_matches_kind_check CHECK ((match_kind = ANY (ARRAY['duplicate'::text, 'support'::text, 'contradiction'::text, 'supersession'::text, 'transition'::text]))),
    CONSTRAINT capture_candidate_matches_reason_check CHECK (((btrim(reason_code) <> ''::text) AND (char_length(reason_code) <= 100))),
    CONSTRAINT capture_candidate_matches_similarity_check CHECK (((similarity_permille >= 0) AND (similarity_permille <= 1000)))
);

ALTER TABLE ONLY public.capture_candidate_matches FORCE ROW LEVEL SECURITY;


--
-- Name: capture_candidates; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.capture_candidates (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    batch_id uuid NOT NULL,
    session_id uuid,
    ordinal integer NOT NULL,
    proposed_scope_id uuid NOT NULL,
    proposed_project_id uuid,
    proposed_owner_principal_id text,
    knowledge_type text NOT NULL,
    origin text NOT NULL,
    title text NOT NULL,
    body_markdown text NOT NULL,
    summary text NOT NULL,
    tags text[] DEFAULT '{}'::text[] NOT NULL,
    sensitivity text NOT NULL,
    confidence_permille integer NOT NULL,
    valid_from timestamp with time zone NOT NULL,
    valid_to timestamp with time zone,
    stale_after timestamp with time zone,
    verification_metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    content_hash text NOT NULL,
    state text DEFAULT 'pending'::text NOT NULL,
    resulting_change_id uuid,
    resulting_outcome text,
    resulting_knowledge_item_id uuid,
    resulting_revision_id uuid,
    decided_by text,
    decision_reason text,
    decided_at timestamp with time zone,
    content_erased boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    source_kind text DEFAULT 'session'::text NOT NULL,
    import_job_id uuid,
    CONSTRAINT capture_candidates_body_check CHECK ((((NOT content_erased) AND (btrim(body_markdown) <> ''::text) AND (octet_length(body_markdown) <= 131072)) OR (content_erased AND (body_markdown = ''::text)))),
    CONSTRAINT capture_candidates_confidence_check CHECK (((confidence_permille >= 0) AND (confidence_permille <= 1000))),
    CONSTRAINT capture_candidates_decider_check CHECK (((decided_by IS NULL) OR ((btrim(decided_by) <> ''::text) AND (char_length(decided_by) <= 255)))),
    CONSTRAINT capture_candidates_decision_shape_check CHECK (((state = 'pending'::text) = ((decided_by IS NULL) AND (decided_at IS NULL) AND (resulting_change_id IS NULL) AND (resulting_outcome IS NULL) AND (resulting_knowledge_item_id IS NULL) AND (resulting_revision_id IS NULL)))),
    CONSTRAINT capture_candidates_erasure_metadata_check CHECK (((NOT content_erased) OR ((verification_metadata = '{}'::jsonb) AND (metadata = '{}'::jsonb)))),
    CONSTRAINT capture_candidates_hash_check CHECK ((content_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT capture_candidates_metadata_check CHECK (((jsonb_typeof(metadata) = 'object'::text) AND (octet_length((metadata)::text) <= 16384))),
    CONSTRAINT capture_candidates_origin_check CHECK ((origin = ANY (ARRAY['observed'::text, 'asserted'::text, 'authored'::text, 'imported'::text]))),
    CONSTRAINT capture_candidates_outcome_check CHECK (((resulting_outcome IS NULL) OR (resulting_outcome = ANY (ARRAY['applied'::text, 'pending_review'::text, 'rejected'::text])))),
    CONSTRAINT capture_candidates_owner_check CHECK (((proposed_owner_principal_id IS NULL) OR ((btrim(proposed_owner_principal_id) <> ''::text) AND (char_length(proposed_owner_principal_id) <= 255)))),
    CONSTRAINT capture_candidates_reason_check CHECK (((decision_reason IS NULL) OR ((btrim(decision_reason) <> ''::text) AND (char_length(decision_reason) <= 1000)))),
    CONSTRAINT capture_candidates_sensitivity_check CHECK ((sensitivity = ANY (ARRAY['public'::text, 'internal'::text, 'confidential'::text, 'restricted'::text]))),
    CONSTRAINT capture_candidates_source_kind_check CHECK ((source_kind = ANY (ARRAY['session'::text, 'okf_import'::text]))),
    CONSTRAINT capture_candidates_source_shape_check CHECK ((((source_kind = 'session'::text) AND (session_id IS NOT NULL) AND (import_job_id IS NULL)) OR ((source_kind = 'okf_import'::text) AND (session_id IS NULL) AND (import_job_id IS NOT NULL)))),
    CONSTRAINT capture_candidates_stale_time_check CHECK (((stale_after IS NULL) OR ((stale_after > valid_from) AND ((valid_to IS NULL) OR (stale_after <= valid_to))))),
    CONSTRAINT capture_candidates_state_check CHECK ((state = ANY (ARRAY['pending'::text, 'accepted'::text, 'edited_and_accepted'::text, 'merged'::text, 'replaced'::text, 'dismissed'::text, 'failed'::text]))),
    CONSTRAINT capture_candidates_summary_check CHECK ((((NOT content_erased) AND (btrim(summary) <> ''::text) AND (char_length(summary) <= 2000)) OR (content_erased AND (summary = ''::text)))),
    CONSTRAINT capture_candidates_tags_check CHECK ((((NOT content_erased) AND public.synveda_knowledge_tags_canonical(tags)) OR (content_erased AND (tags = '{}'::text[])))),
    CONSTRAINT capture_candidates_title_check CHECK ((((NOT content_erased) AND (btrim(title) <> ''::text) AND (char_length(title) <= 300)) OR (content_erased AND (title = ''::text)))),
    CONSTRAINT capture_candidates_type_check CHECK ((knowledge_type = ANY (ARRAY['fact'::text, 'decision'::text, 'preference'::text, 'procedure'::text, 'entity'::text, 'episode'::text, 'convention'::text, 'warning'::text, 'reference'::text]))),
    CONSTRAINT capture_candidates_valid_time_check CHECK (((valid_to IS NULL) OR (valid_to > valid_from))),
    CONSTRAINT capture_candidates_verification_check CHECK (((jsonb_typeof(verification_metadata) = 'object'::text) AND (octet_length((verification_metadata)::text) <= 16384)))
);

ALTER TABLE ONLY public.capture_candidates FORCE ROW LEVEL SECURITY;


--
-- Name: cli_handoffs; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.cli_handoffs (
    code_hash bytea NOT NULL,
    state_hash bytea NOT NULL,
    payload_sealed bytea NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    expires_at timestamp with time zone DEFAULT (now() + '00:01:00'::interval) NOT NULL,
    CONSTRAINT cli_handoffs_code_hash_check CHECK ((octet_length(code_hash) = 32)),
    CONSTRAINT cli_handoffs_expiry_check CHECK ((expires_at > created_at)),
    CONSTRAINT cli_handoffs_payload_check CHECK (((octet_length(payload_sealed) >= 51) AND (octet_length(payload_sealed) <= 131072))),
    CONSTRAINT cli_handoffs_state_hash_check CHECK ((octet_length(state_hash) = 32))
);


--
-- Name: configuration_artifacts; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.configuration_artifacts (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    governing_scope_id uuid NOT NULL,
    name text NOT NULL,
    current_version_id uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    created_by text NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_by text NOT NULL,
    CONSTRAINT configuration_artifacts_actor_check CHECK (((btrim(created_by) <> ''::text) AND (length(created_by) <= 255) AND (btrim(updated_by) <> ''::text) AND (length(updated_by) <= 255))),
    CONSTRAINT configuration_artifacts_name_check CHECK (((btrim(name) = name) AND ((length(name) >= 1) AND (length(name) <= 100))))
);

ALTER TABLE ONLY public.configuration_artifacts FORCE ROW LEVEL SECURITY;


--
-- Name: configuration_bindings; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.configuration_bindings (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    artifact_id uuid NOT NULL,
    pinned_version_id uuid,
    enabled boolean NOT NULL,
    revision bigint DEFAULT 1 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    created_by text NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_by text NOT NULL,
    CONSTRAINT configuration_bindings_actor_check CHECK (((btrim(created_by) <> ''::text) AND (length(created_by) <= 255) AND (btrim(updated_by) <> ''::text) AND (length(updated_by) <= 255))),
    CONSTRAINT configuration_bindings_revision_check CHECK ((revision > 0))
);

ALTER TABLE ONLY public.configuration_bindings FORCE ROW LEVEL SECURITY;


--
-- Name: configuration_changes; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.configuration_changes (
    tenant_id uuid NOT NULL,
    proposal_id uuid NOT NULL,
    command_kind text NOT NULL,
    payload jsonb NOT NULL,
    payload_hash text NOT NULL,
    resulting_artifact_id uuid,
    resulting_version_id uuid,
    resulting_binding_id uuid,
    resulting_binding_revision bigint,
    applied_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT configuration_changes_binding_revision_check CHECK (((resulting_binding_revision IS NULL) OR (resulting_binding_revision > 0))),
    CONSTRAINT configuration_changes_kind_check CHECK ((command_kind = ANY (ARRAY['create'::text, 'publish'::text, 'bind'::text, 'set_binding'::text]))),
    CONSTRAINT configuration_changes_payload_check CHECK (((jsonb_typeof(payload) = 'object'::text) AND (pg_column_size(payload) <= 262144))),
    CONSTRAINT configuration_changes_payload_hash_check CHECK ((payload_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT configuration_changes_result_shape_check CHECK ((((applied_at IS NULL) AND (resulting_binding_revision IS NULL)) OR (applied_at IS NOT NULL)))
);

ALTER TABLE ONLY public.configuration_changes FORCE ROW LEVEL SECURITY;


--
-- Name: configuration_versions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.configuration_versions (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    artifact_id uuid NOT NULL,
    proposal_id uuid NOT NULL,
    ordinal bigint NOT NULL,
    document jsonb NOT NULL,
    content_hash bytea NOT NULL,
    source_template text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    created_by text NOT NULL,
    CONSTRAINT configuration_versions_actor_check CHECK (((btrim(created_by) <> ''::text) AND (length(created_by) <= 255))),
    CONSTRAINT configuration_versions_document_check CHECK (((jsonb_typeof(document) = 'object'::text) AND (pg_column_size(document) <= 131072))),
    CONSTRAINT configuration_versions_hash_check CHECK ((octet_length(content_hash) = 32)),
    CONSTRAINT configuration_versions_ordinal_check CHECK ((ordinal > 0)),
    CONSTRAINT configuration_versions_template_check CHECK (((source_template IS NULL) OR (source_template = ANY (ARRAY['personal'::text, 'team'::text, 'enterprise'::text]))))
);

ALTER TABLE ONLY public.configuration_versions FORCE ROW LEVEL SECURITY;


--
-- Name: console_sessions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.console_sessions (
    token_hash bytea NOT NULL,
    issuer text NOT NULL,
    access_expires_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    last_seen_at timestamp with time zone DEFAULT now() NOT NULL,
    absolute_expires_at timestamp with time zone NOT NULL,
    access_token_sealed bytea NOT NULL,
    refresh_token_sealed bytea,
    CONSTRAINT console_sessions_access_token_check CHECK (((octet_length(access_token_sealed) >= 51) AND (octet_length(access_token_sealed) <= 8242))),
    CONSTRAINT console_sessions_expiry_check CHECK ((absolute_expires_at > created_at)),
    CONSTRAINT console_sessions_hash_check CHECK ((octet_length(token_hash) = 32)),
    CONSTRAINT console_sessions_issuer_check CHECK (((length(issuer) >= 1) AND (length(issuer) <= 512))),
    CONSTRAINT console_sessions_refresh_token_check CHECK (((refresh_token_sealed IS NULL) OR ((octet_length(refresh_token_sealed) >= 51) AND (octet_length(refresh_token_sealed) <= 8242))))
);


--
-- Name: context_candidates; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.context_candidates (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    context_run_id uuid NOT NULL,
    ordinal integer NOT NULL,
    knowledge_item_id uuid,
    knowledge_revision_id uuid,
    content_hash text NOT NULL,
    scope_id uuid,
    lifecycle_state text,
    keyword_score_micros integer DEFAULT 0 NOT NULL,
    semantic_score_micros integer DEFAULT 0 NOT NULL,
    freshness_score_micros integer DEFAULT 0 NOT NULL,
    pin_score_micros integer DEFAULT 0 NOT NULL,
    current_state_score_micros integer DEFAULT 0 NOT NULL,
    final_score_micros integer NOT NULL,
    reason_codes text[] NOT NULL,
    exclusion_reason text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    channel text NOT NULL,
    capture_candidate_id uuid,
    anchor_score_micros integer NOT NULL,
    edge_weight_micros integer NOT NULL,
    hop_penalty_micros integer NOT NULL,
    CONSTRAINT context_candidates_address_shape_check CHECK ((((channel = 'current_knowledge'::text) AND (capture_candidate_id IS NULL) AND (((knowledge_item_id IS NULL) AND (knowledge_revision_id IS NULL) AND (scope_id IS NULL)) OR ((knowledge_item_id IS NOT NULL) AND (knowledge_revision_id IS NOT NULL) AND (scope_id IS NOT NULL)))) OR ((channel = 'unreviewed_candidates'::text) AND (knowledge_item_id IS NULL) AND (knowledge_revision_id IS NULL) AND (lifecycle_state IS NULL) AND (((capture_candidate_id IS NULL) AND (scope_id IS NULL)) OR ((capture_candidate_id IS NOT NULL) AND (scope_id IS NOT NULL)))))),
    CONSTRAINT context_candidates_channel_check CHECK ((channel = ANY (ARRAY['current_knowledge'::text, 'unreviewed_candidates'::text]))),
    CONSTRAINT context_candidates_exclusion_check CHECK (((exclusion_reason IS NULL) OR (exclusion_reason = ANY (ARRAY['semantic_match'::text, 'keyword_match'::text, 'project_convention'::text, 'personal_preference'::text, 'freshness_boost'::text, 'explicit_pin'::text, 'superseded'::text, 'stale'::text, 'outside_task_scope'::text, 'token_budget'::text, 'excerpt'::text, 'duplicate'::text, 'graph_expansion'::text, 'contradiction_warning'::text])))),
    CONSTRAINT context_candidates_hash_check CHECK ((content_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT context_candidates_lifecycle_check CHECK (((lifecycle_state IS NULL) OR (lifecycle_state = ANY (ARRAY['active'::text, 'stale'::text, 'superseded'::text, 'archived'::text, 'erasure_pending'::text, 'erased'::text])))),
    CONSTRAINT context_candidates_ordinal_check CHECK ((ordinal >= 0)),
    CONSTRAINT context_candidates_reasons_check CHECK (((cardinality(reason_codes) >= 1) AND (cardinality(reason_codes) <= 14) AND (array_position(reason_codes, NULL::text) IS NULL) AND (reason_codes <@ ARRAY['semantic_match'::text, 'keyword_match'::text, 'project_convention'::text, 'personal_preference'::text, 'freshness_boost'::text, 'explicit_pin'::text, 'superseded'::text, 'stale'::text, 'outside_task_scope'::text, 'token_budget'::text, 'excerpt'::text, 'duplicate'::text, 'graph_expansion'::text, 'contradiction_warning'::text]))),
    CONSTRAINT context_candidates_scores_check CHECK (((keyword_score_micros >= 0) AND (keyword_score_micros <= 1000000) AND ((semantic_score_micros >= 0) AND (semantic_score_micros <= 1000000)) AND ((anchor_score_micros >= 0) AND (anchor_score_micros <= 5000000)) AND ((edge_weight_micros >= 0) AND (edge_weight_micros <= 2000000)) AND ((hop_penalty_micros >= 0) AND (hop_penalty_micros <= 1000000)) AND ((freshness_score_micros >= 0) AND (freshness_score_micros <= 1000000)) AND ((pin_score_micros >= 0) AND (pin_score_micros <= 1000000)) AND ((current_state_score_micros >= 0) AND (current_state_score_micros <= 1000000)) AND ((final_score_micros >= 0) AND (final_score_micros <= 5000000))))
);

ALTER TABLE ONLY public.context_candidates FORCE ROW LEVEL SECURITY;


--
-- Name: context_feedback; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.context_feedback (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    context_run_id uuid NOT NULL,
    context_selection_id uuid NOT NULL,
    knowledge_revision_id uuid NOT NULL,
    feedback_type text NOT NULL,
    principal_id text NOT NULL,
    idempotency_key text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT context_feedback_idempotency_check CHECK (((btrim(idempotency_key) <> ''::text) AND (char_length(idempotency_key) <= 200))),
    CONSTRAINT context_feedback_principal_check CHECK (((btrim(principal_id) <> ''::text) AND (char_length(principal_id) <= 255))),
    CONSTRAINT context_feedback_type_check CHECK ((feedback_type = ANY (ARRAY['referenced_by_agent'::text, 'accepted_by_user'::text, 'helpful'::text, 'unhelpful'::text, 'caused_correction'::text])))
);

ALTER TABLE ONLY public.context_feedback FORCE ROW LEVEL SECURITY;


--
-- Name: context_graph_steps; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.context_graph_steps (
    tenant_id uuid NOT NULL,
    context_run_id uuid NOT NULL,
    context_candidate_id uuid NOT NULL,
    ordinal integer NOT NULL,
    hop smallint NOT NULL,
    relation_id uuid,
    relation_hash text NOT NULL,
    relation_type text NOT NULL,
    direction text NOT NULL,
    from_item_id uuid,
    from_revision_id uuid,
    to_item_id uuid,
    to_revision_id uuid,
    asserting_revision_id uuid,
    from_content_hash text NOT NULL,
    to_content_hash text NOT NULL,
    edge_weight_micros integer NOT NULL,
    supporting boolean NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT context_graph_steps_address_shape_check CHECK ((((relation_id IS NULL) AND (from_item_id IS NULL) AND (from_revision_id IS NULL) AND (to_item_id IS NULL) AND (to_revision_id IS NULL) AND (asserting_revision_id IS NULL)) OR ((relation_id IS NOT NULL) AND (from_item_id IS NOT NULL) AND (from_revision_id IS NOT NULL) AND (to_item_id IS NOT NULL) AND (to_revision_id IS NOT NULL) AND (asserting_revision_id IS NOT NULL)))),
    CONSTRAINT context_graph_steps_direction_check CHECK ((direction = ANY (ARRAY['outbound'::text, 'inbound'::text]))),
    CONSTRAINT context_graph_steps_evidence_check CHECK (((supporting AND (relation_type = ANY (ARRAY['supports'::text, 'supersedes'::text, 'derived_from'::text, 'references'::text, 'related_to'::text, 'transitions_to'::text])) AND ((edge_weight_micros >= 1) AND (edge_weight_micros <= 1000000))) OR ((NOT supporting) AND (relation_type = 'contradicts'::text) AND (edge_weight_micros = 0)))),
    CONSTRAINT context_graph_steps_hash_check CHECK (((relation_hash ~ '^[0-9a-f]{64}$'::text) AND (from_content_hash ~ '^[0-9a-f]{64}$'::text) AND (to_content_hash ~ '^[0-9a-f]{64}$'::text))),
    CONSTRAINT context_graph_steps_position_check CHECK (((ordinal >= 0) AND (ordinal <= 1) AND ((hop >= 1) AND (hop <= 2)) AND (ordinal = (hop - 1)))),
    CONSTRAINT context_graph_steps_type_check CHECK ((relation_type = ANY (ARRAY['supports'::text, 'contradicts'::text, 'supersedes'::text, 'derived_from'::text, 'references'::text, 'related_to'::text, 'transitions_to'::text])))
);

ALTER TABLE ONLY public.context_graph_steps FORCE ROW LEVEL SECURITY;


--
-- Name: context_pack_chunks; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.context_pack_chunks (
    tenant_id uuid NOT NULL,
    id uuid NOT NULL,
    scope_id uuid NOT NULL,
    pack_name text NOT NULL,
    document_name text NOT NULL,
    title text NOT NULL,
    sensitivity text NOT NULL,
    document_hash bytea NOT NULL,
    ordinal integer NOT NULL,
    heading text,
    content text NOT NULL,
    content_hash bytea NOT NULL,
    CONSTRAINT context_pack_chunks_content_check CHECK (((length(content) >= 1) AND (length(content) <= 65536))),
    CONSTRAINT context_pack_chunks_content_hash_check CHECK ((octet_length(content_hash) = 32)),
    CONSTRAINT context_pack_chunks_document_hash_check CHECK ((octet_length(document_hash) = 32)),
    CONSTRAINT context_pack_chunks_heading_check CHECK (((heading IS NULL) OR (length(heading) <= 512))),
    CONSTRAINT context_pack_chunks_id_v7 CHECK ((((get_byte(uuid_send(id), 8) & 192) <> 128) OR ((get_byte(uuid_send(id), 6) >> 4) = 7))),
    CONSTRAINT context_pack_chunks_ordinal_check CHECK (((ordinal >= 0) AND (ordinal <= 511))),
    CONSTRAINT context_pack_chunks_sensitivity_check CHECK ((sensitivity = ANY (ARRAY['public'::text, 'internal'::text, 'confidential'::text]))),
    CONSTRAINT context_pack_chunks_title_check CHECK (((length(title) >= 0) AND (length(title) <= 160)))
);

ALTER TABLE ONLY public.context_pack_chunks FORCE ROW LEVEL SECURITY;


--
-- Name: context_pack_documents; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.context_pack_documents (
    tenant_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    pack_name text NOT NULL,
    document_name text NOT NULL,
    title text NOT NULL,
    sensitivity text NOT NULL,
    object_hash bytea NOT NULL,
    chunks integer NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    created_by uuid NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_by uuid NOT NULL,
    CONSTRAINT context_pack_documents_chunks_check CHECK (((chunks >= 0) AND (chunks <= 512))),
    CONSTRAINT context_pack_documents_name_check CHECK (((length(document_name) >= 1) AND (length(document_name) <= 128))),
    CONSTRAINT context_pack_documents_sensitivity_check CHECK ((sensitivity = ANY (ARRAY['public'::text, 'internal'::text, 'confidential'::text]))),
    CONSTRAINT context_pack_documents_title_check CHECK (((length(title) >= 0) AND (length(title) <= 160)))
);

ALTER TABLE ONLY public.context_pack_documents FORCE ROW LEVEL SECURITY;


--
-- Name: context_packs; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.context_packs (
    tenant_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    name text NOT NULL,
    description text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    created_by uuid NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_by uuid NOT NULL,
    CONSTRAINT context_packs_description_check CHECK (((length(description) >= 0) AND (length(description) <= 512))),
    CONSTRAINT context_packs_name_check CHECK (((length(name) >= 1) AND (length(name) <= 64)))
);

ALTER TABLE ONLY public.context_packs FORCE ROW LEVEL SECURITY;


--
-- Name: context_run_delivery_dependencies; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.context_run_delivery_dependencies (
    tenant_id uuid NOT NULL,
    context_run_id uuid NOT NULL,
    dependency_version integer NOT NULL,
    manifest_hash text NOT NULL,
    manifest jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT context_delivery_hash_check CHECK ((manifest_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT context_delivery_manifest_check CHECK (((jsonb_typeof(manifest) = 'object'::text) AND (octet_length((manifest)::text) <= 524288))),
    CONSTRAINT context_delivery_version_check CHECK ((dependency_version = ANY (ARRAY[1, 2, 3])))
);

ALTER TABLE ONLY public.context_run_delivery_dependencies FORCE ROW LEVEL SECURITY;


--
-- Name: context_selections; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.context_selections (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    context_run_id uuid NOT NULL,
    rank integer NOT NULL,
    knowledge_item_id uuid,
    knowledge_revision_id uuid,
    content_hash text NOT NULL,
    token_count integer NOT NULL,
    reason_codes text[] NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    channel text NOT NULL,
    capture_candidate_id uuid,
    context_candidate_id uuid NOT NULL,
    CONSTRAINT context_selections_address_shape_check CHECK ((((channel = 'current_knowledge'::text) AND (capture_candidate_id IS NULL) AND (((knowledge_item_id IS NULL) AND (knowledge_revision_id IS NULL)) OR ((knowledge_item_id IS NOT NULL) AND (knowledge_revision_id IS NOT NULL)))) OR ((channel = 'unreviewed_candidates'::text) AND (knowledge_item_id IS NULL) AND (knowledge_revision_id IS NULL)))),
    CONSTRAINT context_selections_channel_check CHECK ((channel = ANY (ARRAY['current_knowledge'::text, 'unreviewed_candidates'::text]))),
    CONSTRAINT context_selections_hash_check CHECK ((content_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT context_selections_rank_check CHECK ((rank >= 1)),
    CONSTRAINT context_selections_reasons_check CHECK (((cardinality(reason_codes) >= 1) AND (cardinality(reason_codes) <= 14) AND (array_position(reason_codes, NULL::text) IS NULL) AND (reason_codes <@ ARRAY['semantic_match'::text, 'keyword_match'::text, 'project_convention'::text, 'personal_preference'::text, 'freshness_boost'::text, 'explicit_pin'::text, 'superseded'::text, 'stale'::text, 'outside_task_scope'::text, 'token_budget'::text, 'excerpt'::text, 'duplicate'::text, 'graph_expansion'::text, 'contradiction_warning'::text]))),
    CONSTRAINT context_selections_token_check CHECK ((token_count >= 0))
);

ALTER TABLE ONLY public.context_selections FORCE ROW LEVEL SECURITY;


--
-- Name: deployment_keys; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.deployment_keys (
    version integer NOT NULL,
    wrapped_dek bytea NOT NULL,
    kek_ref text NOT NULL,
    algorithm text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    retired_at timestamp with time zone,
    CONSTRAINT deployment_keys_algorithm_check CHECK ((algorithm = 'xchacha20-poly1305'::text)),
    CONSTRAINT deployment_keys_kek_ref_check CHECK (((length(kek_ref) >= 1) AND (length(kek_ref) <= 256))),
    CONSTRAINT deployment_keys_retired_check CHECK (((retired_at IS NULL) OR (retired_at >= created_at))),
    CONSTRAINT deployment_keys_version_check CHECK ((version >= 1)),
    CONSTRAINT deployment_keys_wrapped_check CHECK ((octet_length(wrapped_dek) = 82))
);


--
-- Name: directory_sync_state; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.directory_sync_state (
    tenant_id uuid NOT NULL,
    connector text NOT NULL,
    passes_completed bigint DEFAULT 0 NOT NULL,
    last_pass_at timestamp with time zone,
    last_complete_pass_at timestamp with time zone,
    breaker_tripped_at timestamp with time zone,
    breaker_would_have_sealed integer,
    seal_authorised_at timestamp with time zone,
    seal_authorised_until timestamp with time zone,
    seal_authorised_ceiling integer,
    seal_authorised_by text,
    seal_authorised_reason text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT directory_sync_state_authorisation_by_check CHECK (((seal_authorised_by IS NULL) OR ((length(seal_authorised_by) >= 1) AND (length(seal_authorised_by) <= 255)))),
    CONSTRAINT directory_sync_state_authorisation_ceiling_check CHECK (((seal_authorised_ceiling IS NULL) OR (seal_authorised_ceiling > 0))),
    CONSTRAINT directory_sync_state_authorisation_pair_check CHECK ((num_nonnulls(seal_authorised_at, seal_authorised_until, seal_authorised_ceiling, seal_authorised_by, seal_authorised_reason) = ANY (ARRAY[0, 5]))),
    CONSTRAINT directory_sync_state_authorisation_reason_check CHECK (((seal_authorised_reason IS NULL) OR ((length(seal_authorised_reason) >= 1) AND (length(seal_authorised_reason) <= 512)))),
    CONSTRAINT directory_sync_state_authorisation_window_check CHECK (((seal_authorised_until IS NULL) OR (seal_authorised_until > seal_authorised_at))),
    CONSTRAINT directory_sync_state_breaker_count_check CHECK (((breaker_would_have_sealed IS NULL) OR (breaker_would_have_sealed > 0))),
    CONSTRAINT directory_sync_state_breaker_pair_check CHECK (((breaker_tripped_at IS NULL) = (breaker_would_have_sealed IS NULL))),
    CONSTRAINT directory_sync_state_complete_pass_check CHECK (((last_complete_pass_at IS NULL) OR (passes_completed > 0))),
    CONSTRAINT directory_sync_state_connector_check CHECK (((length(connector) >= 1) AND (length(connector) <= 64))),
    CONSTRAINT directory_sync_state_passes_check CHECK ((passes_completed >= 0))
);

ALTER TABLE ONLY public.directory_sync_state FORCE ROW LEVEL SECURITY;


--
-- Name: durable_operations; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.durable_operations (
    tenant_id uuid NOT NULL,
    id uuid NOT NULL,
    kind text NOT NULL,
    operation_version smallint DEFAULT 1 NOT NULL,
    state text DEFAULT 'pending'::text NOT NULL,
    proposal_id uuid,
    knowledge_item_id uuid,
    skill_version_id uuid,
    requested_by_identity_id uuid,
    authorized_at timestamp with time zone DEFAULT now() NOT NULL,
    input_hash text NOT NULL,
    progress_percent smallint DEFAULT 0 NOT NULL,
    attempts integer DEFAULT 0 NOT NULL,
    next_attempt_at timestamp with time zone DEFAULT now(),
    cancel_requested_at timestamp with time zone,
    lease_owner text,
    lease_expires_at timestamp with time zone,
    last_error_code text,
    result jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    started_at timestamp with time zone,
    completed_at timestamp with time zone,
    CONSTRAINT durable_operations_attempts_check CHECK ((attempts >= 0)),
    CONSTRAINT durable_operations_cancellation_check CHECK (((state = 'cancelled'::text) = (cancel_requested_at IS NOT NULL))),
    CONSTRAINT durable_operations_error_check CHECK (((last_error_code IS NULL) OR (last_error_code ~ '^[a-z][a-z0-9_]{0,63}$'::text))),
    CONSTRAINT durable_operations_input_hash_check CHECK ((input_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT durable_operations_kind_check CHECK ((kind = ANY (ARRAY['knowledge_erasure'::text, 'skill_validation'::text, 'source_cleanup'::text, 'source_reconcile'::text]))),
    CONSTRAINT durable_operations_lease_owner_check CHECK (((lease_owner IS NULL) OR ((btrim(lease_owner) <> ''::text) AND (char_length(lease_owner) <= 255)))),
    CONSTRAINT durable_operations_operation_version_check CHECK ((operation_version = 1)),
    CONSTRAINT durable_operations_progress_check CHECK (((progress_percent >= 0) AND (progress_percent <= 100))),
    CONSTRAINT durable_operations_result_object_check CHECK ((jsonb_typeof(result) = 'object'::text)),
    CONSTRAINT durable_operations_result_size_check CHECK ((octet_length((result)::text) <= 16384)),
    CONSTRAINT durable_operations_state_check CHECK ((state = ANY (ARRAY['pending'::text, 'running'::text, 'succeeded'::text, 'failed'::text, 'blocked'::text, 'cancelled'::text, 'dead_lettered'::text]))),
    CONSTRAINT durable_operations_target_check CHECK ((((kind = 'knowledge_erasure'::text) AND (proposal_id IS NOT NULL) AND (knowledge_item_id IS NOT NULL) AND (skill_version_id IS NULL) AND (requested_by_identity_id IS NULL) AND (cancel_requested_at IS NULL)) OR ((kind = 'skill_validation'::text) AND (proposal_id IS NULL) AND (knowledge_item_id IS NULL) AND (skill_version_id IS NOT NULL) AND (requested_by_identity_id IS NOT NULL)) OR ((kind = 'source_cleanup'::text) AND (proposal_id IS NOT NULL) AND (knowledge_item_id IS NULL) AND (skill_version_id IS NULL) AND (requested_by_identity_id IS NULL) AND (cancel_requested_at IS NULL)) OR ((kind = 'source_reconcile'::text) AND (proposal_id IS NULL) AND (knowledge_item_id IS NULL) AND (skill_version_id IS NULL) AND (requested_by_identity_id IS NOT NULL)))),
    CONSTRAINT durable_operations_time_check CHECK ((((state = 'pending'::text) AND (started_at IS NULL) AND (completed_at IS NULL) AND (lease_owner IS NULL) AND (lease_expires_at IS NULL) AND (next_attempt_at IS NOT NULL) AND (last_error_code IS NULL) AND (progress_percent = 0)) OR ((state = 'running'::text) AND (started_at IS NOT NULL) AND (completed_at IS NULL) AND (lease_owner IS NOT NULL) AND (lease_expires_at IS NOT NULL) AND (next_attempt_at IS NULL) AND (last_error_code IS NULL) AND (progress_percent < 100)) OR ((state = 'failed'::text) AND (started_at IS NOT NULL) AND (completed_at IS NULL) AND (lease_owner IS NULL) AND (lease_expires_at IS NULL) AND (next_attempt_at IS NOT NULL) AND (last_error_code IS NOT NULL) AND (progress_percent < 100)) OR ((state = 'succeeded'::text) AND (completed_at IS NOT NULL) AND (lease_owner IS NULL) AND (lease_expires_at IS NULL) AND (next_attempt_at IS NULL) AND (last_error_code IS NULL) AND (progress_percent = 100)) OR ((state = 'blocked'::text) AND (completed_at IS NOT NULL) AND (lease_owner IS NULL) AND (lease_expires_at IS NULL) AND (next_attempt_at IS NULL) AND (last_error_code IS NOT NULL)) OR ((state = 'cancelled'::text) AND (completed_at IS NOT NULL) AND (lease_owner IS NULL) AND (lease_expires_at IS NULL) AND (next_attempt_at IS NULL) AND (cancel_requested_at IS NOT NULL)) OR ((state = 'dead_lettered'::text) AND (completed_at IS NOT NULL) AND (lease_owner IS NULL) AND (lease_expires_at IS NULL) AND (next_attempt_at IS NULL) AND (last_error_code IS NOT NULL))))
);

ALTER TABLE ONLY public.durable_operations FORCE ROW LEVEL SECURITY;


--
-- Name: group_members; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.group_members (
    tenant_id uuid NOT NULL,
    group_id uuid NOT NULL,
    identity_id uuid NOT NULL,
    source text DEFAULT 'direct'::text NOT NULL,
    added_by text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT group_members_added_by_check CHECK (((added_by IS NULL) OR ((length(added_by) >= 1) AND (length(added_by) <= 255)))),
    CONSTRAINT group_members_source_check CHECK ((source = ANY (ARRAY['owner'::text, 'direct'::text, 'invite'::text, 'directory'::text, 'automation'::text])))
);

ALTER TABLE ONLY public.group_members FORCE ROW LEVEL SECURITY;


--
-- Name: groups; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.groups (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    slug text NOT NULL,
    display_name text NOT NULL,
    description text,
    source text DEFAULT 'direct'::text NOT NULL,
    status text DEFAULT 'active'::text NOT NULL,
    revision bigint DEFAULT 1 NOT NULL,
    created_by text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    directory_source text,
    directory_resource_id text,
    directory_external_id text,
    CONSTRAINT groups_created_by_check CHECK (((created_by IS NULL) OR ((length(created_by) >= 1) AND (length(created_by) <= 255)))),
    CONSTRAINT groups_description_check CHECK (((description IS NULL) OR ((btrim(description) <> ''::text) AND (length(description) <= 2000)))),
    CONSTRAINT groups_directory_external_check CHECK (((directory_external_id IS NULL) OR ((btrim(directory_external_id) = directory_external_id) AND ((length(directory_external_id) >= 1) AND (length(directory_external_id) <= 255))))),
    CONSTRAINT groups_directory_resource_check CHECK (((directory_resource_id IS NULL) OR ((btrim(directory_resource_id) = directory_resource_id) AND ((length(directory_resource_id) >= 1) AND (length(directory_resource_id) <= 255))))),
    CONSTRAINT groups_directory_shape_check CHECK ((((source = 'directory'::text) = ((directory_source IS NOT NULL) AND (directory_resource_id IS NOT NULL))) AND ((source = 'directory'::text) OR (directory_external_id IS NULL)))),
    CONSTRAINT groups_directory_source_check CHECK (((directory_source IS NULL) OR ((btrim(directory_source) = directory_source) AND ((length(directory_source) >= 1) AND (length(directory_source) <= 64))))),
    CONSTRAINT groups_display_name_check CHECK (((btrim(display_name) <> ''::text) AND (length(display_name) <= 200))),
    CONSTRAINT groups_revision_check CHECK ((revision >= 1)),
    CONSTRAINT groups_slug_check CHECK ((slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'::text)),
    CONSTRAINT groups_source_check CHECK ((source = ANY (ARRAY['direct'::text, 'directory'::text]))),
    CONSTRAINT groups_status_check CHECK ((status = ANY (ARRAY['active'::text, 'archived'::text]))),
    CONSTRAINT groups_updated_check CHECK ((updated_at >= created_at))
);

ALTER TABLE ONLY public.groups FORCE ROW LEVEL SECURITY;


--
-- Name: historical_knowledge_dependencies; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.historical_knowledge_dependencies (
    tenant_id uuid NOT NULL,
    dependent_kind text NOT NULL,
    dependent_id uuid NOT NULL,
    knowledge_item_id uuid NOT NULL,
    revision_id uuid NOT NULL,
    content_hash text NOT NULL,
    CONSTRAINT historical_knowledge_dependencies_content_hash_check CHECK ((content_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT historical_knowledge_dependencies_dependent_kind_check CHECK ((dependent_kind = ANY (ARRAY['session_event'::text, 'capture_candidate'::text, 'knowledge_source'::text, 'knowledge_revision'::text, 'knowledge_change'::text, 'context_run'::text])))
);

ALTER TABLE ONLY public.historical_knowledge_dependencies FORCE ROW LEVEL SECURITY;


--
-- Name: idempotency_records; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.idempotency_records (
    tenant_id uuid NOT NULL,
    subject text NOT NULL,
    operation text NOT NULL,
    idempotency_key text NOT NULL,
    request_digest bytea NOT NULL,
    resource_id uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT idempotency_records_digest_check CHECK ((octet_length(request_digest) = 32)),
    CONSTRAINT idempotency_records_key_check CHECK (((length(idempotency_key) >= 1) AND (length(idempotency_key) <= 255))),
    CONSTRAINT idempotency_records_operation_check CHECK (((length(operation) >= 1) AND (length(operation) <= 64))),
    CONSTRAINT idempotency_records_subject_check CHECK (((length(subject) >= 1) AND (length(subject) <= 255)))
);

ALTER TABLE ONLY public.idempotency_records FORCE ROW LEVEL SECURITY;


--
-- Name: identities; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.identities (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    subject text,
    email text,
    display_name text,
    scope_id uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    kind text DEFAULT 'user'::text NOT NULL,
    status text DEFAULT 'active'::text NOT NULL,
    departed_at timestamp with time zone,
    CONSTRAINT identities_departed_at_check CHECK (((status = 'departed'::text) = (departed_at IS NOT NULL))),
    CONSTRAINT identities_kind_check CHECK ((kind = ANY (ARRAY['user'::text, 'service'::text]))),
    CONSTRAINT identities_status_check CHECK ((status = ANY (ARRAY['active'::text, 'departed'::text]))),
    CONSTRAINT identities_subject_check CHECK (((length(subject) >= 1) AND (length(subject) <= 255)))
);

ALTER TABLE ONLY public.identities FORCE ROW LEVEL SECURITY;


--
-- Name: import_artifacts; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.import_artifacts (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    job_id uuid NOT NULL,
    ordinal integer NOT NULL,
    logical_path text NOT NULL,
    artifact_kind text NOT NULL,
    content_hash text NOT NULL,
    frontmatter jsonb NOT NULL,
    body_markdown text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    content_erased boolean DEFAULT false NOT NULL,
    CONSTRAINT import_artifacts_body_check CHECK ((octet_length(body_markdown) <= 262144)),
    CONSTRAINT import_artifacts_frontmatter_check CHECK (((jsonb_typeof(frontmatter) = 'object'::text) AND (octet_length((frontmatter)::text) <= 32768))),
    CONSTRAINT import_artifacts_hash_check CHECK ((content_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT import_artifacts_kind_check CHECK ((artifact_kind = ANY (ARRAY['concept'::text, 'index'::text, 'log'::text]))),
    CONSTRAINT import_artifacts_ordinal_check CHECK (((ordinal >= 1) AND (ordinal <= 2000))),
    CONSTRAINT import_artifacts_path_check CHECK (((content_erased AND (logical_path = ('erased/'::text || (id)::text)) AND (body_markdown = ''::text) AND (frontmatter = '{}'::jsonb)) OR ((NOT content_erased) AND (btrim(logical_path) <> ''::text) AND (char_length(logical_path) <= 1000) AND ("left"(logical_path, 1) <> '/'::text) AND (POSITION(('\'::text) IN (logical_path)) = 0) AND (logical_path !~ '(^|/)\.\.?(/|$)'::text))))
);

ALTER TABLE ONLY public.import_artifacts FORCE ROW LEVEL SECURITY;


--
-- Name: import_jobs; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.import_jobs (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    project_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    principal_id text NOT NULL,
    format text NOT NULL,
    format_version text NOT NULL,
    specification_commit text NOT NULL,
    source_kind text NOT NULL,
    source_locator text NOT NULL,
    source_revision text,
    bundle_digest text NOT NULL,
    state text DEFAULT 'planned'::text NOT NULL,
    artifact_count integer NOT NULL,
    mapping_count integer NOT NULL,
    candidate_count integer DEFAULT 0 NOT NULL,
    capture_batch_id uuid,
    error_code text,
    notices jsonb DEFAULT '[]'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    completed_at timestamp with time zone,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    repository_id uuid,
    repository_identity_digest text,
    managed_source_id text,
    managed_review_digest text,
    CONSTRAINT import_job_repository_shape CHECK ((((repository_id IS NULL) AND (repository_identity_digest IS NULL)) OR ((repository_id IS NOT NULL) AND (repository_identity_digest ~ '^[0-9a-f]{64}$'::text)))),
    CONSTRAINT import_jobs_count_check CHECK (((artifact_count >= 1) AND (artifact_count <= 2000) AND ((mapping_count >= 1) AND (mapping_count <= artifact_count)) AND ((candidate_count >= 0) AND (candidate_count <= mapping_count)))),
    CONSTRAINT import_jobs_digest_check CHECK ((bundle_digest ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT import_jobs_error_check CHECK (((error_code IS NULL) OR ((btrim(error_code) <> ''::text) AND (char_length(error_code) <= 100)))),
    CONSTRAINT import_jobs_format_check CHECK ((format = 'okf'::text)),
    CONSTRAINT import_jobs_notices_check CHECK (((jsonb_typeof(notices) = 'array'::text) AND (octet_length((notices)::text) <= 32768))),
    CONSTRAINT import_jobs_principal_check CHECK (((btrim(principal_id) <> ''::text) AND (char_length(principal_id) <= 255))),
    CONSTRAINT import_jobs_source_kind_check CHECK ((source_kind = ANY (ARRAY['directory'::text, 'zip'::text, 'tar'::text, 'git'::text]))),
    CONSTRAINT import_jobs_source_locator_check CHECK (((btrim(source_locator) <> ''::text) AND (char_length(source_locator) <= 1000))),
    CONSTRAINT import_jobs_source_revision_check CHECK ((((source_kind = 'git'::text) AND (source_revision IS NOT NULL)) OR (source_kind <> 'git'::text))),
    CONSTRAINT import_jobs_source_revision_value_check CHECK (((source_revision IS NULL) OR ((btrim(source_revision) <> ''::text) AND (char_length(source_revision) <= 255)))),
    CONSTRAINT import_jobs_specification_check CHECK ((specification_commit = 'ad30107c31c06aec8a7d5636e0d1058118604e6f'::text)),
    CONSTRAINT import_jobs_state_check CHECK ((state = ANY (ARRAY['planned'::text, 'materialized'::text, 'failed'::text]))),
    CONSTRAINT import_jobs_state_shape_check CHECK ((((state = 'planned'::text) AND (capture_batch_id IS NULL) AND (candidate_count = 0) AND (completed_at IS NULL) AND (error_code IS NULL)) OR ((state = 'materialized'::text) AND (capture_batch_id IS NOT NULL) AND (completed_at IS NOT NULL) AND (error_code IS NULL)) OR ((state = 'failed'::text) AND (capture_batch_id IS NULL) AND (candidate_count = 0) AND (completed_at IS NOT NULL) AND (error_code IS NOT NULL)))),
    CONSTRAINT import_jobs_time_check CHECK (((updated_at >= created_at) AND ((completed_at IS NULL) OR (completed_at >= created_at)))),
    CONSTRAINT import_jobs_version_check CHECK ((format_version = '0.2'::text)),
    CONSTRAINT import_managed_source_shape CHECK ((((managed_source_id IS NULL) AND (managed_review_digest IS NULL)) OR ((managed_source_id IS NOT NULL) AND (managed_source_id ~ '^[a-z0-9][a-z0-9-]{0,47}$'::text) AND (managed_review_digest IS NOT NULL) AND (managed_review_digest ~ '^[0-9a-f]{64}$'::text) AND (repository_id IS NOT NULL) AND (repository_identity_digest IS NOT NULL))))
);

ALTER TABLE ONLY public.import_jobs FORCE ROW LEVEL SECURITY;


--
-- Name: import_mappings; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.import_mappings (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    job_id uuid NOT NULL,
    artifact_id uuid NOT NULL,
    ordinal integer NOT NULL,
    okf_type text NOT NULL,
    knowledge_type text NOT NULL,
    title text NOT NULL,
    body_markdown text NOT NULL,
    summary text NOT NULL,
    tags text[] DEFAULT '{}'::text[] NOT NULL,
    sensitivity text NOT NULL,
    confidence_permille integer NOT NULL,
    valid_from timestamp with time zone NOT NULL,
    valid_to timestamp with time zone,
    stale_after timestamp with time zone,
    verification_metadata jsonb NOT NULL,
    metadata jsonb NOT NULL,
    content_hash text NOT NULL,
    classification text NOT NULL,
    matched_item_id uuid,
    matched_revision_id uuid,
    proposed_relations jsonb DEFAULT '[]'::jsonb NOT NULL,
    materializable boolean NOT NULL,
    candidate_id uuid,
    content_erased boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT import_mappings_body_check CHECK ((((NOT content_erased) AND (btrim(body_markdown) <> ''::text) AND (octet_length(body_markdown) <= 131072)) OR (content_erased AND (body_markdown = ''::text)))),
    CONSTRAINT import_mappings_classification_check CHECK ((classification = ANY (ARRAY['addition'::text, 'update'::text, 'duplicate'::text, 'conflict'::text]))),
    CONSTRAINT import_mappings_confidence_check CHECK (((confidence_permille >= 0) AND (confidence_permille <= 1000))),
    CONSTRAINT import_mappings_erasure_check CHECK (((NOT content_erased) OR ((title = ''::text) AND (body_markdown = ''::text) AND (summary = ''::text) AND (tags = '{}'::text[]) AND (verification_metadata = '{}'::jsonb) AND (metadata = '{}'::jsonb) AND (proposed_relations = '[]'::jsonb) AND (matched_item_id IS NULL) AND (matched_revision_id IS NULL) AND (NOT materializable)))),
    CONSTRAINT import_mappings_hash_check CHECK ((content_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT import_mappings_knowledge_type_check CHECK ((knowledge_type = ANY (ARRAY['fact'::text, 'decision'::text, 'preference'::text, 'procedure'::text, 'entity'::text, 'episode'::text, 'convention'::text, 'warning'::text, 'reference'::text]))),
    CONSTRAINT import_mappings_match_shape_check CHECK ((content_erased OR (((classification = 'addition'::text) AND (matched_item_id IS NULL) AND (matched_revision_id IS NULL)) OR ((classification <> 'addition'::text) AND (matched_item_id IS NOT NULL) AND (matched_revision_id IS NOT NULL))))),
    CONSTRAINT import_mappings_metadata_check CHECK (((jsonb_typeof(metadata) = 'object'::text) AND (octet_length((metadata)::text) <= 16384))),
    CONSTRAINT import_mappings_okf_type_check CHECK (((btrim(okf_type) <> ''::text) AND (char_length(okf_type) <= 200))),
    CONSTRAINT import_mappings_ordinal_check CHECK (((ordinal >= 1) AND (ordinal <= 2000))),
    CONSTRAINT import_mappings_relations_check CHECK (((jsonb_typeof(proposed_relations) = 'array'::text) AND (octet_length((proposed_relations)::text) <= 65536))),
    CONSTRAINT import_mappings_sensitivity_check CHECK ((sensitivity = ANY (ARRAY['public'::text, 'internal'::text, 'confidential'::text, 'restricted'::text]))),
    CONSTRAINT import_mappings_stale_time_check CHECK (((stale_after IS NULL) OR ((stale_after > valid_from) AND ((valid_to IS NULL) OR (stale_after <= valid_to))))),
    CONSTRAINT import_mappings_summary_check CHECK ((((NOT content_erased) AND (btrim(summary) <> ''::text) AND (char_length(summary) <= 2000)) OR (content_erased AND (summary = ''::text)))),
    CONSTRAINT import_mappings_tags_check CHECK ((((NOT content_erased) AND public.synveda_knowledge_tags_canonical(tags)) OR (content_erased AND (tags = '{}'::text[])))),
    CONSTRAINT import_mappings_title_check CHECK ((((NOT content_erased) AND (btrim(title) <> ''::text) AND (char_length(title) <= 300)) OR (content_erased AND (title = ''::text)))),
    CONSTRAINT import_mappings_valid_time_check CHECK (((valid_to IS NULL) OR (valid_to > valid_from))),
    CONSTRAINT import_mappings_verification_check CHECK (((jsonb_typeof(verification_metadata) = 'object'::text) AND (octet_length((verification_metadata)::text) <= 16384)))
);

ALTER TABLE ONLY public.import_mappings FORCE ROW LEVEL SECURITY;


--
-- Name: knowledge_change_headers; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.knowledge_change_headers (
    tenant_id uuid NOT NULL,
    proposal_id uuid NOT NULL,
    command_kind text NOT NULL,
    payload_hash text NOT NULL,
    target_item_ids uuid[] NOT NULL,
    source_event_id uuid,
    source_session_id uuid,
    source_scope_id uuid,
    source_payload_hash text,
    source_mode text,
    operation_id uuid,
    applied_at timestamp with time zone,
    source_artifact_id uuid,
    source_artifact_hash text,
    CONSTRAINT artifact_change_shape CHECK (((command_kind <> 'control_artifact'::text) OR ((source_artifact_id IS NOT NULL) AND (source_scope_id IS NOT NULL) AND (source_artifact_hash ~ '^[0-9a-f]{64}$'::text) AND (source_mode = ANY (ARRAY['withdraw'::text, 'forget'::text]))))),
    CONSTRAINT knowledge_change_headers_check CHECK (((command_kind <> 'control_source'::text) OR ((source_event_id IS NOT NULL) AND (source_session_id IS NOT NULL) AND (source_scope_id IS NOT NULL) AND (source_payload_hash ~ '^[0-9a-f]{64}$'::text) AND (source_mode = ANY (ARRAY['withdraw'::text, 'forget'::text]))))),
    CONSTRAINT knowledge_change_headers_payload_hash_check CHECK ((payload_hash ~ '^[0-9a-f]{64}$'::text))
);

ALTER TABLE ONLY public.knowledge_change_headers FORCE ROW LEVEL SECURITY;


--
-- Name: knowledge_changes; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.knowledge_changes (
    tenant_id uuid NOT NULL,
    proposal_id uuid NOT NULL,
    command_kind text NOT NULL,
    target_item_ids uuid[] DEFAULT '{}'::uuid[] NOT NULL,
    payload jsonb,
    payload_hash text NOT NULL,
    resulting_item_id uuid,
    resulting_revision_id uuid,
    operation_id uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    applied_at timestamp with time zone,
    CONSTRAINT knowledge_changes_command_check CHECK ((command_kind = ANY (ARRAY['create'::text, 'edit'::text, 'verify'::text, 'supersede'::text, 'merge'::text, 'archive'::text, 'restore'::text, 'forget'::text, 'resolve_conflict'::text, 'control_source'::text, 'control_artifact'::text]))),
    CONSTRAINT knowledge_changes_hash_check CHECK ((payload_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT knowledge_changes_payload_object_check CHECK (((payload IS NULL) OR (jsonb_typeof(payload) = 'object'::text))),
    CONSTRAINT knowledge_changes_payload_size_check CHECK (((payload IS NULL) OR (octet_length((payload)::text) <= 2097152))),
    CONSTRAINT knowledge_changes_result_shape_check CHECK (((applied_at IS NULL) = ((resulting_item_id IS NULL) AND (resulting_revision_id IS NULL) AND (operation_id IS NULL)))),
    CONSTRAINT knowledge_changes_targets_check CHECK ((cardinality(target_item_ids) <= 200))
);

ALTER TABLE ONLY public.knowledge_changes FORCE ROW LEVEL SECURITY;


--
-- Name: knowledge_conflict_members; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.knowledge_conflict_members (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    conflict_set_id uuid NOT NULL,
    role text NOT NULL,
    knowledge_item_id uuid,
    knowledge_revision_id uuid,
    capture_candidate_id uuid,
    classification text NOT NULL,
    similarity_permille integer NOT NULL,
    reason_code text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT knowledge_conflict_members_classification_check CHECK ((classification = ANY (ARRAY['duplicate'::text, 'support'::text, 'contradiction'::text, 'supersession'::text, 'transition'::text]))),
    CONSTRAINT knowledge_conflict_members_reason_check CHECK (((reason_code ~ '^[a-z][a-z0-9_]*$'::text) AND (length(reason_code) <= 64))),
    CONSTRAINT knowledge_conflict_members_role_check CHECK ((role = ANY (ARRAY['challenger'::text, 'current'::text]))),
    CONSTRAINT knowledge_conflict_members_shape_check CHECK ((((knowledge_item_id IS NOT NULL) AND (knowledge_revision_id IS NOT NULL) AND (capture_candidate_id IS NULL)) OR ((knowledge_item_id IS NULL) AND (knowledge_revision_id IS NULL) AND (capture_candidate_id IS NOT NULL) AND (role = 'challenger'::text)))),
    CONSTRAINT knowledge_conflict_members_similarity_check CHECK (((similarity_permille >= 0) AND (similarity_permille <= 1000)))
);

ALTER TABLE ONLY public.knowledge_conflict_members FORCE ROW LEVEL SECURITY;


--
-- Name: knowledge_conflict_sets; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.knowledge_conflict_sets (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    project_id uuid,
    classification text NOT NULL,
    status text DEFAULT 'open'::text NOT NULL,
    revision bigint DEFAULT 1 NOT NULL,
    capture_candidate_id uuid,
    resolution_change_id uuid,
    resolution text,
    created_by text NOT NULL,
    resolved_by text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    resolved_at timestamp with time zone,
    CONSTRAINT knowledge_conflict_sets_actor_check CHECK (((btrim(created_by) <> ''::text) AND (length(created_by) <= 255) AND ((resolved_by IS NULL) OR ((btrim(resolved_by) <> ''::text) AND (length(resolved_by) <= 255))))),
    CONSTRAINT knowledge_conflict_sets_classification_check CHECK ((classification = ANY (ARRAY['duplicate'::text, 'support'::text, 'contradiction'::text, 'supersession'::text, 'transition'::text]))),
    CONSTRAINT knowledge_conflict_sets_resolution_check CHECK (((resolution IS NULL) OR (resolution = ANY (ARRAY['keep_separate'::text, 'support'::text, 'duplicate'::text, 'supersede'::text, 'transition'::text, 'archive'::text])))),
    CONSTRAINT knowledge_conflict_sets_resolution_shape_check CHECK ((((status = 'open'::text) AND (resolution_change_id IS NULL) AND (resolution IS NULL) AND (resolved_by IS NULL) AND (resolved_at IS NULL)) OR ((status = 'pending_review'::text) AND (resolution_change_id IS NOT NULL) AND (resolution IS NOT NULL) AND (resolved_by IS NOT NULL) AND (resolved_at IS NULL)) OR ((status = ANY (ARRAY['resolved'::text, 'dismissed'::text])) AND (resolution_change_id IS NOT NULL) AND (resolution IS NOT NULL) AND (resolved_by IS NOT NULL) AND (resolved_at IS NOT NULL)))),
    CONSTRAINT knowledge_conflict_sets_revision_check CHECK ((revision > 0)),
    CONSTRAINT knowledge_conflict_sets_status_check CHECK ((status = ANY (ARRAY['open'::text, 'pending_review'::text, 'resolved'::text, 'dismissed'::text]))),
    CONSTRAINT knowledge_conflict_sets_time_check CHECK (((updated_at >= created_at) AND ((resolved_at IS NULL) OR (resolved_at >= created_at))))
);

ALTER TABLE ONLY public.knowledge_conflict_sets FORCE ROW LEVEL SECURITY;


--
-- Name: knowledge_copy_dependencies; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.knowledge_copy_dependencies (
    tenant_id uuid NOT NULL,
    dependent_kind text NOT NULL,
    dependent_id uuid NOT NULL,
    knowledge_item_id uuid NOT NULL,
    revision_id uuid NOT NULL,
    content_hash text NOT NULL,
    CONSTRAINT knowledge_copy_dependencies_content_hash_check CHECK ((content_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT knowledge_copy_dependencies_dependent_kind_check CHECK ((dependent_kind = ANY (ARRAY['session_event'::text, 'context_run'::text])))
);

ALTER TABLE ONLY public.knowledge_copy_dependencies FORCE ROW LEVEL SECURITY;


--
-- Name: knowledge_erasure_tombstones; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.knowledge_erasure_tombstones (
    tenant_id uuid NOT NULL,
    knowledge_item_id uuid NOT NULL,
    proposal_id uuid NOT NULL,
    operation_id uuid NOT NULL,
    revision_hashes jsonb NOT NULL,
    actor_hash text NOT NULL,
    reason_hash text NOT NULL,
    erased_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT knowledge_erasure_tombstones_actor_hash_check CHECK ((actor_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT knowledge_erasure_tombstones_reason_hash_check CHECK ((reason_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT knowledge_erasure_tombstones_revisions_check CHECK (((jsonb_typeof(revision_hashes) = 'array'::text) AND (octet_length((revision_hashes)::text) <= 65536)))
);

ALTER TABLE ONLY public.knowledge_erasure_tombstones FORCE ROW LEVEL SECURITY;


--
-- Name: knowledge_index_invalidations; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.knowledge_index_invalidations (
    tenant_id uuid NOT NULL,
    operation_id uuid NOT NULL,
    revision_id uuid NOT NULL,
    content_hash text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    processed_at timestamp with time zone,
    CONSTRAINT knowledge_index_invalidations_hash_check CHECK ((content_hash ~ '^[0-9a-f]{64}$'::text))
);

ALTER TABLE ONLY public.knowledge_index_invalidations FORCE ROW LEVEL SECURITY;


--
-- Name: knowledge_items; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.knowledge_items (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    project_id uuid,
    owner_principal_id text,
    knowledge_type text NOT NULL,
    origin text NOT NULL,
    lifecycle_state text DEFAULT 'active'::text NOT NULL,
    current_revision_id uuid NOT NULL,
    created_by text,
    updated_by text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    tx_from timestamp with time zone DEFAULT now() NOT NULL,
    tx_to timestamp with time zone,
    CONSTRAINT knowledge_items_created_by_check CHECK (((created_by IS NULL) OR ((btrim(created_by) <> ''::text) AND (char_length(created_by) <= 255)))),
    CONSTRAINT knowledge_items_current_tx_open_check CHECK ((tx_to IS NULL)),
    CONSTRAINT knowledge_items_lifecycle_check CHECK ((lifecycle_state = ANY (ARRAY['active'::text, 'stale'::text, 'transitional'::text, 'superseded'::text, 'archived'::text, 'erasure_pending'::text, 'erased'::text]))),
    CONSTRAINT knowledge_items_origin_check CHECK ((origin = ANY (ARRAY['observed'::text, 'asserted'::text, 'authored'::text, 'imported'::text]))),
    CONSTRAINT knowledge_items_owner_check CHECK (((owner_principal_id IS NULL) OR ((btrim(owner_principal_id) <> ''::text) AND (char_length(owner_principal_id) <= 255)))),
    CONSTRAINT knowledge_items_time_check CHECK (((updated_at >= created_at) AND (tx_from >= created_at))),
    CONSTRAINT knowledge_items_type_check CHECK ((knowledge_type = ANY (ARRAY['fact'::text, 'decision'::text, 'preference'::text, 'procedure'::text, 'entity'::text, 'episode'::text, 'convention'::text, 'warning'::text, 'reference'::text]))),
    CONSTRAINT knowledge_items_updated_by_check CHECK (((updated_by IS NULL) OR ((btrim(updated_by) <> ''::text) AND (char_length(updated_by) <= 255))))
);

ALTER TABLE ONLY public.knowledge_items FORCE ROW LEVEL SECURITY;


--
-- Name: knowledge_items_history; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.knowledge_items_history (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    project_id uuid,
    owner_principal_id text,
    knowledge_type text NOT NULL,
    origin text NOT NULL,
    lifecycle_state text NOT NULL,
    current_revision_id uuid NOT NULL,
    created_by text,
    updated_by text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    tx_from timestamp with time zone NOT NULL,
    tx_to timestamp with time zone NOT NULL,
    CONSTRAINT knowledge_items_history_created_by_check CHECK (((created_by IS NULL) OR ((btrim(created_by) <> ''::text) AND (char_length(created_by) <= 255)))),
    CONSTRAINT knowledge_items_history_lifecycle_check CHECK ((lifecycle_state = ANY (ARRAY['active'::text, 'stale'::text, 'transitional'::text, 'superseded'::text, 'archived'::text, 'erasure_pending'::text, 'erased'::text]))),
    CONSTRAINT knowledge_items_history_origin_check CHECK ((origin = ANY (ARRAY['observed'::text, 'asserted'::text, 'authored'::text, 'imported'::text]))),
    CONSTRAINT knowledge_items_history_owner_check CHECK (((owner_principal_id IS NULL) OR ((btrim(owner_principal_id) <> ''::text) AND (char_length(owner_principal_id) <= 255)))),
    CONSTRAINT knowledge_items_history_time_check CHECK (((updated_at >= created_at) AND (tx_from >= created_at) AND (tx_to > tx_from))),
    CONSTRAINT knowledge_items_history_type_check CHECK ((knowledge_type = ANY (ARRAY['fact'::text, 'decision'::text, 'preference'::text, 'procedure'::text, 'entity'::text, 'episode'::text, 'convention'::text, 'warning'::text, 'reference'::text]))),
    CONSTRAINT knowledge_items_history_updated_by_check CHECK (((updated_by IS NULL) OR ((btrim(updated_by) <> ''::text) AND (char_length(updated_by) <= 255))))
);

ALTER TABLE ONLY public.knowledge_items_history FORCE ROW LEVEL SECURITY;


--
-- Name: knowledge_relations; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.knowledge_relations (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    source_item_id uuid NOT NULL,
    target_item_id uuid NOT NULL,
    asserting_revision_id uuid NOT NULL,
    relation_type text NOT NULL,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_by text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    content_erased boolean DEFAULT false NOT NULL,
    CONSTRAINT knowledge_relations_created_by_check CHECK (((created_by IS NULL) OR ((btrim(created_by) <> ''::text) AND (char_length(created_by) <= 255)))),
    CONSTRAINT knowledge_relations_distinct_items_check CHECK ((source_item_id <> target_item_id)),
    CONSTRAINT knowledge_relations_metadata_object_check CHECK ((jsonb_typeof(metadata) = 'object'::text)),
    CONSTRAINT knowledge_relations_metadata_size_check CHECK ((octet_length((metadata)::text) <= 16384)),
    CONSTRAINT knowledge_relations_type_check CHECK ((relation_type = ANY (ARRAY['supports'::text, 'duplicates'::text, 'contradicts'::text, 'supersedes'::text, 'derived_from'::text, 'references'::text, 'related_to'::text, 'transitions_to'::text])))
);

ALTER TABLE ONLY public.knowledge_relations FORCE ROW LEVEL SECURITY;


--
-- Name: knowledge_revision_embeddings; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.knowledge_revision_embeddings (
    tenant_id uuid NOT NULL,
    knowledge_revision_id uuid NOT NULL,
    model text NOT NULL,
    dim integer NOT NULL,
    embedding public.vector NOT NULL,
    embedded_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT knowledge_revision_embeddings_dim_check CHECK (((dim > 0) AND (dim = public.vector_dims(embedding)))),
    CONSTRAINT knowledge_revision_embeddings_model_check CHECK (((btrim(model) <> ''::text) AND (char_length(model) <= 512)))
);

ALTER TABLE ONLY public.knowledge_revision_embeddings FORCE ROW LEVEL SECURITY;


--
-- Name: knowledge_revision_headers; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.knowledge_revision_headers (
    tenant_id uuid NOT NULL,
    revision_id uuid NOT NULL,
    knowledge_item_id uuid NOT NULL,
    revision_number bigint NOT NULL,
    sensitivity text NOT NULL,
    content_hash text NOT NULL,
    valid_from timestamp with time zone NOT NULL,
    valid_to timestamp with time zone,
    transaction_time timestamp with time zone NOT NULL,
    legal_hold boolean NOT NULL,
    retention_hold boolean NOT NULL,
    CONSTRAINT knowledge_revision_headers_content_hash_check CHECK ((content_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT knowledge_revision_headers_revision_number_check CHECK ((revision_number > 0)),
    CONSTRAINT knowledge_revision_headers_sensitivity_check CHECK ((sensitivity = ANY (ARRAY['public'::text, 'internal'::text, 'confidential'::text, 'restricted'::text])))
);

ALTER TABLE ONLY public.knowledge_revision_headers FORCE ROW LEVEL SECURITY;


--
-- Name: knowledge_revision_source_headers; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.knowledge_revision_source_headers (
    tenant_id uuid NOT NULL,
    knowledge_revision_id uuid NOT NULL,
    knowledge_source_id uuid NOT NULL
);

ALTER TABLE ONLY public.knowledge_revision_source_headers FORCE ROW LEVEL SECURITY;


--
-- Name: knowledge_revision_sources; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.knowledge_revision_sources (
    tenant_id uuid NOT NULL,
    knowledge_revision_id uuid NOT NULL,
    knowledge_source_id uuid NOT NULL,
    ordinal integer NOT NULL,
    linked_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT knowledge_revision_sources_ordinal_check CHECK ((ordinal >= 1))
);

ALTER TABLE ONLY public.knowledge_revision_sources FORCE ROW LEVEL SECURITY;


--
-- Name: knowledge_revisions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.knowledge_revisions (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    knowledge_item_id uuid NOT NULL,
    revision_number bigint NOT NULL,
    title text NOT NULL,
    body_markdown text NOT NULL,
    summary text NOT NULL,
    tags text[] DEFAULT '{}'::text[] NOT NULL,
    sensitivity text NOT NULL,
    confidence_permille integer NOT NULL,
    valid_from timestamp with time zone NOT NULL,
    valid_to timestamp with time zone,
    stale_after timestamp with time zone,
    verification_metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    content_hash text NOT NULL,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_by text,
    transaction_time timestamp with time zone DEFAULT now() NOT NULL,
    search_document tsvector GENERATED ALWAYS AS (((setweight(to_tsvector('simple'::regconfig, title), 'A'::"char") || setweight(to_tsvector('simple'::regconfig, summary), 'B'::"char")) || setweight(to_tsvector('simple'::regconfig, body_markdown), 'C'::"char"))) STORED,
    content_erased boolean DEFAULT false NOT NULL,
    CONSTRAINT knowledge_revisions_body_check CHECK (((btrim(body_markdown) <> ''::text) AND (octet_length(body_markdown) <= 131072))),
    CONSTRAINT knowledge_revisions_confidence_check CHECK (((confidence_permille >= 0) AND (confidence_permille <= 1000))),
    CONSTRAINT knowledge_revisions_content_hash_check CHECK ((content_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT knowledge_revisions_created_by_check CHECK (((created_by IS NULL) OR ((btrim(created_by) <> ''::text) AND (char_length(created_by) <= 255)))),
    CONSTRAINT knowledge_revisions_metadata_object_check CHECK ((jsonb_typeof(metadata) = 'object'::text)),
    CONSTRAINT knowledge_revisions_metadata_size_check CHECK ((octet_length((metadata)::text) <= 16384)),
    CONSTRAINT knowledge_revisions_number_check CHECK ((revision_number >= 1)),
    CONSTRAINT knowledge_revisions_sensitivity_check CHECK ((sensitivity = ANY (ARRAY['public'::text, 'internal'::text, 'confidential'::text, 'restricted'::text]))),
    CONSTRAINT knowledge_revisions_stale_time_check CHECK (((stale_after IS NULL) OR ((stale_after > valid_from) AND ((valid_to IS NULL) OR (stale_after <= valid_to))))),
    CONSTRAINT knowledge_revisions_summary_check CHECK (((btrim(summary) <> ''::text) AND (char_length(summary) <= 2000))),
    CONSTRAINT knowledge_revisions_tags_check CHECK (public.synveda_knowledge_tags_canonical(tags)),
    CONSTRAINT knowledge_revisions_title_check CHECK (((btrim(title) <> ''::text) AND (char_length(title) <= 300))),
    CONSTRAINT knowledge_revisions_valid_time_check CHECK (((valid_to IS NULL) OR (valid_to > valid_from))),
    CONSTRAINT knowledge_revisions_verification_object_check CHECK ((jsonb_typeof(verification_metadata) = 'object'::text)),
    CONSTRAINT knowledge_revisions_verification_size_check CHECK ((octet_length((verification_metadata)::text) <= 16384))
);

ALTER TABLE ONLY public.knowledge_revisions FORCE ROW LEVEL SECURITY;


--
-- Name: knowledge_sources; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.knowledge_sources (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    source_type text NOT NULL,
    session_event_id uuid,
    locator text,
    source_revision text,
    content_hash text,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_by text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    content_erased boolean DEFAULT false NOT NULL,
    import_artifact_id uuid,
    CONSTRAINT knowledge_source_artifact_shape CHECK (((import_artifact_id IS NULL) OR ((source_type = 'okf'::text) AND (session_event_id IS NULL) AND (content_hash IS NOT NULL)))),
    CONSTRAINT knowledge_sources_content_hash_check CHECK (((content_hash IS NULL) OR (content_hash ~ '^[0-9a-f]{64}$'::text))),
    CONSTRAINT knowledge_sources_created_by_check CHECK (((created_by IS NULL) OR ((btrim(created_by) <> ''::text) AND (char_length(created_by) <= 255)))),
    CONSTRAINT knowledge_sources_locator_check CHECK (((locator IS NULL) OR ((btrim(locator) <> ''::text) AND (char_length(locator) <= 2048)))),
    CONSTRAINT knowledge_sources_metadata_object_check CHECK ((jsonb_typeof(metadata) = 'object'::text)),
    CONSTRAINT knowledge_sources_metadata_size_check CHECK ((octet_length((metadata)::text) <= 16384)),
    CONSTRAINT knowledge_sources_revision_check CHECK (((source_revision IS NULL) OR ((btrim(source_revision) <> ''::text) AND (char_length(source_revision) <= 512)))),
    CONSTRAINT knowledge_sources_shape_check CHECK (((content_erased AND (locator IS NULL) AND (source_revision IS NULL) AND (metadata = '{}'::jsonb)) OR ((NOT content_erased) AND (((source_type = 'session_event'::text) AND (session_event_id IS NOT NULL) AND (locator IS NULL)) OR ((source_type = 'manual'::text) AND (session_event_id IS NULL) AND (locator IS NULL) AND (source_revision IS NULL)) OR ((source_type = ANY (ARRAY['document'::text, 'repository'::text, 'url'::text, 'okf'::text, 'system_derived'::text])) AND (session_event_id IS NULL) AND (locator IS NOT NULL)))))),
    CONSTRAINT knowledge_sources_type_check CHECK ((source_type = ANY (ARRAY['session_event'::text, 'manual'::text, 'document'::text, 'repository'::text, 'url'::text, 'okf'::text, 'system_derived'::text])))
);

ALTER TABLE ONLY public.knowledge_sources FORCE ROW LEVEL SECURITY;


--
-- Name: operation_attempts; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.operation_attempts (
    tenant_id uuid NOT NULL,
    operation_id uuid NOT NULL,
    attempt_number integer NOT NULL,
    worker_id text NOT NULL,
    state text DEFAULT 'running'::text NOT NULL,
    started_at timestamp with time zone DEFAULT now() NOT NULL,
    heartbeat_at timestamp with time zone DEFAULT now() NOT NULL,
    completed_at timestamp with time zone,
    safe_error_code text,
    CONSTRAINT operation_attempts_error_check CHECK (((safe_error_code IS NULL) OR (safe_error_code ~ '^[a-z][a-z0-9_]{0,63}$'::text))),
    CONSTRAINT operation_attempts_number_check CHECK ((attempt_number > 0)),
    CONSTRAINT operation_attempts_state_check CHECK ((state = ANY (ARRAY['running'::text, 'succeeded'::text, 'retry_scheduled'::text, 'cancelled'::text, 'dead_lettered'::text]))),
    CONSTRAINT operation_attempts_time_check CHECK ((((state = 'running'::text) AND (completed_at IS NULL) AND (safe_error_code IS NULL)) OR ((state = 'succeeded'::text) AND (completed_at IS NOT NULL) AND (safe_error_code IS NULL)) OR ((state = 'cancelled'::text) AND (completed_at IS NOT NULL)) OR ((state = ANY (ARRAY['retry_scheduled'::text, 'dead_lettered'::text])) AND (completed_at IS NOT NULL) AND (safe_error_code IS NOT NULL)))),
    CONSTRAINT operation_attempts_worker_check CHECK (((btrim(worker_id) <> ''::text) AND (char_length(worker_id) <= 255)))
);

ALTER TABLE ONLY public.operation_attempts FORCE ROW LEVEL SECURITY;


--
-- Name: operation_outbox; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.operation_outbox (
    tenant_id uuid NOT NULL,
    operation_id uuid NOT NULL,
    operation_version smallint NOT NULL,
    state text DEFAULT 'pending'::text NOT NULL,
    dispatch_attempts integer DEFAULT 0 NOT NULL,
    submission_failures integer DEFAULT 0 NOT NULL,
    lease_owner text,
    lease_expires_at timestamp with time zone,
    next_dispatch_at timestamp with time zone DEFAULT now(),
    last_error_code text,
    dispatched_at timestamp with time zone,
    completed_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT operation_outbox_attempts_check CHECK ((dispatch_attempts >= 0)),
    CONSTRAINT operation_outbox_error_check CHECK (((last_error_code IS NULL) OR (last_error_code ~ '^[a-z][a-z0-9_]{0,63}$'::text))),
    CONSTRAINT operation_outbox_failures_check CHECK ((submission_failures >= 0)),
    CONSTRAINT operation_outbox_lease_owner_check CHECK (((lease_owner IS NULL) OR ((btrim(lease_owner) <> ''::text) AND (char_length(lease_owner) <= 255)))),
    CONSTRAINT operation_outbox_state_check CHECK ((state = ANY (ARRAY['pending'::text, 'claimed'::text, 'dispatched'::text, 'completed'::text]))),
    CONSTRAINT operation_outbox_time_check CHECK ((((state = 'pending'::text) AND (lease_owner IS NULL) AND (lease_expires_at IS NULL) AND (next_dispatch_at IS NOT NULL) AND (completed_at IS NULL)) OR ((state = 'claimed'::text) AND (lease_owner IS NOT NULL) AND (lease_expires_at IS NOT NULL) AND (next_dispatch_at IS NULL) AND (completed_at IS NULL)) OR ((state = 'dispatched'::text) AND (lease_owner IS NULL) AND (lease_expires_at IS NULL) AND (next_dispatch_at IS NULL) AND (dispatched_at IS NOT NULL) AND (completed_at IS NULL)) OR ((state = 'completed'::text) AND (lease_owner IS NULL) AND (lease_expires_at IS NULL) AND (next_dispatch_at IS NULL) AND (completed_at IS NOT NULL)))),
    CONSTRAINT operation_outbox_version_check CHECK ((operation_version = 1))
);

ALTER TABLE ONLY public.operation_outbox FORCE ROW LEVEL SECURITY;


--
-- Name: pending_invites; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.pending_invites (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    role_key text NOT NULL,
    email text,
    token_hash bytea NOT NULL,
    status text DEFAULT 'pending'::text NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    created_by text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    accepted_by text,
    accepted_at timestamp with time zone,
    revoked_by text,
    revoked_at timestamp with time zone,
    CONSTRAINT pending_invites_accepted_by_check CHECK (((accepted_by IS NULL) OR ((length(accepted_by) >= 1) AND (length(accepted_by) <= 255)))),
    CONSTRAINT pending_invites_accepted_shape_check CHECK (((status = 'accepted'::text) = ((accepted_at IS NOT NULL) AND (accepted_by IS NOT NULL)))),
    CONSTRAINT pending_invites_created_by_check CHECK (((created_by IS NULL) OR ((length(created_by) >= 1) AND (length(created_by) <= 255)))),
    CONSTRAINT pending_invites_email_check CHECK (((email IS NULL) OR ((btrim(email) <> ''::text) AND (length(email) <= 320)))),
    CONSTRAINT pending_invites_expiry_check CHECK ((expires_at > created_at)),
    CONSTRAINT pending_invites_hash_check CHECK ((octet_length(token_hash) = 32)),
    CONSTRAINT pending_invites_revoked_by_check CHECK (((revoked_by IS NULL) OR ((length(revoked_by) >= 1) AND (length(revoked_by) <= 255)))),
    CONSTRAINT pending_invites_revoked_shape_check CHECK (((status = 'revoked'::text) = (revoked_at IS NOT NULL))),
    CONSTRAINT pending_invites_role_check CHECK ((role_key = ANY (ARRAY['owner'::text, 'member'::text, 'viewer'::text, 'reviewer'::text, 'curator'::text, 'administrator'::text]))),
    CONSTRAINT pending_invites_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'accepted'::text, 'revoked'::text])))
);

ALTER TABLE ONLY public.pending_invites FORCE ROW LEVEL SECURITY;


--
-- Name: pending_logins; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.pending_logins (
    state_hash bytea NOT NULL,
    correlation_hash bytea,
    payload_sealed bytea NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    expires_at timestamp with time zone DEFAULT (now() + '00:10:00'::interval) NOT NULL,
    CONSTRAINT pending_logins_correlation_hash_check CHECK (((correlation_hash IS NULL) OR (octet_length(correlation_hash) = 32))),
    CONSTRAINT pending_logins_expiry_check CHECK ((expires_at > created_at)),
    CONSTRAINT pending_logins_payload_check CHECK (((octet_length(payload_sealed) >= 51) AND (octet_length(payload_sealed) <= 8192))),
    CONSTRAINT pending_logins_state_hash_check CHECK ((octet_length(state_hash) = 32))
);


--
-- Name: policy_packs; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.policy_packs (
    tenant_id uuid NOT NULL,
    name text NOT NULL,
    version bigint NOT NULL,
    source text NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    redaction jsonb,
    composition jsonb,
    approvals jsonb,
    scan jsonb,
    quality jsonb,
    CONSTRAINT policy_packs_name_check CHECK ((name ~ '^[a-z0-9][a-z0-9-]{0,62}$'::text)),
    CONSTRAINT policy_packs_name_reserved_check CHECK ((name <> ALL (ARRAY['regulated-strict'::text, 'standard'::text, 'open-collaboration'::text, 'bootstrap'::text]))),
    CONSTRAINT policy_packs_source_check CHECK ((length(source) > 0)),
    CONSTRAINT policy_packs_version_check CHECK ((version >= 1))
);

ALTER TABLE ONLY public.policy_packs FORCE ROW LEVEL SECURITY;


--
-- Name: policy_relaxation_changes; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.policy_relaxation_changes (
    tenant_id uuid NOT NULL,
    proposal_id uuid NOT NULL,
    command_kind text NOT NULL,
    payload jsonb NOT NULL,
    payload_hash text NOT NULL,
    resulting_relaxation_id uuid,
    resulting_version_id uuid,
    resulting_revision bigint,
    applied_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT policy_relaxation_changes_kind_check CHECK ((command_kind = ANY (ARRAY['create'::text, 'revise'::text, 'revoke'::text]))),
    CONSTRAINT policy_relaxation_changes_payload_check CHECK (((jsonb_typeof(payload) = 'object'::text) AND (pg_column_size(payload) <= 32768))),
    CONSTRAINT policy_relaxation_changes_payload_hash_check CHECK ((payload_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT policy_relaxation_changes_result_shape_check CHECK ((((applied_at IS NULL) AND (resulting_revision IS NULL)) OR ((applied_at IS NOT NULL) AND (resulting_relaxation_id IS NOT NULL) AND (resulting_revision IS NOT NULL) AND (resulting_revision > 0))))
);

ALTER TABLE ONLY public.policy_relaxation_changes FORCE ROW LEVEL SECURITY;


--
-- Name: policy_relaxation_versions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.policy_relaxation_versions (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    relaxation_id uuid NOT NULL,
    proposal_id uuid NOT NULL,
    ordinal bigint NOT NULL,
    subject_identity_id uuid NOT NULL,
    subject_principal_id text NOT NULL,
    target_scope_id uuid NOT NULL,
    action text NOT NULL,
    max_sensitivity text NOT NULL,
    requested_start_at timestamp with time zone NOT NULL,
    requested_end_at timestamp with time zone NOT NULL,
    effective_start_at timestamp with time zone NOT NULL,
    hard_expires_at timestamp with time zone NOT NULL,
    reason text NOT NULL,
    configuration_version_id uuid,
    configuration_hash text NOT NULL,
    content_hash bytea NOT NULL,
    creator_id uuid NOT NULL,
    approver_ids uuid[] DEFAULT '{}'::uuid[] NOT NULL,
    auto_applied boolean NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT policy_relaxation_versions_action_check CHECK ((action = 'knowledge.read'::text)),
    CONSTRAINT policy_relaxation_versions_approvers_check CHECK ((array_position(approver_ids, NULL::uuid) IS NULL)),
    CONSTRAINT policy_relaxation_versions_auto_apply_check CHECK (((cardinality(approver_ids) = 0) = auto_applied)),
    CONSTRAINT policy_relaxation_versions_configuration_hash_check CHECK ((configuration_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT policy_relaxation_versions_content_hash_check CHECK ((octet_length(content_hash) = 32)),
    CONSTRAINT policy_relaxation_versions_effective_window_check CHECK (((effective_start_at >= created_at) AND (effective_start_at >= requested_start_at) AND (hard_expires_at > effective_start_at) AND (hard_expires_at <= requested_end_at) AND (hard_expires_at <= (created_at + '90 days'::interval)))),
    CONSTRAINT policy_relaxation_versions_ordinal_check CHECK ((ordinal > 0)),
    CONSTRAINT policy_relaxation_versions_reason_check CHECK (((btrim(reason) = reason) AND ((char_length(reason) >= 1) AND (char_length(reason) <= 512)))),
    CONSTRAINT policy_relaxation_versions_requested_window_check CHECK (((requested_start_at < requested_end_at) AND (requested_end_at <= (requested_start_at + '90 days'::interval)))),
    CONSTRAINT policy_relaxation_versions_sensitivity_check CHECK ((max_sensitivity = ANY (ARRAY['public'::text, 'internal'::text, 'confidential'::text, 'restricted'::text]))),
    CONSTRAINT policy_relaxation_versions_subject_check CHECK (((btrim(subject_principal_id) = subject_principal_id) AND ((char_length(subject_principal_id) >= 1) AND (char_length(subject_principal_id) <= 255))))
);

ALTER TABLE ONLY public.policy_relaxation_versions FORCE ROW LEVEL SECURITY;


--
-- Name: policy_relaxations; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.policy_relaxations (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    governing_scope_id uuid NOT NULL,
    current_version_id uuid NOT NULL,
    revision bigint DEFAULT 1 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    created_by uuid NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_by uuid NOT NULL,
    revoked_at timestamp with time zone,
    revoked_by uuid,
    revocation_proposal_id uuid,
    revocation_reason text,
    expiry_recorded_at timestamp with time zone,
    CONSTRAINT policy_relaxations_revision_check CHECK ((revision > 0)),
    CONSTRAINT policy_relaxations_revocation_reason_check CHECK (((revocation_reason IS NULL) OR ((btrim(revocation_reason) = revocation_reason) AND ((char_length(revocation_reason) >= 1) AND (char_length(revocation_reason) <= 512))))),
    CONSTRAINT policy_relaxations_revocation_shape_check CHECK ((((revoked_at IS NULL) AND (revoked_by IS NULL) AND (revocation_proposal_id IS NULL) AND (revocation_reason IS NULL)) OR ((revoked_at IS NOT NULL) AND (revoked_by IS NOT NULL) AND (revocation_proposal_id IS NOT NULL) AND (revocation_reason IS NOT NULL))))
);

ALTER TABLE ONLY public.policy_relaxations FORCE ROW LEVEL SECURITY;


--
-- Name: procedure_candidate_headers; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.procedure_candidate_headers (
    tenant_id uuid NOT NULL,
    candidate_id uuid NOT NULL,
    request_event_id uuid NOT NULL,
    content_hash text NOT NULL,
    CONSTRAINT procedure_candidate_headers_content_hash_check CHECK ((content_hash ~ '^[0-9a-f]{64}$'::text))
);

ALTER TABLE ONLY public.procedure_candidate_headers FORCE ROW LEVEL SECURITY;


--
-- Name: procedure_evaluation_attempts; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.procedure_evaluation_attempts (
    tenant_id uuid NOT NULL,
    id uuid NOT NULL,
    candidate_id uuid NOT NULL,
    candidate_hash text NOT NULL,
    request_hash text NOT NULL,
    suite_id text NOT NULL,
    evaluator_id uuid NOT NULL,
    configuration_hash text NOT NULL,
    specification_hash text NOT NULL,
    specification jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT procedure_evaluation_attempts_check CHECK (((candidate_hash ~ '^[0-9a-f]{64}$'::text) AND (request_hash ~ '^[0-9a-f]{64}$'::text) AND (configuration_hash ~ '^[0-9a-f]{64}$'::text) AND (specification_hash ~ '^[0-9a-f]{64}$'::text))),
    CONSTRAINT procedure_evaluation_attempts_specification_check CHECK (((jsonb_typeof(specification) = 'object'::text) AND (octet_length((specification)::text) <= 16384))),
    CONSTRAINT procedure_evaluation_attempts_suite_id_check CHECK (((length(suite_id) >= 1) AND (length(suite_id) <= 128)))
);

ALTER TABLE ONLY public.procedure_evaluation_attempts FORCE ROW LEVEL SECURITY;


--
-- Name: procedure_evaluation_results; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.procedure_evaluation_results (
    tenant_id uuid NOT NULL,
    attempt_id uuid NOT NULL,
    reporter_id uuid NOT NULL,
    request_hash text NOT NULL,
    qualified boolean NOT NULL,
    results jsonb NOT NULL,
    observed_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT procedure_evaluation_results_request_hash_check CHECK ((request_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT procedure_evaluation_results_results_check CHECK (((jsonb_typeof(results) = 'array'::text) AND ((jsonb_array_length(results) >= 1) AND (jsonb_array_length(results) <= 32)) AND (octet_length((results)::text) <= 16384)))
);

ALTER TABLE ONLY public.procedure_evaluation_results FORCE ROW LEVEL SECURITY;


--
-- Name: procedure_improvement_headers; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.procedure_improvement_headers (
    tenant_id uuid NOT NULL,
    event_id uuid NOT NULL,
    session_id uuid NOT NULL,
    identity_hash text NOT NULL,
    request_hash text NOT NULL,
    procedure_item_id uuid NOT NULL,
    procedure_revision_id uuid NOT NULL,
    procedure_hash text NOT NULL,
    content_hash text NOT NULL,
    CONSTRAINT procedure_improvement_headers_check CHECK (((identity_hash ~ '^[0-9a-f]{64}$'::text) AND (request_hash ~ '^[0-9a-f]{64}$'::text) AND (procedure_hash ~ '^[0-9a-f]{64}$'::text) AND (content_hash ~ '^[0-9a-f]{64}$'::text)))
);

ALTER TABLE ONLY public.procedure_improvement_headers FORCE ROW LEVEL SECURITY;


--
-- Name: procedure_outcome_headers; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.procedure_outcome_headers (
    tenant_id uuid NOT NULL,
    event_id uuid NOT NULL,
    session_id uuid NOT NULL,
    procedure_item_id uuid NOT NULL,
    procedure_revision_id uuid NOT NULL,
    procedure_hash text NOT NULL,
    request_hash text NOT NULL,
    reporter_id uuid NOT NULL,
    evidence_class text NOT NULL,
    outcome text NOT NULL,
    environment_digest text NOT NULL,
    skill_versions uuid[] NOT NULL,
    context_run_id uuid,
    context_block_hash text,
    received_at timestamp with time zone NOT NULL,
    CONSTRAINT procedure_outcome_headers_check CHECK ((((context_run_id IS NULL) = (context_block_hash IS NULL)) AND ((context_block_hash IS NULL) OR (context_block_hash ~ '^[0-9a-f]{64}$'::text)))),
    CONSTRAINT procedure_outcome_headers_check1 CHECK (((procedure_hash ~ '^[0-9a-f]{64}$'::text) AND (request_hash ~ '^[0-9a-f]{64}$'::text) AND (environment_digest ~ '^[0-9a-f]{64}$'::text))),
    CONSTRAINT procedure_outcome_headers_evidence_class_check CHECK ((evidence_class = ANY (ARRAY['client_reported'::text, 'authenticated_runner'::text]))),
    CONSTRAINT procedure_outcome_headers_outcome_check CHECK ((outcome = ANY (ARRAY['succeeded'::text, 'failed'::text, 'unknown'::text]))),
    CONSTRAINT procedure_outcome_headers_skill_versions_check CHECK (((cardinality(skill_versions) <= 8) AND (array_position(skill_versions, NULL::uuid) IS NULL)))
);

ALTER TABLE ONLY public.procedure_outcome_headers FORCE ROW LEVEL SECURITY;


--
-- Name: project_repositories; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.project_repositories (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    project_id uuid NOT NULL,
    provider text NOT NULL,
    canonical_uri text NOT NULL,
    repository_owner text,
    repository_name text NOT NULL,
    default_branch text,
    local_fingerprint text,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_by uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT project_repositories_branch_check CHECK (((default_branch IS NULL) OR ((btrim(default_branch) <> ''::text) AND (length(default_branch) <= 255)))),
    CONSTRAINT project_repositories_fingerprint_check CHECK (((local_fingerprint IS NULL) OR (local_fingerprint ~ '^[0-9a-f]{40,128}$'::text))),
    CONSTRAINT project_repositories_local_shape_check CHECK (((provider = 'local'::text) = (canonical_uri ~~ 'git+fingerprint:%'::text))),
    CONSTRAINT project_repositories_metadata_object_check CHECK ((jsonb_typeof(metadata) = 'object'::text)),
    CONSTRAINT project_repositories_metadata_size_check CHECK ((octet_length((metadata)::text) <= 8192)),
    CONSTRAINT project_repositories_name_check CHECK (((btrim(repository_name) <> ''::text) AND (length(repository_name) <= 255))),
    CONSTRAINT project_repositories_owner_check CHECK (((repository_owner IS NULL) OR ((btrim(repository_owner) <> ''::text) AND (length(repository_owner) <= 255)))),
    CONSTRAINT project_repositories_provider_check CHECK ((provider = ANY (ARRAY['github'::text, 'gitlab'::text, 'bitbucket'::text, 'azure_devops'::text, 'generic_git'::text, 'local'::text]))),
    CONSTRAINT project_repositories_updated_check CHECK ((updated_at >= created_at)),
    CONSTRAINT project_repositories_uri_check CHECK (((canonical_uri ~ '^https://[a-z0-9][a-z0-9._-]*/[^[:space:]]+$'::text) OR (canonical_uri ~ '^git\+fingerprint:[0-9a-f]{40,128}$'::text))),
    CONSTRAINT project_repositories_uri_length_check CHECK (((length(canonical_uri) >= 1) AND (length(canonical_uri) <= 512)))
);

ALTER TABLE ONLY public.project_repositories FORCE ROW LEVEL SECURITY;


--
-- Name: projects; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.projects (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    scope_kind text NOT NULL,
    workspace_scope_id uuid NOT NULL,
    slug text NOT NULL,
    display_name text NOT NULL,
    description text,
    status text DEFAULT 'active'::text NOT NULL,
    revision bigint DEFAULT 1 NOT NULL,
    created_by uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT projects_description_check CHECK (((description IS NULL) OR ((btrim(description) <> ''::text) AND (length(description) <= 2000)))),
    CONSTRAINT projects_display_name_check CHECK (((btrim(display_name) <> ''::text) AND (length(display_name) <= 200))),
    CONSTRAINT projects_revision_check CHECK ((revision >= 1)),
    CONSTRAINT projects_scope_kind_check CHECK ((scope_kind = 'project'::text)),
    CONSTRAINT projects_slug_check CHECK ((slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'::text)),
    CONSTRAINT projects_status_check CHECK ((status = ANY (ARRAY['active'::text, 'archived'::text]))),
    CONSTRAINT projects_updated_check CHECK ((updated_at >= created_at))
);

ALTER TABLE ONLY public.projects FORCE ROW LEVEL SECURITY;


--
-- Name: prompts; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.prompts (
    tenant_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    name text NOT NULL,
    description text NOT NULL,
    template text NOT NULL,
    variables jsonb NOT NULL,
    sensitivity text NOT NULL,
    object_hash bytea NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    created_by uuid NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_by uuid NOT NULL,
    CONSTRAINT prompts_description_check CHECK (((length(description) >= 0) AND (length(description) <= 512))),
    CONSTRAINT prompts_name_check CHECK (((length(name) >= 1) AND (length(name) <= 128))),
    CONSTRAINT prompts_sensitivity_check CHECK ((sensitivity = ANY (ARRAY['public'::text, 'internal'::text, 'confidential'::text]))),
    CONSTRAINT prompts_template_check CHECK (((length(template) >= 1) AND (length(template) <= 32768))),
    CONSTRAINT prompts_variables_check CHECK ((jsonb_typeof(variables) = 'array'::text))
);

ALTER TABLE ONLY public.prompts FORCE ROW LEVEL SECURITY;


--
-- Name: schema_metadata; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.schema_metadata (
    id boolean DEFAULT true NOT NULL,
    epoch integer NOT NULL,
    baseline_revision integer NOT NULL,
    migration_head text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    created_by_version text NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT schema_metadata_baseline_revision_check CHECK ((baseline_revision >= 1)),
    CONSTRAINT schema_metadata_created_by_version_check CHECK (((length(created_by_version) >= 1) AND (length(created_by_version) <= 64))),
    CONSTRAINT schema_metadata_epoch_check CHECK ((epoch >= 1)),
    CONSTRAINT schema_metadata_migration_head_check CHECK (((length(migration_head) >= 1) AND (length(migration_head) <= 64))),
    CONSTRAINT schema_metadata_single_row CHECK (id),
    CONSTRAINT schema_metadata_updated_check CHECK ((updated_at >= created_at))
);


--
-- Name: scim_credentials; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.scim_credentials (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    token_hash bytea NOT NULL,
    label text NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    revoked_at timestamp with time zone,
    last_used_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    created_by text NOT NULL,
    CONSTRAINT scim_credentials_created_by_check CHECK (((length(created_by) >= 1) AND (length(created_by) <= 255))),
    CONSTRAINT scim_credentials_expiry_check CHECK ((expires_at > created_at)),
    CONSTRAINT scim_credentials_hash_check CHECK ((octet_length(token_hash) = 32)),
    CONSTRAINT scim_credentials_label_check CHECK (((length(label) >= 1) AND (length(label) <= 128)))
);

ALTER TABLE ONLY public.scim_credentials FORCE ROW LEVEL SECURITY;


--
-- Name: scim_users; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.scim_users (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    external_id text,
    user_name text NOT NULL,
    active boolean DEFAULT true NOT NULL,
    display_name text,
    given_name text,
    family_name text,
    work_email text,
    identity_id uuid,
    version bigint DEFAULT 1 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    missing_since timestamp with time zone,
    missing_passes integer DEFAULT 0 NOT NULL,
    directory_source text NOT NULL,
    CONSTRAINT scim_users_directory_source_check CHECK (((btrim(directory_source) = directory_source) AND ((length(directory_source) >= 1) AND (length(directory_source) <= 64)))),
    CONSTRAINT scim_users_external_id_check CHECK (((external_id IS NULL) OR ((length(external_id) >= 1) AND (length(external_id) <= 255)))),
    CONSTRAINT scim_users_missing_pair_check CHECK (((missing_passes = 0) = (missing_since IS NULL))),
    CONSTRAINT scim_users_missing_passes_check CHECK ((missing_passes >= 0)),
    CONSTRAINT scim_users_user_name_check CHECK (((length(user_name) >= 1) AND (length(user_name) <= 255))),
    CONSTRAINT scim_users_version_check CHECK ((version > 0))
);

ALTER TABLE ONLY public.scim_users FORCE ROW LEVEL SECURITY;


--
-- Name: scope_closure; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.scope_closure (
    tenant_id uuid NOT NULL,
    ancestor_id uuid NOT NULL,
    descendant_id uuid NOT NULL,
    distance integer NOT NULL,
    CONSTRAINT scope_closure_distance_check CHECK ((distance >= 0)),
    CONSTRAINT scope_closure_self_row_check CHECK (((ancestor_id = descendant_id) = (distance = 0)))
);

ALTER TABLE ONLY public.scope_closure FORCE ROW LEVEL SECURITY;


--
-- Name: scope_grants; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.scope_grants (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    subject_kind text NOT NULL,
    principal_id text,
    group_id uuid,
    role_key text NOT NULL,
    source text NOT NULL,
    invite_id uuid,
    granted_by text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    directory_source text,
    directory_resource_id text,
    CONSTRAINT scope_grants_directory_resource_check CHECK (((directory_resource_id IS NULL) OR ((btrim(directory_resource_id) = directory_resource_id) AND ((length(directory_resource_id) >= 1) AND (length(directory_resource_id) <= 255))))),
    CONSTRAINT scope_grants_directory_shape_check CHECK ((((source = 'directory'::text) = ((directory_source IS NOT NULL) AND (directory_resource_id IS NOT NULL))) AND ((source <> 'directory'::text) OR (subject_kind = 'group'::text)))),
    CONSTRAINT scope_grants_directory_source_check CHECK (((directory_source IS NULL) OR ((btrim(directory_source) = directory_source) AND ((length(directory_source) >= 1) AND (length(directory_source) <= 64))))),
    CONSTRAINT scope_grants_granted_by_check CHECK (((granted_by IS NULL) OR ((length(granted_by) >= 1) AND (length(granted_by) <= 255)))),
    CONSTRAINT scope_grants_group_shape_check CHECK (((subject_kind = 'group'::text) = (group_id IS NOT NULL))),
    CONSTRAINT scope_grants_invite_shape_check CHECK (((source = 'invite'::text) = (invite_id IS NOT NULL))),
    CONSTRAINT scope_grants_principal_length_check CHECK (((principal_id IS NULL) OR ((btrim(principal_id) <> ''::text) AND (length(principal_id) <= 255)))),
    CONSTRAINT scope_grants_principal_shape_check CHECK (((subject_kind = 'principal'::text) = (principal_id IS NOT NULL))),
    CONSTRAINT scope_grants_role_check CHECK ((role_key = ANY (ARRAY['owner'::text, 'member'::text, 'viewer'::text, 'reviewer'::text, 'curator'::text, 'administrator'::text]))),
    CONSTRAINT scope_grants_source_check CHECK ((source = ANY (ARRAY['owner'::text, 'direct'::text, 'invite'::text, 'directory'::text, 'automation'::text]))),
    CONSTRAINT scope_grants_subject_check CHECK ((subject_kind = ANY (ARRAY['principal'::text, 'group'::text])))
);

ALTER TABLE ONLY public.scope_grants FORCE ROW LEVEL SECURITY;


--
-- Name: scopes; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.scopes (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    kind text NOT NULL,
    parent_scope_id uuid,
    parent_kind text,
    slug text NOT NULL,
    display_name text NOT NULL,
    status text DEFAULT 'active'::text NOT NULL,
    attributes jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_by uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    principal_id text,
    CONSTRAINT scopes_attributes_object_check CHECK ((jsonb_typeof(attributes) = 'object'::text)),
    CONSTRAINT scopes_attributes_size_check CHECK ((octet_length((attributes)::text) <= 16384)),
    CONSTRAINT scopes_display_name_check CHECK (((btrim(display_name) <> ''::text) AND (length(display_name) <= 200))),
    CONSTRAINT scopes_kind_check CHECK ((kind = ANY (ARRAY['tenant'::text, 'org_unit'::text, 'workspace'::text, 'project'::text, 'principal'::text]))),
    CONSTRAINT scopes_parent_kind_present_check CHECK (((parent_scope_id IS NULL) = (parent_kind IS NULL))),
    CONSTRAINT scopes_placement_check CHECK (
CASE kind
    WHEN 'tenant'::text THEN (parent_kind IS NULL)
    WHEN 'org_unit'::text THEN (parent_kind = ANY (ARRAY['tenant'::text, 'org_unit'::text]))
    WHEN 'workspace'::text THEN (parent_kind = ANY (ARRAY['tenant'::text, 'org_unit'::text]))
    WHEN 'project'::text THEN (parent_kind = 'workspace'::text)
    WHEN 'principal'::text THEN (parent_kind = ANY (ARRAY['tenant'::text, 'org_unit'::text, 'workspace'::text]))
    ELSE false
END),
    CONSTRAINT scopes_principal_id_check CHECK (((principal_id IS NULL) OR ((btrim(principal_id) <> ''::text) AND (length(principal_id) <= 255)))),
    CONSTRAINT scopes_principal_id_shape_check CHECK (((principal_id IS NULL) <> (kind = 'principal'::text))),
    CONSTRAINT scopes_root_shape_check CHECK (((parent_scope_id IS NULL) = (kind = 'tenant'::text))),
    CONSTRAINT scopes_slug_check CHECK ((slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'::text)),
    CONSTRAINT scopes_status_check CHECK ((status = ANY (ARRAY['active'::text, 'archived'::text]))),
    CONSTRAINT scopes_updated_check CHECK ((updated_at >= created_at))
);

ALTER TABLE ONLY public.scopes FORCE ROW LEVEL SECURITY;


--
-- Name: session_context_runs; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.session_context_runs (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    session_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    principal_id text NOT NULL,
    query text,
    rendered text NOT NULL,
    block_hash text NOT NULL,
    tokens integer NOT NULL,
    budget_tokens integer NOT NULL,
    entry_count integer NOT NULL,
    degraded text[] DEFAULT '{}'::text[] NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    skills jsonb DEFAULT '[]'::jsonb NOT NULL,
    workspace_id uuid,
    project_id uuid,
    query_hash text,
    requested_budget_tokens integer,
    candidate_count integer DEFAULT 0 NOT NULL,
    selection_count integer DEFAULT 0 NOT NULL,
    as_of timestamp with time zone,
    retrieval_version text,
    embedding_model text,
    index_version text,
    graph_version text,
    trace_retention_mode text,
    completion_status text,
    policy_exclusion boolean DEFAULT false NOT NULL,
    configuration_version_id uuid,
    configuration_hash text,
    checkpoint_event_id uuid,
    restart_event_ids uuid[] DEFAULT '{}'::uuid[] NOT NULL,
    CONSTRAINT context_runs_configuration_shape_check CHECK ((((configuration_version_id IS NULL) OR (configuration_hash IS NOT NULL)) AND ((configuration_hash IS NULL) OR (configuration_hash ~ '^[0-9a-f]{64}$'::text)))),
    CONSTRAINT session_context_runs_block_hash_check CHECK ((block_hash ~ '^[0-9a-f]{1,128}$'::text)),
    CONSTRAINT session_context_runs_completion_check CHECK ((completion_status = ANY (ARRAY['pending'::text, 'completed'::text, 'failed'::text]))),
    CONSTRAINT session_context_runs_embedding_model_check CHECK (((embedding_model IS NULL) OR ((btrim(embedding_model) <> ''::text) AND (char_length(embedding_model) <= 300)))),
    CONSTRAINT session_context_runs_graph_version_check CHECK (((graph_version IS NULL) OR ((btrim(graph_version) <> ''::text) AND (char_length(graph_version) <= 200)))),
    CONSTRAINT session_context_runs_index_version_check CHECK (((btrim(index_version) <> ''::text) AND (char_length(index_version) <= 200))),
    CONSTRAINT session_context_runs_planner_shape_check CHECK ((((workspace_id IS NULL) AND (as_of IS NULL) AND (retrieval_version IS NULL) AND (index_version IS NULL) AND (trace_retention_mode IS NULL) AND (completion_status IS NULL)) OR ((workspace_id IS NOT NULL) AND (as_of IS NOT NULL) AND (retrieval_version IS NOT NULL) AND (index_version IS NOT NULL) AND (trace_retention_mode IS NOT NULL) AND (completion_status IS NOT NULL)))),
    CONSTRAINT session_context_runs_principal_check CHECK (((btrim(principal_id) <> ''::text) AND (length(principal_id) <= 255))),
    CONSTRAINT session_context_runs_query_check CHECK (((query IS NULL) OR ((btrim(query) <> ''::text) AND (length(query) <= 4096)))),
    CONSTRAINT session_context_runs_query_hash_check CHECK (((as_of IS NULL) OR (((query IS NULL) = (query_hash IS NULL)) AND ((query_hash IS NULL) OR (query_hash ~ '^[0-9a-f]{64}$'::text))))),
    CONSTRAINT session_context_runs_requested_budget_check CHECK (((requested_budget_tokens IS NULL) OR (requested_budget_tokens > 0))),
    CONSTRAINT session_context_runs_restart_events_check CHECK ((cardinality(restart_event_ids) <= 16)),
    CONSTRAINT session_context_runs_retrieval_version_check CHECK (((btrim(retrieval_version) <> ''::text) AND (char_length(retrieval_version) <= 200))),
    CONSTRAINT session_context_runs_skills_array_check CHECK ((jsonb_typeof(skills) = 'array'::text)),
    CONSTRAINT session_context_runs_tokens_check CHECK (((tokens >= 0) AND (budget_tokens >= 0) AND (entry_count >= 0))),
    CONSTRAINT session_context_runs_trace_counts_check CHECK (((candidate_count >= 0) AND (selection_count >= 0))),
    CONSTRAINT session_context_runs_trace_retention_check CHECK ((trace_retention_mode = ANY (ARRAY['full'::text, 'redacted'::text, 'hashes_only'::text, 'disabled'::text])))
);

ALTER TABLE ONLY public.session_context_runs FORCE ROW LEVEL SECURITY;


--
-- Name: session_event_quarantine; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.session_event_quarantine (
    event_id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    session_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    findings jsonb NOT NULL,
    state text DEFAULT 'pending'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    reviewer_subject text,
    reviewed_at timestamp with time zone,
    review_reason text,
    CONSTRAINT session_event_quarantine_findings_array_check CHECK ((jsonb_typeof(findings) = 'array'::text)),
    CONSTRAINT session_event_quarantine_reason_check CHECK (((review_reason IS NULL) OR ((length(review_reason) >= 1) AND (length(review_reason) <= 1000)))),
    CONSTRAINT session_event_quarantine_review_check CHECK (((state = 'pending'::text) = ((reviewer_subject IS NULL) AND (reviewed_at IS NULL)))),
    CONSTRAINT session_event_quarantine_reviewer_check CHECK (((reviewer_subject IS NULL) OR ((length(reviewer_subject) >= 1) AND (length(reviewer_subject) <= 255)))),
    CONSTRAINT session_event_quarantine_state_check CHECK ((state = ANY (ARRAY['pending'::text, 'released'::text, 'rejected'::text])))
);

ALTER TABLE ONLY public.session_event_quarantine FORCE ROW LEVEL SECURITY;


--
-- Name: session_events; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.session_events (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    session_id uuid NOT NULL,
    event_type text NOT NULL,
    event_schema_version integer DEFAULT 1 NOT NULL,
    client_event_id text NOT NULL,
    sequence bigint NOT NULL,
    occurred_at timestamp with time zone NOT NULL,
    received_at timestamp with time zone DEFAULT now() NOT NULL,
    payload jsonb DEFAULT '{}'::jsonb NOT NULL,
    payload_hash text NOT NULL,
    source_payload_hash text NOT NULL,
    redactions jsonb,
    content_erased boolean DEFAULT false NOT NULL,
    CONSTRAINT session_events_client_id_check CHECK (((btrim(client_event_id) <> ''::text) AND (length(client_event_id) <= 200))),
    CONSTRAINT session_events_payload_hash_check CHECK ((payload_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT session_events_payload_object_check CHECK ((jsonb_typeof(payload) = 'object'::text)),
    CONSTRAINT session_events_payload_size_check CHECK ((octet_length((payload)::text) <= 65536)),
    CONSTRAINT session_events_redactions_array_check CHECK (((redactions IS NULL) OR (jsonb_typeof(redactions) = 'array'::text))),
    CONSTRAINT session_events_schema_version_check CHECK (((event_schema_version >= 1) AND (event_schema_version <= 1000))),
    CONSTRAINT session_events_sequence_check CHECK ((sequence >= 1)),
    CONSTRAINT session_events_source_payload_hash_check CHECK ((source_payload_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT session_events_type_check CHECK ((event_type = ANY (ARRAY['session.started'::text, 'session.ended'::text, 'message.user'::text, 'message.assistant'::text, 'tool.invoked'::text, 'tool.result'::text, 'file.read'::text, 'file.changed'::text, 'command.executed'::text, 'skill.loaded'::text, 'context.requested'::text, 'adapter.warning'::text, 'memory.asserted'::text, 'session.compaction_boundary'::text, 'session.checkpoint'::text, 'session.task_state'::text, 'task.outcome'::text, 'procedure.improvement'::text])))
);

ALTER TABLE ONLY public.session_events FORCE ROW LEVEL SECURITY;


--
-- Name: session_snapshot_headers; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.session_snapshot_headers (
    tenant_id uuid NOT NULL,
    id uuid NOT NULL,
    session_id uuid NOT NULL,
    sequence bigint NOT NULL,
    event_type text NOT NULL,
    payload_hash text NOT NULL,
    client_event_id_digest text NOT NULL,
    request_hash text,
    CONSTRAINT session_snapshot_headers_check CHECK ((((event_type = 'session.task_state'::text) AND (request_hash ~ '^[0-9a-f]{64}$'::text)) OR ((event_type = 'session.checkpoint'::text) AND (request_hash IS NULL)))),
    CONSTRAINT session_snapshot_headers_client_event_id_digest_check CHECK ((client_event_id_digest ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT session_snapshot_headers_event_type_check CHECK ((event_type = ANY (ARRAY['session.checkpoint'::text, 'session.task_state'::text]))),
    CONSTRAINT session_snapshot_headers_payload_hash_check CHECK ((payload_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT session_snapshot_headers_sequence_check CHECK ((sequence > 0))
);

ALTER TABLE ONLY public.session_snapshot_headers FORCE ROW LEVEL SECURITY;


--
-- Name: sessions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.sessions (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    project_id uuid,
    workspace_scope_id uuid NOT NULL,
    project_scope_id uuid,
    scope_id uuid NOT NULL,
    principal_id text NOT NULL,
    client_name text NOT NULL,
    client_version text,
    client_installation_id text,
    external_session_id text,
    agent_name text,
    model_name text,
    repository_id uuid,
    branch text,
    task_summary text,
    status text DEFAULT 'active'::text NOT NULL,
    started_at timestamp with time zone DEFAULT now() NOT NULL,
    ended_at timestamp with time zone,
    last_observed_at timestamp with time zone,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    end_reason text,
    CONSTRAINT sessions_agent_name_check CHECK (((agent_name IS NULL) OR ((btrim(agent_name) <> ''::text) AND (length(agent_name) <= 200)))),
    CONSTRAINT sessions_anchor_check CHECK ((scope_id = COALESCE(project_scope_id, workspace_scope_id))),
    CONSTRAINT sessions_branch_check CHECK (((branch IS NULL) OR ((btrim(branch) <> ''::text) AND (length(branch) <= 200)))),
    CONSTRAINT sessions_client_installation_check CHECK (((client_installation_id IS NULL) OR ((btrim(client_installation_id) <> ''::text) AND (length(client_installation_id) <= 200)))),
    CONSTRAINT sessions_client_name_check CHECK ((client_name ~ '^[a-z0-9][a-z0-9.-]{0,63}$'::text)),
    CONSTRAINT sessions_client_version_check CHECK (((client_version IS NULL) OR ((btrim(client_version) <> ''::text) AND (length(client_version) <= 200)))),
    CONSTRAINT sessions_end_reason_check CHECK (((end_reason IS NULL) OR ((btrim(end_reason) <> ''::text) AND (length(end_reason) <= 500)))),
    CONSTRAINT sessions_end_reason_shape_check CHECK (((end_reason IS NULL) OR (status <> 'active'::text))),
    CONSTRAINT sessions_ended_order_check CHECK (((ended_at IS NULL) OR (ended_at >= started_at))),
    CONSTRAINT sessions_ended_shape_check CHECK (((status = ANY (ARRAY['ended'::text, 'abandoned'::text, 'failed'::text])) = (ended_at IS NOT NULL))),
    CONSTRAINT sessions_external_id_check CHECK (((external_session_id IS NULL) OR ((btrim(external_session_id) <> ''::text) AND (length(external_session_id) <= 200)))),
    CONSTRAINT sessions_metadata_object_check CHECK ((jsonb_typeof(metadata) = 'object'::text)),
    CONSTRAINT sessions_metadata_size_check CHECK ((octet_length((metadata)::text) <= 8192)),
    CONSTRAINT sessions_model_name_check CHECK (((model_name IS NULL) OR ((btrim(model_name) <> ''::text) AND (length(model_name) <= 200)))),
    CONSTRAINT sessions_principal_check CHECK (((btrim(principal_id) <> ''::text) AND (length(principal_id) <= 255))),
    CONSTRAINT sessions_project_shape_check CHECK (((project_id IS NULL) = (project_scope_id IS NULL))),
    CONSTRAINT sessions_repository_project_check CHECK (((repository_id IS NULL) OR (project_id IS NOT NULL))),
    CONSTRAINT sessions_status_check CHECK ((status = ANY (ARRAY['active'::text, 'ending'::text, 'ended'::text, 'abandoned'::text, 'failed'::text]))),
    CONSTRAINT sessions_task_summary_check CHECK (((task_summary IS NULL) OR ((btrim(task_summary) <> ''::text) AND (length(task_summary) <= 2000)))),
    CONSTRAINT sessions_updated_check CHECK ((updated_at >= created_at))
);

ALTER TABLE ONLY public.sessions FORCE ROW LEVEL SECURITY;


--
-- Name: skill_bindings; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.skill_bindings (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    skill_id uuid NOT NULL,
    pinned_version_id uuid,
    enabled boolean NOT NULL,
    revision bigint DEFAULT 1 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    created_by uuid NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_by uuid NOT NULL,
    CONSTRAINT skill_bindings_revision_check CHECK ((revision > 0))
);

ALTER TABLE ONLY public.skill_bindings FORCE ROW LEVEL SECURITY;


--
-- Name: skill_changes; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.skill_changes (
    tenant_id uuid NOT NULL,
    proposal_id uuid NOT NULL,
    command_kind text NOT NULL,
    payload jsonb NOT NULL,
    payload_hash text NOT NULL,
    resulting_skill_id uuid,
    resulting_version_id uuid,
    resulting_binding_id uuid,
    resulting_binding_revision bigint,
    applied_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT skill_changes_binding_revision_check CHECK (((resulting_binding_revision IS NULL) OR (resulting_binding_revision > 0))),
    CONSTRAINT skill_changes_kind_check CHECK ((command_kind = ANY (ARRAY['install'::text, 'update'::text, 'bind'::text, 'set_binding'::text]))),
    CONSTRAINT skill_changes_payload_check CHECK (((jsonb_typeof(payload) = 'object'::text) AND (pg_column_size(payload) <= 1048576))),
    CONSTRAINT skill_changes_payload_hash_check CHECK ((length(payload_hash) = 64)),
    CONSTRAINT skill_changes_result_shape_check CHECK ((((applied_at IS NULL) AND (resulting_binding_revision IS NULL)) OR (applied_at IS NOT NULL)))
);

ALTER TABLE ONLY public.skill_changes FORCE ROW LEVEL SECURITY;


--
-- Name: skill_test_runs; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.skill_test_runs (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    version_id uuid NOT NULL,
    operation_id uuid,
    harness text NOT NULL,
    harness_version text NOT NULL,
    outcome text NOT NULL,
    scan_ruleset_version integer NOT NULL,
    rubric_version integer NOT NULL,
    evidence jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    created_by uuid NOT NULL,
    CONSTRAINT skill_test_runs_evidence_check CHECK (((jsonb_typeof(evidence) = 'object'::text) AND (pg_column_size(evidence) <= 32768))),
    CONSTRAINT skill_test_runs_harness_check CHECK ((harness = ANY (ARRAY['validation_sandbox'::text, 'controlled_client'::text]))),
    CONSTRAINT skill_test_runs_harness_version_check CHECK (((length(harness_version) >= 1) AND (length(harness_version) <= 100))),
    CONSTRAINT skill_test_runs_outcome_check CHECK ((outcome = ANY (ARRAY['passed'::text, 'failed'::text, 'error'::text]))),
    CONSTRAINT skill_test_runs_rubric_check CHECK ((rubric_version > 0)),
    CONSTRAINT skill_test_runs_ruleset_check CHECK ((scan_ruleset_version > 0))
);

ALTER TABLE ONLY public.skill_test_runs FORCE ROW LEVEL SECURITY;


--
-- Name: skill_usage_events; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.skill_usage_events (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    binding_id uuid NOT NULL,
    version_id uuid NOT NULL,
    session_id uuid,
    principal_id uuid NOT NULL,
    client_event_id text NOT NULL,
    stage text NOT NULL,
    evidence text NOT NULL,
    resource_path text,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    occurred_at timestamp with time zone NOT NULL,
    received_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT skill_usage_events_client_check CHECK (((length(client_event_id) >= 1) AND (length(client_event_id) <= 200))),
    CONSTRAINT skill_usage_events_evidence_check CHECK ((evidence = ANY (ARRAY['host_observed'::text, 'model_reported'::text]))),
    CONSTRAINT skill_usage_events_metadata_check CHECK (((jsonb_typeof(metadata) = 'object'::text) AND (pg_column_size(metadata) <= 16384))),
    CONSTRAINT skill_usage_events_resource_check CHECK (((resource_path IS NULL) OR ((length(resource_path) >= 1) AND (length(resource_path) <= 128)))),
    CONSTRAINT skill_usage_events_stage_check CHECK ((stage = ANY (ARRAY['advertised'::text, 'discovered'::text, 'activated'::text, 'instructions_loaded'::text, 'resource_loaded'::text, 'script_requested'::text, 'executed'::text, 'outcome_reported'::text])))
);

ALTER TABLE ONLY public.skill_usage_events FORCE ROW LEVEL SECURITY;


--
-- Name: skill_version_files; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.skill_version_files (
    tenant_id uuid NOT NULL,
    version_id uuid NOT NULL,
    path text NOT NULL,
    object_hash bytea NOT NULL,
    chars integer NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT skill_version_files_chars_check CHECK (((chars >= 0) AND (chars <= 65536))),
    CONSTRAINT skill_version_files_hash_check CHECK ((octet_length(object_hash) = 32)),
    CONSTRAINT skill_version_files_path_check CHECK (((length(path) >= 1) AND (length(path) <= 128)))
);

ALTER TABLE ONLY public.skill_version_files FORCE ROW LEVEL SECURITY;


--
-- Name: skill_versions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.skill_versions (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    skill_id uuid NOT NULL,
    ordinal bigint NOT NULL,
    bundle_digest bytea NOT NULL,
    sensitivity text NOT NULL,
    manifest jsonb NOT NULL,
    source_kind text NOT NULL,
    provenance jsonb NOT NULL,
    scan_report jsonb NOT NULL,
    scan_ruleset_version integer NOT NULL,
    quality_score smallint NOT NULL,
    rubric_version integer NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    created_by uuid NOT NULL,
    CONSTRAINT skill_versions_digest_check CHECK ((octet_length(bundle_digest) = 32)),
    CONSTRAINT skill_versions_manifest_check CHECK (((jsonb_typeof(manifest) = 'object'::text) AND (pg_column_size(manifest) <= 32768))),
    CONSTRAINT skill_versions_ordinal_check CHECK ((ordinal > 0)),
    CONSTRAINT skill_versions_provenance_check CHECK (((jsonb_typeof(provenance) = 'object'::text) AND (pg_column_size(provenance) <= 32768))),
    CONSTRAINT skill_versions_quality_check CHECK (((quality_score >= 0) AND (quality_score <= 100))),
    CONSTRAINT skill_versions_rubric_check CHECK ((rubric_version > 0)),
    CONSTRAINT skill_versions_scan_check CHECK (((jsonb_typeof(scan_report) = 'object'::text) AND (pg_column_size(scan_report) <= 65536))),
    CONSTRAINT skill_versions_scan_version_check CHECK ((scan_ruleset_version > 0)),
    CONSTRAINT skill_versions_sensitivity_check CHECK ((sensitivity = ANY (ARRAY['public'::text, 'internal'::text, 'confidential'::text]))),
    CONSTRAINT skill_versions_source_check CHECK ((source_kind = ANY (ARRAY['authored'::text, 'directory'::text, 'archive'::text, 'git'::text, 'registry'::text])))
);

ALTER TABLE ONLY public.skill_versions FORCE ROW LEVEL SECURITY;


--
-- Name: skills; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.skills (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    governing_scope_id uuid NOT NULL,
    name text NOT NULL,
    current_version_id uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    created_by uuid NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_by uuid NOT NULL,
    CONSTRAINT skills_name_check CHECK (((length(name) >= 1) AND (length(name) <= 64)))
);

ALTER TABLE ONLY public.skills FORCE ROW LEVEL SECURITY;


--
-- Name: source_artifact_addresses; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.source_artifact_addresses (
    tenant_id uuid NOT NULL,
    artifact_id uuid NOT NULL,
    job_id uuid NOT NULL,
    project_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    source_identity_digest text NOT NULL,
    path_digest text NOT NULL,
    content_hash text NOT NULL,
    source_key_digest text NOT NULL,
    CONSTRAINT source_artifact_addresses_content_hash_check CHECK ((content_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT source_artifact_addresses_path_digest_check CHECK ((path_digest ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT source_artifact_addresses_source_identity_digest_check CHECK ((source_identity_digest ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT source_artifact_addresses_source_key_digest_check CHECK ((source_key_digest ~ '^[0-9a-f]{64}$'::text))
);

ALTER TABLE ONLY public.source_artifact_addresses FORCE ROW LEVEL SECURITY;


--
-- Name: source_cleanup_targets; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.source_cleanup_targets (
    tenant_id uuid NOT NULL,
    operation_id uuid NOT NULL,
    event_id uuid NOT NULL,
    change_id uuid NOT NULL,
    input_hash text NOT NULL,
    phase smallint DEFAULT 0 NOT NULL,
    cursor_id uuid,
    processed_targets bigint DEFAULT 0 NOT NULL,
    CONSTRAINT source_cleanup_targets_input_hash_check CHECK ((input_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT source_cleanup_targets_phase_check CHECK (((phase >= 0) AND (phase <= 8))),
    CONSTRAINT source_cleanup_targets_processed_targets_check CHECK ((processed_targets >= 0))
);

ALTER TABLE ONLY public.source_cleanup_targets FORCE ROW LEVEL SECURITY;


--
-- Name: source_dependencies; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.source_dependencies (
    tenant_id uuid NOT NULL,
    dependent_kind text NOT NULL,
    dependent_id uuid NOT NULL,
    source_event_id uuid NOT NULL,
    CONSTRAINT source_dependencies_dependent_kind_check CHECK ((dependent_kind = ANY (ARRAY['session_event'::text, 'knowledge_revision'::text, 'knowledge_source'::text, 'capture_candidate'::text, 'knowledge_change'::text, 'context_run'::text])))
);

ALTER TABLE ONLY public.source_dependencies FORCE ROW LEVEL SECURITY;


--
-- Name: source_event_addresses; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.source_event_addresses (
    tenant_id uuid NOT NULL,
    event_id uuid NOT NULL,
    session_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    project_id uuid,
    sequence bigint NOT NULL,
    client_event_id_digest text NOT NULL,
    source_payload_hash text NOT NULL,
    payload_hash text NOT NULL,
    CONSTRAINT source_event_addresses_client_event_id_digest_check CHECK ((client_event_id_digest ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT source_event_addresses_payload_hash_check CHECK ((payload_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT source_event_addresses_sequence_check CHECK ((sequence > 0)),
    CONSTRAINT source_event_addresses_source_payload_hash_check CHECK ((source_payload_hash ~ '^[0-9a-f]{64}$'::text))
);

ALTER TABLE ONLY public.source_event_addresses FORCE ROW LEVEL SECURITY;


--
-- Name: source_observation_paths; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.source_observation_paths (
    tenant_id uuid NOT NULL,
    observation_id uuid NOT NULL,
    path_digest text NOT NULL,
    state text NOT NULL,
    blob_oid text,
    content_hash text,
    artifact_id uuid,
    CONSTRAINT source_observation_paths_check CHECK ((((state = 'missing'::text) AND (blob_oid IS NULL) AND (content_hash IS NULL) AND (artifact_id IS NULL)) OR ((state = ANY (ARRAY['present'::text, 'suppressed'::text])) AND (blob_oid IS NOT NULL) AND (content_hash IS NOT NULL) AND (blob_oid ~ '^[0-9a-f]{40}$'::text) AND (content_hash ~ '^[0-9a-f]{64}$'::text)))),
    CONSTRAINT source_observation_paths_path_digest_check CHECK ((path_digest ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT source_observation_paths_state_check CHECK ((state = ANY (ARRAY['present'::text, 'missing'::text, 'suppressed'::text])))
);

ALTER TABLE ONLY public.source_observation_paths FORCE ROW LEVEL SECURITY;


--
-- Name: source_observations; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.source_observations (
    tenant_id uuid NOT NULL,
    id uuid NOT NULL,
    project_id uuid NOT NULL,
    source_id text NOT NULL,
    operation_id uuid NOT NULL,
    previous_id uuid,
    descriptor_hash text NOT NULL,
    commit_oid text NOT NULL,
    tree_oid text NOT NULL,
    observed_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    import_job_id uuid,
    CONSTRAINT source_observations_commit_oid_check CHECK ((commit_oid ~ '^[0-9a-f]{40}$'::text)),
    CONSTRAINT source_observations_descriptor_hash_check CHECK ((descriptor_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT source_observations_tree_oid_check CHECK ((tree_oid ~ '^[0-9a-f]{40}$'::text))
);

ALTER TABLE ONLY public.source_observations FORCE ROW LEVEL SECURITY;


--
-- Name: source_reconcile_heads; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.source_reconcile_heads (
    tenant_id uuid NOT NULL,
    project_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    source_id text NOT NULL,
    repository_id uuid NOT NULL,
    repository_identity_digest text NOT NULL,
    observation_id uuid,
    active_operation_id uuid,
    last_operation_id uuid,
    CONSTRAINT source_reconcile_heads_repository_identity_digest_check CHECK ((repository_identity_digest ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT source_reconcile_heads_source_id_check CHECK ((source_id ~ '^[a-z0-9][a-z0-9-]{0,47}$'::text))
);

ALTER TABLE ONLY public.source_reconcile_heads FORCE ROW LEVEL SECURITY;


--
-- Name: source_reconcile_requests; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.source_reconcile_requests (
    tenant_id uuid NOT NULL,
    operation_id uuid NOT NULL,
    project_id uuid NOT NULL,
    source_id text NOT NULL,
    repository_id uuid NOT NULL,
    configuration_version_id uuid NOT NULL,
    descriptor_hash text NOT NULL,
    custody_digest text NOT NULL,
    credential_revision bigint,
    previous_observation_id uuid,
    path_count integer NOT NULL,
    commit_oid text,
    tree_oid text,
    CONSTRAINT source_reconcile_requests_check CHECK ((((commit_oid IS NULL) AND (tree_oid IS NULL)) OR ((commit_oid IS NOT NULL) AND (tree_oid IS NOT NULL) AND (commit_oid ~ '^[0-9a-f]{40}$'::text) AND (tree_oid ~ '^[0-9a-f]{40}$'::text)))),
    CONSTRAINT source_reconcile_requests_credential_revision_check CHECK (((credential_revision IS NULL) OR (credential_revision > 0))),
    CONSTRAINT source_reconcile_requests_custody_digest_check CHECK ((custody_digest ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT source_reconcile_requests_descriptor_hash_check CHECK ((descriptor_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT source_reconcile_requests_path_count_check CHECK (((path_count >= 1) AND (path_count <= 32)))
);

ALTER TABLE ONLY public.source_reconcile_requests FORCE ROW LEVEL SECURITY;


--
-- Name: source_repository_attachments; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.source_repository_attachments (
    tenant_id uuid NOT NULL,
    project_id uuid NOT NULL,
    repository_id uuid NOT NULL,
    github_repository_id bigint NOT NULL,
    uri_digest text NOT NULL,
    CONSTRAINT source_repository_attachments_uri_digest_check CHECK ((uri_digest ~ '^[0-9a-f]{64}$'::text))
);

ALTER TABLE ONLY public.source_repository_attachments FORCE ROW LEVEL SECURITY;


--
-- Name: source_repository_roots; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.source_repository_roots (
    tenant_id uuid NOT NULL,
    project_id uuid NOT NULL,
    github_repository_id bigint NOT NULL,
    root_digest text NOT NULL,
    CONSTRAINT source_repository_roots_github_repository_id_check CHECK ((github_repository_id > 0)),
    CONSTRAINT source_repository_roots_root_digest_check CHECK ((root_digest ~ '^[0-9a-f]{64}$'::text))
);

ALTER TABLE ONLY public.source_repository_roots FORCE ROW LEVEL SECURITY;


--
-- Name: source_suppressions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.source_suppressions (
    tenant_id uuid NOT NULL,
    event_id uuid NOT NULL,
    session_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    client_event_id_digest text NOT NULL,
    source_payload_hash text NOT NULL,
    payload_hash text NOT NULL,
    change_id uuid NOT NULL,
    forget_change_id uuid,
    actor_hash text NOT NULL,
    reason_hash text NOT NULL,
    removed_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT source_suppressions_actor_hash_check CHECK ((actor_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT source_suppressions_client_event_id_digest_check CHECK ((client_event_id_digest ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT source_suppressions_payload_hash_check CHECK ((payload_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT source_suppressions_reason_hash_check CHECK ((reason_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT source_suppressions_source_payload_hash_check CHECK ((source_payload_hash ~ '^[0-9a-f]{64}$'::text))
);

ALTER TABLE ONLY public.source_suppressions FORCE ROW LEVEL SECURITY;


--
-- Name: tenant_administrator_bootstraps; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.tenant_administrator_bootstraps (
    tenant_id uuid NOT NULL,
    grant_id uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

ALTER TABLE ONLY public.tenant_administrator_bootstraps FORCE ROW LEVEL SECURITY;


--
-- Name: tenant_keys; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.tenant_keys (
    tenant_id uuid NOT NULL,
    version integer NOT NULL,
    wrapped_dek bytea NOT NULL,
    kek_ref text NOT NULL,
    algorithm text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    retired_at timestamp with time zone,
    CONSTRAINT tenant_keys_algorithm_check CHECK ((algorithm = 'xchacha20-poly1305'::text)),
    CONSTRAINT tenant_keys_kek_ref_check CHECK (((length(kek_ref) >= 1) AND (length(kek_ref) <= 256))),
    CONSTRAINT tenant_keys_retired_check CHECK (((retired_at IS NULL) OR (retired_at >= created_at))),
    CONSTRAINT tenant_keys_version_check CHECK ((version >= 1)),
    CONSTRAINT tenant_keys_wrapped_check CHECK ((octet_length(wrapped_dek) = 82))
);

ALTER TABLE ONLY public.tenant_keys FORCE ROW LEVEL SECURITY;


--
-- Name: tenant_secret_reencryption_jobs; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.tenant_secret_reencryption_jobs (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    from_key_version integer NOT NULL,
    to_key_version integer NOT NULL,
    state text DEFAULT 'pending'::text NOT NULL,
    secrets_total bigint DEFAULT 0 NOT NULL,
    secrets_reencrypted bigint DEFAULT 0 NOT NULL,
    attempt bigint DEFAULT 0 NOT NULL,
    failure_code text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    started_at timestamp with time zone,
    completed_at timestamp with time zone,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT tenant_secret_reencrypt_jobs_counts_check CHECK (((secrets_total >= 0) AND (secrets_reencrypted >= 0) AND (secrets_reencrypted <= secrets_total) AND (attempt >= 0))),
    CONSTRAINT tenant_secret_reencrypt_jobs_failure_check CHECK (((failure_code IS NULL) OR ((failure_code ~ '^[a-z][a-z0-9_]*$'::text) AND (length(failure_code) <= 64)))),
    CONSTRAINT tenant_secret_reencrypt_jobs_shape_check CHECK ((((state = 'pending'::text) AND (started_at IS NULL) AND (completed_at IS NULL) AND (failure_code IS NULL)) OR ((state = 'running'::text) AND (started_at IS NOT NULL) AND (completed_at IS NULL) AND (failure_code IS NULL)) OR ((state = 'completed'::text) AND (started_at IS NOT NULL) AND (completed_at IS NOT NULL) AND (failure_code IS NULL) AND (secrets_reencrypted = secrets_total)) OR ((state = 'failed'::text) AND (started_at IS NOT NULL) AND (completed_at IS NOT NULL) AND (failure_code IS NOT NULL)))),
    CONSTRAINT tenant_secret_reencrypt_jobs_state_check CHECK ((state = ANY (ARRAY['pending'::text, 'running'::text, 'completed'::text, 'failed'::text]))),
    CONSTRAINT tenant_secret_reencrypt_jobs_versions_check CHECK (((from_key_version > 0) AND (to_key_version > from_key_version)))
);

ALTER TABLE ONLY public.tenant_secret_reencryption_jobs FORCE ROW LEVEL SECURITY;


--
-- Name: tenant_secrets; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.tenant_secrets (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    kind text NOT NULL,
    label text NOT NULL,
    provider text,
    state text DEFAULT 'active'::text NOT NULL,
    value_revision bigint DEFAULT 1 NOT NULL,
    key_version integer,
    sealed bytea,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    rotated_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    revoked_at timestamp with time zone,
    CONSTRAINT tenant_secrets_kind_check CHECK ((kind = ANY (ARRAY['directory'::text, 'tool_server'::text, 'model_provider'::text, 'import_export'::text, 'source_read'::text]))),
    CONSTRAINT tenant_secrets_label_check CHECK (((label ~ '^[a-z][a-z0-9]*(\.[a-z][a-z0-9_-]*)*$'::text) AND (length(label) <= 128))),
    CONSTRAINT tenant_secrets_provider_check CHECK (((provider IS NULL) OR ((provider ~ '^[a-z][a-z0-9_-]*$'::text) AND (length(provider) <= 64)))),
    CONSTRAINT tenant_secrets_revision_check CHECK ((value_revision > 0)),
    CONSTRAINT tenant_secrets_sealed_check CHECK (((sealed IS NULL) OR ((octet_length(sealed) >= 51) AND (octet_length(sealed) <= 65586)))),
    CONSTRAINT tenant_secrets_shape_check CHECK ((((state = 'active'::text) AND (sealed IS NOT NULL) AND (key_version > 0) AND (revoked_at IS NULL)) OR ((state = 'revoked'::text) AND (sealed IS NULL) AND (key_version IS NULL) AND (revoked_at IS NOT NULL)))),
    CONSTRAINT tenant_secrets_state_check CHECK ((state = ANY (ARRAY['active'::text, 'revoked'::text])))
);

ALTER TABLE ONLY public.tenant_secrets FORCE ROW LEVEL SECURITY;


--
-- Name: tenants; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.tenants (
    id uuid NOT NULL,
    slug text NOT NULL,
    name text NOT NULL,
    status text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT tenants_slug_check CHECK ((slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'::text)),
    CONSTRAINT tenants_status_check CHECK ((status = ANY (ARRAY['active'::text, 'suspended'::text])))
);


--
-- Name: tool_bindings; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.tool_bindings (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    project_id uuid NOT NULL,
    server_id uuid NOT NULL,
    version_id uuid NOT NULL,
    state text NOT NULL,
    revision bigint DEFAULT 1 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    created_by uuid NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_by uuid NOT NULL,
    CONSTRAINT tool_bindings_revision_check CHECK ((revision > 0)),
    CONSTRAINT tool_bindings_state_check CHECK ((state = ANY (ARRAY['enabled'::text, 'disabled'::text, 'removed'::text])))
);

ALTER TABLE ONLY public.tool_bindings FORCE ROW LEVEL SECURITY;


--
-- Name: tool_changes; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.tool_changes (
    tenant_id uuid NOT NULL,
    proposal_id uuid NOT NULL,
    command_kind text NOT NULL,
    payload jsonb NOT NULL,
    payload_hash text NOT NULL,
    resulting_server_id uuid,
    resulting_version_id uuid,
    resulting_binding_id uuid,
    resulting_binding_revision bigint,
    applied_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT tool_changes_binding_revision_check CHECK (((resulting_binding_revision IS NULL) OR (resulting_binding_revision > 0))),
    CONSTRAINT tool_changes_kind_check CHECK ((command_kind = ANY (ARRAY['register'::text, 'stage_version'::text, 'bind'::text, 'set_binding'::text]))),
    CONSTRAINT tool_changes_payload_check CHECK (((jsonb_typeof(payload) = 'object'::text) AND (pg_column_size(payload) <= 2097152))),
    CONSTRAINT tool_changes_payload_hash_check CHECK ((payload_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT tool_changes_result_shape_check CHECK ((((applied_at IS NULL) AND (resulting_binding_revision IS NULL)) OR (applied_at IS NOT NULL)))
);

ALTER TABLE ONLY public.tool_changes FORCE ROW LEVEL SECURITY;


--
-- Name: tool_server_versions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.tool_server_versions (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    server_id uuid NOT NULL,
    proposal_id uuid NOT NULL,
    ordinal bigint NOT NULL,
    digest bytea NOT NULL,
    protocol_version text NOT NULL,
    descriptor jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    created_by uuid NOT NULL,
    CONSTRAINT tool_server_versions_descriptor_check CHECK (((jsonb_typeof(descriptor) = 'object'::text) AND (pg_column_size(descriptor) <= 131072))),
    CONSTRAINT tool_server_versions_digest_check CHECK ((octet_length(digest) = 32)),
    CONSTRAINT tool_server_versions_ordinal_check CHECK ((ordinal > 0)),
    CONSTRAINT tool_server_versions_protocol_check CHECK ((protocol_version = '2026-07-28'::text))
);

ALTER TABLE ONLY public.tool_server_versions FORCE ROW LEVEL SECURITY;


--
-- Name: tool_servers; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.tool_servers (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    governing_scope_id uuid NOT NULL,
    name text NOT NULL,
    current_version_id uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    created_by uuid NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_by uuid NOT NULL,
    CONSTRAINT tool_servers_name_check CHECK (((btrim(name) <> ''::text) AND (length(name) <= 200)))
);

ALTER TABLE ONLY public.tool_servers FORCE ROW LEVEL SECURITY;


--
-- Name: tool_test_runs; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.tool_test_runs (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    version_id uuid NOT NULL,
    harness text NOT NULL,
    harness_version text NOT NULL,
    outcome text NOT NULL,
    methods text[] NOT NULL,
    latency_ms bigint,
    evidence jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    created_by uuid NOT NULL,
    CONSTRAINT tool_test_runs_evidence_check CHECK (((jsonb_typeof(evidence) = 'object'::text) AND (pg_column_size(evidence) <= 65536))),
    CONSTRAINT tool_test_runs_harness_check CHECK ((harness = ANY (ARRAY['trusted_local_adapter'::text, 'remote_http_adapter'::text]))),
    CONSTRAINT tool_test_runs_harness_version_check CHECK (((btrim(harness_version) <> ''::text) AND (length(harness_version) <= 200))),
    CONSTRAINT tool_test_runs_latency_check CHECK (((latency_ms IS NULL) OR (latency_ms >= 0))),
    CONSTRAINT tool_test_runs_methods_check CHECK (((cardinality(methods) >= 1) AND (cardinality(methods) <= 10))),
    CONSTRAINT tool_test_runs_outcome_check CHECK ((outcome = ANY (ARRAY['passed'::text, 'failed'::text, 'error'::text])))
);

ALTER TABLE ONLY public.tool_test_runs FORCE ROW LEVEL SECURITY;


--
-- Name: unproven_artifact_dependencies; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.unproven_artifact_dependencies (
    tenant_id uuid NOT NULL,
    dependent_kind text NOT NULL,
    dependent_id uuid NOT NULL,
    CONSTRAINT unproven_artifact_dependencies_dependent_kind_check CHECK ((dependent_kind = ANY (ARRAY['knowledge_source'::text, 'knowledge_revision'::text, 'knowledge_change'::text, 'context_run'::text])))
);

ALTER TABLE ONLY public.unproven_artifact_dependencies FORCE ROW LEVEL SECURITY;


--
-- Name: unproven_context_deliveries; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.unproven_context_deliveries (
    tenant_id uuid NOT NULL,
    context_run_id uuid NOT NULL
);

ALTER TABLE ONLY public.unproven_context_deliveries FORCE ROW LEVEL SECURITY;


--
-- Name: vedaflow_commit_parents; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.vedaflow_commit_parents (
    tenant_id uuid NOT NULL,
    commit_hash bytea NOT NULL,
    ordinal integer NOT NULL,
    parent_hash bytea NOT NULL,
    CONSTRAINT vedaflow_commit_parents_ordinal_check CHECK ((ordinal >= 0))
);

ALTER TABLE ONLY public.vedaflow_commit_parents FORCE ROW LEVEL SECURITY;


--
-- Name: vedaflow_commits; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.vedaflow_commits (
    tenant_id uuid NOT NULL,
    hash bytea NOT NULL,
    tree_hash bytea NOT NULL,
    author_id uuid NOT NULL,
    message text NOT NULL,
    committed_at timestamp with time zone NOT NULL,
    policy_snapshot_hash bytea NOT NULL,
    signature bytea,
    signer_key_id text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT vedaflow_commits_hash_check CHECK ((octet_length(hash) = 32)),
    CONSTRAINT vedaflow_commits_message_check CHECK (((length(message) >= 1) AND (length(message) <= 4096))),
    CONSTRAINT vedaflow_commits_policy_snapshot_check CHECK ((octet_length(policy_snapshot_hash) = 32)),
    CONSTRAINT vedaflow_commits_signature_pairing_check CHECK (((signature IS NULL) = (signer_key_id IS NULL))),
    CONSTRAINT vedaflow_commits_signature_size_check CHECK (((signature IS NULL) OR ((octet_length(signature) >= 1) AND (octet_length(signature) <= 1024)))),
    CONSTRAINT vedaflow_commits_signer_key_check CHECK (((signer_key_id IS NULL) OR ((length(signer_key_id) >= 1) AND (length(signer_key_id) <= 128))))
);

ALTER TABLE ONLY public.vedaflow_commits FORCE ROW LEVEL SECURITY;


--
-- Name: vedaflow_objects; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.vedaflow_objects (
    tenant_id uuid NOT NULL,
    hash bytea NOT NULL,
    kind text NOT NULL,
    content bytea NOT NULL,
    size_bytes integer NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT vedaflow_objects_hash_check CHECK ((octet_length(hash) = 32)),
    CONSTRAINT vedaflow_objects_kind_check CHECK ((kind = ANY (ARRAY['knowledge'::text, 'prompt'::text, 'skill'::text, 'tool'::text, 'context-pack'::text, 'policy'::text, 'configuration'::text]))),
    CONSTRAINT vedaflow_objects_size_check CHECK (((size_bytes = octet_length(content)) AND (size_bytes <= 8388608)))
);

ALTER TABLE ONLY public.vedaflow_objects FORCE ROW LEVEL SECURITY;


--
-- Name: vedaflow_proposal_approvals; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.vedaflow_proposal_approvals (
    tenant_id uuid NOT NULL,
    proposal_id uuid NOT NULL,
    approver_id uuid NOT NULL,
    commit_hash bytea NOT NULL,
    verdict text NOT NULL,
    roles text[] NOT NULL,
    approver_subject text NOT NULL,
    comment text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT vedaflow_proposal_approvals_comment_check CHECK (((comment IS NULL) OR ((length(comment) >= 1) AND (length(comment) <= 1000)))),
    CONSTRAINT vedaflow_proposal_approvals_roles_check CHECK ((roles <@ ARRAY['owner'::text, 'member'::text, 'viewer'::text, 'reviewer'::text, 'curator'::text, 'administrator'::text])),
    CONSTRAINT vedaflow_proposal_approvals_subject_check CHECK (((length(approver_subject) >= 1) AND (length(approver_subject) <= 255))),
    CONSTRAINT vedaflow_proposal_approvals_verdict_check CHECK ((verdict = ANY (ARRAY['approve'::text, 'reject'::text])))
);

ALTER TABLE ONLY public.vedaflow_proposal_approvals FORCE ROW LEVEL SECURITY;


--
-- Name: vedaflow_proposals; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.vedaflow_proposals (
    tenant_id uuid NOT NULL,
    id uuid NOT NULL,
    target_scope_id uuid NOT NULL,
    source_scope_id uuid NOT NULL,
    asset_kind text NOT NULL,
    target_channel text NOT NULL,
    commit_hash bytea NOT NULL,
    sensitivity text NOT NULL,
    state text DEFAULT 'open'::text NOT NULL,
    title text NOT NULL,
    proposer_id uuid NOT NULL,
    proposer_subject text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    closed_at timestamp with time zone,
    closed_by uuid,
    close_reason text,
    artifact_references jsonb NOT NULL,
    CONSTRAINT vedaflow_proposals_apply_asset_check CHECK (((target_channel <> 'apply'::text) OR (asset_kind = ANY (ARRAY['knowledge'::text, 'skill'::text, 'tool'::text, 'configuration'::text, 'policy'::text])))),
    CONSTRAINT vedaflow_proposals_artifact_references_check CHECK (public.synveda_valid_artifact_references(artifact_references)),
    CONSTRAINT vedaflow_proposals_asset_check CHECK ((asset_kind = ANY (ARRAY['knowledge'::text, 'prompt'::text, 'skill'::text, 'tool'::text, 'context-pack'::text, 'policy'::text, 'configuration'::text]))),
    CONSTRAINT vedaflow_proposals_channel_check CHECK ((target_channel = ANY (ARRAY['published'::text, 'apply'::text]))),
    CONSTRAINT vedaflow_proposals_closure_check CHECK (((state = 'open'::text) = ((closed_at IS NULL) AND (closed_by IS NULL)))),
    CONSTRAINT vedaflow_proposals_reason_check CHECK (((close_reason IS NULL) OR ((state <> 'open'::text) AND ((length(close_reason) >= 1) AND (length(close_reason) <= 1000))))),
    CONSTRAINT vedaflow_proposals_reject_reason_check CHECK (((state <> 'rejected'::text) OR (close_reason IS NOT NULL))),
    CONSTRAINT vedaflow_proposals_sensitivity_check CHECK ((sensitivity = ANY (ARRAY['public'::text, 'internal'::text, 'confidential'::text, 'restricted'::text]))),
    CONSTRAINT vedaflow_proposals_state_check CHECK ((state = ANY (ARRAY['open'::text, 'rejected'::text, 'withdrawn'::text, 'published'::text, 'applied'::text]))),
    CONSTRAINT vedaflow_proposals_subject_check CHECK (((length(proposer_subject) >= 1) AND (length(proposer_subject) <= 255))),
    CONSTRAINT vedaflow_proposals_title_check CHECK (((length(title) >= 1) AND (length(title) <= 500)))
);

ALTER TABLE ONLY public.vedaflow_proposals FORCE ROW LEVEL SECURITY;


--
-- Name: vedaflow_refs; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.vedaflow_refs (
    tenant_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    name text NOT NULL,
    commit_hash bytea NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_by uuid NOT NULL,
    CONSTRAINT vedaflow_refs_name_check CHECK (((length(name) >= 1) AND (length(name) <= 200)))
);

ALTER TABLE ONLY public.vedaflow_refs FORCE ROW LEVEL SECURITY;


--
-- Name: vedaflow_tree_entries; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.vedaflow_tree_entries (
    tenant_id uuid NOT NULL,
    tree_hash bytea NOT NULL,
    name text NOT NULL,
    object_hash bytea,
    subtree_hash bytea,
    CONSTRAINT vedaflow_tree_entries_name_check CHECK (((length(name) >= 1) AND (length(name) <= 255))),
    CONSTRAINT vedaflow_tree_entries_target_check CHECK (((object_hash IS NULL) <> (subtree_hash IS NULL)))
);

ALTER TABLE ONLY public.vedaflow_tree_entries FORCE ROW LEVEL SECURITY;


--
-- Name: vedaflow_trees; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.vedaflow_trees (
    tenant_id uuid NOT NULL,
    hash bytea NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT vedaflow_trees_hash_check CHECK ((octet_length(hash) = 32))
);

ALTER TABLE ONLY public.vedaflow_trees FORCE ROW LEVEL SECURITY;


--
-- Name: workspaces; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.workspaces (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    scope_id uuid NOT NULL,
    scope_kind text NOT NULL,
    slug text NOT NULL,
    display_name text NOT NULL,
    description text,
    status text DEFAULT 'active'::text NOT NULL,
    revision bigint DEFAULT 1 NOT NULL,
    created_by uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT workspaces_description_check CHECK (((description IS NULL) OR ((btrim(description) <> ''::text) AND (length(description) <= 2000)))),
    CONSTRAINT workspaces_display_name_check CHECK (((btrim(display_name) <> ''::text) AND (length(display_name) <= 200))),
    CONSTRAINT workspaces_revision_check CHECK ((revision >= 1)),
    CONSTRAINT workspaces_scope_kind_check CHECK ((scope_kind = 'workspace'::text)),
    CONSTRAINT workspaces_slug_check CHECK ((slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'::text)),
    CONSTRAINT workspaces_status_check CHECK ((status = ANY (ARRAY['active'::text, 'archived'::text]))),
    CONSTRAINT workspaces_updated_check CHECK ((updated_at >= created_at))
);

ALTER TABLE ONLY public.workspaces FORCE ROW LEVEL SECURITY;


--
-- Name: artifact_cleanup_targets artifact_cleanup_targets_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.artifact_cleanup_targets
    ADD CONSTRAINT artifact_cleanup_targets_pkey PRIMARY KEY (tenant_id, operation_id);


--
-- Name: artifact_dependencies artifact_dependencies_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.artifact_dependencies
    ADD CONSTRAINT artifact_dependencies_pkey PRIMARY KEY (tenant_id, dependent_kind, dependent_id, source_key_digest);


--
-- Name: artifact_suppressions artifact_suppressions_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.artifact_suppressions
    ADD CONSTRAINT artifact_suppressions_pkey PRIMARY KEY (tenant_id, source_key_digest);


--
-- Name: audit_chain_heads audit_chain_heads_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.audit_chain_heads
    ADD CONSTRAINT audit_chain_heads_pk PRIMARY KEY (tenant_id);


--
-- Name: audit_log audit_log_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.audit_log
    ADD CONSTRAINT audit_log_pk PRIMARY KEY (tenant_id, seq);


--
-- Name: capability_snapshots capability_snapshots_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capability_snapshots
    ADD CONSTRAINT capability_snapshots_id_unique UNIQUE (id);


--
-- Name: capability_snapshots capability_snapshots_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capability_snapshots
    ADD CONSTRAINT capability_snapshots_pk PRIMARY KEY (tenant_id, id);


--
-- Name: capability_snapshots capability_snapshots_version_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capability_snapshots
    ADD CONSTRAINT capability_snapshots_version_unique UNIQUE (tenant_id, version_id);


--
-- Name: capture_batch_events capture_batch_events_ordinal_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_batch_events
    ADD CONSTRAINT capture_batch_events_ordinal_unique UNIQUE (tenant_id, batch_id, ordinal);


--
-- Name: capture_batch_events capture_batch_events_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_batch_events
    ADD CONSTRAINT capture_batch_events_pk PRIMARY KEY (tenant_id, batch_id, event_id);


--
-- Name: capture_batches capture_batches_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_batches
    ADD CONSTRAINT capture_batches_pk PRIMARY KEY (id);


--
-- Name: capture_batches capture_batches_tenant_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_batches
    ADD CONSTRAINT capture_batches_tenant_id_unique UNIQUE (tenant_id, id);


--
-- Name: capture_batches capture_batches_tenant_import_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_batches
    ADD CONSTRAINT capture_batches_tenant_import_id_unique UNIQUE (tenant_id, import_job_id, id);


--
-- Name: capture_batches capture_batches_tenant_session_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_batches
    ADD CONSTRAINT capture_batches_tenant_session_id_unique UNIQUE (tenant_id, session_id, id);


--
-- Name: capture_candidate_decisions capture_candidate_decisions_candidate_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidate_decisions
    ADD CONSTRAINT capture_candidate_decisions_candidate_unique UNIQUE (tenant_id, candidate_id);


--
-- Name: capture_candidate_decisions capture_candidate_decisions_key_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidate_decisions
    ADD CONSTRAINT capture_candidate_decisions_key_unique UNIQUE (tenant_id, actor_subject, idempotency_key);


--
-- Name: capture_candidate_decisions capture_candidate_decisions_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidate_decisions
    ADD CONSTRAINT capture_candidate_decisions_pk PRIMARY KEY (id);


--
-- Name: capture_candidate_decisions capture_candidate_decisions_tenant_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidate_decisions
    ADD CONSTRAINT capture_candidate_decisions_tenant_id_unique UNIQUE (tenant_id, id);


--
-- Name: capture_candidate_events capture_candidate_events_ordinal_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidate_events
    ADD CONSTRAINT capture_candidate_events_ordinal_unique UNIQUE (tenant_id, candidate_id, ordinal);


--
-- Name: capture_candidate_events capture_candidate_events_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidate_events
    ADD CONSTRAINT capture_candidate_events_pk PRIMARY KEY (tenant_id, candidate_id, event_id);


--
-- Name: capture_candidate_import_artifacts capture_candidate_import_artifacts_ordinal_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidate_import_artifacts
    ADD CONSTRAINT capture_candidate_import_artifacts_ordinal_unique UNIQUE (tenant_id, candidate_id, ordinal);


--
-- Name: capture_candidate_import_artifacts capture_candidate_import_artifacts_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidate_import_artifacts
    ADD CONSTRAINT capture_candidate_import_artifacts_pk PRIMARY KEY (tenant_id, candidate_id, artifact_id);


--
-- Name: capture_candidate_matches capture_candidate_matches_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidate_matches
    ADD CONSTRAINT capture_candidate_matches_pk PRIMARY KEY (tenant_id, candidate_id, knowledge_item_id, match_kind);


--
-- Name: capture_candidates capture_candidates_batch_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidates
    ADD CONSTRAINT capture_candidates_batch_id_unique UNIQUE (tenant_id, batch_id, id);


--
-- Name: capture_candidates capture_candidates_ordinal_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidates
    ADD CONSTRAINT capture_candidates_ordinal_unique UNIQUE (tenant_id, batch_id, ordinal);


--
-- Name: capture_candidates capture_candidates_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidates
    ADD CONSTRAINT capture_candidates_pk PRIMARY KEY (id);


--
-- Name: capture_candidates capture_candidates_tenant_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidates
    ADD CONSTRAINT capture_candidates_tenant_id_unique UNIQUE (tenant_id, id);


--
-- Name: capture_candidates capture_candidates_tenant_import_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidates
    ADD CONSTRAINT capture_candidates_tenant_import_id_unique UNIQUE (tenant_id, import_job_id, id);


--
-- Name: cli_handoffs cli_handoffs_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.cli_handoffs
    ADD CONSTRAINT cli_handoffs_pkey PRIMARY KEY (code_hash);


--
-- Name: configuration_artifacts configuration_artifacts_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_artifacts
    ADD CONSTRAINT configuration_artifacts_id_unique UNIQUE (id);


--
-- Name: configuration_artifacts configuration_artifacts_name_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_artifacts
    ADD CONSTRAINT configuration_artifacts_name_unique UNIQUE (tenant_id, name);


--
-- Name: configuration_artifacts configuration_artifacts_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_artifacts
    ADD CONSTRAINT configuration_artifacts_pk PRIMARY KEY (tenant_id, id);


--
-- Name: configuration_bindings configuration_bindings_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_bindings
    ADD CONSTRAINT configuration_bindings_id_unique UNIQUE (id);


--
-- Name: configuration_bindings configuration_bindings_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_bindings
    ADD CONSTRAINT configuration_bindings_pk PRIMARY KEY (tenant_id, id);


--
-- Name: configuration_bindings configuration_bindings_scope_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_bindings
    ADD CONSTRAINT configuration_bindings_scope_unique UNIQUE (tenant_id, scope_id);


--
-- Name: configuration_changes configuration_changes_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_changes
    ADD CONSTRAINT configuration_changes_pk PRIMARY KEY (tenant_id, proposal_id);


--
-- Name: configuration_versions configuration_versions_artifact_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_versions
    ADD CONSTRAINT configuration_versions_artifact_id_unique UNIQUE (tenant_id, artifact_id, id);


--
-- Name: configuration_versions configuration_versions_hash_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_versions
    ADD CONSTRAINT configuration_versions_hash_unique UNIQUE (tenant_id, artifact_id, content_hash);


--
-- Name: configuration_versions configuration_versions_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_versions
    ADD CONSTRAINT configuration_versions_id_unique UNIQUE (id);


--
-- Name: configuration_versions configuration_versions_ordinal_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_versions
    ADD CONSTRAINT configuration_versions_ordinal_unique UNIQUE (tenant_id, artifact_id, ordinal);


--
-- Name: configuration_versions configuration_versions_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_versions
    ADD CONSTRAINT configuration_versions_pk PRIMARY KEY (tenant_id, id);


--
-- Name: configuration_versions configuration_versions_proposal_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_versions
    ADD CONSTRAINT configuration_versions_proposal_unique UNIQUE (tenant_id, proposal_id);


--
-- Name: console_sessions console_sessions_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.console_sessions
    ADD CONSTRAINT console_sessions_pk PRIMARY KEY (token_hash);


--
-- Name: context_candidates context_candidates_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_candidates
    ADD CONSTRAINT context_candidates_pk PRIMARY KEY (id);


--
-- Name: context_candidates context_candidates_run_ordinal_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_candidates
    ADD CONSTRAINT context_candidates_run_ordinal_unique UNIQUE (tenant_id, context_run_id, ordinal);


--
-- Name: context_candidates context_candidates_tenant_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_candidates
    ADD CONSTRAINT context_candidates_tenant_id_unique UNIQUE (tenant_id, id);


--
-- Name: context_candidates context_candidates_tenant_run_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_candidates
    ADD CONSTRAINT context_candidates_tenant_run_id_unique UNIQUE (tenant_id, context_run_id, id);


--
-- Name: context_feedback context_feedback_idempotency_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_feedback
    ADD CONSTRAINT context_feedback_idempotency_unique UNIQUE (tenant_id, context_run_id, idempotency_key);


--
-- Name: context_feedback context_feedback_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_feedback
    ADD CONSTRAINT context_feedback_pk PRIMARY KEY (id);


--
-- Name: context_feedback context_feedback_tenant_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_feedback
    ADD CONSTRAINT context_feedback_tenant_id_unique UNIQUE (tenant_id, id);


--
-- Name: context_graph_steps context_graph_steps_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_graph_steps
    ADD CONSTRAINT context_graph_steps_pk PRIMARY KEY (tenant_id, context_candidate_id, ordinal);


--
-- Name: context_pack_chunks context_pack_chunks_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_pack_chunks
    ADD CONSTRAINT context_pack_chunks_id_unique UNIQUE (id);


--
-- Name: context_pack_chunks context_pack_chunks_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_pack_chunks
    ADD CONSTRAINT context_pack_chunks_pk PRIMARY KEY (tenant_id, id);


--
-- Name: context_pack_chunks context_pack_chunks_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_pack_chunks
    ADD CONSTRAINT context_pack_chunks_unique UNIQUE (tenant_id, scope_id, document_hash, ordinal);


--
-- Name: context_pack_documents context_pack_documents_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_pack_documents
    ADD CONSTRAINT context_pack_documents_pk PRIMARY KEY (tenant_id, scope_id, pack_name, document_name);


--
-- Name: context_packs context_packs_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_packs
    ADD CONSTRAINT context_packs_pk PRIMARY KEY (tenant_id, scope_id, name);


--
-- Name: context_run_delivery_dependencies context_run_delivery_dependencies_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_run_delivery_dependencies
    ADD CONSTRAINT context_run_delivery_dependencies_pkey PRIMARY KEY (tenant_id, context_run_id);


--
-- Name: context_selections context_selections_feedback_target_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_selections
    ADD CONSTRAINT context_selections_feedback_target_unique UNIQUE (tenant_id, id, context_run_id, knowledge_revision_id);


--
-- Name: context_selections context_selections_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_selections
    ADD CONSTRAINT context_selections_pk PRIMARY KEY (id);


--
-- Name: context_selections context_selections_run_rank_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_selections
    ADD CONSTRAINT context_selections_run_rank_unique UNIQUE (tenant_id, context_run_id, rank);


--
-- Name: context_selections context_selections_tenant_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_selections
    ADD CONSTRAINT context_selections_tenant_id_unique UNIQUE (tenant_id, id);


--
-- Name: deployment_keys deployment_keys_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.deployment_keys
    ADD CONSTRAINT deployment_keys_pk PRIMARY KEY (version);


--
-- Name: directory_sync_state directory_sync_state_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.directory_sync_state
    ADD CONSTRAINT directory_sync_state_pk PRIMARY KEY (tenant_id);


--
-- Name: durable_operations durable_operations_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.durable_operations
    ADD CONSTRAINT durable_operations_id_unique UNIQUE (id);


--
-- Name: durable_operations durable_operations_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.durable_operations
    ADD CONSTRAINT durable_operations_pk PRIMARY KEY (tenant_id, id);


--
-- Name: durable_operations durable_operations_skill_target_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.durable_operations
    ADD CONSTRAINT durable_operations_skill_target_unique UNIQUE (tenant_id, id, skill_version_id);


--
-- Name: durable_operations durable_operations_version_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.durable_operations
    ADD CONSTRAINT durable_operations_version_unique UNIQUE (tenant_id, id, operation_version);


--
-- Name: group_members group_members_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.group_members
    ADD CONSTRAINT group_members_pk PRIMARY KEY (tenant_id, group_id, identity_id);


--
-- Name: groups groups_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.groups
    ADD CONSTRAINT groups_pk PRIMARY KEY (id);


--
-- Name: groups groups_slug_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.groups
    ADD CONSTRAINT groups_slug_unique UNIQUE (tenant_id, slug);


--
-- Name: groups groups_tenant_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.groups
    ADD CONSTRAINT groups_tenant_id_unique UNIQUE (tenant_id, id);


--
-- Name: historical_knowledge_dependencies historical_knowledge_dependencies_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.historical_knowledge_dependencies
    ADD CONSTRAINT historical_knowledge_dependencies_pkey PRIMARY KEY (tenant_id, dependent_kind, dependent_id, revision_id);


--
-- Name: idempotency_records idempotency_records_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.idempotency_records
    ADD CONSTRAINT idempotency_records_pk PRIMARY KEY (tenant_id, subject, operation, idempotency_key);


--
-- Name: identities identities_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.identities
    ADD CONSTRAINT identities_pk PRIMARY KEY (id);


--
-- Name: identities identities_scope_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.identities
    ADD CONSTRAINT identities_scope_unique UNIQUE (tenant_id, scope_id);


--
-- Name: identities identities_subject_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.identities
    ADD CONSTRAINT identities_subject_unique UNIQUE (tenant_id, subject);


--
-- Name: identities identities_tenant_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.identities
    ADD CONSTRAINT identities_tenant_id_unique UNIQUE (tenant_id, id);


--
-- Name: import_artifacts import_artifacts_job_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_artifacts
    ADD CONSTRAINT import_artifacts_job_id_unique UNIQUE (tenant_id, job_id, id);


--
-- Name: import_artifacts import_artifacts_ordinal_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_artifacts
    ADD CONSTRAINT import_artifacts_ordinal_unique UNIQUE (tenant_id, job_id, ordinal);


--
-- Name: import_artifacts import_artifacts_path_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_artifacts
    ADD CONSTRAINT import_artifacts_path_unique UNIQUE (tenant_id, job_id, logical_path);


--
-- Name: import_artifacts import_artifacts_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_artifacts
    ADD CONSTRAINT import_artifacts_pk PRIMARY KEY (id);


--
-- Name: import_artifacts import_artifacts_tenant_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_artifacts
    ADD CONSTRAINT import_artifacts_tenant_id_unique UNIQUE (tenant_id, id);


--
-- Name: import_jobs import_jobs_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_jobs
    ADD CONSTRAINT import_jobs_pk PRIMARY KEY (id);


--
-- Name: import_jobs import_jobs_tenant_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_jobs
    ADD CONSTRAINT import_jobs_tenant_id_unique UNIQUE (tenant_id, id);


--
-- Name: import_mappings import_mappings_artifact_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_mappings
    ADD CONSTRAINT import_mappings_artifact_unique UNIQUE (tenant_id, job_id, artifact_id);


--
-- Name: import_mappings import_mappings_job_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_mappings
    ADD CONSTRAINT import_mappings_job_id_unique UNIQUE (tenant_id, job_id, id);


--
-- Name: import_mappings import_mappings_ordinal_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_mappings
    ADD CONSTRAINT import_mappings_ordinal_unique UNIQUE (tenant_id, job_id, ordinal);


--
-- Name: import_mappings import_mappings_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_mappings
    ADD CONSTRAINT import_mappings_pk PRIMARY KEY (id);


--
-- Name: import_mappings import_mappings_tenant_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_mappings
    ADD CONSTRAINT import_mappings_tenant_id_unique UNIQUE (tenant_id, id);


--
-- Name: knowledge_change_headers knowledge_change_headers_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_change_headers
    ADD CONSTRAINT knowledge_change_headers_pkey PRIMARY KEY (tenant_id, proposal_id);


--
-- Name: knowledge_changes knowledge_changes_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_changes
    ADD CONSTRAINT knowledge_changes_pk PRIMARY KEY (tenant_id, proposal_id);


--
-- Name: knowledge_conflict_members knowledge_conflict_members_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_conflict_members
    ADD CONSTRAINT knowledge_conflict_members_id_unique UNIQUE (id);


--
-- Name: knowledge_conflict_members knowledge_conflict_members_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_conflict_members
    ADD CONSTRAINT knowledge_conflict_members_pk PRIMARY KEY (tenant_id, id);


--
-- Name: knowledge_conflict_sets knowledge_conflict_sets_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_conflict_sets
    ADD CONSTRAINT knowledge_conflict_sets_id_unique UNIQUE (id);


--
-- Name: knowledge_conflict_sets knowledge_conflict_sets_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_conflict_sets
    ADD CONSTRAINT knowledge_conflict_sets_pk PRIMARY KEY (tenant_id, id);


--
-- Name: knowledge_copy_dependencies knowledge_copy_dependencies_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_copy_dependencies
    ADD CONSTRAINT knowledge_copy_dependencies_pkey PRIMARY KEY (tenant_id, dependent_kind, dependent_id, knowledge_item_id);


--
-- Name: knowledge_erasure_tombstones knowledge_erasure_tombstones_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_erasure_tombstones
    ADD CONSTRAINT knowledge_erasure_tombstones_pk PRIMARY KEY (tenant_id, knowledge_item_id);


--
-- Name: knowledge_index_invalidations knowledge_index_invalidations_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_index_invalidations
    ADD CONSTRAINT knowledge_index_invalidations_pk PRIMARY KEY (tenant_id, operation_id, revision_id);


--
-- Name: knowledge_items_history knowledge_items_history_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_items_history
    ADD CONSTRAINT knowledge_items_history_pk PRIMARY KEY (tenant_id, id, tx_from);


--
-- Name: knowledge_items knowledge_items_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_items
    ADD CONSTRAINT knowledge_items_pk PRIMARY KEY (id);


--
-- Name: knowledge_items knowledge_items_tenant_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_items
    ADD CONSTRAINT knowledge_items_tenant_id_unique UNIQUE (tenant_id, id);


--
-- Name: knowledge_relations knowledge_relations_claim_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_relations
    ADD CONSTRAINT knowledge_relations_claim_unique UNIQUE (tenant_id, source_item_id, target_item_id, asserting_revision_id, relation_type);


--
-- Name: knowledge_relations knowledge_relations_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_relations
    ADD CONSTRAINT knowledge_relations_pk PRIMARY KEY (id);


--
-- Name: knowledge_relations knowledge_relations_tenant_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_relations
    ADD CONSTRAINT knowledge_relations_tenant_id_unique UNIQUE (tenant_id, id);


--
-- Name: knowledge_revision_embeddings knowledge_revision_embeddings_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_revision_embeddings
    ADD CONSTRAINT knowledge_revision_embeddings_pk PRIMARY KEY (tenant_id, knowledge_revision_id, model);


--
-- Name: knowledge_revision_headers knowledge_revision_headers_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_revision_headers
    ADD CONSTRAINT knowledge_revision_headers_pkey PRIMARY KEY (tenant_id, revision_id);


--
-- Name: knowledge_revision_source_headers knowledge_revision_source_headers_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_revision_source_headers
    ADD CONSTRAINT knowledge_revision_source_headers_pkey PRIMARY KEY (tenant_id, knowledge_revision_id, knowledge_source_id);


--
-- Name: knowledge_revision_sources knowledge_revision_sources_ordinal_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_revision_sources
    ADD CONSTRAINT knowledge_revision_sources_ordinal_unique UNIQUE (tenant_id, knowledge_revision_id, ordinal);


--
-- Name: knowledge_revision_sources knowledge_revision_sources_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_revision_sources
    ADD CONSTRAINT knowledge_revision_sources_pk PRIMARY KEY (tenant_id, knowledge_revision_id, knowledge_source_id);


--
-- Name: knowledge_revisions knowledge_revisions_item_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_revisions
    ADD CONSTRAINT knowledge_revisions_item_id_unique UNIQUE (tenant_id, knowledge_item_id, id);


--
-- Name: knowledge_revisions knowledge_revisions_number_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_revisions
    ADD CONSTRAINT knowledge_revisions_number_unique UNIQUE (tenant_id, knowledge_item_id, revision_number);


--
-- Name: knowledge_revisions knowledge_revisions_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_revisions
    ADD CONSTRAINT knowledge_revisions_pk PRIMARY KEY (id);


--
-- Name: knowledge_revisions knowledge_revisions_tenant_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_revisions
    ADD CONSTRAINT knowledge_revisions_tenant_id_unique UNIQUE (tenant_id, id);


--
-- Name: knowledge_sources knowledge_sources_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_sources
    ADD CONSTRAINT knowledge_sources_pk PRIMARY KEY (id);


--
-- Name: knowledge_sources knowledge_sources_tenant_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_sources
    ADD CONSTRAINT knowledge_sources_tenant_id_unique UNIQUE (tenant_id, id);


--
-- Name: operation_attempts operation_attempts_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.operation_attempts
    ADD CONSTRAINT operation_attempts_pk PRIMARY KEY (tenant_id, operation_id, attempt_number);


--
-- Name: operation_outbox operation_outbox_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.operation_outbox
    ADD CONSTRAINT operation_outbox_pk PRIMARY KEY (tenant_id, operation_id);


--
-- Name: pending_invites pending_invites_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.pending_invites
    ADD CONSTRAINT pending_invites_pk PRIMARY KEY (id);


--
-- Name: pending_invites pending_invites_tenant_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.pending_invites
    ADD CONSTRAINT pending_invites_tenant_id_unique UNIQUE (tenant_id, id);


--
-- Name: pending_logins pending_logins_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.pending_logins
    ADD CONSTRAINT pending_logins_pkey PRIMARY KEY (state_hash);


--
-- Name: policy_packs policy_packs_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_packs
    ADD CONSTRAINT policy_packs_pk PRIMARY KEY (tenant_id, name);


--
-- Name: policy_relaxation_changes policy_relaxation_changes_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxation_changes
    ADD CONSTRAINT policy_relaxation_changes_pk PRIMARY KEY (tenant_id, proposal_id);


--
-- Name: policy_relaxation_versions policy_relaxation_versions_aggregate_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxation_versions
    ADD CONSTRAINT policy_relaxation_versions_aggregate_id_unique UNIQUE (tenant_id, relaxation_id, id);


--
-- Name: policy_relaxation_versions policy_relaxation_versions_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxation_versions
    ADD CONSTRAINT policy_relaxation_versions_id_unique UNIQUE (id);


--
-- Name: policy_relaxation_versions policy_relaxation_versions_ordinal_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxation_versions
    ADD CONSTRAINT policy_relaxation_versions_ordinal_unique UNIQUE (tenant_id, relaxation_id, ordinal);


--
-- Name: policy_relaxation_versions policy_relaxation_versions_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxation_versions
    ADD CONSTRAINT policy_relaxation_versions_pk PRIMARY KEY (tenant_id, id);


--
-- Name: policy_relaxation_versions policy_relaxation_versions_proposal_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxation_versions
    ADD CONSTRAINT policy_relaxation_versions_proposal_unique UNIQUE (tenant_id, proposal_id);


--
-- Name: policy_relaxations policy_relaxations_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxations
    ADD CONSTRAINT policy_relaxations_id_unique UNIQUE (id);


--
-- Name: policy_relaxations policy_relaxations_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxations
    ADD CONSTRAINT policy_relaxations_pk PRIMARY KEY (tenant_id, id);


--
-- Name: procedure_candidate_headers procedure_candidate_headers_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.procedure_candidate_headers
    ADD CONSTRAINT procedure_candidate_headers_pkey PRIMARY KEY (tenant_id, candidate_id);


--
-- Name: procedure_candidate_headers procedure_candidate_headers_tenant_id_request_event_id_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.procedure_candidate_headers
    ADD CONSTRAINT procedure_candidate_headers_tenant_id_request_event_id_key UNIQUE (tenant_id, request_event_id);


--
-- Name: procedure_evaluation_attempts procedure_evaluation_attempts_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.procedure_evaluation_attempts
    ADD CONSTRAINT procedure_evaluation_attempts_pkey PRIMARY KEY (tenant_id, id);


--
-- Name: procedure_evaluation_attempts procedure_evaluation_attempts_tenant_id_candidate_id_candid_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.procedure_evaluation_attempts
    ADD CONSTRAINT procedure_evaluation_attempts_tenant_id_candidate_id_candid_key UNIQUE (tenant_id, candidate_id, candidate_hash, configuration_hash, specification_hash);


--
-- Name: procedure_evaluation_results procedure_evaluation_results_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.procedure_evaluation_results
    ADD CONSTRAINT procedure_evaluation_results_pkey PRIMARY KEY (tenant_id, attempt_id);


--
-- Name: procedure_improvement_headers procedure_improvement_headers_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.procedure_improvement_headers
    ADD CONSTRAINT procedure_improvement_headers_pkey PRIMARY KEY (tenant_id, event_id);


--
-- Name: procedure_improvement_headers procedure_improvement_headers_tenant_id_identity_hash_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.procedure_improvement_headers
    ADD CONSTRAINT procedure_improvement_headers_tenant_id_identity_hash_key UNIQUE (tenant_id, identity_hash);


--
-- Name: procedure_outcome_headers procedure_outcome_headers_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.procedure_outcome_headers
    ADD CONSTRAINT procedure_outcome_headers_pkey PRIMARY KEY (tenant_id, event_id);


--
-- Name: project_repositories project_repositories_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.project_repositories
    ADD CONSTRAINT project_repositories_pk PRIMARY KEY (id);


--
-- Name: projects projects_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.projects
    ADD CONSTRAINT projects_pk PRIMARY KEY (id);


--
-- Name: projects projects_scope_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.projects
    ADD CONSTRAINT projects_scope_unique UNIQUE (tenant_id, scope_id);


--
-- Name: projects projects_slug_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.projects
    ADD CONSTRAINT projects_slug_unique UNIQUE (tenant_id, workspace_id, slug);


--
-- Name: projects projects_tenant_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.projects
    ADD CONSTRAINT projects_tenant_id_unique UNIQUE (tenant_id, id);


--
-- Name: prompts prompts_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.prompts
    ADD CONSTRAINT prompts_pk PRIMARY KEY (tenant_id, scope_id, name);


--
-- Name: schema_metadata schema_metadata_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.schema_metadata
    ADD CONSTRAINT schema_metadata_pk PRIMARY KEY (id);


--
-- Name: scim_credentials scim_credentials_hash_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scim_credentials
    ADD CONSTRAINT scim_credentials_hash_unique UNIQUE (token_hash);


--
-- Name: scim_credentials scim_credentials_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scim_credentials
    ADD CONSTRAINT scim_credentials_pk PRIMARY KEY (id);


--
-- Name: scim_users scim_users_identity_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scim_users
    ADD CONSTRAINT scim_users_identity_unique UNIQUE (tenant_id, identity_id);


--
-- Name: scim_users scim_users_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scim_users
    ADD CONSTRAINT scim_users_pk PRIMARY KEY (id);


--
-- Name: scim_users scim_users_tenant_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scim_users
    ADD CONSTRAINT scim_users_tenant_id_unique UNIQUE (tenant_id, id);


--
-- Name: scope_closure scope_closure_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scope_closure
    ADD CONSTRAINT scope_closure_pk PRIMARY KEY (ancestor_id, descendant_id);


--
-- Name: scope_grants scope_grants_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scope_grants
    ADD CONSTRAINT scope_grants_pk PRIMARY KEY (id);


--
-- Name: scopes scopes_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scopes
    ADD CONSTRAINT scopes_pk PRIMARY KEY (id);


--
-- Name: scopes scopes_sibling_slug_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scopes
    ADD CONSTRAINT scopes_sibling_slug_unique UNIQUE (tenant_id, parent_scope_id, slug);


--
-- Name: scopes scopes_tenant_id_kind_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scopes
    ADD CONSTRAINT scopes_tenant_id_kind_unique UNIQUE (tenant_id, id, kind);


--
-- Name: scopes scopes_tenant_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scopes
    ADD CONSTRAINT scopes_tenant_id_unique UNIQUE (tenant_id, id);


--
-- Name: session_context_runs session_context_runs_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.session_context_runs
    ADD CONSTRAINT session_context_runs_pk PRIMARY KEY (id);


--
-- Name: session_context_runs session_context_runs_tenant_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.session_context_runs
    ADD CONSTRAINT session_context_runs_tenant_id_unique UNIQUE (tenant_id, id);


--
-- Name: session_event_quarantine session_event_quarantine_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.session_event_quarantine
    ADD CONSTRAINT session_event_quarantine_pk PRIMARY KEY (event_id);


--
-- Name: session_events session_events_client_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.session_events
    ADD CONSTRAINT session_events_client_unique UNIQUE (tenant_id, session_id, client_event_id);


--
-- Name: session_events session_events_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.session_events
    ADD CONSTRAINT session_events_pk PRIMARY KEY (id);


--
-- Name: session_events session_events_sequence_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.session_events
    ADD CONSTRAINT session_events_sequence_unique UNIQUE (tenant_id, session_id, sequence);


--
-- Name: session_snapshot_headers session_snapshot_headers_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.session_snapshot_headers
    ADD CONSTRAINT session_snapshot_headers_pkey PRIMARY KEY (tenant_id, id);


--
-- Name: session_snapshot_headers session_snapshot_headers_tenant_id_session_id_client_event__key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.session_snapshot_headers
    ADD CONSTRAINT session_snapshot_headers_tenant_id_session_id_client_event__key UNIQUE (tenant_id, session_id, client_event_id_digest);


--
-- Name: sessions sessions_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_pk PRIMARY KEY (id);


--
-- Name: sessions sessions_tenant_id_project_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_tenant_id_project_unique UNIQUE (tenant_id, id, project_id);


--
-- Name: sessions sessions_tenant_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_tenant_id_unique UNIQUE (tenant_id, id);


--
-- Name: sessions sessions_tenant_id_workspace_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_tenant_id_workspace_unique UNIQUE (tenant_id, id, workspace_id);


--
-- Name: skill_bindings skill_bindings_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_bindings
    ADD CONSTRAINT skill_bindings_id_unique UNIQUE (id);


--
-- Name: skill_bindings skill_bindings_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_bindings
    ADD CONSTRAINT skill_bindings_pk PRIMARY KEY (tenant_id, id);


--
-- Name: skill_bindings skill_bindings_target_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_bindings
    ADD CONSTRAINT skill_bindings_target_unique UNIQUE (tenant_id, scope_id, skill_id);


--
-- Name: skill_changes skill_changes_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_changes
    ADD CONSTRAINT skill_changes_pk PRIMARY KEY (tenant_id, proposal_id);


--
-- Name: skill_test_runs skill_test_runs_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_test_runs
    ADD CONSTRAINT skill_test_runs_id_unique UNIQUE (id);


--
-- Name: skill_test_runs skill_test_runs_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_test_runs
    ADD CONSTRAINT skill_test_runs_pk PRIMARY KEY (tenant_id, id);


--
-- Name: skill_usage_events skill_usage_events_client_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_usage_events
    ADD CONSTRAINT skill_usage_events_client_unique UNIQUE (tenant_id, binding_id, client_event_id);


--
-- Name: skill_usage_events skill_usage_events_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_usage_events
    ADD CONSTRAINT skill_usage_events_id_unique UNIQUE (id);


--
-- Name: skill_usage_events skill_usage_events_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_usage_events
    ADD CONSTRAINT skill_usage_events_pk PRIMARY KEY (tenant_id, id);


--
-- Name: skill_version_files skill_version_files_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_version_files
    ADD CONSTRAINT skill_version_files_pk PRIMARY KEY (tenant_id, version_id, path);


--
-- Name: skill_versions skill_versions_digest_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_versions
    ADD CONSTRAINT skill_versions_digest_unique UNIQUE (tenant_id, skill_id, bundle_digest);


--
-- Name: skill_versions skill_versions_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_versions
    ADD CONSTRAINT skill_versions_id_unique UNIQUE (id);


--
-- Name: skill_versions skill_versions_ordinal_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_versions
    ADD CONSTRAINT skill_versions_ordinal_unique UNIQUE (tenant_id, skill_id, ordinal);


--
-- Name: skill_versions skill_versions_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_versions
    ADD CONSTRAINT skill_versions_pk PRIMARY KEY (tenant_id, id);


--
-- Name: skill_versions skill_versions_skill_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_versions
    ADD CONSTRAINT skill_versions_skill_id_unique UNIQUE (tenant_id, skill_id, id);


--
-- Name: skills skills_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skills
    ADD CONSTRAINT skills_id_unique UNIQUE (id);


--
-- Name: skills skills_name_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skills
    ADD CONSTRAINT skills_name_unique UNIQUE (tenant_id, name);


--
-- Name: skills skills_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skills
    ADD CONSTRAINT skills_pk PRIMARY KEY (tenant_id, id);


--
-- Name: source_artifact_addresses source_artifact_addresses_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_artifact_addresses
    ADD CONSTRAINT source_artifact_addresses_pkey PRIMARY KEY (tenant_id, artifact_id);


--
-- Name: source_cleanup_targets source_cleanup_targets_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_cleanup_targets
    ADD CONSTRAINT source_cleanup_targets_pkey PRIMARY KEY (tenant_id, operation_id);


--
-- Name: source_dependencies source_dependencies_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_dependencies
    ADD CONSTRAINT source_dependencies_pkey PRIMARY KEY (tenant_id, dependent_kind, dependent_id, source_event_id);


--
-- Name: source_event_addresses source_event_addresses_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_event_addresses
    ADD CONSTRAINT source_event_addresses_pkey PRIMARY KEY (tenant_id, event_id);


--
-- Name: source_observation_paths source_observation_paths_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_observation_paths
    ADD CONSTRAINT source_observation_paths_pkey PRIMARY KEY (tenant_id, observation_id, path_digest);


--
-- Name: source_observations source_observations_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_observations
    ADD CONSTRAINT source_observations_pkey PRIMARY KEY (tenant_id, id);


--
-- Name: source_observations source_observations_tenant_id_operation_id_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_observations
    ADD CONSTRAINT source_observations_tenant_id_operation_id_key UNIQUE (tenant_id, operation_id);


--
-- Name: source_reconcile_heads source_reconcile_heads_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_reconcile_heads
    ADD CONSTRAINT source_reconcile_heads_pkey PRIMARY KEY (tenant_id, project_id, source_id);


--
-- Name: source_reconcile_requests source_reconcile_requests_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_reconcile_requests
    ADD CONSTRAINT source_reconcile_requests_pkey PRIMARY KEY (tenant_id, operation_id);


--
-- Name: source_repository_attachments source_repository_attachments_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_repository_attachments
    ADD CONSTRAINT source_repository_attachments_pkey PRIMARY KEY (tenant_id, project_id, repository_id);


--
-- Name: source_repository_roots source_repository_roots_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_repository_roots
    ADD CONSTRAINT source_repository_roots_pkey PRIMARY KEY (tenant_id, project_id, github_repository_id);


--
-- Name: source_repository_roots source_repository_roots_tenant_id_project_id_root_digest_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_repository_roots
    ADD CONSTRAINT source_repository_roots_tenant_id_project_id_root_digest_key UNIQUE (tenant_id, project_id, root_digest);


--
-- Name: source_suppressions source_suppressions_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_suppressions
    ADD CONSTRAINT source_suppressions_pkey PRIMARY KEY (tenant_id, event_id);


--
-- Name: tenant_administrator_bootstraps tenant_administrator_bootstraps_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tenant_administrator_bootstraps
    ADD CONSTRAINT tenant_administrator_bootstraps_pk PRIMARY KEY (tenant_id);


--
-- Name: tenant_keys tenant_keys_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tenant_keys
    ADD CONSTRAINT tenant_keys_pk PRIMARY KEY (tenant_id, version);


--
-- Name: tenant_secret_reencryption_jobs tenant_secret_reencrypt_jobs_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tenant_secret_reencryption_jobs
    ADD CONSTRAINT tenant_secret_reencrypt_jobs_id_unique UNIQUE (id);


--
-- Name: tenant_secret_reencryption_jobs tenant_secret_reencrypt_jobs_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tenant_secret_reencryption_jobs
    ADD CONSTRAINT tenant_secret_reencrypt_jobs_pk PRIMARY KEY (tenant_id, id);


--
-- Name: tenant_secret_reencryption_jobs tenant_secret_reencrypt_jobs_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tenant_secret_reencryption_jobs
    ADD CONSTRAINT tenant_secret_reencrypt_jobs_unique UNIQUE (tenant_id, from_key_version, to_key_version);


--
-- Name: tenant_secrets tenant_secrets_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tenant_secrets
    ADD CONSTRAINT tenant_secrets_id_unique UNIQUE (id);


--
-- Name: tenant_secrets tenant_secrets_label_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tenant_secrets
    ADD CONSTRAINT tenant_secrets_label_unique UNIQUE (tenant_id, kind, label);


--
-- Name: tenant_secrets tenant_secrets_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tenant_secrets
    ADD CONSTRAINT tenant_secrets_pk PRIMARY KEY (tenant_id, id);


--
-- Name: tenants tenants_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tenants
    ADD CONSTRAINT tenants_pk PRIMARY KEY (id);


--
-- Name: tenants tenants_slug_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tenants
    ADD CONSTRAINT tenants_slug_unique UNIQUE (slug);


--
-- Name: tool_bindings tool_bindings_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_bindings
    ADD CONSTRAINT tool_bindings_id_unique UNIQUE (id);


--
-- Name: tool_bindings tool_bindings_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_bindings
    ADD CONSTRAINT tool_bindings_pk PRIMARY KEY (tenant_id, id);


--
-- Name: tool_bindings tool_bindings_target_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_bindings
    ADD CONSTRAINT tool_bindings_target_unique UNIQUE (tenant_id, project_id, server_id);


--
-- Name: tool_changes tool_changes_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_changes
    ADD CONSTRAINT tool_changes_pk PRIMARY KEY (tenant_id, proposal_id);


--
-- Name: tool_server_versions tool_server_versions_digest_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_server_versions
    ADD CONSTRAINT tool_server_versions_digest_unique UNIQUE (tenant_id, server_id, digest);


--
-- Name: tool_server_versions tool_server_versions_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_server_versions
    ADD CONSTRAINT tool_server_versions_id_unique UNIQUE (id);


--
-- Name: tool_server_versions tool_server_versions_ordinal_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_server_versions
    ADD CONSTRAINT tool_server_versions_ordinal_unique UNIQUE (tenant_id, server_id, ordinal);


--
-- Name: tool_server_versions tool_server_versions_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_server_versions
    ADD CONSTRAINT tool_server_versions_pk PRIMARY KEY (tenant_id, id);


--
-- Name: tool_server_versions tool_server_versions_proposal_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_server_versions
    ADD CONSTRAINT tool_server_versions_proposal_unique UNIQUE (tenant_id, proposal_id);


--
-- Name: tool_server_versions tool_server_versions_server_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_server_versions
    ADD CONSTRAINT tool_server_versions_server_id_unique UNIQUE (tenant_id, server_id, id);


--
-- Name: tool_servers tool_servers_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_servers
    ADD CONSTRAINT tool_servers_id_unique UNIQUE (id);


--
-- Name: tool_servers tool_servers_name_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_servers
    ADD CONSTRAINT tool_servers_name_unique UNIQUE (tenant_id, name);


--
-- Name: tool_servers tool_servers_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_servers
    ADD CONSTRAINT tool_servers_pk PRIMARY KEY (tenant_id, id);


--
-- Name: tool_test_runs tool_test_runs_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_test_runs
    ADD CONSTRAINT tool_test_runs_id_unique UNIQUE (id);


--
-- Name: tool_test_runs tool_test_runs_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_test_runs
    ADD CONSTRAINT tool_test_runs_pk PRIMARY KEY (tenant_id, id);


--
-- Name: unproven_artifact_dependencies unproven_artifact_dependencies_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.unproven_artifact_dependencies
    ADD CONSTRAINT unproven_artifact_dependencies_pkey PRIMARY KEY (tenant_id, dependent_kind, dependent_id);


--
-- Name: unproven_context_deliveries unproven_context_deliveries_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.unproven_context_deliveries
    ADD CONSTRAINT unproven_context_deliveries_pkey PRIMARY KEY (tenant_id, context_run_id);


--
-- Name: vedaflow_commit_parents vedaflow_commit_parents_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_commit_parents
    ADD CONSTRAINT vedaflow_commit_parents_pk PRIMARY KEY (tenant_id, commit_hash, ordinal);


--
-- Name: vedaflow_commit_parents vedaflow_commit_parents_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_commit_parents
    ADD CONSTRAINT vedaflow_commit_parents_unique UNIQUE (tenant_id, commit_hash, parent_hash);


--
-- Name: vedaflow_commits vedaflow_commits_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_commits
    ADD CONSTRAINT vedaflow_commits_pk PRIMARY KEY (tenant_id, hash);


--
-- Name: vedaflow_objects vedaflow_objects_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_objects
    ADD CONSTRAINT vedaflow_objects_pk PRIMARY KEY (tenant_id, hash);


--
-- Name: vedaflow_proposal_approvals vedaflow_proposal_approvals_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_proposal_approvals
    ADD CONSTRAINT vedaflow_proposal_approvals_pk PRIMARY KEY (tenant_id, proposal_id, approver_id, commit_hash);


--
-- Name: vedaflow_proposals vedaflow_proposals_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_proposals
    ADD CONSTRAINT vedaflow_proposals_pk PRIMARY KEY (tenant_id, id);


--
-- Name: vedaflow_refs vedaflow_refs_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_refs
    ADD CONSTRAINT vedaflow_refs_pk PRIMARY KEY (tenant_id, scope_id, name);


--
-- Name: vedaflow_tree_entries vedaflow_tree_entries_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_tree_entries
    ADD CONSTRAINT vedaflow_tree_entries_pk PRIMARY KEY (tenant_id, tree_hash, name);


--
-- Name: vedaflow_trees vedaflow_trees_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_trees
    ADD CONSTRAINT vedaflow_trees_pk PRIMARY KEY (tenant_id, hash);


--
-- Name: workspaces workspaces_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.workspaces
    ADD CONSTRAINT workspaces_pk PRIMARY KEY (id);


--
-- Name: workspaces workspaces_scope_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.workspaces
    ADD CONSTRAINT workspaces_scope_unique UNIQUE (tenant_id, scope_id);


--
-- Name: workspaces workspaces_slug_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.workspaces
    ADD CONSTRAINT workspaces_slug_unique UNIQUE (tenant_id, slug);


--
-- Name: workspaces workspaces_tenant_id_scope_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.workspaces
    ADD CONSTRAINT workspaces_tenant_id_scope_unique UNIQUE (tenant_id, id, scope_id);


--
-- Name: artifact_address_alias; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX artifact_address_alias ON public.source_artifact_addresses USING btree (tenant_id, source_key_digest, artifact_id);


--
-- Name: artifact_address_job; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX artifact_address_job ON public.source_artifact_addresses USING btree (tenant_id, job_id, artifact_id);


--
-- Name: artifact_dependency_root; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX artifact_dependency_root ON public.artifact_dependencies USING btree (tenant_id, source_key_digest, dependent_kind, dependent_id);


--
-- Name: audit_log_disclosure_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX audit_log_disclosure_idx ON public.audit_log USING gin (tenant_id, payload jsonb_path_ops) WHERE (action = ANY (ARRAY['context.injected'::text, 'context.recalled'::text, 'session.context.composed'::text]));


--
-- Name: audit_log_tenant_action_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX audit_log_tenant_action_idx ON public.audit_log USING btree (tenant_id, action, seq);


--
-- Name: audit_log_tenant_actor_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX audit_log_tenant_actor_idx ON public.audit_log USING btree (tenant_id, actor_subject, seq);


--
-- Name: audit_log_tenant_payload_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX audit_log_tenant_payload_idx ON public.audit_log USING gin (tenant_id, payload jsonb_path_ops);


--
-- Name: audit_log_tenant_time_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX audit_log_tenant_time_idx ON public.audit_log USING btree (tenant_id, occurred_at, seq);


--
-- Name: capture_batches_by_project; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX capture_batches_by_project ON public.capture_batches USING btree (tenant_id, project_id, created_at DESC, id DESC) WHERE (project_id IS NOT NULL);


--
-- Name: capture_batches_by_scope; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX capture_batches_by_scope ON public.capture_batches USING btree (tenant_id, scope_id, created_at DESC, id DESC);


--
-- Name: capture_batches_by_session; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX capture_batches_by_session ON public.capture_batches USING btree (tenant_id, session_id, created_at DESC, id DESC);


--
-- Name: capture_batches_import_unique; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX capture_batches_import_unique ON public.capture_batches USING btree (tenant_id, import_job_id) WHERE (source_kind = 'okf_import'::text);


--
-- Name: capture_batches_pending; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX capture_batches_pending ON public.capture_batches USING btree (tenant_id, created_at, id) WHERE (state = 'pending'::text);


--
-- Name: capture_batches_session_snapshot_unique; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX capture_batches_session_snapshot_unique ON public.capture_batches USING btree (tenant_id, session_id, input_hash) WHERE (source_kind = 'session'::text);


--
-- Name: capture_candidate_matches_by_knowledge; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX capture_candidate_matches_by_knowledge ON public.capture_candidate_matches USING btree (tenant_id, knowledge_item_id, candidate_id);


--
-- Name: capture_candidates_by_batch; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX capture_candidates_by_batch ON public.capture_candidates USING btree (tenant_id, batch_id, ordinal);


--
-- Name: capture_candidates_by_hash; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX capture_candidates_by_hash ON public.capture_candidates USING btree (tenant_id, content_hash);


--
-- Name: capture_candidates_by_result; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX capture_candidates_by_result ON public.capture_candidates USING btree (tenant_id, resulting_knowledge_item_id) WHERE (resulting_knowledge_item_id IS NOT NULL);


--
-- Name: capture_candidates_by_session; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX capture_candidates_by_session ON public.capture_candidates USING btree (tenant_id, session_id, created_at DESC, id DESC);


--
-- Name: capture_candidates_pending; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX capture_candidates_pending ON public.capture_candidates USING btree (tenant_id, proposed_scope_id, created_at, id) WHERE (state = 'pending'::text);


--
-- Name: cli_handoffs_expiry; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX cli_handoffs_expiry ON public.cli_handoffs USING btree (expires_at);


--
-- Name: configuration_bindings_resolution; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX configuration_bindings_resolution ON public.configuration_bindings USING btree (tenant_id, scope_id, enabled) WHERE enabled;


--
-- Name: console_sessions_expiry; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX console_sessions_expiry ON public.console_sessions USING btree (absolute_expires_at);


--
-- Name: context_candidates_by_capture_candidate; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX context_candidates_by_capture_candidate ON public.context_candidates USING btree (tenant_id, capture_candidate_id, created_at DESC) WHERE (capture_candidate_id IS NOT NULL);


--
-- Name: context_candidates_by_revision; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX context_candidates_by_revision ON public.context_candidates USING btree (tenant_id, knowledge_revision_id, created_at DESC) WHERE (knowledge_revision_id IS NOT NULL);


--
-- Name: context_candidates_by_run; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX context_candidates_by_run ON public.context_candidates USING btree (tenant_id, context_run_id, ordinal);


--
-- Name: context_delivery_manifest; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX context_delivery_manifest ON public.context_run_delivery_dependencies USING gin (manifest jsonb_path_ops);


--
-- Name: context_feedback_by_run; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX context_feedback_by_run ON public.context_feedback USING btree (tenant_id, context_run_id, created_at, id);


--
-- Name: context_graph_steps_by_relation; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX context_graph_steps_by_relation ON public.context_graph_steps USING btree (tenant_id, relation_id, created_at) WHERE (relation_id IS NOT NULL);


--
-- Name: context_graph_steps_by_run; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX context_graph_steps_by_run ON public.context_graph_steps USING btree (tenant_id, context_run_id, context_candidate_id, ordinal);


--
-- Name: context_pack_chunks_by_document; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX context_pack_chunks_by_document ON public.context_pack_chunks USING btree (tenant_id, scope_id, document_hash, ordinal);


--
-- Name: context_selections_by_capture_candidate; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX context_selections_by_capture_candidate ON public.context_selections USING btree (tenant_id, capture_candidate_id, created_at DESC) WHERE (capture_candidate_id IS NOT NULL);


--
-- Name: context_selections_by_knowledge; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX context_selections_by_knowledge ON public.context_selections USING btree (tenant_id, knowledge_item_id, created_at DESC, id DESC) WHERE (knowledge_item_id IS NOT NULL);


--
-- Name: context_selections_by_run; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX context_selections_by_run ON public.context_selections USING btree (tenant_id, context_run_id, rank);


--
-- Name: deployment_keys_current; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX deployment_keys_current ON public.deployment_keys USING btree ((true)) WHERE (retired_at IS NULL);


--
-- Name: durable_operations_expired_leases; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX durable_operations_expired_leases ON public.durable_operations USING btree (tenant_id, kind, lease_expires_at, id) WHERE (state = 'running'::text);


--
-- Name: durable_operations_queue; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX durable_operations_queue ON public.durable_operations USING btree (tenant_id, kind, state, next_attempt_at, id) WHERE (state = ANY (ARRAY['pending'::text, 'failed'::text, 'running'::text]));


--
-- Name: group_members_by_identity; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX group_members_by_identity ON public.group_members USING btree (tenant_id, identity_id);


--
-- Name: groups_by_tenant; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX groups_by_tenant ON public.groups USING btree (tenant_id, slug);


--
-- Name: groups_directory_resource_unique; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX groups_directory_resource_unique ON public.groups USING btree (tenant_id, directory_source, directory_resource_id) WHERE ((directory_source IS NOT NULL) AND (directory_resource_id IS NOT NULL));


--
-- Name: historical_knowledge_item; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX historical_knowledge_item ON public.historical_knowledge_dependencies USING btree (tenant_id, knowledge_item_id, dependent_kind, dependent_id);


--
-- Name: idempotency_records_by_age; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX idempotency_records_by_age ON public.idempotency_records USING btree (created_at);


--
-- Name: identities_sealed_scopes; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX identities_sealed_scopes ON public.identities USING btree (tenant_id, scope_id) WHERE (status = 'departed'::text);


--
-- Name: import_jobs_by_project; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX import_jobs_by_project ON public.import_jobs USING btree (tenant_id, project_id, created_at DESC, id DESC);


--
-- Name: import_jobs_by_state; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX import_jobs_by_state ON public.import_jobs USING btree (tenant_id, state, created_at, id);


--
-- Name: import_jobs_source_digest_unique; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX import_jobs_source_digest_unique ON public.import_jobs USING btree (tenant_id, project_id, source_kind, source_locator, bundle_digest, COALESCE(repository_identity_digest, ''::text), COALESCE(managed_source_id, ''::text), COALESCE(managed_review_digest, ''::text));


--
-- Name: import_mappings_by_candidate; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX import_mappings_by_candidate ON public.import_mappings USING btree (tenant_id, candidate_id) WHERE (candidate_id IS NOT NULL);


--
-- Name: import_mappings_by_classification; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX import_mappings_by_classification ON public.import_mappings USING btree (tenant_id, job_id, classification, ordinal);


--
-- Name: import_mappings_by_match; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX import_mappings_by_match ON public.import_mappings USING btree (tenant_id, matched_item_id) WHERE (matched_item_id IS NOT NULL);


--
-- Name: knowledge_changes_by_target; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_changes_by_target ON public.knowledge_changes USING gin (target_item_ids);


--
-- Name: knowledge_conflict_members_by_candidate; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_conflict_members_by_candidate ON public.knowledge_conflict_members USING btree (tenant_id, capture_candidate_id, conflict_set_id) WHERE (capture_candidate_id IS NOT NULL);


--
-- Name: knowledge_conflict_members_by_item; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_conflict_members_by_item ON public.knowledge_conflict_members USING btree (tenant_id, knowledge_item_id, knowledge_revision_id, conflict_set_id) WHERE (knowledge_item_id IS NOT NULL);


--
-- Name: knowledge_conflict_members_one_challenger; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX knowledge_conflict_members_one_challenger ON public.knowledge_conflict_members USING btree (tenant_id, conflict_set_id) WHERE (role = 'challenger'::text);


--
-- Name: knowledge_conflict_members_unique_candidate; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX knowledge_conflict_members_unique_candidate ON public.knowledge_conflict_members USING btree (tenant_id, conflict_set_id, capture_candidate_id) WHERE (capture_candidate_id IS NOT NULL);


--
-- Name: knowledge_conflict_members_unique_knowledge; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX knowledge_conflict_members_unique_knowledge ON public.knowledge_conflict_members USING btree (tenant_id, conflict_set_id, knowledge_item_id, knowledge_revision_id) WHERE (knowledge_item_id IS NOT NULL);


--
-- Name: knowledge_conflict_sets_candidate_open; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX knowledge_conflict_sets_candidate_open ON public.knowledge_conflict_sets USING btree (tenant_id, capture_candidate_id) WHERE ((capture_candidate_id IS NOT NULL) AND (status = ANY (ARRAY['open'::text, 'pending_review'::text])));


--
-- Name: knowledge_conflict_sets_project_queue; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_conflict_sets_project_queue ON public.knowledge_conflict_sets USING btree (tenant_id, project_id, status, updated_at DESC, id DESC) WHERE (project_id IS NOT NULL);


--
-- Name: knowledge_conflict_sets_queue; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_conflict_sets_queue ON public.knowledge_conflict_sets USING btree (tenant_id, status, updated_at DESC, id DESC);


--
-- Name: knowledge_conflict_sets_scope_queue; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_conflict_sets_scope_queue ON public.knowledge_conflict_sets USING btree (tenant_id, scope_id, status, updated_at DESC, id DESC);


--
-- Name: knowledge_copy_item; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_copy_item ON public.knowledge_copy_dependencies USING btree (tenant_id, knowledge_item_id, dependent_kind, dependent_id);


--
-- Name: knowledge_items_as_known; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_items_as_known ON public.knowledge_items USING btree (tenant_id, tx_from, id);


--
-- Name: knowledge_items_by_owner; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_items_by_owner ON public.knowledge_items USING btree (tenant_id, owner_principal_id, lifecycle_state, updated_at DESC, id) WHERE (owner_principal_id IS NOT NULL);


--
-- Name: knowledge_items_by_project; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_items_by_project ON public.knowledge_items USING btree (tenant_id, project_id, lifecycle_state, updated_at DESC, id) WHERE (project_id IS NOT NULL);


--
-- Name: knowledge_items_by_scope; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_items_by_scope ON public.knowledge_items USING btree (tenant_id, scope_id, lifecycle_state, updated_at DESC, id);


--
-- Name: knowledge_items_by_type; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_items_by_type ON public.knowledge_items USING btree (tenant_id, knowledge_type, origin, lifecycle_state);


--
-- Name: knowledge_items_history_as_known; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_items_history_as_known ON public.knowledge_items_history USING btree (tenant_id, id, tx_from, tx_to);


--
-- Name: knowledge_items_history_as_known_scan; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_items_history_as_known_scan ON public.knowledge_items_history USING btree (tenant_id, tx_from, tx_to, id);


--
-- Name: knowledge_items_history_by_scope; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_items_history_by_scope ON public.knowledge_items_history USING btree (tenant_id, scope_id, tx_from, tx_to);


--
-- Name: knowledge_relations_from; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_relations_from ON public.knowledge_relations USING btree (tenant_id, source_item_id, relation_type, created_at);


--
-- Name: knowledge_relations_to; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_relations_to ON public.knowledge_relations USING btree (tenant_id, target_item_id, relation_type, created_at);


--
-- Name: knowledge_revision_embeddings_by_model; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_revision_embeddings_by_model ON public.knowledge_revision_embeddings USING btree (tenant_id, model, embedded_at, knowledge_revision_id);


--
-- Name: knowledge_revision_embeddings_hnsw_1024; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_revision_embeddings_hnsw_1024 ON public.knowledge_revision_embeddings USING hnsw (((embedding)::public.vector(1024)) public.vector_cosine_ops) WHERE (dim = 1024);


--
-- Name: knowledge_revision_embeddings_hnsw_16; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_revision_embeddings_hnsw_16 ON public.knowledge_revision_embeddings USING hnsw (((embedding)::public.vector(16)) public.vector_cosine_ops) WHERE (dim = 16);


--
-- Name: knowledge_revision_sources_by_source; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_revision_sources_by_source ON public.knowledge_revision_sources USING btree (tenant_id, knowledge_source_id, knowledge_revision_id);


--
-- Name: knowledge_revisions_by_hash; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_revisions_by_hash ON public.knowledge_revisions USING btree (tenant_id, content_hash);


--
-- Name: knowledge_revisions_by_item; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_revisions_by_item ON public.knowledge_revisions USING btree (tenant_id, knowledge_item_id, revision_number DESC);


--
-- Name: knowledge_revisions_by_valid_time; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_revisions_by_valid_time ON public.knowledge_revisions USING btree (tenant_id, valid_from, valid_to);


--
-- Name: knowledge_revisions_lexical; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_revisions_lexical ON public.knowledge_revisions USING gin (search_document);


--
-- Name: knowledge_revisions_stale_queue; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_revisions_stale_queue ON public.knowledge_revisions USING btree (tenant_id, stale_after) WHERE (stale_after IS NOT NULL);


--
-- Name: knowledge_sources_by_event; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_sources_by_event ON public.knowledge_sources USING btree (tenant_id, session_event_id) WHERE (session_event_id IS NOT NULL);


--
-- Name: knowledge_sources_by_hash; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_sources_by_hash ON public.knowledge_sources USING btree (tenant_id, content_hash) WHERE (content_hash IS NOT NULL);


--
-- Name: knowledge_sources_by_scope; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX knowledge_sources_by_scope ON public.knowledge_sources USING btree (tenant_id, scope_id, source_type, created_at DESC);


--
-- Name: operation_outbox_expired_leases; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX operation_outbox_expired_leases ON public.operation_outbox USING btree (tenant_id, lease_expires_at, operation_id) WHERE (state = 'claimed'::text);


--
-- Name: operation_outbox_queue; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX operation_outbox_queue ON public.operation_outbox USING btree (tenant_id, state, next_dispatch_at, operation_id) WHERE (state = ANY (ARRAY['pending'::text, 'claimed'::text]));


--
-- Name: operation_outbox_stale_deliveries; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX operation_outbox_stale_deliveries ON public.operation_outbox USING btree (tenant_id, dispatched_at, operation_id) WHERE (state = 'dispatched'::text);


--
-- Name: pending_invites_by_scope; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX pending_invites_by_scope ON public.pending_invites USING btree (tenant_id, scope_id, created_at);


--
-- Name: pending_invites_hash_unique; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX pending_invites_hash_unique ON public.pending_invites USING btree (tenant_id, token_hash);


--
-- Name: pending_logins_expiry; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX pending_logins_expiry ON public.pending_logins USING btree (expires_at);


--
-- Name: policy_relaxation_versions_active_subject_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX policy_relaxation_versions_active_subject_idx ON public.policy_relaxation_versions USING btree (tenant_id, subject_principal_id, effective_start_at, hard_expires_at) INCLUDE (target_scope_id, action, max_sensitivity);


--
-- Name: policy_relaxations_expiry_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX policy_relaxations_expiry_idx ON public.policy_relaxations USING btree (tenant_id, expiry_recorded_at) WHERE ((revoked_at IS NULL) AND (expiry_recorded_at IS NULL));


--
-- Name: policy_relaxations_listing_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX policy_relaxations_listing_idx ON public.policy_relaxations USING btree (tenant_id, updated_at DESC, id DESC);


--
-- Name: procedure_outcome_context; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX procedure_outcome_context ON public.procedure_outcome_headers USING btree (tenant_id, context_run_id, received_at DESC, event_id) WHERE (context_run_id IS NOT NULL);


--
-- Name: project_repositories_by_project; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX project_repositories_by_project ON public.project_repositories USING btree (tenant_id, project_id, created_at);


--
-- Name: project_repositories_tenant_project_id_unique; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX project_repositories_tenant_project_id_unique ON public.project_repositories USING btree (tenant_id, project_id, id);


--
-- Name: project_repositories_unique; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX project_repositories_unique ON public.project_repositories USING btree (tenant_id, project_id, lower(canonical_uri));


--
-- Name: projects_by_workspace; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX projects_by_workspace ON public.projects USING btree (tenant_id, workspace_id, slug);


--
-- Name: projects_tenant_id_workspace_scope_unique; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX projects_tenant_id_workspace_scope_unique ON public.projects USING btree (tenant_id, id, workspace_id, scope_id);


--
-- Name: scim_credentials_by_tenant; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX scim_credentials_by_tenant ON public.scim_credentials USING btree (tenant_id);


--
-- Name: scim_users_external_id_live; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX scim_users_external_id_live ON public.scim_users USING btree (tenant_id, directory_source, external_id) WHERE ((external_id IS NOT NULL) AND active);


--
-- Name: scim_users_missing; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX scim_users_missing ON public.scim_users USING btree (tenant_id, missing_passes) WHERE (active AND (missing_passes > 0));


--
-- Name: scim_users_user_name_live; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX scim_users_user_name_live ON public.scim_users USING btree (tenant_id, lower(user_name)) WHERE active;


--
-- Name: scope_closure_descendant_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX scope_closure_descendant_idx ON public.scope_closure USING btree (descendant_id);


--
-- Name: scope_grants_by_principal; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX scope_grants_by_principal ON public.scope_grants USING btree (tenant_id, principal_id) WHERE (principal_id IS NOT NULL);


--
-- Name: scope_grants_by_scope; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX scope_grants_by_scope ON public.scope_grants USING btree (tenant_id, scope_id);


--
-- Name: scope_grants_group_unique; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX scope_grants_group_unique ON public.scope_grants USING btree (tenant_id, scope_id, group_id, role_key) WHERE (principal_id IS NULL);


--
-- Name: scope_grants_principal_unique; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX scope_grants_principal_unique ON public.scope_grants USING btree (tenant_id, scope_id, principal_id, role_key) WHERE (group_id IS NULL);


--
-- Name: scopes_one_per_principal; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX scopes_one_per_principal ON public.scopes USING btree (tenant_id, principal_id) WHERE (principal_id IS NOT NULL);


--
-- Name: scopes_one_root_per_tenant; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX scopes_one_root_per_tenant ON public.scopes USING btree (tenant_id) WHERE (parent_scope_id IS NULL);


--
-- Name: scopes_parent_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX scopes_parent_idx ON public.scopes USING btree (tenant_id, parent_scope_id);


--
-- Name: scopes_tenant_id_kind_slug_unique; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX scopes_tenant_id_kind_slug_unique ON public.scopes USING btree (tenant_id, id, kind, slug);


--
-- Name: scopes_tenant_id_parent_unique; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX scopes_tenant_id_parent_unique ON public.scopes USING btree (tenant_id, id, parent_scope_id);


--
-- Name: session_context_runs_by_project; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX session_context_runs_by_project ON public.session_context_runs USING btree (tenant_id, project_id, created_at DESC, id DESC) WHERE (project_id IS NOT NULL);


--
-- Name: session_context_runs_by_session; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX session_context_runs_by_session ON public.session_context_runs USING btree (tenant_id, session_id, created_at);


--
-- Name: session_context_runs_by_tenant; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX session_context_runs_by_tenant ON public.session_context_runs USING btree (tenant_id, created_at DESC, id DESC);


--
-- Name: session_event_quarantine_pending_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX session_event_quarantine_pending_idx ON public.session_event_quarantine USING btree (tenant_id, created_at) WHERE (state = 'pending'::text);


--
-- Name: session_events_by_session; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX session_events_by_session ON public.session_events USING btree (tenant_id, session_id, sequence);


--
-- Name: session_events_checkpoints_latest; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX session_events_checkpoints_latest ON public.session_events USING btree (tenant_id, session_id, sequence DESC) WHERE (event_type = 'session.checkpoint'::text);


--
-- Name: session_events_tenant_id_unique; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX session_events_tenant_id_unique ON public.session_events USING btree (tenant_id, id);


--
-- Name: session_events_tenant_session_id_unique; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX session_events_tenant_session_id_unique ON public.session_events USING btree (tenant_id, session_id, id);


--
-- Name: session_snapshot_head; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX session_snapshot_head ON public.session_snapshot_headers USING btree (tenant_id, session_id, event_type, sequence DESC);


--
-- Name: sessions_by_principal; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX sessions_by_principal ON public.sessions USING btree (tenant_id, principal_id, started_at DESC);


--
-- Name: sessions_by_scope; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX sessions_by_scope ON public.sessions USING btree (tenant_id, scope_id, started_at DESC);


--
-- Name: sessions_by_workspace; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX sessions_by_workspace ON public.sessions USING btree (tenant_id, workspace_id, started_at DESC);


--
-- Name: sessions_external_unique; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX sessions_external_unique ON public.sessions USING btree (tenant_id, principal_id, client_name, external_session_id) WHERE (external_session_id IS NOT NULL);


--
-- Name: skill_bindings_available; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX skill_bindings_available ON public.skill_bindings USING btree (tenant_id, scope_id, enabled, skill_id) WHERE enabled;


--
-- Name: skill_test_runs_by_version; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX skill_test_runs_by_version ON public.skill_test_runs USING btree (tenant_id, version_id, created_at DESC, id DESC);


--
-- Name: skill_test_runs_operation_unique; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX skill_test_runs_operation_unique ON public.skill_test_runs USING btree (tenant_id, operation_id) WHERE (operation_id IS NOT NULL);


--
-- Name: skill_usage_events_by_binding; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX skill_usage_events_by_binding ON public.skill_usage_events USING btree (tenant_id, binding_id, received_at DESC, id DESC);


--
-- Name: skill_usage_events_by_version; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX skill_usage_events_by_version ON public.skill_usage_events USING btree (tenant_id, version_id, received_at DESC, id DESC);


--
-- Name: skill_version_files_order; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX skill_version_files_order ON public.skill_version_files USING btree (tenant_id, version_id, path);


--
-- Name: source_dependencies_root; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX source_dependencies_root ON public.source_dependencies USING btree (tenant_id, source_event_id, dependent_kind, dependent_id);


--
-- Name: source_event_session_sequence; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX source_event_session_sequence ON public.source_event_addresses USING btree (tenant_id, session_id, sequence);


--
-- Name: source_head_repository_attachment; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX source_head_repository_attachment ON public.source_reconcile_heads USING btree (tenant_id, project_id, repository_id);


--
-- Name: source_head_repository_root; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX source_head_repository_root ON public.source_reconcile_heads USING btree (tenant_id, project_id, repository_identity_digest);


--
-- Name: source_import_repository_attachment; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX source_import_repository_attachment ON public.import_jobs USING btree (tenant_id, project_id, repository_id) WHERE (repository_id IS NOT NULL);


--
-- Name: source_import_repository_root; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX source_import_repository_root ON public.import_jobs USING btree (tenant_id, project_id, repository_identity_digest) WHERE (repository_id IS NOT NULL);


--
-- Name: source_managed_review_lookup; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX source_managed_review_lookup ON public.import_jobs USING btree (tenant_id, project_id, repository_identity_digest, managed_source_id, bundle_digest, managed_review_digest, created_at, id) WHERE (managed_source_id IS NOT NULL);


--
-- Name: source_observation_artifact; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX source_observation_artifact ON public.source_observation_paths USING btree (tenant_id, artifact_id, observation_id) WHERE (artifact_id IS NOT NULL);


--
-- Name: source_relation_assertion; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX source_relation_assertion ON public.knowledge_relations USING btree (tenant_id, asserting_revision_id, id);


--
-- Name: source_repository_uri; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX source_repository_uri ON public.source_repository_attachments USING btree (tenant_id, project_id, uri_digest);


--
-- Name: source_suppression_alias; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX source_suppression_alias ON public.source_suppressions USING btree (tenant_id, session_id, client_event_id_digest, source_payload_hash);


--
-- Name: source_suppression_hash; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX source_suppression_hash ON public.source_suppressions USING btree (tenant_id, session_id, source_payload_hash);


--
-- Name: tenant_keys_current; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX tenant_keys_current ON public.tenant_keys USING btree (tenant_id) WHERE (retired_at IS NULL);


--
-- Name: tenant_secret_reencrypt_jobs_state; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX tenant_secret_reencrypt_jobs_state ON public.tenant_secret_reencryption_jobs USING btree (tenant_id, state, created_at, id);


--
-- Name: tenant_secrets_key_generation; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX tenant_secrets_key_generation ON public.tenant_secrets USING btree (tenant_id, key_version, id) WHERE (state = 'active'::text);


--
-- Name: tenant_secrets_scope_state; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX tenant_secrets_scope_state ON public.tenant_secrets USING btree (tenant_id, scope_id, kind, state, id);


--
-- Name: tool_bindings_active; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX tool_bindings_active ON public.tool_bindings USING btree (tenant_id, project_id, server_id) WHERE (state = 'enabled'::text);


--
-- Name: tool_test_runs_by_version; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX tool_test_runs_by_version ON public.tool_test_runs USING btree (tenant_id, version_id, created_at DESC, id DESC);


--
-- Name: vedaflow_proposals_artifact_references_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX vedaflow_proposals_artifact_references_idx ON public.vedaflow_proposals USING gin (artifact_references jsonb_path_ops);


--
-- Name: vedaflow_proposals_listing_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX vedaflow_proposals_listing_idx ON public.vedaflow_proposals USING btree (tenant_id, created_at DESC);


--
-- Name: vedaflow_proposals_open_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX vedaflow_proposals_open_idx ON public.vedaflow_proposals USING btree (tenant_id, target_scope_id, created_at) WHERE (state = 'open'::text);


--
-- Name: workspaces_by_tenant; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX workspaces_by_tenant ON public.workspaces USING btree (tenant_id, slug);


--
-- Name: workspaces_tenant_id_unique; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX workspaces_tenant_id_unique ON public.workspaces USING btree (tenant_id, id);


--
-- Name: source_artifact_addresses artifact_addresses_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER artifact_addresses_immutable BEFORE DELETE OR UPDATE OR TRUNCATE ON public.source_artifact_addresses FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();


--
-- Name: artifact_cleanup_targets artifact_cleanup_binding; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER artifact_cleanup_binding BEFORE INSERT ON public.artifact_cleanup_targets FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_control_binding();


--
-- Name: artifact_cleanup_targets artifact_cleanup_progress; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER artifact_cleanup_progress BEFORE UPDATE ON public.artifact_cleanup_targets FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_cleanup_progress();


--
-- Name: artifact_dependencies artifact_dependencies_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER artifact_dependencies_immutable BEFORE DELETE OR UPDATE OR TRUNCATE ON public.artifact_dependencies FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();


--
-- Name: artifact_suppressions artifact_suppression_applied; Type: TRIGGER; Schema: public; Owner: -
--

CREATE CONSTRAINT TRIGGER artifact_suppression_applied AFTER INSERT OR UPDATE ON public.artifact_suppressions DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_applied_binding();


--
-- Name: artifact_suppressions artifact_suppression_binding; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER artifact_suppression_binding BEFORE INSERT OR UPDATE ON public.artifact_suppressions FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_control_binding();


--
-- Name: artifact_suppressions artifact_suppressions_permanent; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER artifact_suppressions_permanent BEFORE DELETE OR TRUNCATE ON public.artifact_suppressions FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();


--
-- Name: audit_log audit_log_no_delete; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER audit_log_no_delete BEFORE DELETE ON public.audit_log FOR EACH ROW EXECUTE FUNCTION public.synveda_audit_log_immutable();


--
-- Name: audit_log audit_log_no_truncate; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER audit_log_no_truncate BEFORE TRUNCATE ON public.audit_log FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_audit_log_immutable();


--
-- Name: audit_log audit_log_no_update; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER audit_log_no_update BEFORE UPDATE ON public.audit_log FOR EACH ROW EXECUTE FUNCTION public.synveda_audit_log_immutable();


--
-- Name: capability_snapshots capability_snapshots_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER capability_snapshots_immutable BEFORE DELETE OR UPDATE ON public.capability_snapshots FOR EACH ROW EXECUTE FUNCTION public.synveda_immutable_tool_row();


--
-- Name: capture_batch_events capture_batch_events_append_only; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER capture_batch_events_append_only BEFORE DELETE OR UPDATE OR TRUNCATE ON public.capture_batch_events FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_capture_append_only();


--
-- Name: capture_batches capture_batches_source_identity; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER capture_batches_source_identity BEFORE INSERT ON public.capture_batches FOR EACH ROW EXECUTE FUNCTION public.synveda_capture_batch_source_identity();


--
-- Name: capture_batches capture_batches_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER capture_batches_transition BEFORE UPDATE ON public.capture_batches FOR EACH ROW EXECUTE FUNCTION public.synveda_capture_batch_transition();


--
-- Name: capture_candidate_decisions capture_candidate_decisions_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER capture_candidate_decisions_transition BEFORE UPDATE ON public.capture_candidate_decisions FOR EACH ROW EXECUTE FUNCTION public.synveda_capture_decision_transition();


--
-- Name: capture_candidate_events capture_candidate_events_append_only; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER capture_candidate_events_append_only BEFORE DELETE OR UPDATE OR TRUNCATE ON public.capture_candidate_events FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_capture_append_only();


--
-- Name: capture_candidate_import_artifacts capture_candidate_import_artifacts_append_only; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER capture_candidate_import_artifacts_append_only BEFORE DELETE OR UPDATE OR TRUNCATE ON public.capture_candidate_import_artifacts FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_import_append_only();


--
-- Name: capture_candidate_matches capture_candidate_matches_append_only; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER capture_candidate_matches_append_only BEFORE DELETE OR UPDATE OR TRUNCATE ON public.capture_candidate_matches FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_capture_append_only();


--
-- Name: capture_candidates capture_candidates_source_identity; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER capture_candidates_source_identity BEFORE INSERT ON public.capture_candidates FOR EACH ROW EXECUTE FUNCTION public.synveda_capture_candidate_source_identity();


--
-- Name: capture_candidates capture_candidates_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER capture_candidates_transition BEFORE UPDATE ON public.capture_candidates FOR EACH ROW EXECUTE FUNCTION public.synveda_capture_candidate_transition();


--
-- Name: capture_candidate_events capture_source_projection; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER capture_source_projection AFTER INSERT ON public.capture_candidate_events FOR EACH ROW EXECUTE FUNCTION public.synveda_source_projection();


--
-- Name: knowledge_change_headers change_header_result; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER change_header_result BEFORE UPDATE ON public.knowledge_change_headers FOR EACH ROW EXECUTE FUNCTION public.synveda_change_header_result();


--
-- Name: knowledge_change_headers change_headers_permanent; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER change_headers_permanent BEFORE DELETE OR TRUNCATE ON public.knowledge_change_headers FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();


--
-- Name: knowledge_changes change_source_projection; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER change_source_projection AFTER INSERT OR UPDATE ON public.knowledge_changes FOR EACH ROW EXECUTE FUNCTION public.synveda_source_projection();


--
-- Name: configuration_artifacts configuration_artifacts_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER configuration_artifacts_transition BEFORE UPDATE ON public.configuration_artifacts FOR EACH ROW EXECUTE FUNCTION public.synveda_configuration_aggregate_transition();


--
-- Name: configuration_bindings configuration_bindings_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER configuration_bindings_transition BEFORE UPDATE ON public.configuration_bindings FOR EACH ROW EXECUTE FUNCTION public.synveda_configuration_binding_transition();


--
-- Name: configuration_changes configuration_changes_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER configuration_changes_transition BEFORE UPDATE ON public.configuration_changes FOR EACH ROW EXECUTE FUNCTION public.synveda_configuration_change_transition();


--
-- Name: configuration_versions configuration_versions_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER configuration_versions_immutable BEFORE DELETE OR UPDATE ON public.configuration_versions FOR EACH ROW EXECUTE FUNCTION public.synveda_immutable_configuration_row();


--
-- Name: configuration_versions configuration_versions_proposal_shape; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER configuration_versions_proposal_shape BEFORE INSERT ON public.configuration_versions FOR EACH ROW EXECUTE FUNCTION public.synveda_configuration_version_matches_proposal();


--
-- Name: context_candidates context_candidates_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER context_candidates_immutable BEFORE DELETE OR UPDATE ON public.context_candidates FOR EACH ROW EXECUTE FUNCTION public.synveda_context_trace_immutable();


--
-- Name: context_run_delivery_dependencies context_delivery_append_only; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER context_delivery_append_only BEFORE DELETE OR UPDATE ON public.context_run_delivery_dependencies FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_capture_append_only();


--
-- Name: context_feedback context_feedback_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER context_feedback_immutable BEFORE DELETE OR UPDATE ON public.context_feedback FOR EACH ROW EXECUTE FUNCTION public.synveda_context_trace_immutable();


--
-- Name: context_graph_steps context_graph_steps_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER context_graph_steps_immutable BEFORE DELETE OR UPDATE ON public.context_graph_steps FOR EACH ROW EXECUTE FUNCTION public.synveda_context_trace_immutable();


--
-- Name: context_pack_chunks context_pack_chunks_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER context_pack_chunks_immutable BEFORE UPDATE ON public.context_pack_chunks FOR EACH ROW EXECUTE FUNCTION public.synveda_context_pack_chunk_immutable();


--
-- Name: context_pack_documents context_pack_documents_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER context_pack_documents_transition BEFORE UPDATE ON public.context_pack_documents FOR EACH ROW EXECUTE FUNCTION public.synveda_context_pack_document_transition();


--
-- Name: context_packs context_packs_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER context_packs_transition BEFORE UPDATE ON public.context_packs FOR EACH ROW EXECUTE FUNCTION public.synveda_context_pack_transition();


--
-- Name: context_selections context_selections_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER context_selections_immutable BEFORE DELETE OR UPDATE ON public.context_selections FOR EACH ROW EXECUTE FUNCTION public.synveda_context_trace_immutable();


--
-- Name: context_run_delivery_dependencies delivery_source_projection; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER delivery_source_projection AFTER INSERT ON public.context_run_delivery_dependencies FOR EACH ROW EXECUTE FUNCTION public.synveda_source_projection();


--
-- Name: durable_operations durable_operations_no_delete; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER durable_operations_no_delete BEFORE DELETE OR TRUNCATE ON public.durable_operations FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: durable_operations durable_operations_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER durable_operations_transition BEFORE UPDATE ON public.durable_operations FOR EACH ROW EXECUTE FUNCTION public.synveda_durable_operation_transition();


--
-- Name: groups groups_immutable_columns; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER groups_immutable_columns BEFORE UPDATE ON public.groups FOR EACH ROW EXECUTE FUNCTION public.synveda_groups_immutable_columns();


--
-- Name: historical_knowledge_dependencies historical_knowledge_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER historical_knowledge_immutable BEFORE DELETE OR UPDATE ON public.historical_knowledge_dependencies FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_capture_append_only();


--
-- Name: import_artifacts import_artifact_source_erasure; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER import_artifact_source_erasure BEFORE UPDATE ON public.import_artifacts FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_content_erasure();


--
-- Name: import_artifacts import_artifacts_append_only; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER import_artifacts_append_only BEFORE DELETE OR TRUNCATE ON public.import_artifacts FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_import_append_only();


--
-- Name: import_jobs import_jobs_project_identity; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER import_jobs_project_identity BEFORE INSERT ON public.import_jobs FOR EACH ROW EXECUTE FUNCTION public.synveda_import_job_project_identity();


--
-- Name: import_jobs import_jobs_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER import_jobs_transition BEFORE UPDATE ON public.import_jobs FOR EACH ROW EXECUTE FUNCTION public.synveda_import_job_transition();


--
-- Name: import_mappings import_mappings_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER import_mappings_transition BEFORE UPDATE ON public.import_mappings FOR EACH ROW EXECUTE FUNCTION public.synveda_import_mapping_transition();


--
-- Name: import_jobs import_repository_identity; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER import_repository_identity BEFORE INSERT ON public.import_jobs FOR EACH ROW EXECUTE FUNCTION public.synveda_import_repository_identity();


--
-- Name: knowledge_changes knowledge_changes_match_proposal; Type: TRIGGER; Schema: public; Owner: -
--

CREATE CONSTRAINT TRIGGER knowledge_changes_match_proposal AFTER INSERT ON public.knowledge_changes DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.synveda_knowledge_change_matches_proposal();


--
-- Name: knowledge_changes knowledge_changes_no_delete; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_changes_no_delete BEFORE DELETE OR TRUNCATE ON public.knowledge_changes FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: knowledge_changes knowledge_changes_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_changes_transition BEFORE UPDATE ON public.knowledge_changes FOR EACH ROW EXECUTE FUNCTION public.synveda_knowledge_change_transition();


--
-- Name: knowledge_conflict_members knowledge_conflict_member_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_conflict_member_immutable BEFORE DELETE OR UPDATE OR TRUNCATE ON public.knowledge_conflict_members FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_conflict_member_immutable();


--
-- Name: knowledge_conflict_sets knowledge_conflict_set_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_conflict_set_transition BEFORE DELETE OR UPDATE ON public.knowledge_conflict_sets FOR EACH ROW EXECUTE FUNCTION public.synveda_knowledge_conflict_set_transition();


--
-- Name: knowledge_copy_dependencies knowledge_copy_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_copy_immutable BEFORE DELETE OR UPDATE ON public.knowledge_copy_dependencies FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_capture_append_only();


--
-- Name: knowledge_erasure_tombstones knowledge_erasure_tombstones_append_only; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_erasure_tombstones_append_only BEFORE DELETE OR UPDATE OR TRUNCATE ON public.knowledge_erasure_tombstones FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: knowledge_index_invalidations knowledge_index_invalidations_no_delete; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_index_invalidations_no_delete BEFORE DELETE OR TRUNCATE ON public.knowledge_index_invalidations FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: knowledge_items knowledge_items_archive; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_items_archive BEFORE DELETE OR UPDATE ON public.knowledge_items FOR EACH ROW EXECUTE FUNCTION public.synveda_knowledge_items_archive();


--
-- Name: knowledge_items_history knowledge_items_history_append_only; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_items_history_append_only BEFORE DELETE OR UPDATE OR TRUNCATE ON public.knowledge_items_history FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();


--
-- Name: knowledge_items knowledge_items_scrub_capture; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_items_scrub_capture BEFORE DELETE ON public.knowledge_items FOR EACH ROW EXECUTE FUNCTION public.synveda_capture_scrub_for_knowledge();


--
-- Name: knowledge_relations knowledge_relations_append_only; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_relations_append_only BEFORE DELETE OR TRUNCATE ON public.knowledge_relations FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();


--
-- Name: knowledge_relations knowledge_relations_source_erasure; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_relations_source_erasure BEFORE UPDATE ON public.knowledge_relations FOR EACH ROW EXECUTE FUNCTION public.synveda_source_content_erasure();


--
-- Name: knowledge_revisions knowledge_revision_delivery_erasure; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_revision_delivery_erasure AFTER DELETE ON public.knowledge_revisions FOR EACH ROW EXECUTE FUNCTION public.synveda_context_delivery_knowledge_erasure();


--
-- Name: knowledge_revision_sources knowledge_revision_sources_append_only; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_revision_sources_append_only BEFORE DELETE OR UPDATE OR TRUNCATE ON public.knowledge_revision_sources FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();


--
-- Name: knowledge_revisions knowledge_revisions_append_only; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_revisions_append_only BEFORE DELETE OR TRUNCATE ON public.knowledge_revisions FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();


--
-- Name: knowledge_revisions knowledge_revisions_require_source; Type: TRIGGER; Schema: public; Owner: -
--

CREATE CONSTRAINT TRIGGER knowledge_revisions_require_source AFTER INSERT ON public.knowledge_revisions DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.synveda_knowledge_revision_has_source();


--
-- Name: knowledge_revisions knowledge_revisions_source_erasure; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_revisions_source_erasure BEFORE UPDATE ON public.knowledge_revisions FOR EACH ROW EXECUTE FUNCTION public.synveda_source_content_erasure();


--
-- Name: knowledge_sources knowledge_sources_append_only; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_sources_append_only BEFORE DELETE OR TRUNCATE ON public.knowledge_sources FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();


--
-- Name: knowledge_sources knowledge_sources_event_scope; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_sources_event_scope BEFORE INSERT ON public.knowledge_sources FOR EACH ROW EXECUTE FUNCTION public.synveda_knowledge_source_event_scope();


--
-- Name: knowledge_sources knowledge_sources_source_erasure; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_sources_source_erasure BEFORE UPDATE ON public.knowledge_sources FOR EACH ROW EXECUTE FUNCTION public.synveda_source_content_erasure();


--
-- Name: import_jobs managed_import_complete; Type: TRIGGER; Schema: public; Owner: -
--

CREATE CONSTRAINT TRIGGER managed_import_complete AFTER INSERT ON public.import_jobs DEFERRABLE INITIALLY DEFERRED FOR EACH ROW WHEN ((new.managed_source_id IS NOT NULL)) EXECUTE FUNCTION public.synveda_managed_import_complete();


--
-- Name: operation_attempts operation_attempts_no_delete; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER operation_attempts_no_delete BEFORE DELETE OR TRUNCATE ON public.operation_attempts FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: operation_attempts operation_attempts_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER operation_attempts_transition BEFORE UPDATE ON public.operation_attempts FOR EACH ROW EXECUTE FUNCTION public.synveda_operation_attempt_transition();


--
-- Name: operation_outbox operation_outbox_no_delete; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER operation_outbox_no_delete BEFORE DELETE OR TRUNCATE ON public.operation_outbox FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: operation_outbox operation_outbox_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER operation_outbox_transition BEFORE UPDATE ON public.operation_outbox FOR EACH ROW EXECUTE FUNCTION public.synveda_operation_outbox_transition();


--
-- Name: pending_invites pending_invites_terminal; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER pending_invites_terminal BEFORE UPDATE ON public.pending_invites FOR EACH ROW EXECUTE FUNCTION public.synveda_invites_are_terminal();


--
-- Name: policy_relaxation_changes policy_relaxation_changes_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER policy_relaxation_changes_transition BEFORE UPDATE ON public.policy_relaxation_changes FOR EACH ROW EXECUTE FUNCTION public.synveda_policy_relaxation_change_transition();


--
-- Name: policy_relaxation_versions policy_relaxation_versions_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER policy_relaxation_versions_immutable BEFORE DELETE OR UPDATE ON public.policy_relaxation_versions FOR EACH ROW EXECUTE FUNCTION public.synveda_immutable_relaxation_version();


--
-- Name: policy_relaxation_versions policy_relaxation_versions_proposal_shape; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER policy_relaxation_versions_proposal_shape BEFORE INSERT ON public.policy_relaxation_versions FOR EACH ROW EXECUTE FUNCTION public.synveda_policy_relaxation_version_matches_proposal();


--
-- Name: policy_relaxations policy_relaxations_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER policy_relaxations_transition BEFORE UPDATE ON public.policy_relaxations FOR EACH ROW EXECUTE FUNCTION public.synveda_policy_relaxation_aggregate_transition();


--
-- Name: procedure_evaluation_attempts procedure_attempt_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER procedure_attempt_immutable BEFORE DELETE OR UPDATE ON public.procedure_evaluation_attempts FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_capture_append_only();


--
-- Name: procedure_candidate_headers procedure_candidate_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER procedure_candidate_immutable BEFORE DELETE OR UPDATE ON public.procedure_candidate_headers FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_capture_append_only();


--
-- Name: procedure_evaluation_attempts procedure_evaluation_guard; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER procedure_evaluation_guard BEFORE INSERT ON public.procedure_evaluation_attempts FOR EACH ROW EXECUTE FUNCTION public.synveda_procedure_evaluation_guard();


--
-- Name: procedure_evaluation_results procedure_evaluation_guard; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER procedure_evaluation_guard BEFORE INSERT ON public.procedure_evaluation_results FOR EACH ROW EXECUTE FUNCTION public.synveda_procedure_evaluation_guard();


--
-- Name: procedure_improvement_headers procedure_improvement_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER procedure_improvement_immutable BEFORE DELETE OR UPDATE ON public.procedure_improvement_headers FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_capture_append_only();


--
-- Name: procedure_outcome_headers procedure_outcome_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER procedure_outcome_immutable BEFORE DELETE OR UPDATE ON public.procedure_outcome_headers FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_capture_append_only();


--
-- Name: procedure_evaluation_results procedure_result_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER procedure_result_immutable BEFORE DELETE OR UPDATE ON public.procedure_evaluation_results FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_capture_append_only();


--
-- Name: project_repositories project_repositories_immutable_columns; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER project_repositories_immutable_columns BEFORE UPDATE ON public.project_repositories FOR EACH ROW EXECUTE FUNCTION public.synveda_repository_immutable_columns();


--
-- Name: projects projects_immutable_columns; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER projects_immutable_columns BEFORE UPDATE ON public.projects FOR EACH ROW EXECUTE FUNCTION public.synveda_subtype_immutable_columns();


--
-- Name: projects projects_immutable_workspace; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER projects_immutable_workspace BEFORE UPDATE ON public.projects FOR EACH ROW EXECUTE FUNCTION public.synveda_projects_immutable_workspace();


--
-- Name: prompts prompts_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER prompts_transition BEFORE UPDATE ON public.prompts FOR EACH ROW EXECUTE FUNCTION public.synveda_prompt_transition();


--
-- Name: knowledge_revision_headers revision_headers_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER revision_headers_immutable BEFORE DELETE OR UPDATE OR TRUNCATE ON public.knowledge_revision_headers FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();


--
-- Name: knowledge_revisions revision_source_header; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER revision_source_header AFTER INSERT ON public.knowledge_revisions FOR EACH ROW EXECUTE FUNCTION public.synveda_source_projection();


--
-- Name: knowledge_revision_source_headers revision_source_headers_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER revision_source_headers_immutable BEFORE DELETE OR UPDATE OR TRUNCATE ON public.knowledge_revision_source_headers FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();


--
-- Name: knowledge_revision_sources revision_source_projection; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER revision_source_projection AFTER INSERT ON public.knowledge_revision_sources FOR EACH ROW EXECUTE FUNCTION public.synveda_source_projection();


--
-- Name: scope_grants scope_grants_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER scope_grants_immutable BEFORE UPDATE ON public.scope_grants FOR EACH ROW EXECUTE FUNCTION public.synveda_grants_are_immutable();


--
-- Name: scope_grants scope_grants_record_initial_administrator; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER scope_grants_record_initial_administrator AFTER INSERT ON public.scope_grants FOR EACH ROW EXECUTE FUNCTION public.synveda_record_initial_administrator();


--
-- Name: scopes scopes_immutable_columns; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER scopes_immutable_columns BEFORE UPDATE ON public.scopes FOR EACH ROW EXECUTE FUNCTION public.synveda_scopes_immutable_columns();


--
-- Name: session_context_runs session_context_runs_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER session_context_runs_immutable BEFORE DELETE OR UPDATE ON public.session_context_runs FOR EACH ROW EXECUTE FUNCTION public.synveda_context_trace_immutable();


--
-- Name: session_context_runs session_context_runs_restart_sources; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER session_context_runs_restart_sources BEFORE INSERT ON public.session_context_runs FOR EACH ROW EXECUTE FUNCTION public.synveda_context_run_restart_sources();


--
-- Name: session_event_quarantine session_event_quarantine_guarded_delete; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER session_event_quarantine_guarded_delete BEFORE DELETE ON public.session_event_quarantine FOR EACH ROW EXECUTE FUNCTION public.synveda_session_event_quarantine_immutable();


--
-- Name: session_event_quarantine session_event_quarantine_no_truncate; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER session_event_quarantine_no_truncate BEFORE TRUNCATE ON public.session_event_quarantine FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_session_event_quarantine_immutable();


--
-- Name: session_event_quarantine session_event_quarantine_review_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER session_event_quarantine_review_transition BEFORE UPDATE ON public.session_event_quarantine FOR EACH ROW EXECUTE FUNCTION public.synveda_session_event_quarantine_transition();


--
-- Name: session_events session_events_guarded_delete; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER session_events_guarded_delete BEFORE DELETE ON public.session_events FOR EACH ROW EXECUTE FUNCTION public.synveda_session_events_immutable();


--
-- Name: session_events session_events_no_truncate; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER session_events_no_truncate BEFORE TRUNCATE ON public.session_events FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_session_events_immutable();


--
-- Name: session_events session_events_source_erasure; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER session_events_source_erasure BEFORE UPDATE ON public.session_events FOR EACH ROW EXECUTE FUNCTION public.synveda_source_content_erasure();


--
-- Name: session_events session_source_projection; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER session_source_projection AFTER INSERT ON public.session_events FOR EACH ROW EXECUTE FUNCTION public.synveda_source_projection();


--
-- Name: sessions sessions_lifecycle; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER sessions_lifecycle BEFORE UPDATE ON public.sessions FOR EACH ROW EXECUTE FUNCTION public.synveda_sessions_lifecycle();


--
-- Name: skill_bindings skill_bindings_shape; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER skill_bindings_shape BEFORE INSERT OR UPDATE ON public.skill_bindings FOR EACH ROW EXECUTE FUNCTION public.synveda_skill_binding_shape();


--
-- Name: skill_bindings skill_bindings_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER skill_bindings_transition BEFORE UPDATE ON public.skill_bindings FOR EACH ROW EXECUTE FUNCTION public.synveda_skill_binding_transition();


--
-- Name: skill_changes skill_changes_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER skill_changes_transition BEFORE UPDATE ON public.skill_changes FOR EACH ROW EXECUTE FUNCTION public.synveda_skill_change_transition();


--
-- Name: skill_test_runs skill_test_runs_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER skill_test_runs_immutable BEFORE DELETE OR UPDATE ON public.skill_test_runs FOR EACH ROW EXECUTE FUNCTION public.synveda_immutable_skill_row();


--
-- Name: skill_usage_events skill_usage_events_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER skill_usage_events_immutable BEFORE DELETE OR UPDATE ON public.skill_usage_events FOR EACH ROW EXECUTE FUNCTION public.synveda_immutable_skill_row();


--
-- Name: skill_version_files skill_version_files_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER skill_version_files_immutable BEFORE DELETE OR UPDATE ON public.skill_version_files FOR EACH ROW EXECUTE FUNCTION public.synveda_immutable_skill_row();


--
-- Name: skill_versions skill_versions_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER skill_versions_immutable BEFORE DELETE OR UPDATE ON public.skill_versions FOR EACH ROW EXECUTE FUNCTION public.synveda_immutable_skill_row();


--
-- Name: skills skills_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER skills_transition BEFORE UPDATE ON public.skills FOR EACH ROW EXECUTE FUNCTION public.synveda_skill_aggregate_transition();


--
-- Name: session_snapshot_headers snapshot_header_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER snapshot_header_immutable BEFORE DELETE OR UPDATE ON public.session_snapshot_headers FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_capture_append_only();


--
-- Name: source_event_addresses source_addresses_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER source_addresses_immutable BEFORE DELETE OR UPDATE OR TRUNCATE ON public.source_event_addresses FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();


--
-- Name: source_cleanup_targets source_cleanup_binding; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER source_cleanup_binding BEFORE INSERT ON public.source_cleanup_targets FOR EACH ROW EXECUTE FUNCTION public.synveda_source_control_binding();


--
-- Name: source_cleanup_targets source_cleanup_progress; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER source_cleanup_progress BEFORE UPDATE ON public.source_cleanup_targets FOR EACH ROW EXECUTE FUNCTION public.synveda_source_cleanup_progress();


--
-- Name: session_context_runs source_delivery_proof; Type: TRIGGER; Schema: public; Owner: -
--

CREATE CONSTRAINT TRIGGER source_delivery_proof AFTER INSERT ON public.session_context_runs DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.synveda_source_delivery_proof();


--
-- Name: source_dependencies source_dependencies_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER source_dependencies_immutable BEFORE DELETE OR UPDATE OR TRUNCATE ON public.source_dependencies FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();


--
-- Name: knowledge_sources source_descriptor_projection; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER source_descriptor_projection AFTER INSERT ON public.knowledge_sources FOR EACH ROW EXECUTE FUNCTION public.synveda_source_projection();


--
-- Name: import_jobs source_managed_import_guard; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER source_managed_import_guard BEFORE INSERT OR UPDATE ON public.import_jobs FOR EACH ROW EXECUTE FUNCTION public.synveda_source_reconcile_guard();


--
-- Name: source_observations source_observation_complete; Type: TRIGGER; Schema: public; Owner: -
--

CREATE CONSTRAINT TRIGGER source_observation_complete AFTER INSERT ON public.source_observations DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.synveda_source_observation_complete();


--
-- Name: source_observations source_observation_guard; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER source_observation_guard BEFORE INSERT OR DELETE OR UPDATE ON public.source_observations FOR EACH ROW EXECUTE FUNCTION public.synveda_source_reconcile_guard();


--
-- Name: source_observation_paths source_observation_path_guard; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER source_observation_path_guard BEFORE INSERT OR DELETE OR UPDATE ON public.source_observation_paths FOR EACH ROW EXECUTE FUNCTION public.synveda_source_reconcile_guard();


--
-- Name: source_reconcile_heads source_reconcile_head_guard; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER source_reconcile_head_guard BEFORE INSERT OR DELETE OR UPDATE ON public.source_reconcile_heads FOR EACH ROW EXECUTE FUNCTION public.synveda_source_reconcile_guard();


--
-- Name: source_reconcile_requests source_reconcile_request_guard; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER source_reconcile_request_guard BEFORE INSERT OR DELETE OR UPDATE ON public.source_reconcile_requests FOR EACH ROW EXECUTE FUNCTION public.synveda_source_reconcile_guard();


--
-- Name: source_repository_attachments source_repository_attachment_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER source_repository_attachment_immutable BEFORE DELETE OR UPDATE ON public.source_repository_attachments FOR EACH ROW EXECUTE FUNCTION public.synveda_source_repository_immutable();


--
-- Name: source_repository_roots source_repository_root_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER source_repository_root_immutable BEFORE DELETE OR UPDATE ON public.source_repository_roots FOR EACH ROW EXECUTE FUNCTION public.synveda_source_repository_immutable();


--
-- Name: source_suppressions source_suppression_applied; Type: TRIGGER; Schema: public; Owner: -
--

CREATE CONSTRAINT TRIGGER source_suppression_applied AFTER INSERT OR UPDATE ON public.source_suppressions DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.synveda_source_applied_binding();


--
-- Name: source_suppressions source_suppression_binding; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER source_suppression_binding BEFORE INSERT OR UPDATE ON public.source_suppressions FOR EACH ROW EXECUTE FUNCTION public.synveda_source_control_binding();


--
-- Name: source_suppressions source_suppression_permanent; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER source_suppression_permanent BEFORE DELETE OR TRUNCATE ON public.source_suppressions FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();


--
-- Name: tenant_secrets tenant_secret_transition_guard; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER tenant_secret_transition_guard BEFORE UPDATE ON public.tenant_secrets FOR EACH ROW EXECUTE FUNCTION public.synveda_tenant_secret_transition_guard();


--
-- Name: tool_bindings tool_binding_version_approved; Type: TRIGGER; Schema: public; Owner: -
--

CREATE CONSTRAINT TRIGGER tool_binding_version_approved AFTER INSERT OR UPDATE ON public.tool_bindings DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.synveda_tool_binding_version_is_approved();


--
-- Name: tool_bindings tool_bindings_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER tool_bindings_transition BEFORE UPDATE ON public.tool_bindings FOR EACH ROW EXECUTE FUNCTION public.synveda_tool_binding_transition();


--
-- Name: tool_changes tool_changes_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER tool_changes_transition BEFORE UPDATE ON public.tool_changes FOR EACH ROW EXECUTE FUNCTION public.synveda_tool_change_transition();


--
-- Name: tool_servers tool_server_current_approved; Type: TRIGGER; Schema: public; Owner: -
--

CREATE CONSTRAINT TRIGGER tool_server_current_approved AFTER INSERT OR UPDATE ON public.tool_servers DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.synveda_tool_server_current_is_approved();


--
-- Name: tool_server_versions tool_server_versions_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER tool_server_versions_immutable BEFORE DELETE OR UPDATE ON public.tool_server_versions FOR EACH ROW EXECUTE FUNCTION public.synveda_immutable_tool_row();


--
-- Name: tool_servers tool_servers_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER tool_servers_transition BEFORE UPDATE ON public.tool_servers FOR EACH ROW EXECUTE FUNCTION public.synveda_tool_server_transition();


--
-- Name: tool_test_runs tool_test_runs_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER tool_test_runs_immutable BEFORE DELETE OR UPDATE ON public.tool_test_runs FOR EACH ROW EXECUTE FUNCTION public.synveda_immutable_tool_row();


--
-- Name: tool_server_versions tool_versions_match_proposal; Type: TRIGGER; Schema: public; Owner: -
--

CREATE CONSTRAINT TRIGGER tool_versions_match_proposal AFTER INSERT ON public.tool_server_versions DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.synveda_tool_version_matches_proposal();


--
-- Name: vedaflow_commit_parents vedaflow_commit_parents_no_delete; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_commit_parents_no_delete BEFORE DELETE ON public.vedaflow_commit_parents FOR EACH ROW EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: vedaflow_commit_parents vedaflow_commit_parents_no_truncate; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_commit_parents_no_truncate BEFORE TRUNCATE ON public.vedaflow_commit_parents FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: vedaflow_commit_parents vedaflow_commit_parents_no_update; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_commit_parents_no_update BEFORE UPDATE ON public.vedaflow_commit_parents FOR EACH ROW EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: vedaflow_commits vedaflow_commits_no_delete; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_commits_no_delete BEFORE DELETE ON public.vedaflow_commits FOR EACH ROW EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: vedaflow_commits vedaflow_commits_no_truncate; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_commits_no_truncate BEFORE TRUNCATE ON public.vedaflow_commits FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: vedaflow_commits vedaflow_commits_no_update; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_commits_no_update BEFORE UPDATE ON public.vedaflow_commits FOR EACH ROW EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: vedaflow_objects vedaflow_objects_no_delete; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_objects_no_delete BEFORE DELETE ON public.vedaflow_objects FOR EACH ROW EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: vedaflow_objects vedaflow_objects_no_truncate; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_objects_no_truncate BEFORE TRUNCATE ON public.vedaflow_objects FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: vedaflow_objects vedaflow_objects_no_update; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_objects_no_update BEFORE UPDATE ON public.vedaflow_objects FOR EACH ROW EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: vedaflow_proposal_approvals vedaflow_proposal_approvals_no_delete; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_proposal_approvals_no_delete BEFORE DELETE ON public.vedaflow_proposal_approvals FOR EACH ROW EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: vedaflow_proposal_approvals vedaflow_proposal_approvals_no_truncate; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_proposal_approvals_no_truncate BEFORE TRUNCATE ON public.vedaflow_proposal_approvals FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: vedaflow_proposal_approvals vedaflow_proposal_approvals_no_update; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_proposal_approvals_no_update BEFORE UPDATE ON public.vedaflow_proposal_approvals FOR EACH ROW EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: vedaflow_proposals vedaflow_proposals_no_delete; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_proposals_no_delete BEFORE DELETE ON public.vedaflow_proposals FOR EACH ROW EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: vedaflow_proposals vedaflow_proposals_no_truncate; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_proposals_no_truncate BEFORE TRUNCATE ON public.vedaflow_proposals FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: vedaflow_proposals vedaflow_proposals_transition; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_proposals_transition BEFORE UPDATE ON public.vedaflow_proposals FOR EACH ROW EXECUTE FUNCTION public.synveda_vedaflow_proposal_transition();


--
-- Name: vedaflow_refs vedaflow_refs_channel_pointers_are_permanent; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_refs_channel_pointers_are_permanent BEFORE DELETE ON public.vedaflow_refs FOR EACH ROW EXECUTE FUNCTION public.synveda_vedaflow_refs_delete_guard();


--
-- Name: vedaflow_refs vedaflow_refs_no_truncate; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_refs_no_truncate BEFORE TRUNCATE ON public.vedaflow_refs FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: vedaflow_tree_entries vedaflow_tree_entries_no_delete; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_tree_entries_no_delete BEFORE DELETE ON public.vedaflow_tree_entries FOR EACH ROW EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: vedaflow_tree_entries vedaflow_tree_entries_no_truncate; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_tree_entries_no_truncate BEFORE TRUNCATE ON public.vedaflow_tree_entries FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: vedaflow_tree_entries vedaflow_tree_entries_no_update; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_tree_entries_no_update BEFORE UPDATE ON public.vedaflow_tree_entries FOR EACH ROW EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: vedaflow_trees vedaflow_trees_no_delete; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_trees_no_delete BEFORE DELETE ON public.vedaflow_trees FOR EACH ROW EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: vedaflow_trees vedaflow_trees_no_truncate; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_trees_no_truncate BEFORE TRUNCATE ON public.vedaflow_trees FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: vedaflow_trees vedaflow_trees_no_update; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER vedaflow_trees_no_update BEFORE UPDATE ON public.vedaflow_trees FOR EACH ROW EXECUTE FUNCTION public.synveda_vedaflow_immutable();


--
-- Name: workspaces workspaces_immutable_columns; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER workspaces_immutable_columns BEFORE UPDATE ON public.workspaces FOR EACH ROW EXECUTE FUNCTION public.synveda_subtype_immutable_columns();


--
-- Name: import_artifacts zz_artifact_address; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER zz_artifact_address AFTER INSERT ON public.import_artifacts FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_projection();


--
-- Name: capture_candidate_import_artifacts zz_artifact_candidate; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER zz_artifact_candidate AFTER INSERT ON public.capture_candidate_import_artifacts FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_projection();


--
-- Name: knowledge_changes zz_artifact_change; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER zz_artifact_change AFTER INSERT ON public.knowledge_changes FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_projection();


--
-- Name: context_run_delivery_dependencies zz_artifact_delivery; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER zz_artifact_delivery AFTER INSERT ON public.context_run_delivery_dependencies FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_projection();


--
-- Name: knowledge_sources zz_artifact_descriptor; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER zz_artifact_descriptor AFTER INSERT ON public.knowledge_sources FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_projection();


--
-- Name: import_mappings zz_artifact_mapping; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER zz_artifact_mapping AFTER INSERT ON public.import_mappings FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_projection();


--
-- Name: knowledge_revision_sources zz_artifact_revision; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER zz_artifact_revision AFTER INSERT ON public.knowledge_revision_sources FOR EACH ROW EXECUTE FUNCTION public.synveda_artifact_projection();


--
-- Name: context_run_delivery_dependencies zzzz_task_delivery_projection; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER zzzz_task_delivery_projection AFTER INSERT ON public.context_run_delivery_dependencies FOR EACH ROW EXECUTE FUNCTION public.synveda_task_evidence_projection();


--
-- Name: session_events zzzz_task_event_projection; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER zzzz_task_event_projection AFTER INSERT ON public.session_events FOR EACH ROW EXECUTE FUNCTION public.synveda_task_evidence_projection();


--
-- Name: capture_candidate_events zzzzzz_procedure_projection; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER zzzzzz_procedure_projection AFTER INSERT ON public.capture_candidate_events FOR EACH ROW EXECUTE FUNCTION public.synveda_procedure_projection();


--
-- Name: capture_candidates zzzzzz_procedure_projection; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER zzzzzz_procedure_projection AFTER INSERT ON public.capture_candidates FOR EACH ROW EXECUTE FUNCTION public.synveda_procedure_projection();


--
-- Name: context_run_delivery_dependencies zzzzzz_procedure_projection; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER zzzzzz_procedure_projection AFTER INSERT ON public.context_run_delivery_dependencies FOR EACH ROW EXECUTE FUNCTION public.synveda_procedure_projection();


--
-- Name: knowledge_changes zzzzzz_procedure_projection; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER zzzzzz_procedure_projection AFTER INSERT ON public.knowledge_changes FOR EACH ROW EXECUTE FUNCTION public.synveda_procedure_projection();


--
-- Name: knowledge_revision_sources zzzzzz_procedure_projection; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER zzzzzz_procedure_projection AFTER INSERT ON public.knowledge_revision_sources FOR EACH ROW EXECUTE FUNCTION public.synveda_procedure_projection();


--
-- Name: knowledge_sources zzzzzz_procedure_projection; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER zzzzzz_procedure_projection AFTER INSERT ON public.knowledge_sources FOR EACH ROW EXECUTE FUNCTION public.synveda_procedure_projection();


--
-- Name: session_events zzzzzz_procedure_projection; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER zzzzzz_procedure_projection AFTER INSERT ON public.session_events FOR EACH ROW EXECUTE FUNCTION public.synveda_procedure_projection();


--
-- Name: artifact_cleanup_targets artifact_cleanup_targets_tenant_id_artifact_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.artifact_cleanup_targets
    ADD CONSTRAINT artifact_cleanup_targets_tenant_id_artifact_id_fkey FOREIGN KEY (tenant_id, artifact_id) REFERENCES public.source_artifact_addresses(tenant_id, artifact_id);


--
-- Name: artifact_cleanup_targets artifact_cleanup_targets_tenant_id_change_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.artifact_cleanup_targets
    ADD CONSTRAINT artifact_cleanup_targets_tenant_id_change_id_fkey FOREIGN KEY (tenant_id, change_id) REFERENCES public.knowledge_changes(tenant_id, proposal_id);


--
-- Name: artifact_cleanup_targets artifact_cleanup_targets_tenant_id_operation_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.artifact_cleanup_targets
    ADD CONSTRAINT artifact_cleanup_targets_tenant_id_operation_id_fkey FOREIGN KEY (tenant_id, operation_id) REFERENCES public.durable_operations(tenant_id, id);


--
-- Name: artifact_cleanup_targets artifact_cleanup_targets_tenant_id_source_key_digest_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.artifact_cleanup_targets
    ADD CONSTRAINT artifact_cleanup_targets_tenant_id_source_key_digest_fkey FOREIGN KEY (tenant_id, source_key_digest) REFERENCES public.artifact_suppressions(tenant_id, source_key_digest);


--
-- Name: artifact_suppressions artifact_suppressions_tenant_id_artifact_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.artifact_suppressions
    ADD CONSTRAINT artifact_suppressions_tenant_id_artifact_id_fkey FOREIGN KEY (tenant_id, artifact_id) REFERENCES public.source_artifact_addresses(tenant_id, artifact_id);


--
-- Name: artifact_suppressions artifact_suppressions_tenant_id_change_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.artifact_suppressions
    ADD CONSTRAINT artifact_suppressions_tenant_id_change_id_fkey FOREIGN KEY (tenant_id, change_id) REFERENCES public.knowledge_changes(tenant_id, proposal_id);


--
-- Name: artifact_suppressions artifact_suppressions_tenant_id_forget_change_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.artifact_suppressions
    ADD CONSTRAINT artifact_suppressions_tenant_id_forget_change_id_fkey FOREIGN KEY (tenant_id, forget_change_id) REFERENCES public.knowledge_changes(tenant_id, proposal_id);


--
-- Name: capability_snapshots capability_snapshots_version_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capability_snapshots
    ADD CONSTRAINT capability_snapshots_version_fk FOREIGN KEY (tenant_id, version_id) REFERENCES public.tool_server_versions(tenant_id, id);


--
-- Name: capture_batch_events capture_batch_events_batch_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_batch_events
    ADD CONSTRAINT capture_batch_events_batch_fk FOREIGN KEY (tenant_id, session_id, batch_id) REFERENCES public.capture_batches(tenant_id, session_id, id);


--
-- Name: capture_batch_events capture_batch_events_event_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_batch_events
    ADD CONSTRAINT capture_batch_events_event_fk FOREIGN KEY (tenant_id, session_id, event_id) REFERENCES public.session_events(tenant_id, session_id, id);


--
-- Name: capture_batch_events capture_batch_events_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_batch_events
    ADD CONSTRAINT capture_batch_events_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: capture_batches capture_batches_configuration_version_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_batches
    ADD CONSTRAINT capture_batches_configuration_version_fk FOREIGN KEY (tenant_id, configuration_version_id) REFERENCES public.configuration_versions(tenant_id, id);


--
-- Name: capture_batches capture_batches_import_job_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_batches
    ADD CONSTRAINT capture_batches_import_job_fk FOREIGN KEY (tenant_id, import_job_id) REFERENCES public.import_jobs(tenant_id, id);


--
-- Name: capture_batches capture_batches_project_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_batches
    ADD CONSTRAINT capture_batches_project_fk FOREIGN KEY (tenant_id, project_id) REFERENCES public.projects(tenant_id, id);


--
-- Name: capture_batches capture_batches_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_batches
    ADD CONSTRAINT capture_batches_scope_fk FOREIGN KEY (tenant_id, scope_id) REFERENCES public.scopes(tenant_id, id);


--
-- Name: capture_batches capture_batches_session_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_batches
    ADD CONSTRAINT capture_batches_session_fk FOREIGN KEY (tenant_id, session_id) REFERENCES public.sessions(tenant_id, id);


--
-- Name: capture_batches capture_batches_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_batches
    ADD CONSTRAINT capture_batches_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: capture_batches capture_batches_workspace_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_batches
    ADD CONSTRAINT capture_batches_workspace_fk FOREIGN KEY (tenant_id, workspace_id) REFERENCES public.workspaces(tenant_id, id);


--
-- Name: capture_candidate_decisions capture_candidate_decisions_candidate_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidate_decisions
    ADD CONSTRAINT capture_candidate_decisions_candidate_fk FOREIGN KEY (tenant_id, candidate_id) REFERENCES public.capture_candidates(tenant_id, id);


--
-- Name: capture_candidate_decisions capture_candidate_decisions_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidate_decisions
    ADD CONSTRAINT capture_candidate_decisions_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: capture_candidate_events capture_candidate_events_candidate_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidate_events
    ADD CONSTRAINT capture_candidate_events_candidate_fk FOREIGN KEY (tenant_id, batch_id, candidate_id) REFERENCES public.capture_candidates(tenant_id, batch_id, id);


--
-- Name: capture_candidate_events capture_candidate_events_frozen_event_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidate_events
    ADD CONSTRAINT capture_candidate_events_frozen_event_fk FOREIGN KEY (tenant_id, batch_id, event_id) REFERENCES public.capture_batch_events(tenant_id, batch_id, event_id);


--
-- Name: capture_candidate_events capture_candidate_events_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidate_events
    ADD CONSTRAINT capture_candidate_events_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: capture_candidate_import_artifacts capture_candidate_import_artifacts_artifact_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidate_import_artifacts
    ADD CONSTRAINT capture_candidate_import_artifacts_artifact_fk FOREIGN KEY (tenant_id, import_job_id, artifact_id) REFERENCES public.import_artifacts(tenant_id, job_id, id);


--
-- Name: capture_candidate_import_artifacts capture_candidate_import_artifacts_candidate_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidate_import_artifacts
    ADD CONSTRAINT capture_candidate_import_artifacts_candidate_fk FOREIGN KEY (tenant_id, import_job_id, candidate_id) REFERENCES public.capture_candidates(tenant_id, import_job_id, id);


--
-- Name: capture_candidate_import_artifacts capture_candidate_import_artifacts_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidate_import_artifacts
    ADD CONSTRAINT capture_candidate_import_artifacts_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: capture_candidate_matches capture_candidate_matches_candidate_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidate_matches
    ADD CONSTRAINT capture_candidate_matches_candidate_fk FOREIGN KEY (tenant_id, candidate_id) REFERENCES public.capture_candidates(tenant_id, id);


--
-- Name: capture_candidate_matches capture_candidate_matches_knowledge_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidate_matches
    ADD CONSTRAINT capture_candidate_matches_knowledge_fk FOREIGN KEY (tenant_id, knowledge_item_id, knowledge_revision_id) REFERENCES public.knowledge_revisions(tenant_id, knowledge_item_id, id) ON DELETE CASCADE;


--
-- Name: capture_candidate_matches capture_candidate_matches_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidate_matches
    ADD CONSTRAINT capture_candidate_matches_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: capture_candidates capture_candidates_batch_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidates
    ADD CONSTRAINT capture_candidates_batch_fk FOREIGN KEY (tenant_id, batch_id) REFERENCES public.capture_batches(tenant_id, id);


--
-- Name: capture_candidates capture_candidates_import_job_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidates
    ADD CONSTRAINT capture_candidates_import_job_fk FOREIGN KEY (tenant_id, import_job_id) REFERENCES public.import_jobs(tenant_id, id);


--
-- Name: capture_candidates capture_candidates_project_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidates
    ADD CONSTRAINT capture_candidates_project_fk FOREIGN KEY (tenant_id, proposed_project_id) REFERENCES public.projects(tenant_id, id);


--
-- Name: capture_candidates capture_candidates_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidates
    ADD CONSTRAINT capture_candidates_scope_fk FOREIGN KEY (tenant_id, proposed_scope_id) REFERENCES public.scopes(tenant_id, id);


--
-- Name: capture_candidates capture_candidates_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.capture_candidates
    ADD CONSTRAINT capture_candidates_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: configuration_artifacts configuration_artifacts_current_version_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_artifacts
    ADD CONSTRAINT configuration_artifacts_current_version_fk FOREIGN KEY (tenant_id, id, current_version_id) REFERENCES public.configuration_versions(tenant_id, artifact_id, id) DEFERRABLE INITIALLY DEFERRED;


--
-- Name: configuration_artifacts configuration_artifacts_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_artifacts
    ADD CONSTRAINT configuration_artifacts_scope_fk FOREIGN KEY (tenant_id, governing_scope_id) REFERENCES public.scopes(tenant_id, id);


--
-- Name: configuration_artifacts configuration_artifacts_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_artifacts
    ADD CONSTRAINT configuration_artifacts_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: configuration_bindings configuration_bindings_artifact_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_bindings
    ADD CONSTRAINT configuration_bindings_artifact_fk FOREIGN KEY (tenant_id, artifact_id) REFERENCES public.configuration_artifacts(tenant_id, id);


--
-- Name: configuration_bindings configuration_bindings_pin_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_bindings
    ADD CONSTRAINT configuration_bindings_pin_fk FOREIGN KEY (tenant_id, artifact_id, pinned_version_id) REFERENCES public.configuration_versions(tenant_id, artifact_id, id);


--
-- Name: configuration_bindings configuration_bindings_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_bindings
    ADD CONSTRAINT configuration_bindings_scope_fk FOREIGN KEY (tenant_id, scope_id) REFERENCES public.scopes(tenant_id, id);


--
-- Name: configuration_changes configuration_changes_proposal_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_changes
    ADD CONSTRAINT configuration_changes_proposal_fk FOREIGN KEY (tenant_id, proposal_id) REFERENCES public.vedaflow_proposals(tenant_id, id);


--
-- Name: configuration_changes configuration_changes_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_changes
    ADD CONSTRAINT configuration_changes_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: configuration_versions configuration_versions_artifact_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_versions
    ADD CONSTRAINT configuration_versions_artifact_fk FOREIGN KEY (tenant_id, artifact_id) REFERENCES public.configuration_artifacts(tenant_id, id);


--
-- Name: configuration_versions configuration_versions_proposal_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.configuration_versions
    ADD CONSTRAINT configuration_versions_proposal_fk FOREIGN KEY (tenant_id, proposal_id) REFERENCES public.vedaflow_proposals(tenant_id, id);


--
-- Name: context_candidates context_candidates_capture_candidate_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_candidates
    ADD CONSTRAINT context_candidates_capture_candidate_fk FOREIGN KEY (tenant_id, capture_candidate_id) REFERENCES public.capture_candidates(tenant_id, id);


--
-- Name: context_candidates context_candidates_item_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_candidates
    ADD CONSTRAINT context_candidates_item_fk FOREIGN KEY (tenant_id, knowledge_item_id) REFERENCES public.knowledge_items(tenant_id, id);


--
-- Name: context_candidates context_candidates_revision_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_candidates
    ADD CONSTRAINT context_candidates_revision_fk FOREIGN KEY (tenant_id, knowledge_item_id, knowledge_revision_id) REFERENCES public.knowledge_revisions(tenant_id, knowledge_item_id, id);


--
-- Name: context_candidates context_candidates_run_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_candidates
    ADD CONSTRAINT context_candidates_run_fk FOREIGN KEY (tenant_id, context_run_id) REFERENCES public.session_context_runs(tenant_id, id);


--
-- Name: context_candidates context_candidates_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_candidates
    ADD CONSTRAINT context_candidates_scope_fk FOREIGN KEY (tenant_id, scope_id) REFERENCES public.scopes(tenant_id, id);


--
-- Name: context_candidates context_candidates_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_candidates
    ADD CONSTRAINT context_candidates_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: context_run_delivery_dependencies context_delivery_run_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_run_delivery_dependencies
    ADD CONSTRAINT context_delivery_run_fk FOREIGN KEY (tenant_id, context_run_id) REFERENCES public.session_context_runs(tenant_id, id) ON DELETE CASCADE;


--
-- Name: context_feedback context_feedback_selection_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_feedback
    ADD CONSTRAINT context_feedback_selection_fk FOREIGN KEY (tenant_id, context_selection_id, context_run_id, knowledge_revision_id) REFERENCES public.context_selections(tenant_id, id, context_run_id, knowledge_revision_id);


--
-- Name: context_feedback context_feedback_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_feedback
    ADD CONSTRAINT context_feedback_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: context_graph_steps context_graph_steps_asserting_revision_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_graph_steps
    ADD CONSTRAINT context_graph_steps_asserting_revision_fk FOREIGN KEY (tenant_id, asserting_revision_id) REFERENCES public.knowledge_revisions(tenant_id, id);


--
-- Name: context_graph_steps context_graph_steps_candidate_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_graph_steps
    ADD CONSTRAINT context_graph_steps_candidate_fk FOREIGN KEY (tenant_id, context_run_id, context_candidate_id) REFERENCES public.context_candidates(tenant_id, context_run_id, id);


--
-- Name: context_graph_steps context_graph_steps_from_revision_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_graph_steps
    ADD CONSTRAINT context_graph_steps_from_revision_fk FOREIGN KEY (tenant_id, from_item_id, from_revision_id) REFERENCES public.knowledge_revisions(tenant_id, knowledge_item_id, id);


--
-- Name: context_graph_steps context_graph_steps_relation_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_graph_steps
    ADD CONSTRAINT context_graph_steps_relation_fk FOREIGN KEY (tenant_id, relation_id) REFERENCES public.knowledge_relations(tenant_id, id);


--
-- Name: context_graph_steps context_graph_steps_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_graph_steps
    ADD CONSTRAINT context_graph_steps_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: context_graph_steps context_graph_steps_to_revision_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_graph_steps
    ADD CONSTRAINT context_graph_steps_to_revision_fk FOREIGN KEY (tenant_id, to_item_id, to_revision_id) REFERENCES public.knowledge_revisions(tenant_id, knowledge_item_id, id);


--
-- Name: context_pack_chunks context_pack_chunks_object_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_pack_chunks
    ADD CONSTRAINT context_pack_chunks_object_fk FOREIGN KEY (tenant_id, document_hash) REFERENCES public.vedaflow_objects(tenant_id, hash);


--
-- Name: context_pack_chunks context_pack_chunks_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_pack_chunks
    ADD CONSTRAINT context_pack_chunks_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: context_pack_documents context_pack_documents_object_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_pack_documents
    ADD CONSTRAINT context_pack_documents_object_fk FOREIGN KEY (tenant_id, object_hash) REFERENCES public.vedaflow_objects(tenant_id, hash);


--
-- Name: context_pack_documents context_pack_documents_pack_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_pack_documents
    ADD CONSTRAINT context_pack_documents_pack_fk FOREIGN KEY (tenant_id, scope_id, pack_name) REFERENCES public.context_packs(tenant_id, scope_id, name);


--
-- Name: context_packs context_packs_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_packs
    ADD CONSTRAINT context_packs_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: session_context_runs context_runs_configuration_version_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.session_context_runs
    ADD CONSTRAINT context_runs_configuration_version_fk FOREIGN KEY (tenant_id, configuration_version_id) REFERENCES public.configuration_versions(tenant_id, id);


--
-- Name: context_selections context_selections_candidate_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_selections
    ADD CONSTRAINT context_selections_candidate_fk FOREIGN KEY (tenant_id, context_run_id, context_candidate_id) REFERENCES public.context_candidates(tenant_id, context_run_id, id);


--
-- Name: context_selections context_selections_capture_candidate_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_selections
    ADD CONSTRAINT context_selections_capture_candidate_fk FOREIGN KEY (tenant_id, capture_candidate_id) REFERENCES public.capture_candidates(tenant_id, id);


--
-- Name: context_selections context_selections_revision_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_selections
    ADD CONSTRAINT context_selections_revision_fk FOREIGN KEY (tenant_id, knowledge_item_id, knowledge_revision_id) REFERENCES public.knowledge_revisions(tenant_id, knowledge_item_id, id);


--
-- Name: context_selections context_selections_run_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_selections
    ADD CONSTRAINT context_selections_run_fk FOREIGN KEY (tenant_id, context_run_id) REFERENCES public.session_context_runs(tenant_id, id);


--
-- Name: context_selections context_selections_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.context_selections
    ADD CONSTRAINT context_selections_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: directory_sync_state directory_sync_state_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.directory_sync_state
    ADD CONSTRAINT directory_sync_state_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: durable_operations durable_operations_proposal_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.durable_operations
    ADD CONSTRAINT durable_operations_proposal_fk FOREIGN KEY (tenant_id, proposal_id) REFERENCES public.vedaflow_proposals(tenant_id, id);


--
-- Name: durable_operations durable_operations_requester_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.durable_operations
    ADD CONSTRAINT durable_operations_requester_fk FOREIGN KEY (tenant_id, requested_by_identity_id) REFERENCES public.identities(tenant_id, id);


--
-- Name: durable_operations durable_operations_skill_version_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.durable_operations
    ADD CONSTRAINT durable_operations_skill_version_fk FOREIGN KEY (tenant_id, skill_version_id) REFERENCES public.skill_versions(tenant_id, id);


--
-- Name: durable_operations durable_operations_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.durable_operations
    ADD CONSTRAINT durable_operations_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: group_members group_members_group_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.group_members
    ADD CONSTRAINT group_members_group_fk FOREIGN KEY (tenant_id, group_id) REFERENCES public.groups(tenant_id, id) ON DELETE CASCADE;


--
-- Name: group_members group_members_identity_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.group_members
    ADD CONSTRAINT group_members_identity_fk FOREIGN KEY (tenant_id, identity_id) REFERENCES public.identities(tenant_id, id);


--
-- Name: group_members group_members_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.group_members
    ADD CONSTRAINT group_members_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: groups groups_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.groups
    ADD CONSTRAINT groups_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: historical_knowledge_dependencies historical_knowledge_dependencies_tenant_id_revision_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.historical_knowledge_dependencies
    ADD CONSTRAINT historical_knowledge_dependencies_tenant_id_revision_id_fkey FOREIGN KEY (tenant_id, revision_id) REFERENCES public.knowledge_revision_headers(tenant_id, revision_id);


--
-- Name: idempotency_records idempotency_records_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.idempotency_records
    ADD CONSTRAINT idempotency_records_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: identities identities_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.identities
    ADD CONSTRAINT identities_scope_fk FOREIGN KEY (tenant_id, scope_id) REFERENCES public.scopes(tenant_id, id);


--
-- Name: identities identities_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.identities
    ADD CONSTRAINT identities_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: import_artifacts import_artifacts_job_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_artifacts
    ADD CONSTRAINT import_artifacts_job_fk FOREIGN KEY (tenant_id, job_id) REFERENCES public.import_jobs(tenant_id, id);


--
-- Name: import_artifacts import_artifacts_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_artifacts
    ADD CONSTRAINT import_artifacts_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: import_jobs import_jobs_capture_batch_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_jobs
    ADD CONSTRAINT import_jobs_capture_batch_fk FOREIGN KEY (tenant_id, capture_batch_id) REFERENCES public.capture_batches(tenant_id, id);


--
-- Name: import_jobs import_jobs_project_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_jobs
    ADD CONSTRAINT import_jobs_project_fk FOREIGN KEY (tenant_id, project_id) REFERENCES public.projects(tenant_id, id);


--
-- Name: import_jobs import_jobs_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_jobs
    ADD CONSTRAINT import_jobs_scope_fk FOREIGN KEY (tenant_id, scope_id) REFERENCES public.scopes(tenant_id, id);


--
-- Name: import_jobs import_jobs_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_jobs
    ADD CONSTRAINT import_jobs_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: import_jobs import_jobs_tenant_id_project_id_managed_source_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_jobs
    ADD CONSTRAINT import_jobs_tenant_id_project_id_managed_source_id_fkey FOREIGN KEY (tenant_id, project_id, managed_source_id) REFERENCES public.source_reconcile_heads(tenant_id, project_id, source_id);


--
-- Name: import_jobs import_jobs_workspace_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_jobs
    ADD CONSTRAINT import_jobs_workspace_fk FOREIGN KEY (tenant_id, workspace_id) REFERENCES public.workspaces(tenant_id, id);


--
-- Name: import_mappings import_mappings_artifact_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_mappings
    ADD CONSTRAINT import_mappings_artifact_fk FOREIGN KEY (tenant_id, job_id, artifact_id) REFERENCES public.import_artifacts(tenant_id, job_id, id);


--
-- Name: import_mappings import_mappings_candidate_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_mappings
    ADD CONSTRAINT import_mappings_candidate_fk FOREIGN KEY (tenant_id, candidate_id) REFERENCES public.capture_candidates(tenant_id, id);


--
-- Name: import_mappings import_mappings_job_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_mappings
    ADD CONSTRAINT import_mappings_job_fk FOREIGN KEY (tenant_id, job_id) REFERENCES public.import_jobs(tenant_id, id);


--
-- Name: import_mappings import_mappings_matched_revision_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_mappings
    ADD CONSTRAINT import_mappings_matched_revision_fk FOREIGN KEY (tenant_id, matched_item_id, matched_revision_id) REFERENCES public.knowledge_revisions(tenant_id, knowledge_item_id, id);


--
-- Name: import_mappings import_mappings_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_mappings
    ADD CONSTRAINT import_mappings_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: knowledge_changes knowledge_changes_proposal_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_changes
    ADD CONSTRAINT knowledge_changes_proposal_fk FOREIGN KEY (tenant_id, proposal_id) REFERENCES public.vedaflow_proposals(tenant_id, id);


--
-- Name: knowledge_changes knowledge_changes_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_changes
    ADD CONSTRAINT knowledge_changes_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: knowledge_conflict_members knowledge_conflict_members_candidate_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_conflict_members
    ADD CONSTRAINT knowledge_conflict_members_candidate_fk FOREIGN KEY (tenant_id, capture_candidate_id) REFERENCES public.capture_candidates(tenant_id, id);


--
-- Name: knowledge_conflict_members knowledge_conflict_members_item_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_conflict_members
    ADD CONSTRAINT knowledge_conflict_members_item_fk FOREIGN KEY (tenant_id, knowledge_item_id, knowledge_revision_id) REFERENCES public.knowledge_revisions(tenant_id, knowledge_item_id, id);


--
-- Name: knowledge_conflict_members knowledge_conflict_members_set_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_conflict_members
    ADD CONSTRAINT knowledge_conflict_members_set_fk FOREIGN KEY (tenant_id, conflict_set_id) REFERENCES public.knowledge_conflict_sets(tenant_id, id);


--
-- Name: knowledge_conflict_sets knowledge_conflict_sets_candidate_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_conflict_sets
    ADD CONSTRAINT knowledge_conflict_sets_candidate_fk FOREIGN KEY (tenant_id, capture_candidate_id) REFERENCES public.capture_candidates(tenant_id, id);


--
-- Name: knowledge_conflict_sets knowledge_conflict_sets_change_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_conflict_sets
    ADD CONSTRAINT knowledge_conflict_sets_change_fk FOREIGN KEY (tenant_id, resolution_change_id) REFERENCES public.knowledge_changes(tenant_id, proposal_id) DEFERRABLE INITIALLY DEFERRED;


--
-- Name: knowledge_conflict_sets knowledge_conflict_sets_project_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_conflict_sets
    ADD CONSTRAINT knowledge_conflict_sets_project_fk FOREIGN KEY (tenant_id, project_id) REFERENCES public.projects(tenant_id, id);


--
-- Name: knowledge_conflict_sets knowledge_conflict_sets_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_conflict_sets
    ADD CONSTRAINT knowledge_conflict_sets_scope_fk FOREIGN KEY (tenant_id, scope_id) REFERENCES public.scopes(tenant_id, id);


--
-- Name: knowledge_conflict_sets knowledge_conflict_sets_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_conflict_sets
    ADD CONSTRAINT knowledge_conflict_sets_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: knowledge_copy_dependencies knowledge_copy_dependencies_tenant_id_revision_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_copy_dependencies
    ADD CONSTRAINT knowledge_copy_dependencies_tenant_id_revision_id_fkey FOREIGN KEY (tenant_id, revision_id) REFERENCES public.knowledge_revision_headers(tenant_id, revision_id);


--
-- Name: knowledge_erasure_tombstones knowledge_erasure_tombstones_operation_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_erasure_tombstones
    ADD CONSTRAINT knowledge_erasure_tombstones_operation_fk FOREIGN KEY (tenant_id, operation_id) REFERENCES public.durable_operations(tenant_id, id);


--
-- Name: knowledge_erasure_tombstones knowledge_erasure_tombstones_proposal_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_erasure_tombstones
    ADD CONSTRAINT knowledge_erasure_tombstones_proposal_fk FOREIGN KEY (tenant_id, proposal_id) REFERENCES public.vedaflow_proposals(tenant_id, id);


--
-- Name: knowledge_erasure_tombstones knowledge_erasure_tombstones_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_erasure_tombstones
    ADD CONSTRAINT knowledge_erasure_tombstones_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: knowledge_index_invalidations knowledge_index_invalidations_operation_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_index_invalidations
    ADD CONSTRAINT knowledge_index_invalidations_operation_fk FOREIGN KEY (tenant_id, operation_id) REFERENCES public.durable_operations(tenant_id, id);


--
-- Name: knowledge_index_invalidations knowledge_index_invalidations_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_index_invalidations
    ADD CONSTRAINT knowledge_index_invalidations_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: knowledge_items knowledge_items_current_revision_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_items
    ADD CONSTRAINT knowledge_items_current_revision_fk FOREIGN KEY (tenant_id, id, current_revision_id) REFERENCES public.knowledge_revisions(tenant_id, knowledge_item_id, id) DEFERRABLE INITIALLY DEFERRED;


--
-- Name: knowledge_items_history knowledge_items_history_current_revision_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_items_history
    ADD CONSTRAINT knowledge_items_history_current_revision_fk FOREIGN KEY (tenant_id, id, current_revision_id) REFERENCES public.knowledge_revisions(tenant_id, knowledge_item_id, id);


--
-- Name: knowledge_items_history knowledge_items_history_project_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_items_history
    ADD CONSTRAINT knowledge_items_history_project_fk FOREIGN KEY (tenant_id, project_id) REFERENCES public.projects(tenant_id, id);


--
-- Name: knowledge_items_history knowledge_items_history_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_items_history
    ADD CONSTRAINT knowledge_items_history_scope_fk FOREIGN KEY (tenant_id, scope_id) REFERENCES public.scopes(tenant_id, id);


--
-- Name: knowledge_items_history knowledge_items_history_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_items_history
    ADD CONSTRAINT knowledge_items_history_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: knowledge_items knowledge_items_project_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_items
    ADD CONSTRAINT knowledge_items_project_fk FOREIGN KEY (tenant_id, project_id) REFERENCES public.projects(tenant_id, id);


--
-- Name: knowledge_items knowledge_items_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_items
    ADD CONSTRAINT knowledge_items_scope_fk FOREIGN KEY (tenant_id, scope_id) REFERENCES public.scopes(tenant_id, id);


--
-- Name: knowledge_items knowledge_items_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_items
    ADD CONSTRAINT knowledge_items_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: knowledge_relations knowledge_relations_asserting_revision_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_relations
    ADD CONSTRAINT knowledge_relations_asserting_revision_fk FOREIGN KEY (tenant_id, source_item_id, asserting_revision_id) REFERENCES public.knowledge_revisions(tenant_id, knowledge_item_id, id);


--
-- Name: knowledge_relations knowledge_relations_source_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_relations
    ADD CONSTRAINT knowledge_relations_source_fk FOREIGN KEY (tenant_id, source_item_id) REFERENCES public.knowledge_items(tenant_id, id);


--
-- Name: knowledge_relations knowledge_relations_target_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_relations
    ADD CONSTRAINT knowledge_relations_target_fk FOREIGN KEY (tenant_id, target_item_id) REFERENCES public.knowledge_items(tenant_id, id);


--
-- Name: knowledge_relations knowledge_relations_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_relations
    ADD CONSTRAINT knowledge_relations_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: knowledge_revision_embeddings knowledge_revision_embeddings_revision_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_revision_embeddings
    ADD CONSTRAINT knowledge_revision_embeddings_revision_fk FOREIGN KEY (tenant_id, knowledge_revision_id) REFERENCES public.knowledge_revisions(tenant_id, id) ON DELETE CASCADE;


--
-- Name: knowledge_revision_embeddings knowledge_revision_embeddings_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_revision_embeddings
    ADD CONSTRAINT knowledge_revision_embeddings_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: knowledge_revision_sources knowledge_revision_sources_revision_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_revision_sources
    ADD CONSTRAINT knowledge_revision_sources_revision_fk FOREIGN KEY (tenant_id, knowledge_revision_id) REFERENCES public.knowledge_revisions(tenant_id, id);


--
-- Name: knowledge_revision_sources knowledge_revision_sources_source_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_revision_sources
    ADD CONSTRAINT knowledge_revision_sources_source_fk FOREIGN KEY (tenant_id, knowledge_source_id) REFERENCES public.knowledge_sources(tenant_id, id);


--
-- Name: knowledge_revision_sources knowledge_revision_sources_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_revision_sources
    ADD CONSTRAINT knowledge_revision_sources_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: knowledge_revisions knowledge_revisions_item_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_revisions
    ADD CONSTRAINT knowledge_revisions_item_fk FOREIGN KEY (tenant_id, knowledge_item_id) REFERENCES public.knowledge_items(tenant_id, id) DEFERRABLE INITIALLY DEFERRED;


--
-- Name: knowledge_revisions knowledge_revisions_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_revisions
    ADD CONSTRAINT knowledge_revisions_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: knowledge_sources knowledge_source_artifact_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_sources
    ADD CONSTRAINT knowledge_source_artifact_fk FOREIGN KEY (tenant_id, import_artifact_id) REFERENCES public.import_artifacts(tenant_id, id);


--
-- Name: knowledge_sources knowledge_sources_event_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_sources
    ADD CONSTRAINT knowledge_sources_event_fk FOREIGN KEY (tenant_id, session_event_id) REFERENCES public.session_events(tenant_id, id);


--
-- Name: knowledge_sources knowledge_sources_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_sources
    ADD CONSTRAINT knowledge_sources_scope_fk FOREIGN KEY (tenant_id, scope_id) REFERENCES public.scopes(tenant_id, id);


--
-- Name: knowledge_sources knowledge_sources_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.knowledge_sources
    ADD CONSTRAINT knowledge_sources_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: operation_attempts operation_attempts_operation_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.operation_attempts
    ADD CONSTRAINT operation_attempts_operation_fk FOREIGN KEY (tenant_id, operation_id) REFERENCES public.durable_operations(tenant_id, id);


--
-- Name: operation_attempts operation_attempts_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.operation_attempts
    ADD CONSTRAINT operation_attempts_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: operation_outbox operation_outbox_operation_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.operation_outbox
    ADD CONSTRAINT operation_outbox_operation_fk FOREIGN KEY (tenant_id, operation_id, operation_version) REFERENCES public.durable_operations(tenant_id, id, operation_version);


--
-- Name: operation_outbox operation_outbox_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.operation_outbox
    ADD CONSTRAINT operation_outbox_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: pending_invites pending_invites_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.pending_invites
    ADD CONSTRAINT pending_invites_scope_fk FOREIGN KEY (tenant_id, scope_id) REFERENCES public.scopes(tenant_id, id);


--
-- Name: pending_invites pending_invites_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.pending_invites
    ADD CONSTRAINT pending_invites_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: policy_packs policy_packs_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_packs
    ADD CONSTRAINT policy_packs_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: policy_relaxation_changes policy_relaxation_changes_proposal_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxation_changes
    ADD CONSTRAINT policy_relaxation_changes_proposal_fk FOREIGN KEY (tenant_id, proposal_id) REFERENCES public.vedaflow_proposals(tenant_id, id);


--
-- Name: policy_relaxation_changes policy_relaxation_changes_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxation_changes
    ADD CONSTRAINT policy_relaxation_changes_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: policy_relaxation_versions policy_relaxation_versions_aggregate_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxation_versions
    ADD CONSTRAINT policy_relaxation_versions_aggregate_fk FOREIGN KEY (tenant_id, relaxation_id) REFERENCES public.policy_relaxations(tenant_id, id);


--
-- Name: policy_relaxation_versions policy_relaxation_versions_configuration_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxation_versions
    ADD CONSTRAINT policy_relaxation_versions_configuration_fk FOREIGN KEY (tenant_id, configuration_version_id) REFERENCES public.configuration_versions(tenant_id, id);


--
-- Name: policy_relaxation_versions policy_relaxation_versions_creator_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxation_versions
    ADD CONSTRAINT policy_relaxation_versions_creator_fk FOREIGN KEY (tenant_id, creator_id) REFERENCES public.identities(tenant_id, id);


--
-- Name: policy_relaxation_versions policy_relaxation_versions_proposal_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxation_versions
    ADD CONSTRAINT policy_relaxation_versions_proposal_fk FOREIGN KEY (tenant_id, proposal_id) REFERENCES public.vedaflow_proposals(tenant_id, id);


--
-- Name: policy_relaxation_versions policy_relaxation_versions_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxation_versions
    ADD CONSTRAINT policy_relaxation_versions_scope_fk FOREIGN KEY (tenant_id, target_scope_id) REFERENCES public.scopes(tenant_id, id);


--
-- Name: policy_relaxation_versions policy_relaxation_versions_subject_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxation_versions
    ADD CONSTRAINT policy_relaxation_versions_subject_fk FOREIGN KEY (tenant_id, subject_identity_id) REFERENCES public.identities(tenant_id, id);


--
-- Name: policy_relaxations policy_relaxations_created_by_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxations
    ADD CONSTRAINT policy_relaxations_created_by_fk FOREIGN KEY (tenant_id, created_by) REFERENCES public.identities(tenant_id, id);


--
-- Name: policy_relaxations policy_relaxations_current_version_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxations
    ADD CONSTRAINT policy_relaxations_current_version_fk FOREIGN KEY (tenant_id, id, current_version_id) REFERENCES public.policy_relaxation_versions(tenant_id, relaxation_id, id) DEFERRABLE INITIALLY DEFERRED;


--
-- Name: policy_relaxations policy_relaxations_revocation_proposal_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxations
    ADD CONSTRAINT policy_relaxations_revocation_proposal_fk FOREIGN KEY (tenant_id, revocation_proposal_id) REFERENCES public.vedaflow_proposals(tenant_id, id);


--
-- Name: policy_relaxations policy_relaxations_revoked_by_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxations
    ADD CONSTRAINT policy_relaxations_revoked_by_fk FOREIGN KEY (tenant_id, revoked_by) REFERENCES public.identities(tenant_id, id);


--
-- Name: policy_relaxations policy_relaxations_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxations
    ADD CONSTRAINT policy_relaxations_scope_fk FOREIGN KEY (tenant_id, governing_scope_id) REFERENCES public.scopes(tenant_id, id);


--
-- Name: policy_relaxations policy_relaxations_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxations
    ADD CONSTRAINT policy_relaxations_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: policy_relaxations policy_relaxations_updated_by_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.policy_relaxations
    ADD CONSTRAINT policy_relaxations_updated_by_fk FOREIGN KEY (tenant_id, updated_by) REFERENCES public.identities(tenant_id, id);


--
-- Name: procedure_candidate_headers procedure_candidate_headers_tenant_id_request_event_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.procedure_candidate_headers
    ADD CONSTRAINT procedure_candidate_headers_tenant_id_request_event_id_fkey FOREIGN KEY (tenant_id, request_event_id) REFERENCES public.procedure_improvement_headers(tenant_id, event_id);


--
-- Name: procedure_evaluation_attempts procedure_evaluation_attempts_tenant_id_candidate_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.procedure_evaluation_attempts
    ADD CONSTRAINT procedure_evaluation_attempts_tenant_id_candidate_id_fkey FOREIGN KEY (tenant_id, candidate_id) REFERENCES public.procedure_candidate_headers(tenant_id, candidate_id);


--
-- Name: procedure_evaluation_results procedure_evaluation_results_tenant_id_attempt_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.procedure_evaluation_results
    ADD CONSTRAINT procedure_evaluation_results_tenant_id_attempt_id_fkey FOREIGN KEY (tenant_id, attempt_id) REFERENCES public.procedure_evaluation_attempts(tenant_id, id);


--
-- Name: procedure_improvement_headers procedure_improvement_headers_tenant_id_event_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.procedure_improvement_headers
    ADD CONSTRAINT procedure_improvement_headers_tenant_id_event_id_fkey FOREIGN KEY (tenant_id, event_id) REFERENCES public.source_event_addresses(tenant_id, event_id);


--
-- Name: procedure_improvement_headers procedure_improvement_headers_tenant_id_procedure_revision_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.procedure_improvement_headers
    ADD CONSTRAINT procedure_improvement_headers_tenant_id_procedure_revision_fkey FOREIGN KEY (tenant_id, procedure_revision_id) REFERENCES public.knowledge_revision_headers(tenant_id, revision_id);


--
-- Name: procedure_outcome_headers procedure_outcome_headers_tenant_id_event_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.procedure_outcome_headers
    ADD CONSTRAINT procedure_outcome_headers_tenant_id_event_id_fkey FOREIGN KEY (tenant_id, event_id) REFERENCES public.source_event_addresses(tenant_id, event_id);


--
-- Name: procedure_outcome_headers procedure_outcome_headers_tenant_id_procedure_revision_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.procedure_outcome_headers
    ADD CONSTRAINT procedure_outcome_headers_tenant_id_procedure_revision_id_fkey FOREIGN KEY (tenant_id, procedure_revision_id) REFERENCES public.knowledge_revision_headers(tenant_id, revision_id);


--
-- Name: project_repositories project_repositories_project_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.project_repositories
    ADD CONSTRAINT project_repositories_project_fk FOREIGN KEY (tenant_id, project_id) REFERENCES public.projects(tenant_id, id);


--
-- Name: project_repositories project_repositories_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.project_repositories
    ADD CONSTRAINT project_repositories_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: projects projects_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.projects
    ADD CONSTRAINT projects_scope_fk FOREIGN KEY (tenant_id, scope_id, scope_kind, slug) REFERENCES public.scopes(tenant_id, id, kind, slug);


--
-- Name: projects projects_scope_parent_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.projects
    ADD CONSTRAINT projects_scope_parent_fk FOREIGN KEY (tenant_id, scope_id, workspace_scope_id) REFERENCES public.scopes(tenant_id, id, parent_scope_id);


--
-- Name: projects projects_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.projects
    ADD CONSTRAINT projects_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: projects projects_workspace_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.projects
    ADD CONSTRAINT projects_workspace_fk FOREIGN KEY (tenant_id, workspace_id, workspace_scope_id) REFERENCES public.workspaces(tenant_id, id, scope_id);


--
-- Name: prompts prompts_object_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.prompts
    ADD CONSTRAINT prompts_object_fk FOREIGN KEY (tenant_id, object_hash) REFERENCES public.vedaflow_objects(tenant_id, hash);


--
-- Name: prompts prompts_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.prompts
    ADD CONSTRAINT prompts_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: scim_credentials scim_credentials_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scim_credentials
    ADD CONSTRAINT scim_credentials_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: scim_users scim_users_identity_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scim_users
    ADD CONSTRAINT scim_users_identity_fk FOREIGN KEY (tenant_id, identity_id) REFERENCES public.identities(tenant_id, id);


--
-- Name: scim_users scim_users_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scim_users
    ADD CONSTRAINT scim_users_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: scope_closure scope_closure_ancestor_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scope_closure
    ADD CONSTRAINT scope_closure_ancestor_fk FOREIGN KEY (tenant_id, ancestor_id) REFERENCES public.scopes(tenant_id, id) ON DELETE CASCADE;


--
-- Name: scope_closure scope_closure_descendant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scope_closure
    ADD CONSTRAINT scope_closure_descendant_fk FOREIGN KEY (tenant_id, descendant_id) REFERENCES public.scopes(tenant_id, id) ON DELETE CASCADE;


--
-- Name: scope_grants scope_grants_group_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scope_grants
    ADD CONSTRAINT scope_grants_group_fk FOREIGN KEY (tenant_id, group_id) REFERENCES public.groups(tenant_id, id) ON DELETE CASCADE;


--
-- Name: scope_grants scope_grants_invite_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scope_grants
    ADD CONSTRAINT scope_grants_invite_fk FOREIGN KEY (tenant_id, invite_id) REFERENCES public.pending_invites(tenant_id, id);


--
-- Name: scope_grants scope_grants_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scope_grants
    ADD CONSTRAINT scope_grants_scope_fk FOREIGN KEY (tenant_id, scope_id) REFERENCES public.scopes(tenant_id, id);


--
-- Name: scope_grants scope_grants_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scope_grants
    ADD CONSTRAINT scope_grants_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: scopes scopes_parent_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scopes
    ADD CONSTRAINT scopes_parent_fk FOREIGN KEY (tenant_id, parent_scope_id, parent_kind) REFERENCES public.scopes(tenant_id, id, kind);


--
-- Name: scopes scopes_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scopes
    ADD CONSTRAINT scopes_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: session_context_runs session_context_runs_project_session_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.session_context_runs
    ADD CONSTRAINT session_context_runs_project_session_fk FOREIGN KEY (tenant_id, session_id, project_id) REFERENCES public.sessions(tenant_id, id, project_id);


--
-- Name: session_context_runs session_context_runs_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.session_context_runs
    ADD CONSTRAINT session_context_runs_scope_fk FOREIGN KEY (tenant_id, scope_id) REFERENCES public.scopes(tenant_id, id);


--
-- Name: session_context_runs session_context_runs_session_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.session_context_runs
    ADD CONSTRAINT session_context_runs_session_fk FOREIGN KEY (tenant_id, session_id) REFERENCES public.sessions(tenant_id, id);


--
-- Name: session_context_runs session_context_runs_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.session_context_runs
    ADD CONSTRAINT session_context_runs_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: session_context_runs session_context_runs_workspace_session_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.session_context_runs
    ADD CONSTRAINT session_context_runs_workspace_session_fk FOREIGN KEY (tenant_id, session_id, workspace_id) REFERENCES public.sessions(tenant_id, id, workspace_id);


--
-- Name: session_event_quarantine session_event_quarantine_event_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.session_event_quarantine
    ADD CONSTRAINT session_event_quarantine_event_fk FOREIGN KEY (tenant_id, event_id) REFERENCES public.session_events(tenant_id, id);


--
-- Name: session_event_quarantine session_event_quarantine_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.session_event_quarantine
    ADD CONSTRAINT session_event_quarantine_scope_fk FOREIGN KEY (tenant_id, scope_id) REFERENCES public.scopes(tenant_id, id);


--
-- Name: session_event_quarantine session_event_quarantine_session_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.session_event_quarantine
    ADD CONSTRAINT session_event_quarantine_session_fk FOREIGN KEY (tenant_id, session_id) REFERENCES public.sessions(tenant_id, id);


--
-- Name: session_event_quarantine session_event_quarantine_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.session_event_quarantine
    ADD CONSTRAINT session_event_quarantine_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: session_events session_events_session_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.session_events
    ADD CONSTRAINT session_events_session_fk FOREIGN KEY (tenant_id, session_id) REFERENCES public.sessions(tenant_id, id);


--
-- Name: session_events session_events_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.session_events
    ADD CONSTRAINT session_events_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: session_snapshot_headers session_snapshot_headers_tenant_id_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.session_snapshot_headers
    ADD CONSTRAINT session_snapshot_headers_tenant_id_id_fkey FOREIGN KEY (tenant_id, id) REFERENCES public.source_event_addresses(tenant_id, event_id);


--
-- Name: sessions sessions_project_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_project_fk FOREIGN KEY (tenant_id, project_id, workspace_id, project_scope_id) REFERENCES public.projects(tenant_id, id, workspace_id, scope_id);


--
-- Name: sessions sessions_repository_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_repository_fk FOREIGN KEY (tenant_id, project_id, repository_id) REFERENCES public.project_repositories(tenant_id, project_id, id);


--
-- Name: sessions sessions_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: sessions sessions_workspace_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_workspace_fk FOREIGN KEY (tenant_id, workspace_id, workspace_scope_id) REFERENCES public.workspaces(tenant_id, id, scope_id);


--
-- Name: skill_bindings skill_bindings_pin_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_bindings
    ADD CONSTRAINT skill_bindings_pin_fk FOREIGN KEY (tenant_id, skill_id, pinned_version_id) REFERENCES public.skill_versions(tenant_id, skill_id, id);


--
-- Name: skill_bindings skill_bindings_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_bindings
    ADD CONSTRAINT skill_bindings_scope_fk FOREIGN KEY (tenant_id, scope_id) REFERENCES public.scopes(tenant_id, id);


--
-- Name: skill_bindings skill_bindings_skill_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_bindings
    ADD CONSTRAINT skill_bindings_skill_fk FOREIGN KEY (tenant_id, skill_id) REFERENCES public.skills(tenant_id, id);


--
-- Name: skill_changes skill_changes_proposal_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_changes
    ADD CONSTRAINT skill_changes_proposal_fk FOREIGN KEY (tenant_id, proposal_id) REFERENCES public.vedaflow_proposals(tenant_id, id);


--
-- Name: skill_changes skill_changes_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_changes
    ADD CONSTRAINT skill_changes_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: skill_test_runs skill_test_runs_operation_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_test_runs
    ADD CONSTRAINT skill_test_runs_operation_fk FOREIGN KEY (tenant_id, operation_id, version_id) REFERENCES public.durable_operations(tenant_id, id, skill_version_id);


--
-- Name: skill_test_runs skill_test_runs_version_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_test_runs
    ADD CONSTRAINT skill_test_runs_version_fk FOREIGN KEY (tenant_id, version_id) REFERENCES public.skill_versions(tenant_id, id);


--
-- Name: skill_usage_events skill_usage_events_binding_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_usage_events
    ADD CONSTRAINT skill_usage_events_binding_fk FOREIGN KEY (tenant_id, binding_id) REFERENCES public.skill_bindings(tenant_id, id);


--
-- Name: skill_usage_events skill_usage_events_session_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_usage_events
    ADD CONSTRAINT skill_usage_events_session_fk FOREIGN KEY (tenant_id, session_id) REFERENCES public.sessions(tenant_id, id);


--
-- Name: skill_usage_events skill_usage_events_version_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_usage_events
    ADD CONSTRAINT skill_usage_events_version_fk FOREIGN KEY (tenant_id, version_id) REFERENCES public.skill_versions(tenant_id, id);


--
-- Name: skill_version_files skill_version_files_object_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_version_files
    ADD CONSTRAINT skill_version_files_object_fk FOREIGN KEY (tenant_id, object_hash) REFERENCES public.vedaflow_objects(tenant_id, hash);


--
-- Name: skill_version_files skill_version_files_version_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_version_files
    ADD CONSTRAINT skill_version_files_version_fk FOREIGN KEY (tenant_id, version_id) REFERENCES public.skill_versions(tenant_id, id);


--
-- Name: skill_versions skill_versions_skill_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skill_versions
    ADD CONSTRAINT skill_versions_skill_fk FOREIGN KEY (tenant_id, skill_id) REFERENCES public.skills(tenant_id, id);


--
-- Name: skills skills_current_version_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skills
    ADD CONSTRAINT skills_current_version_fk FOREIGN KEY (tenant_id, id, current_version_id) REFERENCES public.skill_versions(tenant_id, skill_id, id) DEFERRABLE INITIALLY DEFERRED;


--
-- Name: skills skills_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skills
    ADD CONSTRAINT skills_scope_fk FOREIGN KEY (tenant_id, governing_scope_id) REFERENCES public.scopes(tenant_id, id);


--
-- Name: skills skills_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.skills
    ADD CONSTRAINT skills_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: source_cleanup_targets source_cleanup_targets_tenant_id_change_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_cleanup_targets
    ADD CONSTRAINT source_cleanup_targets_tenant_id_change_id_fkey FOREIGN KEY (tenant_id, change_id) REFERENCES public.knowledge_changes(tenant_id, proposal_id);


--
-- Name: source_cleanup_targets source_cleanup_targets_tenant_id_event_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_cleanup_targets
    ADD CONSTRAINT source_cleanup_targets_tenant_id_event_id_fkey FOREIGN KEY (tenant_id, event_id) REFERENCES public.source_suppressions(tenant_id, event_id);


--
-- Name: source_cleanup_targets source_cleanup_targets_tenant_id_operation_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_cleanup_targets
    ADD CONSTRAINT source_cleanup_targets_tenant_id_operation_id_fkey FOREIGN KEY (tenant_id, operation_id) REFERENCES public.durable_operations(tenant_id, id);


--
-- Name: source_observation_paths source_observation_paths_tenant_id_artifact_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_observation_paths
    ADD CONSTRAINT source_observation_paths_tenant_id_artifact_id_fkey FOREIGN KEY (tenant_id, artifact_id) REFERENCES public.source_artifact_addresses(tenant_id, artifact_id);


--
-- Name: source_observation_paths source_observation_paths_tenant_id_observation_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_observation_paths
    ADD CONSTRAINT source_observation_paths_tenant_id_observation_id_fkey FOREIGN KEY (tenant_id, observation_id) REFERENCES public.source_observations(tenant_id, id);


--
-- Name: source_observations source_observations_tenant_id_import_job_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_observations
    ADD CONSTRAINT source_observations_tenant_id_import_job_id_fkey FOREIGN KEY (tenant_id, import_job_id) REFERENCES public.import_jobs(tenant_id, id);


--
-- Name: source_observations source_observations_tenant_id_operation_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_observations
    ADD CONSTRAINT source_observations_tenant_id_operation_id_fkey FOREIGN KEY (tenant_id, operation_id) REFERENCES public.source_reconcile_requests(tenant_id, operation_id);


--
-- Name: source_observations source_observations_tenant_id_previous_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_observations
    ADD CONSTRAINT source_observations_tenant_id_previous_id_fkey FOREIGN KEY (tenant_id, previous_id) REFERENCES public.source_observations(tenant_id, id);


--
-- Name: source_observations source_observations_tenant_id_project_id_source_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_observations
    ADD CONSTRAINT source_observations_tenant_id_project_id_source_id_fkey FOREIGN KEY (tenant_id, project_id, source_id) REFERENCES public.source_reconcile_heads(tenant_id, project_id, source_id);


--
-- Name: source_reconcile_heads source_reconcile_heads_tenant_id_active_operation_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_reconcile_heads
    ADD CONSTRAINT source_reconcile_heads_tenant_id_active_operation_id_fkey FOREIGN KEY (tenant_id, active_operation_id) REFERENCES public.durable_operations(tenant_id, id);


--
-- Name: source_reconcile_heads source_reconcile_heads_tenant_id_last_operation_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_reconcile_heads
    ADD CONSTRAINT source_reconcile_heads_tenant_id_last_operation_id_fkey FOREIGN KEY (tenant_id, last_operation_id) REFERENCES public.durable_operations(tenant_id, id);


--
-- Name: source_reconcile_heads source_reconcile_heads_tenant_id_observation_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_reconcile_heads
    ADD CONSTRAINT source_reconcile_heads_tenant_id_observation_id_fkey FOREIGN KEY (tenant_id, observation_id) REFERENCES public.source_observations(tenant_id, id);


--
-- Name: source_reconcile_heads source_reconcile_heads_tenant_id_project_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_reconcile_heads
    ADD CONSTRAINT source_reconcile_heads_tenant_id_project_id_fkey FOREIGN KEY (tenant_id, project_id) REFERENCES public.projects(tenant_id, id);


--
-- Name: source_reconcile_heads source_reconcile_heads_tenant_id_scope_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_reconcile_heads
    ADD CONSTRAINT source_reconcile_heads_tenant_id_scope_id_fkey FOREIGN KEY (tenant_id, scope_id) REFERENCES public.scopes(tenant_id, id);


--
-- Name: source_reconcile_requests source_reconcile_requests_tenant_id_configuration_version__fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_reconcile_requests
    ADD CONSTRAINT source_reconcile_requests_tenant_id_configuration_version__fkey FOREIGN KEY (tenant_id, configuration_version_id) REFERENCES public.configuration_versions(tenant_id, id);


--
-- Name: source_reconcile_requests source_reconcile_requests_tenant_id_operation_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_reconcile_requests
    ADD CONSTRAINT source_reconcile_requests_tenant_id_operation_id_fkey FOREIGN KEY (tenant_id, operation_id) REFERENCES public.durable_operations(tenant_id, id);


--
-- Name: source_reconcile_requests source_reconcile_requests_tenant_id_previous_observation_i_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_reconcile_requests
    ADD CONSTRAINT source_reconcile_requests_tenant_id_previous_observation_i_fkey FOREIGN KEY (tenant_id, previous_observation_id) REFERENCES public.source_observations(tenant_id, id);


--
-- Name: source_reconcile_requests source_reconcile_requests_tenant_id_project_id_source_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_reconcile_requests
    ADD CONSTRAINT source_reconcile_requests_tenant_id_project_id_source_id_fkey FOREIGN KEY (tenant_id, project_id, source_id) REFERENCES public.source_reconcile_heads(tenant_id, project_id, source_id);


--
-- Name: source_repository_attachments source_repository_attachments_tenant_id_project_id_github__fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_repository_attachments
    ADD CONSTRAINT source_repository_attachments_tenant_id_project_id_github__fkey FOREIGN KEY (tenant_id, project_id, github_repository_id) REFERENCES public.source_repository_roots(tenant_id, project_id, github_repository_id);


--
-- Name: source_repository_roots source_repository_roots_tenant_id_project_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_repository_roots
    ADD CONSTRAINT source_repository_roots_tenant_id_project_id_fkey FOREIGN KEY (tenant_id, project_id) REFERENCES public.projects(tenant_id, id);


--
-- Name: source_suppressions source_suppressions_tenant_id_change_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_suppressions
    ADD CONSTRAINT source_suppressions_tenant_id_change_id_fkey FOREIGN KEY (tenant_id, change_id) REFERENCES public.knowledge_changes(tenant_id, proposal_id);


--
-- Name: source_suppressions source_suppressions_tenant_id_event_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_suppressions
    ADD CONSTRAINT source_suppressions_tenant_id_event_id_fkey FOREIGN KEY (tenant_id, event_id) REFERENCES public.source_event_addresses(tenant_id, event_id);


--
-- Name: source_suppressions source_suppressions_tenant_id_forget_change_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.source_suppressions
    ADD CONSTRAINT source_suppressions_tenant_id_forget_change_id_fkey FOREIGN KEY (tenant_id, forget_change_id) REFERENCES public.knowledge_changes(tenant_id, proposal_id);


--
-- Name: tenant_administrator_bootstraps tenant_administrator_bootstraps_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tenant_administrator_bootstraps
    ADD CONSTRAINT tenant_administrator_bootstraps_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: tenant_keys tenant_keys_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tenant_keys
    ADD CONSTRAINT tenant_keys_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: tenant_secret_reencryption_jobs tenant_secret_reencrypt_jobs_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tenant_secret_reencryption_jobs
    ADD CONSTRAINT tenant_secret_reencrypt_jobs_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: tenant_secrets tenant_secrets_key_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tenant_secrets
    ADD CONSTRAINT tenant_secrets_key_fk FOREIGN KEY (tenant_id, key_version) REFERENCES public.tenant_keys(tenant_id, version);


--
-- Name: tenant_secrets tenant_secrets_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tenant_secrets
    ADD CONSTRAINT tenant_secrets_scope_fk FOREIGN KEY (tenant_id, scope_id) REFERENCES public.scopes(tenant_id, id);


--
-- Name: tenant_secrets tenant_secrets_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tenant_secrets
    ADD CONSTRAINT tenant_secrets_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: tool_bindings tool_bindings_project_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_bindings
    ADD CONSTRAINT tool_bindings_project_fk FOREIGN KEY (tenant_id, project_id) REFERENCES public.projects(tenant_id, id);


--
-- Name: tool_bindings tool_bindings_server_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_bindings
    ADD CONSTRAINT tool_bindings_server_fk FOREIGN KEY (tenant_id, server_id) REFERENCES public.tool_servers(tenant_id, id);


--
-- Name: tool_bindings tool_bindings_version_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_bindings
    ADD CONSTRAINT tool_bindings_version_fk FOREIGN KEY (tenant_id, server_id, version_id) REFERENCES public.tool_server_versions(tenant_id, server_id, id);


--
-- Name: tool_changes tool_changes_proposal_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_changes
    ADD CONSTRAINT tool_changes_proposal_fk FOREIGN KEY (tenant_id, proposal_id) REFERENCES public.vedaflow_proposals(tenant_id, id);


--
-- Name: tool_changes tool_changes_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_changes
    ADD CONSTRAINT tool_changes_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: tool_server_versions tool_server_versions_proposal_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_server_versions
    ADD CONSTRAINT tool_server_versions_proposal_fk FOREIGN KEY (tenant_id, proposal_id) REFERENCES public.vedaflow_proposals(tenant_id, id);


--
-- Name: tool_server_versions tool_server_versions_server_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_server_versions
    ADD CONSTRAINT tool_server_versions_server_fk FOREIGN KEY (tenant_id, server_id) REFERENCES public.tool_servers(tenant_id, id);


--
-- Name: tool_servers tool_servers_current_version_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_servers
    ADD CONSTRAINT tool_servers_current_version_fk FOREIGN KEY (tenant_id, id, current_version_id) REFERENCES public.tool_server_versions(tenant_id, server_id, id) DEFERRABLE INITIALLY DEFERRED;


--
-- Name: tool_servers tool_servers_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_servers
    ADD CONSTRAINT tool_servers_scope_fk FOREIGN KEY (tenant_id, governing_scope_id) REFERENCES public.scopes(tenant_id, id);


--
-- Name: tool_servers tool_servers_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_servers
    ADD CONSTRAINT tool_servers_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: tool_test_runs tool_test_runs_version_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tool_test_runs
    ADD CONSTRAINT tool_test_runs_version_fk FOREIGN KEY (tenant_id, version_id) REFERENCES public.tool_server_versions(tenant_id, id);


--
-- Name: unproven_context_deliveries unproven_context_deliveries_tenant_id_context_run_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.unproven_context_deliveries
    ADD CONSTRAINT unproven_context_deliveries_tenant_id_context_run_id_fkey FOREIGN KEY (tenant_id, context_run_id) REFERENCES public.session_context_runs(tenant_id, id) ON DELETE CASCADE;


--
-- Name: vedaflow_commit_parents vedaflow_commit_parents_commit_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_commit_parents
    ADD CONSTRAINT vedaflow_commit_parents_commit_fk FOREIGN KEY (tenant_id, commit_hash) REFERENCES public.vedaflow_commits(tenant_id, hash);


--
-- Name: vedaflow_commit_parents vedaflow_commit_parents_parent_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_commit_parents
    ADD CONSTRAINT vedaflow_commit_parents_parent_fk FOREIGN KEY (tenant_id, parent_hash) REFERENCES public.vedaflow_commits(tenant_id, hash);


--
-- Name: vedaflow_commits vedaflow_commits_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_commits
    ADD CONSTRAINT vedaflow_commits_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: vedaflow_commits vedaflow_commits_tree_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_commits
    ADD CONSTRAINT vedaflow_commits_tree_fk FOREIGN KEY (tenant_id, tree_hash) REFERENCES public.vedaflow_trees(tenant_id, hash);


--
-- Name: vedaflow_objects vedaflow_objects_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_objects
    ADD CONSTRAINT vedaflow_objects_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: vedaflow_proposal_approvals vedaflow_proposal_approvals_commit_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_proposal_approvals
    ADD CONSTRAINT vedaflow_proposal_approvals_commit_fk FOREIGN KEY (tenant_id, commit_hash) REFERENCES public.vedaflow_commits(tenant_id, hash);


--
-- Name: vedaflow_proposal_approvals vedaflow_proposal_approvals_proposal_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_proposal_approvals
    ADD CONSTRAINT vedaflow_proposal_approvals_proposal_fk FOREIGN KEY (tenant_id, proposal_id) REFERENCES public.vedaflow_proposals(tenant_id, id);


--
-- Name: vedaflow_proposal_approvals vedaflow_proposal_approvals_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_proposal_approvals
    ADD CONSTRAINT vedaflow_proposal_approvals_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: vedaflow_proposals vedaflow_proposals_commit_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_proposals
    ADD CONSTRAINT vedaflow_proposals_commit_fk FOREIGN KEY (tenant_id, commit_hash) REFERENCES public.vedaflow_commits(tenant_id, hash);


--
-- Name: vedaflow_proposals vedaflow_proposals_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_proposals
    ADD CONSTRAINT vedaflow_proposals_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: vedaflow_refs vedaflow_refs_commit_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_refs
    ADD CONSTRAINT vedaflow_refs_commit_fk FOREIGN KEY (tenant_id, commit_hash) REFERENCES public.vedaflow_commits(tenant_id, hash);


--
-- Name: vedaflow_refs vedaflow_refs_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_refs
    ADD CONSTRAINT vedaflow_refs_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: vedaflow_tree_entries vedaflow_tree_entries_object_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_tree_entries
    ADD CONSTRAINT vedaflow_tree_entries_object_fk FOREIGN KEY (tenant_id, object_hash) REFERENCES public.vedaflow_objects(tenant_id, hash);


--
-- Name: vedaflow_tree_entries vedaflow_tree_entries_subtree_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_tree_entries
    ADD CONSTRAINT vedaflow_tree_entries_subtree_fk FOREIGN KEY (tenant_id, subtree_hash) REFERENCES public.vedaflow_trees(tenant_id, hash);


--
-- Name: vedaflow_tree_entries vedaflow_tree_entries_tree_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_tree_entries
    ADD CONSTRAINT vedaflow_tree_entries_tree_fk FOREIGN KEY (tenant_id, tree_hash) REFERENCES public.vedaflow_trees(tenant_id, hash);


--
-- Name: vedaflow_trees vedaflow_trees_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.vedaflow_trees
    ADD CONSTRAINT vedaflow_trees_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: workspaces workspaces_scope_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.workspaces
    ADD CONSTRAINT workspaces_scope_fk FOREIGN KEY (tenant_id, scope_id, scope_kind, slug) REFERENCES public.scopes(tenant_id, id, kind, slug);


--
-- Name: workspaces workspaces_tenant_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.workspaces
    ADD CONSTRAINT workspaces_tenant_fk FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);


--
-- Name: source_artifact_addresses artifact_address_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY artifact_address_tenant ON public.source_artifact_addresses USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: artifact_cleanup_targets; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.artifact_cleanup_targets ENABLE ROW LEVEL SECURITY;

--
-- Name: artifact_cleanup_targets artifact_cleanup_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY artifact_cleanup_tenant ON public.artifact_cleanup_targets USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: artifact_dependencies; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.artifact_dependencies ENABLE ROW LEVEL SECURITY;

--
-- Name: artifact_dependencies artifact_dependency_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY artifact_dependency_tenant ON public.artifact_dependencies USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: import_artifacts artifact_source_eligibility; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY artifact_source_eligibility ON public.import_artifacts AS RESTRICTIVE FOR SELECT USING ((((NOT content_erased) AND public.synveda_source_available(tenant_id, 'import_artifact'::text, id)) OR public.synveda_artifact_cleanup_authorized(tenant_id, 'import_artifact'::text, id)));


--
-- Name: artifact_suppressions artifact_suppression_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY artifact_suppression_tenant ON public.artifact_suppressions USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: artifact_suppressions; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.artifact_suppressions ENABLE ROW LEVEL SECURITY;

--
-- Name: audit_chain_heads; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.audit_chain_heads ENABLE ROW LEVEL SECURITY;

--
-- Name: audit_chain_heads audit_chain_heads_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY audit_chain_heads_tenant_isolation ON public.audit_chain_heads USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: audit_log; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.audit_log ENABLE ROW LEVEL SECURITY;

--
-- Name: audit_log audit_log_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY audit_log_tenant_isolation ON public.audit_log USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: capture_candidates candidate_source_eligibility; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY candidate_source_eligibility ON public.capture_candidates AS RESTRICTIVE FOR SELECT USING (public.synveda_source_available(tenant_id, 'capture_candidate'::text, id));


--
-- Name: capability_snapshots; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.capability_snapshots ENABLE ROW LEVEL SECURITY;

--
-- Name: capability_snapshots capability_snapshots_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY capability_snapshots_tenant_isolation ON public.capability_snapshots USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: capture_batch_events; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.capture_batch_events ENABLE ROW LEVEL SECURITY;

--
-- Name: capture_batch_events capture_batch_events_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY capture_batch_events_tenant_isolation ON public.capture_batch_events USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: capture_batches; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.capture_batches ENABLE ROW LEVEL SECURITY;

--
-- Name: capture_batches capture_batches_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY capture_batches_tenant_isolation ON public.capture_batches USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: capture_candidate_decisions; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.capture_candidate_decisions ENABLE ROW LEVEL SECURITY;

--
-- Name: capture_candidate_decisions capture_candidate_decisions_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY capture_candidate_decisions_tenant_isolation ON public.capture_candidate_decisions USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: capture_candidate_events; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.capture_candidate_events ENABLE ROW LEVEL SECURITY;

--
-- Name: capture_candidate_events capture_candidate_events_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY capture_candidate_events_tenant_isolation ON public.capture_candidate_events USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: capture_candidate_import_artifacts; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.capture_candidate_import_artifacts ENABLE ROW LEVEL SECURITY;

--
-- Name: capture_candidate_import_artifacts capture_candidate_import_artifacts_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY capture_candidate_import_artifacts_tenant_isolation ON public.capture_candidate_import_artifacts USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: capture_candidate_matches; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.capture_candidate_matches ENABLE ROW LEVEL SECURITY;

--
-- Name: capture_candidate_matches capture_candidate_matches_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY capture_candidate_matches_tenant_isolation ON public.capture_candidate_matches USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: capture_candidates; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.capture_candidates ENABLE ROW LEVEL SECURITY;

--
-- Name: capture_candidates capture_candidates_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY capture_candidates_tenant_isolation ON public.capture_candidates USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: knowledge_changes change_source_eligibility; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY change_source_eligibility ON public.knowledge_changes AS RESTRICTIVE FOR SELECT USING (((command_kind = ANY (ARRAY['control_source'::text, 'control_artifact'::text])) OR public.synveda_source_available(tenant_id, 'knowledge_change'::text, proposal_id)));


--
-- Name: configuration_artifacts; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.configuration_artifacts ENABLE ROW LEVEL SECURITY;

--
-- Name: configuration_artifacts configuration_artifacts_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY configuration_artifacts_tenant_isolation ON public.configuration_artifacts USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: configuration_bindings; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.configuration_bindings ENABLE ROW LEVEL SECURITY;

--
-- Name: configuration_bindings configuration_bindings_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY configuration_bindings_tenant_isolation ON public.configuration_bindings USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: configuration_changes; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.configuration_changes ENABLE ROW LEVEL SECURITY;

--
-- Name: configuration_changes configuration_changes_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY configuration_changes_tenant_isolation ON public.configuration_changes USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: configuration_versions; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.configuration_versions ENABLE ROW LEVEL SECURITY;

--
-- Name: configuration_versions configuration_versions_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY configuration_versions_tenant_isolation ON public.configuration_versions USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: context_candidates; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.context_candidates ENABLE ROW LEVEL SECURITY;

--
-- Name: context_candidates context_candidates_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY context_candidates_tenant_isolation ON public.context_candidates USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: context_run_delivery_dependencies context_delivery_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY context_delivery_tenant ON public.context_run_delivery_dependencies USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: context_feedback; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.context_feedback ENABLE ROW LEVEL SECURITY;

--
-- Name: context_feedback context_feedback_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY context_feedback_tenant_isolation ON public.context_feedback USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: context_graph_steps; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.context_graph_steps ENABLE ROW LEVEL SECURITY;

--
-- Name: context_graph_steps context_graph_steps_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY context_graph_steps_tenant_isolation ON public.context_graph_steps USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: context_pack_chunks; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.context_pack_chunks ENABLE ROW LEVEL SECURITY;

--
-- Name: context_pack_chunks context_pack_chunks_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY context_pack_chunks_tenant_isolation ON public.context_pack_chunks USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: context_pack_documents; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.context_pack_documents ENABLE ROW LEVEL SECURITY;

--
-- Name: context_pack_documents context_pack_documents_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY context_pack_documents_tenant_isolation ON public.context_pack_documents USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: context_packs; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.context_packs ENABLE ROW LEVEL SECURITY;

--
-- Name: context_packs context_packs_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY context_packs_tenant_isolation ON public.context_packs USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: context_run_delivery_dependencies; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.context_run_delivery_dependencies ENABLE ROW LEVEL SECURITY;

--
-- Name: context_selections; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.context_selections ENABLE ROW LEVEL SECURITY;

--
-- Name: context_selections context_selections_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY context_selections_tenant_isolation ON public.context_selections USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: session_context_runs context_source_eligibility; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY context_source_eligibility ON public.session_context_runs AS RESTRICTIVE FOR SELECT USING ((((NOT (EXISTS ( SELECT 1
   FROM public.unproven_context_deliveries unproven
  WHERE ((unproven.tenant_id = session_context_runs.tenant_id) AND (unproven.context_run_id = session_context_runs.id))))) OR (rendered = ''::text) OR public.synveda_item_cleanup_authorized(tenant_id, 'context_run'::text, id)) AND public.synveda_source_available(tenant_id, 'context_run'::text, id)));


--
-- Name: capture_candidate_decisions decision_source_eligibility; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY decision_source_eligibility ON public.capture_candidate_decisions AS RESTRICTIVE FOR SELECT USING (public.synveda_source_available(tenant_id, 'capture_candidate'::text, candidate_id));


--
-- Name: knowledge_sources descriptor_source_eligibility; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY descriptor_source_eligibility ON public.knowledge_sources AS RESTRICTIVE FOR SELECT USING ((((NOT content_erased) AND public.synveda_source_available(tenant_id, 'knowledge_source'::text, id)) OR public.synveda_source_cleanup_authorized(tenant_id, 'knowledge_source'::text, id) OR public.synveda_artifact_cleanup_authorized(tenant_id, 'knowledge_source'::text, id) OR public.synveda_item_cleanup_authorized(tenant_id, 'knowledge_source'::text, id)));


--
-- Name: directory_sync_state; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.directory_sync_state ENABLE ROW LEVEL SECURITY;

--
-- Name: directory_sync_state directory_sync_state_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY directory_sync_state_tenant_isolation ON public.directory_sync_state USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: durable_operations; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.durable_operations ENABLE ROW LEVEL SECURITY;

--
-- Name: durable_operations durable_operations_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY durable_operations_tenant_isolation ON public.durable_operations USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: knowledge_revision_embeddings embedding_source_admission; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY embedding_source_admission ON public.knowledge_revision_embeddings AS RESTRICTIVE FOR INSERT WITH CHECK (public.synveda_source_admissible(tenant_id, 'knowledge_revision'::text, knowledge_revision_id));


--
-- Name: knowledge_revision_embeddings embedding_source_eligibility; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY embedding_source_eligibility ON public.knowledge_revision_embeddings AS RESTRICTIVE FOR SELECT USING (public.synveda_source_available(tenant_id, 'knowledge_revision'::text, knowledge_revision_id));


--
-- Name: group_members; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.group_members ENABLE ROW LEVEL SECURITY;

--
-- Name: group_members group_members_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY group_members_tenant_isolation ON public.group_members USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: groups; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.groups ENABLE ROW LEVEL SECURITY;

--
-- Name: groups groups_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY groups_tenant_isolation ON public.groups USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: historical_knowledge_dependencies; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.historical_knowledge_dependencies ENABLE ROW LEVEL SECURITY;

--
-- Name: historical_knowledge_dependencies historical_knowledge_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY historical_knowledge_tenant ON public.historical_knowledge_dependencies USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: idempotency_records; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.idempotency_records ENABLE ROW LEVEL SECURITY;

--
-- Name: idempotency_records idempotency_records_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY idempotency_records_tenant_isolation ON public.idempotency_records USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: identities; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.identities ENABLE ROW LEVEL SECURITY;

--
-- Name: identities identities_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY identities_tenant_isolation ON public.identities USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: import_artifacts; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.import_artifacts ENABLE ROW LEVEL SECURITY;

--
-- Name: import_artifacts import_artifacts_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY import_artifacts_tenant_isolation ON public.import_artifacts USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: import_jobs; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.import_jobs ENABLE ROW LEVEL SECURITY;

--
-- Name: import_jobs import_jobs_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY import_jobs_tenant_isolation ON public.import_jobs USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: import_mappings; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.import_mappings ENABLE ROW LEVEL SECURITY;

--
-- Name: import_mappings import_mappings_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY import_mappings_tenant_isolation ON public.import_mappings USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: knowledge_change_headers knowledge_change_header_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY knowledge_change_header_tenant ON public.knowledge_change_headers USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: knowledge_change_headers; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.knowledge_change_headers ENABLE ROW LEVEL SECURITY;

--
-- Name: knowledge_changes; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.knowledge_changes ENABLE ROW LEVEL SECURITY;

--
-- Name: knowledge_changes knowledge_changes_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY knowledge_changes_tenant_isolation ON public.knowledge_changes USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: knowledge_conflict_members; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.knowledge_conflict_members ENABLE ROW LEVEL SECURITY;

--
-- Name: knowledge_conflict_members knowledge_conflict_members_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY knowledge_conflict_members_tenant_isolation ON public.knowledge_conflict_members USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: knowledge_conflict_sets; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.knowledge_conflict_sets ENABLE ROW LEVEL SECURITY;

--
-- Name: knowledge_conflict_sets knowledge_conflict_sets_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY knowledge_conflict_sets_tenant_isolation ON public.knowledge_conflict_sets USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: knowledge_copy_dependencies; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.knowledge_copy_dependencies ENABLE ROW LEVEL SECURITY;

--
-- Name: knowledge_copy_dependencies knowledge_copy_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY knowledge_copy_tenant ON public.knowledge_copy_dependencies USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: knowledge_erasure_tombstones; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.knowledge_erasure_tombstones ENABLE ROW LEVEL SECURITY;

--
-- Name: knowledge_erasure_tombstones knowledge_erasure_tombstones_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY knowledge_erasure_tombstones_tenant_isolation ON public.knowledge_erasure_tombstones USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: knowledge_index_invalidations; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.knowledge_index_invalidations ENABLE ROW LEVEL SECURITY;

--
-- Name: knowledge_index_invalidations knowledge_index_invalidations_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY knowledge_index_invalidations_tenant_isolation ON public.knowledge_index_invalidations USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: knowledge_items; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.knowledge_items ENABLE ROW LEVEL SECURITY;

--
-- Name: knowledge_items_history; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.knowledge_items_history ENABLE ROW LEVEL SECURITY;

--
-- Name: knowledge_items_history knowledge_items_history_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY knowledge_items_history_tenant_isolation ON public.knowledge_items_history USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: knowledge_items knowledge_items_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY knowledge_items_tenant_isolation ON public.knowledge_items USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: knowledge_relations; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.knowledge_relations ENABLE ROW LEVEL SECURITY;

--
-- Name: knowledge_relations knowledge_relations_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY knowledge_relations_tenant_isolation ON public.knowledge_relations USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: knowledge_revision_embeddings; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.knowledge_revision_embeddings ENABLE ROW LEVEL SECURITY;

--
-- Name: knowledge_revision_embeddings knowledge_revision_embeddings_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY knowledge_revision_embeddings_tenant_isolation ON public.knowledge_revision_embeddings USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: knowledge_revision_headers knowledge_revision_header_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY knowledge_revision_header_tenant ON public.knowledge_revision_headers USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: knowledge_revision_headers; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.knowledge_revision_headers ENABLE ROW LEVEL SECURITY;

--
-- Name: knowledge_revision_source_headers knowledge_revision_source_header_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY knowledge_revision_source_header_tenant ON public.knowledge_revision_source_headers USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: knowledge_revision_source_headers; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.knowledge_revision_source_headers ENABLE ROW LEVEL SECURITY;

--
-- Name: knowledge_revision_sources; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.knowledge_revision_sources ENABLE ROW LEVEL SECURITY;

--
-- Name: knowledge_revision_sources knowledge_revision_sources_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY knowledge_revision_sources_tenant_isolation ON public.knowledge_revision_sources USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: knowledge_revisions; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.knowledge_revisions ENABLE ROW LEVEL SECURITY;

--
-- Name: knowledge_revisions knowledge_revisions_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY knowledge_revisions_tenant_isolation ON public.knowledge_revisions USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: knowledge_sources; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.knowledge_sources ENABLE ROW LEVEL SECURITY;

--
-- Name: knowledge_sources knowledge_sources_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY knowledge_sources_tenant_isolation ON public.knowledge_sources USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: import_mappings mapping_source_eligibility; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY mapping_source_eligibility ON public.import_mappings AS RESTRICTIVE FOR SELECT USING ((public.synveda_source_available(tenant_id, 'import_mapping'::text, id) OR public.synveda_item_cleanup_authorized(tenant_id, 'knowledge_item'::text, matched_item_id) OR public.synveda_item_cleanup_authorized(tenant_id, 'capture_candidate'::text, candidate_id)));


--
-- Name: operation_attempts; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.operation_attempts ENABLE ROW LEVEL SECURITY;

--
-- Name: operation_attempts operation_attempts_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY operation_attempts_tenant_isolation ON public.operation_attempts USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: operation_outbox; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.operation_outbox ENABLE ROW LEVEL SECURITY;

--
-- Name: operation_outbox operation_outbox_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY operation_outbox_tenant_isolation ON public.operation_outbox USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: pending_invites; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.pending_invites ENABLE ROW LEVEL SECURITY;

--
-- Name: pending_invites pending_invites_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY pending_invites_tenant_isolation ON public.pending_invites USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: policy_packs; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.policy_packs ENABLE ROW LEVEL SECURITY;

--
-- Name: policy_packs policy_packs_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY policy_packs_tenant_isolation ON public.policy_packs USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: policy_relaxation_changes; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.policy_relaxation_changes ENABLE ROW LEVEL SECURITY;

--
-- Name: policy_relaxation_changes policy_relaxation_changes_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY policy_relaxation_changes_tenant_isolation ON public.policy_relaxation_changes USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: policy_relaxation_versions; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.policy_relaxation_versions ENABLE ROW LEVEL SECURITY;

--
-- Name: policy_relaxation_versions policy_relaxation_versions_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY policy_relaxation_versions_tenant_isolation ON public.policy_relaxation_versions USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: policy_relaxations; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.policy_relaxations ENABLE ROW LEVEL SECURITY;

--
-- Name: policy_relaxations policy_relaxations_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY policy_relaxations_tenant_isolation ON public.policy_relaxations USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: procedure_evaluation_attempts procedure_attempt_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY procedure_attempt_tenant ON public.procedure_evaluation_attempts USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: procedure_candidate_headers; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.procedure_candidate_headers ENABLE ROW LEVEL SECURITY;

--
-- Name: procedure_candidate_headers procedure_candidate_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY procedure_candidate_tenant ON public.procedure_candidate_headers USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: procedure_evaluation_attempts; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.procedure_evaluation_attempts ENABLE ROW LEVEL SECURITY;

--
-- Name: procedure_evaluation_results; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.procedure_evaluation_results ENABLE ROW LEVEL SECURITY;

--
-- Name: procedure_improvement_headers; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.procedure_improvement_headers ENABLE ROW LEVEL SECURITY;

--
-- Name: procedure_improvement_headers procedure_improvement_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY procedure_improvement_tenant ON public.procedure_improvement_headers USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: procedure_outcome_headers; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.procedure_outcome_headers ENABLE ROW LEVEL SECURITY;

--
-- Name: procedure_outcome_headers procedure_outcome_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY procedure_outcome_tenant ON public.procedure_outcome_headers USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: procedure_evaluation_results procedure_result_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY procedure_result_tenant ON public.procedure_evaluation_results USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: project_repositories; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.project_repositories ENABLE ROW LEVEL SECURITY;

--
-- Name: project_repositories project_repositories_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY project_repositories_tenant_isolation ON public.project_repositories USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: projects; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.projects ENABLE ROW LEVEL SECURITY;

--
-- Name: projects projects_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY projects_tenant_isolation ON public.projects USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: prompts; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.prompts ENABLE ROW LEVEL SECURITY;

--
-- Name: prompts prompts_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY prompts_tenant_isolation ON public.prompts USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: knowledge_relations relation_source_eligibility; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY relation_source_eligibility ON public.knowledge_relations AS RESTRICTIVE FOR SELECT USING ((public.synveda_source_available(tenant_id, 'knowledge_revision'::text, asserting_revision_id) OR public.synveda_item_cleanup_authorized(tenant_id, 'knowledge_item'::text, source_item_id) OR public.synveda_item_cleanup_authorized(tenant_id, 'knowledge_item'::text, target_item_id)));


--
-- Name: knowledge_revisions revision_source_eligibility; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY revision_source_eligibility ON public.knowledge_revisions AS RESTRICTIVE FOR SELECT USING ((((NOT content_erased) AND public.synveda_source_available(tenant_id, 'knowledge_revision'::text, id)) OR public.synveda_source_cleanup_authorized(tenant_id, 'knowledge_revision'::text, id) OR public.synveda_artifact_cleanup_authorized(tenant_id, 'knowledge_revision'::text, id) OR public.synveda_item_cleanup_authorized(tenant_id, 'knowledge_revision'::text, id)));


--
-- Name: scim_credentials; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.scim_credentials ENABLE ROW LEVEL SECURITY;

--
-- Name: scim_credentials scim_credentials_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY scim_credentials_tenant_isolation ON public.scim_credentials USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: scim_users; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.scim_users ENABLE ROW LEVEL SECURITY;

--
-- Name: scim_users scim_users_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY scim_users_tenant_isolation ON public.scim_users USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: scope_closure; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.scope_closure ENABLE ROW LEVEL SECURITY;

--
-- Name: scope_closure scope_closure_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY scope_closure_tenant_isolation ON public.scope_closure USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: scope_grants; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.scope_grants ENABLE ROW LEVEL SECURITY;

--
-- Name: scope_grants scope_grants_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY scope_grants_tenant_isolation ON public.scope_grants USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: scopes; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.scopes ENABLE ROW LEVEL SECURITY;

--
-- Name: scopes scopes_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY scopes_tenant_isolation ON public.scopes USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: session_context_runs; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.session_context_runs ENABLE ROW LEVEL SECURITY;

--
-- Name: session_context_runs session_context_runs_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY session_context_runs_tenant_isolation ON public.session_context_runs USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: session_event_quarantine; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.session_event_quarantine ENABLE ROW LEVEL SECURITY;

--
-- Name: session_event_quarantine session_event_quarantine_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY session_event_quarantine_tenant_isolation ON public.session_event_quarantine USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: session_events; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.session_events ENABLE ROW LEVEL SECURITY;

--
-- Name: session_events session_events_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY session_events_tenant_isolation ON public.session_events USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: session_snapshot_headers; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.session_snapshot_headers ENABLE ROW LEVEL SECURITY;

--
-- Name: session_events session_source_eligibility; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY session_source_eligibility ON public.session_events AS RESTRICTIVE FOR SELECT USING ((((NOT content_erased) AND public.synveda_source_available(tenant_id, 'session_event'::text, id)) OR public.synveda_source_cleanup_authorized(tenant_id, 'session_event'::text, id) OR public.synveda_artifact_cleanup_authorized(tenant_id, 'session_event'::text, id) OR public.synveda_item_cleanup_authorized(tenant_id, 'session_event'::text, id)));


--
-- Name: sessions; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.sessions ENABLE ROW LEVEL SECURITY;

--
-- Name: sessions sessions_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY sessions_tenant_isolation ON public.sessions USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: skill_bindings; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.skill_bindings ENABLE ROW LEVEL SECURITY;

--
-- Name: skill_bindings skill_bindings_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY skill_bindings_tenant_isolation ON public.skill_bindings USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: skill_changes; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.skill_changes ENABLE ROW LEVEL SECURITY;

--
-- Name: skill_changes skill_changes_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY skill_changes_tenant_isolation ON public.skill_changes USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: skill_test_runs; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.skill_test_runs ENABLE ROW LEVEL SECURITY;

--
-- Name: skill_test_runs skill_test_runs_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY skill_test_runs_tenant_isolation ON public.skill_test_runs USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: skill_usage_events; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.skill_usage_events ENABLE ROW LEVEL SECURITY;

--
-- Name: skill_usage_events skill_usage_events_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY skill_usage_events_tenant_isolation ON public.skill_usage_events USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: skill_version_files; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.skill_version_files ENABLE ROW LEVEL SECURITY;

--
-- Name: skill_version_files skill_version_files_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY skill_version_files_tenant_isolation ON public.skill_version_files USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: skill_versions; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.skill_versions ENABLE ROW LEVEL SECURITY;

--
-- Name: skill_versions skill_versions_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY skill_versions_tenant_isolation ON public.skill_versions USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: skills; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.skills ENABLE ROW LEVEL SECURITY;

--
-- Name: skills skills_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY skills_tenant_isolation ON public.skills USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: session_snapshot_headers snapshot_header_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY snapshot_header_tenant ON public.session_snapshot_headers USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: source_artifact_addresses; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.source_artifact_addresses ENABLE ROW LEVEL SECURITY;

--
-- Name: source_cleanup_targets source_cleanup_target_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY source_cleanup_target_tenant ON public.source_cleanup_targets USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: source_cleanup_targets; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.source_cleanup_targets ENABLE ROW LEVEL SECURITY;

--
-- Name: source_dependencies; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.source_dependencies ENABLE ROW LEVEL SECURITY;

--
-- Name: source_dependencies source_dependency_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY source_dependency_tenant ON public.source_dependencies USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: source_event_addresses source_event_address_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY source_event_address_tenant ON public.source_event_addresses USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: source_event_addresses; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.source_event_addresses ENABLE ROW LEVEL SECURITY;

--
-- Name: source_observation_paths source_observation_path_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY source_observation_path_tenant ON public.source_observation_paths USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: source_observation_paths; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.source_observation_paths ENABLE ROW LEVEL SECURITY;

--
-- Name: source_observations source_observation_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY source_observation_tenant ON public.source_observations USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: source_observations; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.source_observations ENABLE ROW LEVEL SECURITY;

--
-- Name: source_reconcile_heads source_reconcile_head_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY source_reconcile_head_tenant ON public.source_reconcile_heads USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: source_reconcile_heads; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.source_reconcile_heads ENABLE ROW LEVEL SECURITY;

--
-- Name: source_reconcile_requests source_reconcile_request_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY source_reconcile_request_tenant ON public.source_reconcile_requests USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: source_reconcile_requests; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.source_reconcile_requests ENABLE ROW LEVEL SECURITY;

--
-- Name: source_repository_attachments source_repository_attachment_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY source_repository_attachment_tenant ON public.source_repository_attachments USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: source_repository_attachments; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.source_repository_attachments ENABLE ROW LEVEL SECURITY;

--
-- Name: source_repository_roots source_repository_root_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY source_repository_root_tenant ON public.source_repository_roots USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: source_repository_roots; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.source_repository_roots ENABLE ROW LEVEL SECURITY;

--
-- Name: source_suppressions source_suppression_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY source_suppression_tenant ON public.source_suppressions USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: source_suppressions; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.source_suppressions ENABLE ROW LEVEL SECURITY;

--
-- Name: tenant_administrator_bootstraps; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.tenant_administrator_bootstraps ENABLE ROW LEVEL SECURITY;

--
-- Name: tenant_administrator_bootstraps tenant_administrator_bootstraps_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY tenant_administrator_bootstraps_tenant_isolation ON public.tenant_administrator_bootstraps USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: tenant_keys; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.tenant_keys ENABLE ROW LEVEL SECURITY;

--
-- Name: tenant_keys tenant_keys_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY tenant_keys_tenant_isolation ON public.tenant_keys USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: tenant_secret_reencryption_jobs tenant_secret_reencrypt_jobs_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY tenant_secret_reencrypt_jobs_tenant_isolation ON public.tenant_secret_reencryption_jobs USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: tenant_secret_reencryption_jobs; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.tenant_secret_reencryption_jobs ENABLE ROW LEVEL SECURITY;

--
-- Name: tenant_secrets; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.tenant_secrets ENABLE ROW LEVEL SECURITY;

--
-- Name: tenant_secrets tenant_secrets_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY tenant_secrets_tenant_isolation ON public.tenant_secrets USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: tool_bindings; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.tool_bindings ENABLE ROW LEVEL SECURITY;

--
-- Name: tool_bindings tool_bindings_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY tool_bindings_tenant_isolation ON public.tool_bindings USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: tool_changes; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.tool_changes ENABLE ROW LEVEL SECURITY;

--
-- Name: tool_changes tool_changes_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY tool_changes_tenant_isolation ON public.tool_changes USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: tool_server_versions; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.tool_server_versions ENABLE ROW LEVEL SECURITY;

--
-- Name: tool_server_versions tool_server_versions_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY tool_server_versions_tenant_isolation ON public.tool_server_versions USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: tool_servers; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.tool_servers ENABLE ROW LEVEL SECURITY;

--
-- Name: tool_servers tool_servers_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY tool_servers_tenant_isolation ON public.tool_servers USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: tool_test_runs; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.tool_test_runs ENABLE ROW LEVEL SECURITY;

--
-- Name: tool_test_runs tool_test_runs_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY tool_test_runs_tenant_isolation ON public.tool_test_runs USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: unproven_artifact_dependencies; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.unproven_artifact_dependencies ENABLE ROW LEVEL SECURITY;

--
-- Name: unproven_artifact_dependencies unproven_artifact_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY unproven_artifact_tenant ON public.unproven_artifact_dependencies USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: unproven_context_deliveries; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.unproven_context_deliveries ENABLE ROW LEVEL SECURITY;

--
-- Name: unproven_context_deliveries unproven_context_delivery_tenant; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY unproven_context_delivery_tenant ON public.unproven_context_deliveries USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: vedaflow_commit_parents; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.vedaflow_commit_parents ENABLE ROW LEVEL SECURITY;

--
-- Name: vedaflow_commit_parents vedaflow_commit_parents_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY vedaflow_commit_parents_tenant_isolation ON public.vedaflow_commit_parents USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: vedaflow_commits; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.vedaflow_commits ENABLE ROW LEVEL SECURITY;

--
-- Name: vedaflow_commits vedaflow_commits_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY vedaflow_commits_tenant_isolation ON public.vedaflow_commits USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: vedaflow_objects; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.vedaflow_objects ENABLE ROW LEVEL SECURITY;

--
-- Name: vedaflow_objects vedaflow_objects_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY vedaflow_objects_tenant_isolation ON public.vedaflow_objects USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: vedaflow_proposal_approvals; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.vedaflow_proposal_approvals ENABLE ROW LEVEL SECURITY;

--
-- Name: vedaflow_proposal_approvals vedaflow_proposal_approvals_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY vedaflow_proposal_approvals_tenant_isolation ON public.vedaflow_proposal_approvals USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: vedaflow_proposals; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.vedaflow_proposals ENABLE ROW LEVEL SECURITY;

--
-- Name: vedaflow_proposals vedaflow_proposals_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY vedaflow_proposals_tenant_isolation ON public.vedaflow_proposals USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: vedaflow_refs; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.vedaflow_refs ENABLE ROW LEVEL SECURITY;

--
-- Name: vedaflow_refs vedaflow_refs_only_pins_are_deletable; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY vedaflow_refs_only_pins_are_deletable ON public.vedaflow_refs AS RESTRICTIVE FOR DELETE USING ((name ~~ 'pin/%'::text));


--
-- Name: vedaflow_refs vedaflow_refs_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY vedaflow_refs_tenant_isolation ON public.vedaflow_refs USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: vedaflow_tree_entries; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.vedaflow_tree_entries ENABLE ROW LEVEL SECURITY;

--
-- Name: vedaflow_tree_entries vedaflow_tree_entries_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY vedaflow_tree_entries_tenant_isolation ON public.vedaflow_tree_entries USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: vedaflow_trees; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.vedaflow_trees ENABLE ROW LEVEL SECURITY;

--
-- Name: vedaflow_trees vedaflow_trees_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY vedaflow_trees_tenant_isolation ON public.vedaflow_trees USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: workspaces; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.workspaces ENABLE ROW LEVEL SECURITY;

--
-- Name: workspaces workspaces_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY workspaces_tenant_isolation ON public.workspaces USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


--
-- Name: SCHEMA public; Type: ACL; Schema: -; Owner: -
--

GRANT USAGE ON SCHEMA public TO synveda_app;


--
-- Name: FUNCTION synveda_artifact_applied_binding(); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_artifact_applied_binding() FROM PUBLIC;


--
-- Name: FUNCTION synveda_artifact_cleanup_authorized(t uuid, k text, d uuid); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_artifact_cleanup_authorized(t uuid, k text, d uuid) FROM PUBLIC;
GRANT ALL ON FUNCTION public.synveda_artifact_cleanup_authorized(t uuid, k text, d uuid) TO synveda_app;


--
-- Name: FUNCTION synveda_artifact_cleanup_chunk(t uuid, o uuid, w text, a integer, n integer); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_artifact_cleanup_chunk(t uuid, o uuid, w text, a integer, n integer) FROM PUBLIC;
GRANT ALL ON FUNCTION public.synveda_artifact_cleanup_chunk(t uuid, o uuid, w text, a integer, n integer) TO synveda_app;


--
-- Name: FUNCTION synveda_artifact_cleanup_progress(); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_artifact_cleanup_progress() FROM PUBLIC;


--
-- Name: FUNCTION synveda_artifact_content_erasure(); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_artifact_content_erasure() FROM PUBLIC;


--
-- Name: FUNCTION synveda_artifact_control_binding(); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_artifact_control_binding() FROM PUBLIC;


--
-- Name: FUNCTION synveda_artifact_projection(); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_artifact_projection() FROM PUBLIC;


--
-- Name: FUNCTION synveda_artifact_source_key(s uuid, r text, k text, l text, p text, h text); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_artifact_source_key(s uuid, r text, k text, l text, p text, h text) FROM PUBLIC;
GRANT ALL ON FUNCTION public.synveda_artifact_source_key(s uuid, r text, k text, l text, p text, h text) TO synveda_app;


--
-- Name: FUNCTION synveda_bind_github_source_identity(wanted_tenant uuid, wanted_project uuid, wanted_scope uuid, wanted_repository uuid, wanted_github_id bigint); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_bind_github_source_identity(wanted_tenant uuid, wanted_project uuid, wanted_scope uuid, wanted_repository uuid, wanted_github_id bigint) FROM PUBLIC;
GRANT ALL ON FUNCTION public.synveda_bind_github_source_identity(wanted_tenant uuid, wanted_project uuid, wanted_scope uuid, wanted_repository uuid, wanted_github_id bigint) TO synveda_app;


--
-- Name: FUNCTION synveda_change_header_result(); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_change_header_result() FROM PUBLIC;


--
-- Name: FUNCTION synveda_context_delivery_knowledge_erasure(); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_context_delivery_knowledge_erasure() FROM PUBLIC;


--
-- Name: FUNCTION synveda_erase_knowledge(wanted_tenant uuid, wanted_item uuid, wanted_proposal uuid, wanted_operation uuid, wanted_actor_hash text, wanted_reason_hash text); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_erase_knowledge(wanted_tenant uuid, wanted_item uuid, wanted_proposal uuid, wanted_operation uuid, wanted_actor_hash text, wanted_reason_hash text) FROM PUBLIC;
GRANT ALL ON FUNCTION public.synveda_erase_knowledge(wanted_tenant uuid, wanted_item uuid, wanted_proposal uuid, wanted_operation uuid, wanted_actor_hash text, wanted_reason_hash text) TO synveda_app;


--
-- Name: FUNCTION synveda_import_repository_identity(); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_import_repository_identity() FROM PUBLIC;


--
-- Name: FUNCTION synveda_import_source_artifact(t uuid, s jsonb); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_import_source_artifact(t uuid, s jsonb) FROM PUBLIC;
GRANT ALL ON FUNCTION public.synveda_import_source_artifact(t uuid, s jsonb) TO synveda_app;


--
-- Name: FUNCTION synveda_inherit_historical_evidence(t uuid, k text, d uuid, pk text, p uuid); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_inherit_historical_evidence(t uuid, k text, d uuid, pk text, p uuid) FROM PUBLIC;


--
-- Name: FUNCTION synveda_inherit_procedure_evidence(t uuid, k text, d uuid, pk text, p uuid); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_inherit_procedure_evidence(t uuid, k text, d uuid, pk text, p uuid) FROM PUBLIC;


--
-- Name: FUNCTION synveda_inherit_task_evidence(t uuid, k text, d uuid, pk text, p uuid); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_inherit_task_evidence(t uuid, k text, d uuid, pk text, p uuid) FROM PUBLIC;


--
-- Name: FUNCTION synveda_item_cleanup_authorized(t uuid, k text, d uuid); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_item_cleanup_authorized(t uuid, k text, d uuid) FROM PUBLIC;
GRANT ALL ON FUNCTION public.synveda_item_cleanup_authorized(t uuid, k text, d uuid) TO synveda_app;


--
-- Name: FUNCTION synveda_lock_procedure_request(t uuid, e uuid); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_lock_procedure_request(t uuid, e uuid) FROM PUBLIC;
GRANT ALL ON FUNCTION public.synveda_lock_procedure_request(t uuid, e uuid) TO synveda_app;


--
-- Name: FUNCTION synveda_managed_import_complete(); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_managed_import_complete() FROM PUBLIC;


--
-- Name: FUNCTION synveda_procedure_evaluation_guard(); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_procedure_evaluation_guard() FROM PUBLIC;


--
-- Name: FUNCTION synveda_procedure_projection(); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_procedure_projection() FROM PUBLIC;


--
-- Name: FUNCTION synveda_repository_source_identity(wanted_tenant uuid, wanted_project uuid, wanted_repository uuid); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_repository_source_identity(wanted_tenant uuid, wanted_project uuid, wanted_repository uuid) FROM PUBLIC;
GRANT ALL ON FUNCTION public.synveda_repository_source_identity(wanted_tenant uuid, wanted_project uuid, wanted_repository uuid) TO synveda_app;


--
-- Name: FUNCTION synveda_source_admissible(t uuid, k text, d uuid); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_source_admissible(t uuid, k text, d uuid) FROM PUBLIC;
GRANT ALL ON FUNCTION public.synveda_source_admissible(t uuid, k text, d uuid) TO synveda_app;


--
-- Name: FUNCTION synveda_source_applied_binding(); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_source_applied_binding() FROM PUBLIC;


--
-- Name: FUNCTION synveda_source_available(t uuid, k text, d uuid); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_source_available(t uuid, k text, d uuid) FROM PUBLIC;
GRANT ALL ON FUNCTION public.synveda_source_available(t uuid, k text, d uuid) TO synveda_app;


--
-- Name: FUNCTION synveda_source_cleanup_authorized(t uuid, k text, d uuid); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_source_cleanup_authorized(t uuid, k text, d uuid) FROM PUBLIC;
GRANT ALL ON FUNCTION public.synveda_source_cleanup_authorized(t uuid, k text, d uuid) TO synveda_app;


--
-- Name: FUNCTION synveda_source_cleanup_chunk(t uuid, o uuid, w text, a integer, n integer); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_source_cleanup_chunk(t uuid, o uuid, w text, a integer, n integer) FROM PUBLIC;
GRANT ALL ON FUNCTION public.synveda_source_cleanup_chunk(t uuid, o uuid, w text, a integer, n integer) TO synveda_app;


--
-- Name: FUNCTION synveda_source_cleanup_progress(); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_source_cleanup_progress() FROM PUBLIC;


--
-- Name: FUNCTION synveda_source_content_erasure(); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_source_content_erasure() FROM PUBLIC;


--
-- Name: FUNCTION synveda_source_control_binding(); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_source_control_binding() FROM PUBLIC;


--
-- Name: FUNCTION synveda_source_delivery_proof(); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_source_delivery_proof() FROM PUBLIC;


--
-- Name: FUNCTION synveda_source_observation_complete(); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_source_observation_complete() FROM PUBLIC;


--
-- Name: FUNCTION synveda_source_projection(); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_source_projection() FROM PUBLIC;


--
-- Name: FUNCTION synveda_source_reconcile_guard(); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_source_reconcile_guard() FROM PUBLIC;


--
-- Name: FUNCTION synveda_source_repository_immutable(); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_source_repository_immutable() FROM PUBLIC;


--
-- Name: FUNCTION synveda_task_evidence_projection(); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_task_evidence_projection() FROM PUBLIC;


--
-- Name: TABLE artifact_cleanup_targets; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.artifact_cleanup_targets TO synveda_app;


--
-- Name: COLUMN artifact_cleanup_targets.tenant_id; Type: ACL; Schema: public; Owner: -
--

GRANT INSERT(tenant_id) ON TABLE public.artifact_cleanup_targets TO synveda_app;


--
-- Name: COLUMN artifact_cleanup_targets.operation_id; Type: ACL; Schema: public; Owner: -
--

GRANT INSERT(operation_id) ON TABLE public.artifact_cleanup_targets TO synveda_app;


--
-- Name: COLUMN artifact_cleanup_targets.artifact_id; Type: ACL; Schema: public; Owner: -
--

GRANT INSERT(artifact_id) ON TABLE public.artifact_cleanup_targets TO synveda_app;


--
-- Name: COLUMN artifact_cleanup_targets.source_key_digest; Type: ACL; Schema: public; Owner: -
--

GRANT INSERT(source_key_digest) ON TABLE public.artifact_cleanup_targets TO synveda_app;


--
-- Name: COLUMN artifact_cleanup_targets.change_id; Type: ACL; Schema: public; Owner: -
--

GRANT INSERT(change_id) ON TABLE public.artifact_cleanup_targets TO synveda_app;


--
-- Name: COLUMN artifact_cleanup_targets.input_hash; Type: ACL; Schema: public; Owner: -
--

GRANT INSERT(input_hash) ON TABLE public.artifact_cleanup_targets TO synveda_app;


--
-- Name: TABLE artifact_dependencies; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.artifact_dependencies TO synveda_app;


--
-- Name: TABLE artifact_suppressions; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.artifact_suppressions TO synveda_app;


--
-- Name: COLUMN artifact_suppressions.forget_change_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(forget_change_id) ON TABLE public.artifact_suppressions TO synveda_app;


--
-- Name: TABLE audit_chain_heads; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,UPDATE ON TABLE public.audit_chain_heads TO synveda_app;


--
-- Name: TABLE audit_log; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.audit_log TO synveda_app;


--
-- Name: TABLE capability_snapshots; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.capability_snapshots TO synveda_app;


--
-- Name: TABLE capture_batch_events; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.capture_batch_events TO synveda_app;


--
-- Name: TABLE capture_batches; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.capture_batches TO synveda_app;


--
-- Name: COLUMN capture_batches.state; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(state) ON TABLE public.capture_batches TO synveda_app;


--
-- Name: COLUMN capture_batches.extractor_method; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(extractor_method) ON TABLE public.capture_batches TO synveda_app;


--
-- Name: COLUMN capture_batches.model_version; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(model_version) ON TABLE public.capture_batches TO synveda_app;


--
-- Name: COLUMN capture_batches.attempts; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(attempts) ON TABLE public.capture_batches TO synveda_app;


--
-- Name: COLUMN capture_batches.lease_owner; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(lease_owner) ON TABLE public.capture_batches TO synveda_app;


--
-- Name: COLUMN capture_batches.lease_expires_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(lease_expires_at) ON TABLE public.capture_batches TO synveda_app;


--
-- Name: COLUMN capture_batches.candidate_count; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(candidate_count) ON TABLE public.capture_batches TO synveda_app;


--
-- Name: COLUMN capture_batches.error_code; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(error_code) ON TABLE public.capture_batches TO synveda_app;


--
-- Name: COLUMN capture_batches.started_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(started_at) ON TABLE public.capture_batches TO synveda_app;


--
-- Name: COLUMN capture_batches.completed_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(completed_at) ON TABLE public.capture_batches TO synveda_app;


--
-- Name: COLUMN capture_batches.updated_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(updated_at) ON TABLE public.capture_batches TO synveda_app;


--
-- Name: TABLE capture_candidate_decisions; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.capture_candidate_decisions TO synveda_app;


--
-- Name: COLUMN capture_candidate_decisions.state; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(state) ON TABLE public.capture_candidate_decisions TO synveda_app;


--
-- Name: COLUMN capture_candidate_decisions.payload; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(payload) ON TABLE public.capture_candidate_decisions TO synveda_app;


--
-- Name: COLUMN capture_candidate_decisions.resulting_change_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_change_id) ON TABLE public.capture_candidate_decisions TO synveda_app;


--
-- Name: COLUMN capture_candidate_decisions.resulting_outcome; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_outcome) ON TABLE public.capture_candidate_decisions TO synveda_app;


--
-- Name: COLUMN capture_candidate_decisions.resulting_knowledge_item_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_knowledge_item_id) ON TABLE public.capture_candidate_decisions TO synveda_app;


--
-- Name: COLUMN capture_candidate_decisions.resulting_revision_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_revision_id) ON TABLE public.capture_candidate_decisions TO synveda_app;


--
-- Name: COLUMN capture_candidate_decisions.error_code; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(error_code) ON TABLE public.capture_candidate_decisions TO synveda_app;


--
-- Name: COLUMN capture_candidate_decisions.completed_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(completed_at) ON TABLE public.capture_candidate_decisions TO synveda_app;


--
-- Name: TABLE capture_candidate_events; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.capture_candidate_events TO synveda_app;


--
-- Name: TABLE capture_candidate_import_artifacts; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.capture_candidate_import_artifacts TO synveda_app;


--
-- Name: TABLE capture_candidate_matches; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,DELETE ON TABLE public.capture_candidate_matches TO synveda_app;


--
-- Name: TABLE capture_candidates; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.capture_candidates TO synveda_app;


--
-- Name: COLUMN capture_candidates.title; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(title) ON TABLE public.capture_candidates TO synveda_app;


--
-- Name: COLUMN capture_candidates.body_markdown; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(body_markdown) ON TABLE public.capture_candidates TO synveda_app;


--
-- Name: COLUMN capture_candidates.summary; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(summary) ON TABLE public.capture_candidates TO synveda_app;


--
-- Name: COLUMN capture_candidates.tags; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(tags) ON TABLE public.capture_candidates TO synveda_app;


--
-- Name: COLUMN capture_candidates.verification_metadata; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(verification_metadata) ON TABLE public.capture_candidates TO synveda_app;


--
-- Name: COLUMN capture_candidates.metadata; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(metadata) ON TABLE public.capture_candidates TO synveda_app;


--
-- Name: COLUMN capture_candidates.state; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(state) ON TABLE public.capture_candidates TO synveda_app;


--
-- Name: COLUMN capture_candidates.resulting_change_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_change_id) ON TABLE public.capture_candidates TO synveda_app;


--
-- Name: COLUMN capture_candidates.resulting_outcome; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_outcome) ON TABLE public.capture_candidates TO synveda_app;


--
-- Name: COLUMN capture_candidates.resulting_knowledge_item_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_knowledge_item_id) ON TABLE public.capture_candidates TO synveda_app;


--
-- Name: COLUMN capture_candidates.resulting_revision_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_revision_id) ON TABLE public.capture_candidates TO synveda_app;


--
-- Name: COLUMN capture_candidates.decided_by; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(decided_by) ON TABLE public.capture_candidates TO synveda_app;


--
-- Name: COLUMN capture_candidates.decision_reason; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(decision_reason) ON TABLE public.capture_candidates TO synveda_app;


--
-- Name: COLUMN capture_candidates.decided_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(decided_at) ON TABLE public.capture_candidates TO synveda_app;


--
-- Name: COLUMN capture_candidates.content_erased; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(content_erased) ON TABLE public.capture_candidates TO synveda_app;


--
-- Name: TABLE cli_handoffs; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,DELETE ON TABLE public.cli_handoffs TO synveda_app;


--
-- Name: TABLE configuration_artifacts; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.configuration_artifacts TO synveda_app;


--
-- Name: COLUMN configuration_artifacts.current_version_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(current_version_id) ON TABLE public.configuration_artifacts TO synveda_app;


--
-- Name: COLUMN configuration_artifacts.updated_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(updated_at) ON TABLE public.configuration_artifacts TO synveda_app;


--
-- Name: COLUMN configuration_artifacts.updated_by; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(updated_by) ON TABLE public.configuration_artifacts TO synveda_app;


--
-- Name: TABLE configuration_bindings; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.configuration_bindings TO synveda_app;


--
-- Name: COLUMN configuration_bindings.artifact_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(artifact_id) ON TABLE public.configuration_bindings TO synveda_app;


--
-- Name: COLUMN configuration_bindings.pinned_version_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(pinned_version_id) ON TABLE public.configuration_bindings TO synveda_app;


--
-- Name: COLUMN configuration_bindings.enabled; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(enabled) ON TABLE public.configuration_bindings TO synveda_app;


--
-- Name: COLUMN configuration_bindings.revision; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(revision) ON TABLE public.configuration_bindings TO synveda_app;


--
-- Name: COLUMN configuration_bindings.updated_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(updated_at) ON TABLE public.configuration_bindings TO synveda_app;


--
-- Name: COLUMN configuration_bindings.updated_by; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(updated_by) ON TABLE public.configuration_bindings TO synveda_app;


--
-- Name: TABLE configuration_changes; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.configuration_changes TO synveda_app;


--
-- Name: COLUMN configuration_changes.resulting_artifact_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_artifact_id) ON TABLE public.configuration_changes TO synveda_app;


--
-- Name: COLUMN configuration_changes.resulting_version_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_version_id) ON TABLE public.configuration_changes TO synveda_app;


--
-- Name: COLUMN configuration_changes.resulting_binding_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_binding_id) ON TABLE public.configuration_changes TO synveda_app;


--
-- Name: COLUMN configuration_changes.resulting_binding_revision; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_binding_revision) ON TABLE public.configuration_changes TO synveda_app;


--
-- Name: COLUMN configuration_changes.applied_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(applied_at) ON TABLE public.configuration_changes TO synveda_app;


--
-- Name: TABLE configuration_versions; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.configuration_versions TO synveda_app;


--
-- Name: TABLE console_sessions; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE public.console_sessions TO synveda_app;


--
-- Name: TABLE context_candidates; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.context_candidates TO synveda_app;


--
-- Name: TABLE context_feedback; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.context_feedback TO synveda_app;


--
-- Name: TABLE context_graph_steps; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.context_graph_steps TO synveda_app;


--
-- Name: TABLE context_pack_chunks; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.context_pack_chunks TO synveda_app;


--
-- Name: TABLE context_pack_documents; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,UPDATE ON TABLE public.context_pack_documents TO synveda_app;


--
-- Name: TABLE context_packs; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,UPDATE ON TABLE public.context_packs TO synveda_app;


--
-- Name: TABLE context_run_delivery_dependencies; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.context_run_delivery_dependencies TO synveda_app;


--
-- Name: TABLE context_selections; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.context_selections TO synveda_app;


--
-- Name: TABLE deployment_keys; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,UPDATE ON TABLE public.deployment_keys TO synveda_app;


--
-- Name: TABLE directory_sync_state; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,UPDATE ON TABLE public.directory_sync_state TO synveda_app;


--
-- Name: TABLE durable_operations; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.durable_operations TO synveda_app;


--
-- Name: COLUMN durable_operations.state; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(state) ON TABLE public.durable_operations TO synveda_app;


--
-- Name: COLUMN durable_operations.progress_percent; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(progress_percent) ON TABLE public.durable_operations TO synveda_app;


--
-- Name: COLUMN durable_operations.attempts; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(attempts) ON TABLE public.durable_operations TO synveda_app;


--
-- Name: COLUMN durable_operations.next_attempt_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(next_attempt_at) ON TABLE public.durable_operations TO synveda_app;


--
-- Name: COLUMN durable_operations.cancel_requested_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(cancel_requested_at) ON TABLE public.durable_operations TO synveda_app;


--
-- Name: COLUMN durable_operations.lease_owner; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(lease_owner) ON TABLE public.durable_operations TO synveda_app;


--
-- Name: COLUMN durable_operations.lease_expires_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(lease_expires_at) ON TABLE public.durable_operations TO synveda_app;


--
-- Name: COLUMN durable_operations.last_error_code; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(last_error_code) ON TABLE public.durable_operations TO synveda_app;


--
-- Name: COLUMN durable_operations.result; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(result) ON TABLE public.durable_operations TO synveda_app;


--
-- Name: COLUMN durable_operations.updated_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(updated_at) ON TABLE public.durable_operations TO synveda_app;


--
-- Name: COLUMN durable_operations.started_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(started_at) ON TABLE public.durable_operations TO synveda_app;


--
-- Name: COLUMN durable_operations.completed_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(completed_at) ON TABLE public.durable_operations TO synveda_app;


--
-- Name: TABLE group_members; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,DELETE ON TABLE public.group_members TO synveda_app;


--
-- Name: TABLE groups; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,UPDATE ON TABLE public.groups TO synveda_app;


--
-- Name: TABLE historical_knowledge_dependencies; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.historical_knowledge_dependencies TO synveda_app;


--
-- Name: TABLE idempotency_records; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,DELETE ON TABLE public.idempotency_records TO synveda_app;


--
-- Name: TABLE identities; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE public.identities TO synveda_app;


--
-- Name: TABLE import_artifacts; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.import_artifacts TO synveda_app;


--
-- Name: TABLE import_jobs; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.import_jobs TO synveda_app;


--
-- Name: COLUMN import_jobs.state; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(state) ON TABLE public.import_jobs TO synveda_app;


--
-- Name: COLUMN import_jobs.candidate_count; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(candidate_count) ON TABLE public.import_jobs TO synveda_app;


--
-- Name: COLUMN import_jobs.capture_batch_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(capture_batch_id) ON TABLE public.import_jobs TO synveda_app;


--
-- Name: COLUMN import_jobs.error_code; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(error_code) ON TABLE public.import_jobs TO synveda_app;


--
-- Name: COLUMN import_jobs.completed_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(completed_at) ON TABLE public.import_jobs TO synveda_app;


--
-- Name: COLUMN import_jobs.updated_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(updated_at) ON TABLE public.import_jobs TO synveda_app;


--
-- Name: TABLE import_mappings; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.import_mappings TO synveda_app;


--
-- Name: COLUMN import_mappings.candidate_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(candidate_id) ON TABLE public.import_mappings TO synveda_app;


--
-- Name: TABLE knowledge_change_headers; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.knowledge_change_headers TO synveda_app;


--
-- Name: TABLE knowledge_changes; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.knowledge_changes TO synveda_app;


--
-- Name: COLUMN knowledge_changes.payload; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(payload) ON TABLE public.knowledge_changes TO synveda_app;


--
-- Name: COLUMN knowledge_changes.resulting_item_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_item_id) ON TABLE public.knowledge_changes TO synveda_app;


--
-- Name: COLUMN knowledge_changes.resulting_revision_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_revision_id) ON TABLE public.knowledge_changes TO synveda_app;


--
-- Name: COLUMN knowledge_changes.operation_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(operation_id) ON TABLE public.knowledge_changes TO synveda_app;


--
-- Name: COLUMN knowledge_changes.applied_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(applied_at) ON TABLE public.knowledge_changes TO synveda_app;


--
-- Name: TABLE knowledge_conflict_members; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.knowledge_conflict_members TO synveda_app;


--
-- Name: TABLE knowledge_conflict_sets; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,UPDATE ON TABLE public.knowledge_conflict_sets TO synveda_app;


--
-- Name: TABLE knowledge_copy_dependencies; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.knowledge_copy_dependencies TO synveda_app;


--
-- Name: TABLE knowledge_erasure_tombstones; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.knowledge_erasure_tombstones TO synveda_app;


--
-- Name: TABLE knowledge_index_invalidations; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.knowledge_index_invalidations TO synveda_app;


--
-- Name: COLUMN knowledge_index_invalidations.processed_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(processed_at) ON TABLE public.knowledge_index_invalidations TO synveda_app;


--
-- Name: TABLE knowledge_items; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.knowledge_items TO synveda_app;


--
-- Name: COLUMN knowledge_items.scope_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(scope_id) ON TABLE public.knowledge_items TO synveda_app;


--
-- Name: COLUMN knowledge_items.project_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(project_id) ON TABLE public.knowledge_items TO synveda_app;


--
-- Name: COLUMN knowledge_items.owner_principal_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(owner_principal_id) ON TABLE public.knowledge_items TO synveda_app;


--
-- Name: COLUMN knowledge_items.knowledge_type; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(knowledge_type) ON TABLE public.knowledge_items TO synveda_app;


--
-- Name: COLUMN knowledge_items.lifecycle_state; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(lifecycle_state) ON TABLE public.knowledge_items TO synveda_app;


--
-- Name: COLUMN knowledge_items.current_revision_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(current_revision_id) ON TABLE public.knowledge_items TO synveda_app;


--
-- Name: COLUMN knowledge_items.updated_by; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(updated_by) ON TABLE public.knowledge_items TO synveda_app;


--
-- Name: TABLE knowledge_items_history; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.knowledge_items_history TO synveda_app;


--
-- Name: TABLE knowledge_relations; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.knowledge_relations TO synveda_app;


--
-- Name: TABLE knowledge_revision_embeddings; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.knowledge_revision_embeddings TO synveda_app;


--
-- Name: TABLE knowledge_revision_headers; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.knowledge_revision_headers TO synveda_app;


--
-- Name: TABLE knowledge_revision_source_headers; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.knowledge_revision_source_headers TO synveda_app;


--
-- Name: TABLE knowledge_revision_sources; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.knowledge_revision_sources TO synveda_app;


--
-- Name: TABLE knowledge_revisions; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.knowledge_revisions TO synveda_app;


--
-- Name: TABLE knowledge_sources; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.knowledge_sources TO synveda_app;


--
-- Name: TABLE operation_attempts; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.operation_attempts TO synveda_app;


--
-- Name: COLUMN operation_attempts.state; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(state) ON TABLE public.operation_attempts TO synveda_app;


--
-- Name: COLUMN operation_attempts.heartbeat_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(heartbeat_at) ON TABLE public.operation_attempts TO synveda_app;


--
-- Name: COLUMN operation_attempts.completed_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(completed_at) ON TABLE public.operation_attempts TO synveda_app;


--
-- Name: COLUMN operation_attempts.safe_error_code; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(safe_error_code) ON TABLE public.operation_attempts TO synveda_app;


--
-- Name: TABLE operation_outbox; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.operation_outbox TO synveda_app;


--
-- Name: COLUMN operation_outbox.state; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(state) ON TABLE public.operation_outbox TO synveda_app;


--
-- Name: COLUMN operation_outbox.dispatch_attempts; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(dispatch_attempts) ON TABLE public.operation_outbox TO synveda_app;


--
-- Name: COLUMN operation_outbox.submission_failures; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(submission_failures) ON TABLE public.operation_outbox TO synveda_app;


--
-- Name: COLUMN operation_outbox.lease_owner; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(lease_owner) ON TABLE public.operation_outbox TO synveda_app;


--
-- Name: COLUMN operation_outbox.lease_expires_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(lease_expires_at) ON TABLE public.operation_outbox TO synveda_app;


--
-- Name: COLUMN operation_outbox.next_dispatch_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(next_dispatch_at) ON TABLE public.operation_outbox TO synveda_app;


--
-- Name: COLUMN operation_outbox.last_error_code; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(last_error_code) ON TABLE public.operation_outbox TO synveda_app;


--
-- Name: COLUMN operation_outbox.dispatched_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(dispatched_at) ON TABLE public.operation_outbox TO synveda_app;


--
-- Name: COLUMN operation_outbox.completed_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(completed_at) ON TABLE public.operation_outbox TO synveda_app;


--
-- Name: COLUMN operation_outbox.updated_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(updated_at) ON TABLE public.operation_outbox TO synveda_app;


--
-- Name: TABLE pending_invites; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE public.pending_invites TO synveda_app;


--
-- Name: TABLE pending_logins; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,DELETE ON TABLE public.pending_logins TO synveda_app;


--
-- Name: TABLE policy_packs; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE public.policy_packs TO synveda_app;


--
-- Name: TABLE policy_relaxation_changes; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.policy_relaxation_changes TO synveda_app;


--
-- Name: COLUMN policy_relaxation_changes.resulting_relaxation_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_relaxation_id) ON TABLE public.policy_relaxation_changes TO synveda_app;


--
-- Name: COLUMN policy_relaxation_changes.resulting_version_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_version_id) ON TABLE public.policy_relaxation_changes TO synveda_app;


--
-- Name: COLUMN policy_relaxation_changes.resulting_revision; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_revision) ON TABLE public.policy_relaxation_changes TO synveda_app;


--
-- Name: COLUMN policy_relaxation_changes.applied_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(applied_at) ON TABLE public.policy_relaxation_changes TO synveda_app;


--
-- Name: TABLE policy_relaxation_versions; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.policy_relaxation_versions TO synveda_app;


--
-- Name: TABLE policy_relaxations; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.policy_relaxations TO synveda_app;


--
-- Name: COLUMN policy_relaxations.current_version_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(current_version_id) ON TABLE public.policy_relaxations TO synveda_app;


--
-- Name: COLUMN policy_relaxations.revision; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(revision) ON TABLE public.policy_relaxations TO synveda_app;


--
-- Name: COLUMN policy_relaxations.updated_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(updated_at) ON TABLE public.policy_relaxations TO synveda_app;


--
-- Name: COLUMN policy_relaxations.updated_by; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(updated_by) ON TABLE public.policy_relaxations TO synveda_app;


--
-- Name: COLUMN policy_relaxations.revoked_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(revoked_at) ON TABLE public.policy_relaxations TO synveda_app;


--
-- Name: COLUMN policy_relaxations.revoked_by; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(revoked_by) ON TABLE public.policy_relaxations TO synveda_app;


--
-- Name: COLUMN policy_relaxations.revocation_proposal_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(revocation_proposal_id) ON TABLE public.policy_relaxations TO synveda_app;


--
-- Name: COLUMN policy_relaxations.revocation_reason; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(revocation_reason) ON TABLE public.policy_relaxations TO synveda_app;


--
-- Name: COLUMN policy_relaxations.expiry_recorded_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(expiry_recorded_at) ON TABLE public.policy_relaxations TO synveda_app;


--
-- Name: TABLE procedure_candidate_headers; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.procedure_candidate_headers TO synveda_app;


--
-- Name: TABLE procedure_evaluation_attempts; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.procedure_evaluation_attempts TO synveda_app;


--
-- Name: TABLE procedure_evaluation_results; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.procedure_evaluation_results TO synveda_app;


--
-- Name: TABLE procedure_improvement_headers; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.procedure_improvement_headers TO synveda_app;


--
-- Name: TABLE procedure_outcome_headers; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.procedure_outcome_headers TO synveda_app;


--
-- Name: TABLE project_repositories; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE public.project_repositories TO synveda_app;


--
-- Name: TABLE projects; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,UPDATE ON TABLE public.projects TO synveda_app;


--
-- Name: TABLE prompts; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,UPDATE ON TABLE public.prompts TO synveda_app;


--
-- Name: TABLE schema_metadata; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.schema_metadata TO synveda_app;


--
-- Name: TABLE scim_credentials; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,UPDATE ON TABLE public.scim_credentials TO synveda_app;


--
-- Name: TABLE scim_users; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE public.scim_users TO synveda_app;


--
-- Name: TABLE scope_closure; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,DELETE ON TABLE public.scope_closure TO synveda_app;


--
-- Name: TABLE scope_grants; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,DELETE ON TABLE public.scope_grants TO synveda_app;


--
-- Name: TABLE scopes; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,UPDATE ON TABLE public.scopes TO synveda_app;


--
-- Name: TABLE session_context_runs; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.session_context_runs TO synveda_app;


--
-- Name: TABLE session_event_quarantine; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,DELETE ON TABLE public.session_event_quarantine TO synveda_app;


--
-- Name: COLUMN session_event_quarantine.state; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(state) ON TABLE public.session_event_quarantine TO synveda_app;


--
-- Name: COLUMN session_event_quarantine.reviewer_subject; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(reviewer_subject) ON TABLE public.session_event_quarantine TO synveda_app;


--
-- Name: COLUMN session_event_quarantine.reviewed_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(reviewed_at) ON TABLE public.session_event_quarantine TO synveda_app;


--
-- Name: COLUMN session_event_quarantine.review_reason; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(review_reason) ON TABLE public.session_event_quarantine TO synveda_app;


--
-- Name: TABLE session_events; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,DELETE ON TABLE public.session_events TO synveda_app;


--
-- Name: TABLE session_snapshot_headers; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.session_snapshot_headers TO synveda_app;


--
-- Name: TABLE sessions; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,UPDATE ON TABLE public.sessions TO synveda_app;


--
-- Name: TABLE skill_bindings; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.skill_bindings TO synveda_app;


--
-- Name: COLUMN skill_bindings.pinned_version_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(pinned_version_id) ON TABLE public.skill_bindings TO synveda_app;


--
-- Name: COLUMN skill_bindings.enabled; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(enabled) ON TABLE public.skill_bindings TO synveda_app;


--
-- Name: COLUMN skill_bindings.revision; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(revision) ON TABLE public.skill_bindings TO synveda_app;


--
-- Name: COLUMN skill_bindings.updated_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(updated_at) ON TABLE public.skill_bindings TO synveda_app;


--
-- Name: COLUMN skill_bindings.updated_by; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(updated_by) ON TABLE public.skill_bindings TO synveda_app;


--
-- Name: TABLE skill_changes; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.skill_changes TO synveda_app;


--
-- Name: COLUMN skill_changes.resulting_skill_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_skill_id) ON TABLE public.skill_changes TO synveda_app;


--
-- Name: COLUMN skill_changes.resulting_version_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_version_id) ON TABLE public.skill_changes TO synveda_app;


--
-- Name: COLUMN skill_changes.resulting_binding_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_binding_id) ON TABLE public.skill_changes TO synveda_app;


--
-- Name: COLUMN skill_changes.resulting_binding_revision; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_binding_revision) ON TABLE public.skill_changes TO synveda_app;


--
-- Name: COLUMN skill_changes.applied_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(applied_at) ON TABLE public.skill_changes TO synveda_app;


--
-- Name: TABLE skill_test_runs; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.skill_test_runs TO synveda_app;


--
-- Name: TABLE skill_usage_events; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.skill_usage_events TO synveda_app;


--
-- Name: TABLE skill_version_files; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.skill_version_files TO synveda_app;


--
-- Name: TABLE skill_versions; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.skill_versions TO synveda_app;


--
-- Name: TABLE skills; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.skills TO synveda_app;


--
-- Name: COLUMN skills.current_version_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(current_version_id) ON TABLE public.skills TO synveda_app;


--
-- Name: COLUMN skills.updated_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(updated_at) ON TABLE public.skills TO synveda_app;


--
-- Name: COLUMN skills.updated_by; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(updated_by) ON TABLE public.skills TO synveda_app;


--
-- Name: TABLE source_artifact_addresses; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.source_artifact_addresses TO synveda_app;


--
-- Name: TABLE source_cleanup_targets; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.source_cleanup_targets TO synveda_app;


--
-- Name: COLUMN source_cleanup_targets.tenant_id; Type: ACL; Schema: public; Owner: -
--

GRANT INSERT(tenant_id) ON TABLE public.source_cleanup_targets TO synveda_app;


--
-- Name: COLUMN source_cleanup_targets.operation_id; Type: ACL; Schema: public; Owner: -
--

GRANT INSERT(operation_id) ON TABLE public.source_cleanup_targets TO synveda_app;


--
-- Name: COLUMN source_cleanup_targets.event_id; Type: ACL; Schema: public; Owner: -
--

GRANT INSERT(event_id) ON TABLE public.source_cleanup_targets TO synveda_app;


--
-- Name: COLUMN source_cleanup_targets.change_id; Type: ACL; Schema: public; Owner: -
--

GRANT INSERT(change_id) ON TABLE public.source_cleanup_targets TO synveda_app;


--
-- Name: COLUMN source_cleanup_targets.input_hash; Type: ACL; Schema: public; Owner: -
--

GRANT INSERT(input_hash) ON TABLE public.source_cleanup_targets TO synveda_app;


--
-- Name: TABLE source_dependencies; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.source_dependencies TO synveda_app;


--
-- Name: TABLE source_event_addresses; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.source_event_addresses TO synveda_app;


--
-- Name: TABLE source_observation_paths; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.source_observation_paths TO synveda_app;


--
-- Name: TABLE source_observations; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.source_observations TO synveda_app;


--
-- Name: TABLE source_reconcile_heads; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,UPDATE ON TABLE public.source_reconcile_heads TO synveda_app;


--
-- Name: TABLE source_reconcile_requests; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,UPDATE ON TABLE public.source_reconcile_requests TO synveda_app;


--
-- Name: TABLE source_repository_attachments; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.source_repository_attachments TO synveda_app;


--
-- Name: TABLE source_repository_roots; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.source_repository_roots TO synveda_app;


--
-- Name: TABLE source_suppressions; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.source_suppressions TO synveda_app;


--
-- Name: COLUMN source_suppressions.forget_change_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(forget_change_id) ON TABLE public.source_suppressions TO synveda_app;


--
-- Name: TABLE tenant_administrator_bootstraps; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.tenant_administrator_bootstraps TO synveda_app;


--
-- Name: TABLE tenant_keys; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,UPDATE ON TABLE public.tenant_keys TO synveda_app;


--
-- Name: TABLE tenant_secret_reencryption_jobs; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,UPDATE ON TABLE public.tenant_secret_reencryption_jobs TO synveda_app;


--
-- Name: TABLE tenant_secrets; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,UPDATE ON TABLE public.tenant_secrets TO synveda_app;


--
-- Name: TABLE tenants; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.tenants TO synveda_app;


--
-- Name: TABLE tool_bindings; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.tool_bindings TO synveda_app;


--
-- Name: COLUMN tool_bindings.version_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(version_id) ON TABLE public.tool_bindings TO synveda_app;


--
-- Name: COLUMN tool_bindings.state; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(state) ON TABLE public.tool_bindings TO synveda_app;


--
-- Name: COLUMN tool_bindings.revision; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(revision) ON TABLE public.tool_bindings TO synveda_app;


--
-- Name: COLUMN tool_bindings.updated_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(updated_at) ON TABLE public.tool_bindings TO synveda_app;


--
-- Name: COLUMN tool_bindings.updated_by; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(updated_by) ON TABLE public.tool_bindings TO synveda_app;


--
-- Name: TABLE tool_changes; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.tool_changes TO synveda_app;


--
-- Name: COLUMN tool_changes.resulting_server_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_server_id) ON TABLE public.tool_changes TO synveda_app;


--
-- Name: COLUMN tool_changes.resulting_version_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_version_id) ON TABLE public.tool_changes TO synveda_app;


--
-- Name: COLUMN tool_changes.resulting_binding_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_binding_id) ON TABLE public.tool_changes TO synveda_app;


--
-- Name: COLUMN tool_changes.resulting_binding_revision; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(resulting_binding_revision) ON TABLE public.tool_changes TO synveda_app;


--
-- Name: COLUMN tool_changes.applied_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(applied_at) ON TABLE public.tool_changes TO synveda_app;


--
-- Name: TABLE tool_server_versions; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.tool_server_versions TO synveda_app;


--
-- Name: TABLE tool_servers; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.tool_servers TO synveda_app;


--
-- Name: COLUMN tool_servers.current_version_id; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(current_version_id) ON TABLE public.tool_servers TO synveda_app;


--
-- Name: COLUMN tool_servers.updated_at; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(updated_at) ON TABLE public.tool_servers TO synveda_app;


--
-- Name: COLUMN tool_servers.updated_by; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(updated_by) ON TABLE public.tool_servers TO synveda_app;


--
-- Name: TABLE tool_test_runs; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.tool_test_runs TO synveda_app;


--
-- Name: TABLE unproven_artifact_dependencies; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.unproven_artifact_dependencies TO synveda_app;


--
-- Name: TABLE unproven_context_deliveries; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.unproven_context_deliveries TO synveda_app;


--
-- Name: TABLE vedaflow_commit_parents; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.vedaflow_commit_parents TO synveda_app;


--
-- Name: TABLE vedaflow_commits; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.vedaflow_commits TO synveda_app;


--
-- Name: TABLE vedaflow_objects; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.vedaflow_objects TO synveda_app;


--
-- Name: TABLE vedaflow_proposal_approvals; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.vedaflow_proposal_approvals TO synveda_app;


--
-- Name: TABLE vedaflow_proposals; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,UPDATE ON TABLE public.vedaflow_proposals TO synveda_app;


--
-- Name: TABLE vedaflow_refs; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE public.vedaflow_refs TO synveda_app;


--
-- Name: TABLE vedaflow_tree_entries; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.vedaflow_tree_entries TO synveda_app;


--
-- Name: TABLE vedaflow_trees; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.vedaflow_trees TO synveda_app;


--
-- Name: TABLE workspaces; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,UPDATE ON TABLE public.workspaces TO synveda_app;


--
-- PostgreSQL database dump complete
--


