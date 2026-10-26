-- ADR-0157: tenant erasure. A suspended tenant becomes `erasing` and then
-- `erased`; the remaining row is the tombstone. A `tenant_erasure` durable
-- operation fences the deleting steps. Every trigger that otherwise refuses
-- deletion, and every restrictive policy that hides retained rows, admits a
-- delete or read only inside a transaction run by the database owner that
-- names the current tenant as erasing. Ordinary roles are never the owner,
-- and an UPDATE or INSERT never reaches the exception.
ALTER TABLE public.tenants DROP CONSTRAINT tenants_status_check;
ALTER TABLE public.tenants ADD CONSTRAINT tenants_status_check
 CHECK (status = ANY (ARRAY['active'::text, 'suspended'::text, 'erasing'::text, 'erased'::text]));

ALTER TABLE public.durable_operations DROP CONSTRAINT durable_operations_kind_check;
ALTER TABLE public.durable_operations ADD CONSTRAINT durable_operations_kind_check
 CHECK (kind = ANY (ARRAY['knowledge_erasure'::text, 'skill_validation'::text, 'source_cleanup'::text,
   'source_reconcile'::text, 'tenant_erasure'::text]));
ALTER TABLE public.durable_operations DROP CONSTRAINT durable_operations_target_check;
ALTER TABLE public.durable_operations ADD CONSTRAINT durable_operations_target_check
 CHECK (((kind = 'knowledge_erasure'::text) AND (proposal_id IS NOT NULL) AND (knowledge_item_id IS NOT NULL)
   AND (skill_version_id IS NULL) AND (requested_by_identity_id IS NULL) AND (cancel_requested_at IS NULL))
 OR ((kind = 'skill_validation'::text) AND (proposal_id IS NULL) AND (knowledge_item_id IS NULL)
   AND (skill_version_id IS NOT NULL) AND (requested_by_identity_id IS NOT NULL))
 OR ((kind = 'source_cleanup'::text) AND (proposal_id IS NOT NULL) AND (knowledge_item_id IS NULL)
   AND (skill_version_id IS NULL) AND (requested_by_identity_id IS NULL) AND (cancel_requested_at IS NULL))
 OR ((kind = 'source_reconcile'::text) AND (proposal_id IS NULL) AND (knowledge_item_id IS NULL)
   AND (skill_version_id IS NULL) AND (requested_by_identity_id IS NOT NULL))
 OR ((kind = 'tenant_erasure'::text) AND (proposal_id IS NULL) AND (knowledge_item_id IS NULL)
   AND (skill_version_id IS NULL) AND (requested_by_identity_id IS NULL) AND (cancel_requested_at IS NULL)));

-- Inlined into the policies below; ordinary rows never reach it because it
-- is the last alternative of each restrictive policy.
CREATE FUNCTION public.synveda_tenant_erasure_authorized(t uuid) RETURNS boolean
 LANGUAGE sql STABLE PARALLEL SAFE AS $$
 SELECT t IS NOT NULL
   AND coalesce(current_setting('synveda.tenant_erasure', true), '') = t::text
   AND t = public.synveda_current_tenant()
   AND current_user = pg_catalog.pg_get_userbyid((
       SELECT database.datdba FROM pg_catalog.pg_database AS database
       WHERE database.datname = pg_catalog.current_database()))
   AND EXISTS (SELECT 1 FROM public.tenants WHERE id = t AND status = 'erasing')
$$;
REVOKE ALL ON FUNCTION public.synveda_tenant_erasure_authorized(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.synveda_tenant_erasure_authorized(uuid) TO synveda_app;

-- Guards that refuse deletion. Each body is unchanged after the exception.
CREATE OR REPLACE FUNCTION public.synveda_capture_append_only()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
begin
    if tg_op = 'DELETE' then
        if public.synveda_tenant_erasure_authorized(public.synveda_current_tenant()) then
            if tg_level = 'ROW' and tg_when = 'BEFORE' then
                return old;
            end if;
            return null;
        end if;
    end if;
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
$function$;

CREATE OR REPLACE FUNCTION public.synveda_capture_scrub_for_knowledge()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
begin
    if tg_op = 'DELETE' then
        if public.synveda_tenant_erasure_authorized(public.synveda_current_tenant()) then
            if tg_level = 'ROW' and tg_when = 'BEFORE' then
                return old;
            end if;
            return null;
        end if;
    end if;
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
$function$;

CREATE OR REPLACE FUNCTION public.synveda_context_delivery_knowledge_erasure()
 RETURNS trigger
 LANGUAGE plpgsql
 SECURITY DEFINER
 SET search_path TO 'public', 'pg_temp'
AS $function$
begin
    if tg_op = 'DELETE' then
        if public.synveda_tenant_erasure_authorized(public.synveda_current_tenant()) then
            if tg_level = 'ROW' and tg_when = 'BEFORE' then
                return old;
            end if;
            return null;
        end if;
    end if;
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
$function$;

CREATE OR REPLACE FUNCTION public.synveda_context_trace_immutable()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
declare
    new_row jsonb;
    old_row jsonb;
begin
    if tg_op = 'DELETE' then
        if public.synveda_tenant_erasure_authorized(public.synveda_current_tenant()) then
            if tg_level = 'ROW' and tg_when = 'BEFORE' then
                return old;
            end if;
            return null;
        end if;
    end if;
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
$function$;

CREATE OR REPLACE FUNCTION public.synveda_immutable_configuration_row()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
begin
    if tg_op = 'DELETE' then
        if public.synveda_tenant_erasure_authorized(public.synveda_current_tenant()) then
            if tg_level = 'ROW' and tg_when = 'BEFORE' then
                return old;
            end if;
            return null;
        end if;
    end if;
    raise exception '% rows are immutable (CPR-30)', tg_table_name;
end
$function$;

CREATE OR REPLACE FUNCTION public.synveda_immutable_relaxation_version()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
begin
    if tg_op = 'DELETE' then
        if public.synveda_tenant_erasure_authorized(public.synveda_current_tenant()) then
            if tg_level = 'ROW' and tg_when = 'BEFORE' then
                return old;
            end if;
            return null;
        end if;
    end if;
    raise exception 'Relaxation versions are immutable (CPR-31)';
end
$function$;

CREATE OR REPLACE FUNCTION public.synveda_immutable_skill_row()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
begin
    if tg_op = 'DELETE' then
        if public.synveda_tenant_erasure_authorized(public.synveda_current_tenant()) then
            if tg_level = 'ROW' and tg_when = 'BEFORE' then
                return old;
            end if;
            return null;
        end if;
    end if;
    raise exception '% rows are immutable (CPR-23)', tg_table_name;
end
$function$;

CREATE OR REPLACE FUNCTION public.synveda_immutable_tool_row()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
begin
    if tg_op = 'DELETE' then
        if public.synveda_tenant_erasure_authorized(public.synveda_current_tenant()) then
            if tg_level = 'ROW' and tg_when = 'BEFORE' then
                return old;
            end if;
            return null;
        end if;
    end if;
    raise exception '% rows are immutable (CPR-25)', tg_table_name;
end
$function$;

CREATE OR REPLACE FUNCTION public.synveda_import_append_only()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
begin
    if tg_op = 'DELETE' then
        if public.synveda_tenant_erasure_authorized(public.synveda_current_tenant()) then
            if tg_level = 'ROW' and tg_when = 'BEFORE' then
                return old;
            end if;
            return null;
        end if;
    end if;
    raise exception '% is append-only (CPR-27, ADR-0087)', tg_table_name;
end
$function$;

CREATE OR REPLACE FUNCTION public.synveda_knowledge_append_only()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
begin
    if tg_op = 'DELETE' then
        if public.synveda_tenant_erasure_authorized(public.synveda_current_tenant()) then
            if tg_level = 'ROW' and tg_when = 'BEFORE' then
                return old;
            end if;
            return null;
        end if;
    end if;
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
$function$;

CREATE OR REPLACE FUNCTION public.synveda_knowledge_conflict_member_immutable()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
begin
    if tg_op = 'DELETE' then
        if public.synveda_tenant_erasure_authorized(public.synveda_current_tenant()) then
            if tg_level = 'ROW' and tg_when = 'BEFORE' then
                return old;
            end if;
            return null;
        end if;
    end if;
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
$function$;

CREATE OR REPLACE FUNCTION public.synveda_knowledge_conflict_set_transition()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
begin
    if tg_op = 'DELETE' then
        if public.synveda_tenant_erasure_authorized(public.synveda_current_tenant()) then
            if tg_level = 'ROW' and tg_when = 'BEFORE' then
                return old;
            end if;
            return null;
        end if;
    end if;
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
$function$;

CREATE OR REPLACE FUNCTION public.synveda_knowledge_items_archive()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
declare
    changed_at timestamptz;
begin
    if tg_op = 'DELETE' then
        if public.synveda_tenant_erasure_authorized(public.synveda_current_tenant()) then
            if tg_level = 'ROW' and tg_when = 'BEFORE' then
                return old;
            end if;
            return null;
        end if;
    end if;
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
$function$;

CREATE OR REPLACE FUNCTION public.synveda_session_event_quarantine_immutable()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
begin
    if tg_op = 'DELETE' then
        if public.synveda_tenant_erasure_authorized(public.synveda_current_tenant()) then
            if tg_level = 'ROW' and tg_when = 'BEFORE' then
                return old;
            end if;
            return null;
        end if;
    end if;
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
$function$;

CREATE OR REPLACE FUNCTION public.synveda_session_events_immutable()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
begin
    if tg_op = 'DELETE' then
        if public.synveda_tenant_erasure_authorized(public.synveda_current_tenant()) then
            if tg_level = 'ROW' and tg_when = 'BEFORE' then
                return old;
            end if;
            return null;
        end if;
    end if;
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
$function$;

CREATE OR REPLACE FUNCTION public.synveda_source_reconcile_guard()
 RETURNS trigger
 LANGUAGE plpgsql
 SET search_path TO 'pg_catalog', 'public'
AS $function$
DECLARE digest text; operation public.durable_operations%ROWTYPE; request public.source_reconcile_requests%ROWTYPE;
 head public.source_reconcile_heads%ROWTYPE;
BEGIN
    if tg_op = 'DELETE' then
        if public.synveda_tenant_erasure_authorized(public.synveda_current_tenant()) then
            if tg_level = 'ROW' and tg_when = 'BEFORE' then
                return old;
            end if;
            return null;
        end if;
    end if;
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
END $function$;

CREATE OR REPLACE FUNCTION public.synveda_source_repository_immutable()
 RETURNS trigger
 LANGUAGE plpgsql
 SET search_path TO 'pg_catalog', 'public'
AS $function$
BEGIN
    if tg_op = 'DELETE' then
        if public.synveda_tenant_erasure_authorized(public.synveda_current_tenant()) then
            if tg_level = 'ROW' and tg_when = 'BEFORE' then
                return old;
            end if;
            return null;
        end if;
    end if;
 RAISE EXCEPTION 'source repository identity is immutable' USING ERRCODE='23514';
END $function$;

CREATE OR REPLACE FUNCTION public.synveda_vedaflow_immutable()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
begin
    if tg_op = 'DELETE' then
        if public.synveda_tenant_erasure_authorized(public.synveda_current_tenant()) then
            if tg_level = 'ROW' and tg_when = 'BEFORE' then
                return old;
            end if;
            return null;
        end if;
    end if;
    raise exception '% is append-only (FLOW-1, ADR-0030)', tg_table_name;
end
$function$;

CREATE OR REPLACE FUNCTION public.synveda_vedaflow_refs_delete_guard()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
begin
    if tg_op = 'DELETE' then
        if public.synveda_tenant_erasure_authorized(public.synveda_current_tenant()) then
            if tg_level = 'ROW' and tg_when = 'BEFORE' then
                return old;
            end if;
            return null;
        end if;
    end if;
    if old.name not like 'pin/%' then
        raise exception
            'vedaflow_refs.% is a channel pointer; only pins (pin/*) may be deleted (FLOW-7, ADR-0036)',
            old.name;
    end if;
    return old;
end
$function$;

-- Restrictive eligibility policies hide withdrawn, suppressed and erased rows
-- from every role, the owner included. Erasure must see and delete them.
ALTER POLICY decision_source_eligibility ON public.capture_candidate_decisions
 USING ((public.synveda_source_available(tenant_id, 'capture_candidate'::text, candidate_id)) OR public.synveda_tenant_erasure_authorized(tenant_id));
ALTER POLICY candidate_source_eligibility ON public.capture_candidates
 USING ((public.synveda_source_available(tenant_id, 'capture_candidate'::text, id)) OR public.synveda_tenant_erasure_authorized(tenant_id));
ALTER POLICY artifact_source_eligibility ON public.import_artifacts
 USING (((((NOT content_erased) AND public.synveda_source_available(tenant_id, 'import_artifact'::text, id)) OR public.synveda_artifact_cleanup_authorized(tenant_id, 'import_artifact'::text, id))) OR public.synveda_tenant_erasure_authorized(tenant_id));
ALTER POLICY mapping_source_eligibility ON public.import_mappings
 USING (((public.synveda_source_available(tenant_id, 'import_mapping'::text, id) OR public.synveda_item_cleanup_authorized(tenant_id, 'knowledge_item'::text, matched_item_id) OR public.synveda_item_cleanup_authorized(tenant_id, 'capture_candidate'::text, candidate_id))) OR public.synveda_tenant_erasure_authorized(tenant_id));
ALTER POLICY change_source_eligibility ON public.knowledge_changes
 USING ((((command_kind = ANY (ARRAY['control_source'::text, 'control_artifact'::text])) OR public.synveda_source_available(tenant_id, 'knowledge_change'::text, proposal_id))) OR public.synveda_tenant_erasure_authorized(tenant_id));
ALTER POLICY relation_source_eligibility ON public.knowledge_relations
 USING (((public.synveda_source_available(tenant_id, 'knowledge_revision'::text, asserting_revision_id) OR public.synveda_item_cleanup_authorized(tenant_id, 'knowledge_item'::text, source_item_id) OR public.synveda_item_cleanup_authorized(tenant_id, 'knowledge_item'::text, target_item_id))) OR public.synveda_tenant_erasure_authorized(tenant_id));
ALTER POLICY embedding_source_eligibility ON public.knowledge_revision_embeddings
 USING ((public.synveda_source_available(tenant_id, 'knowledge_revision'::text, knowledge_revision_id)) OR public.synveda_tenant_erasure_authorized(tenant_id));
ALTER POLICY revision_source_eligibility ON public.knowledge_revisions
 USING (((((NOT content_erased) AND public.synveda_source_available(tenant_id, 'knowledge_revision'::text, id)) OR public.synveda_source_cleanup_authorized(tenant_id, 'knowledge_revision'::text, id) OR public.synveda_artifact_cleanup_authorized(tenant_id, 'knowledge_revision'::text, id) OR public.synveda_item_cleanup_authorized(tenant_id, 'knowledge_revision'::text, id))) OR public.synveda_tenant_erasure_authorized(tenant_id));
ALTER POLICY descriptor_source_eligibility ON public.knowledge_sources
 USING (((((NOT content_erased) AND public.synveda_source_available(tenant_id, 'knowledge_source'::text, id)) OR public.synveda_source_cleanup_authorized(tenant_id, 'knowledge_source'::text, id) OR public.synveda_artifact_cleanup_authorized(tenant_id, 'knowledge_source'::text, id) OR public.synveda_item_cleanup_authorized(tenant_id, 'knowledge_source'::text, id))) OR public.synveda_tenant_erasure_authorized(tenant_id));
ALTER POLICY context_source_eligibility ON public.session_context_runs
 USING (((((NOT (EXISTS ( SELECT 1
   FROM public.unproven_context_deliveries unproven
  WHERE ((unproven.tenant_id = session_context_runs.tenant_id) AND (unproven.context_run_id = session_context_runs.id))))) OR (rendered = ''::text) OR public.synveda_item_cleanup_authorized(tenant_id, 'context_run'::text, id)) AND public.synveda_source_available(tenant_id, 'context_run'::text, id))) OR public.synveda_tenant_erasure_authorized(tenant_id));
ALTER POLICY session_source_eligibility ON public.session_events
 USING (((((NOT content_erased) AND public.synveda_source_available(tenant_id, 'session_event'::text, id)) OR public.synveda_source_cleanup_authorized(tenant_id, 'session_event'::text, id) OR public.synveda_artifact_cleanup_authorized(tenant_id, 'session_event'::text, id) OR public.synveda_item_cleanup_authorized(tenant_id, 'session_event'::text, id))) OR public.synveda_tenant_erasure_authorized(tenant_id));
ALTER POLICY vedaflow_refs_only_pins_are_deletable ON public.vedaflow_refs
 USING (((name ~~ 'pin/%'::text)) OR public.synveda_tenant_erasure_authorized(tenant_id));
