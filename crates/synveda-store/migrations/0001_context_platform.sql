-- CPR-43 / ADR-0069: epoch-3 context-platform baseline.
-- Fresh installs only: this file contains schema and grants, never old-data DML.
-- Generated from the exact epoch-2 head plus the reviewed hard-cut deletions.

--
-- PostgreSQL database dump
--


-- Dumped from database version 17.11 (Debian 17.11-1.pgdg12+2)
-- Dumped by pg_dump version 18.6

SET statement_timeout = 0;
SET lock_timeout = 0;
SET idle_in_transaction_session_timeout = 0;
SET transaction_timeout = 0;
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;

-- Cluster roles and required extensions are deployment infrastructure, not
-- application schema history (ADR-0069 decision 13; CPR-45). The deployment
-- bootstrap must establish the safe NOLOGIN `synveda_app` role and install
-- `btree_gin` and `vector` before this narrow migration owner runs the
-- baseline. Keeping that authority out of 0001 lets the migrator own domain
-- objects without CREATEROLE or superuser privileges.


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
       or new.proposal_id <> old.proposal_id
       or new.knowledge_item_id is distinct from old.knowledge_item_id
       or new.input_hash <> old.input_hash
       or new.created_at <> old.created_at then
        raise exception 'durable operation identity is immutable';
    end if;
    if old.state in ('succeeded', 'blocked') then
        raise exception 'durable operation % is terminal', old.id;
    end if;
    if not (
        (old.state = 'pending' and new.state in ('running', 'blocked'))
        or (old.state = 'running' and new.state in ('succeeded', 'failed', 'blocked'))
        or (old.state = 'failed' and new.state in ('running', 'blocked'))
    ) then
        raise exception 'invalid durable operation transition % -> %', old.state, new.state;
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
    perform set_config('synveda.knowledge_erasure', 'off', true);

    update durable_operations
       set state = 'succeeded', completed_at = now(), updated_at = now(),
           lease_owner = null, lease_expires_at = null, last_error_code = null,
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
        if not new.content_erased or old.content_erased
           or new.title <> '' or new.body_markdown <> '' or new.summary <> ''
           or new.tags <> '{}'::text[]
           or new.verification_metadata <> '{}'::jsonb
           or new.metadata <> '{}'::jsonb
           or new.proposed_relations <> '[]'::jsonb
           or new.materializable
           or new.matched_item_id is not null
           or new.matched_revision_id is not null
           or (to_jsonb(new) - array[
                   'title', 'body_markdown', 'summary', 'tags',
                   'verification_metadata', 'metadata', 'proposed_relations',
                   'matched_item_id', 'matched_revision_id', 'materializable',
                   'content_erased'
               ])
               <> (to_jsonb(old) - array[
                   'title', 'body_markdown', 'summary', 'tags',
                   'verification_metadata', 'metadata', 'proposed_relations',
                   'matched_item_id', 'matched_revision_id', 'materializable',
                   'content_erased'
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

SET default_table_access_method = heap;

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
    CONSTRAINT context_candidates_exclusion_check CHECK (((exclusion_reason IS NULL) OR (exclusion_reason = ANY (ARRAY['semantic_match'::text, 'keyword_match'::text, 'project_convention'::text, 'personal_preference'::text, 'freshness_boost'::text, 'explicit_pin'::text, 'superseded'::text, 'stale'::text, 'outside_task_scope'::text, 'token_budget'::text, 'duplicate'::text, 'graph_expansion'::text, 'contradiction_warning'::text])))),
    CONSTRAINT context_candidates_hash_check CHECK ((content_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT context_candidates_lifecycle_check CHECK (((lifecycle_state IS NULL) OR (lifecycle_state = ANY (ARRAY['active'::text, 'stale'::text, 'superseded'::text, 'archived'::text, 'erasure_pending'::text, 'erased'::text])))),
    CONSTRAINT context_candidates_ordinal_check CHECK ((ordinal >= 0)),
    CONSTRAINT context_candidates_reasons_check CHECK ((((cardinality(reason_codes) >= 1) AND (cardinality(reason_codes) <= 13)) AND (array_position(reason_codes, NULL::text) IS NULL) AND (reason_codes <@ ARRAY['semantic_match'::text, 'keyword_match'::text, 'project_convention'::text, 'personal_preference'::text, 'freshness_boost'::text, 'explicit_pin'::text, 'superseded'::text, 'stale'::text, 'outside_task_scope'::text, 'token_budget'::text, 'duplicate'::text, 'graph_expansion'::text, 'contradiction_warning'::text]))),
    CONSTRAINT context_candidates_scores_check CHECK ((((keyword_score_micros >= 0) AND (keyword_score_micros <= 1000000)) AND ((semantic_score_micros >= 0) AND (semantic_score_micros <= 1000000)) AND ((anchor_score_micros >= 0) AND (anchor_score_micros <= 5000000)) AND ((edge_weight_micros >= 0) AND (edge_weight_micros <= 2000000)) AND ((hop_penalty_micros >= 0) AND (hop_penalty_micros <= 1000000)) AND ((freshness_score_micros >= 0) AND (freshness_score_micros <= 1000000)) AND ((pin_score_micros >= 0) AND (pin_score_micros <= 1000000)) AND ((current_state_score_micros >= 0) AND (current_state_score_micros <= 1000000)) AND ((final_score_micros >= 0) AND (final_score_micros <= 5000000))))
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
    CONSTRAINT context_graph_steps_position_check CHECK ((((ordinal >= 0) AND (ordinal <= 1)) AND ((hop >= 1) AND (hop <= 2)) AND (ordinal = (hop - 1)))),
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
    CONSTRAINT context_pack_chunks_id_v7 CHECK ((uuid_extract_version(id) = 7)),
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
    CONSTRAINT context_selections_reasons_check CHECK ((((cardinality(reason_codes) >= 1) AND (cardinality(reason_codes) <= 13)) AND (array_position(reason_codes, NULL::text) IS NULL) AND (reason_codes <@ ARRAY['semantic_match'::text, 'keyword_match'::text, 'project_convention'::text, 'personal_preference'::text, 'freshness_boost'::text, 'explicit_pin'::text, 'superseded'::text, 'stale'::text, 'outside_task_scope'::text, 'token_budget'::text, 'duplicate'::text, 'graph_expansion'::text, 'contradiction_warning'::text]))),
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
    state text DEFAULT 'pending'::text NOT NULL,
    proposal_id uuid NOT NULL,
    knowledge_item_id uuid,
    input_hash text NOT NULL,
    attempts integer DEFAULT 0 NOT NULL,
    lease_owner text,
    lease_expires_at timestamp with time zone,
    last_error_code text,
    result jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    started_at timestamp with time zone,
    completed_at timestamp with time zone,
    CONSTRAINT durable_operations_attempts_check CHECK ((attempts >= 0)),
    CONSTRAINT durable_operations_error_check CHECK (((last_error_code IS NULL) OR ((btrim(last_error_code) <> ''::text) AND (char_length(last_error_code) <= 128)))),
    CONSTRAINT durable_operations_input_hash_check CHECK ((input_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT durable_operations_kind_check CHECK ((kind = 'knowledge_erasure'::text)),
    CONSTRAINT durable_operations_lease_owner_check CHECK (((lease_owner IS NULL) OR ((btrim(lease_owner) <> ''::text) AND (char_length(lease_owner) <= 255)))),
    CONSTRAINT durable_operations_result_object_check CHECK ((jsonb_typeof(result) = 'object'::text)),
    CONSTRAINT durable_operations_result_size_check CHECK ((octet_length((result)::text) <= 16384)),
    CONSTRAINT durable_operations_state_check CHECK ((state = ANY (ARRAY['pending'::text, 'running'::text, 'succeeded'::text, 'failed'::text, 'blocked'::text]))),
    CONSTRAINT durable_operations_time_check CHECK ((((state = 'pending'::text) AND (started_at IS NULL) AND (completed_at IS NULL) AND (lease_owner IS NULL) AND (lease_expires_at IS NULL)) OR ((state = 'running'::text) AND (started_at IS NOT NULL) AND (completed_at IS NULL) AND (lease_owner IS NOT NULL) AND (lease_expires_at IS NOT NULL)) OR ((state = ANY (ARRAY['succeeded'::text, 'failed'::text, 'blocked'::text])) AND (completed_at IS NOT NULL) AND (lease_owner IS NULL) AND (lease_expires_at IS NULL))))
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
    CONSTRAINT import_artifacts_body_check CHECK ((octet_length(body_markdown) <= 262144)),
    CONSTRAINT import_artifacts_frontmatter_check CHECK (((jsonb_typeof(frontmatter) = 'object'::text) AND (octet_length((frontmatter)::text) <= 32768))),
    CONSTRAINT import_artifacts_hash_check CHECK ((content_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT import_artifacts_kind_check CHECK ((artifact_kind = ANY (ARRAY['concept'::text, 'index'::text, 'log'::text]))),
    CONSTRAINT import_artifacts_ordinal_check CHECK (((ordinal >= 1) AND (ordinal <= 2000))),
    CONSTRAINT import_artifacts_path_check CHECK (((btrim(logical_path) <> ''::text) AND (char_length(logical_path) <= 1000) AND ("left"(logical_path, 1) <> '/'::text) AND (POSITION(('\'::text) IN (logical_path)) = 0) AND (logical_path !~ '(^|/)\.\.?(/|$)'::text)))
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
    CONSTRAINT import_jobs_count_check CHECK ((((artifact_count >= 1) AND (artifact_count <= 2000)) AND ((mapping_count >= 1) AND (mapping_count <= artifact_count)) AND ((candidate_count >= 0) AND (candidate_count <= mapping_count)))),
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
    CONSTRAINT import_jobs_version_check CHECK ((format_version = '0.2'::text))
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
    CONSTRAINT import_mappings_hash_check CHECK ((content_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT import_mappings_knowledge_type_check CHECK ((knowledge_type = ANY (ARRAY['fact'::text, 'decision'::text, 'preference'::text, 'procedure'::text, 'entity'::text, 'episode'::text, 'convention'::text, 'warning'::text, 'reference'::text]))),
    CONSTRAINT import_mappings_erasure_check CHECK (((NOT content_erased) OR ((title = ''::text) AND (body_markdown = ''::text) AND (summary = ''::text) AND (tags = '{}'::text[]) AND (verification_metadata = '{}'::jsonb) AND (metadata = '{}'::jsonb) AND (proposed_relations = '[]'::jsonb) AND (matched_item_id IS NULL) AND (matched_revision_id IS NULL) AND (NOT materializable)))),
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
    CONSTRAINT knowledge_changes_command_check CHECK ((command_kind = ANY (ARRAY['create'::text, 'edit'::text, 'verify'::text, 'supersede'::text, 'merge'::text, 'archive'::text, 'restore'::text, 'forget'::text, 'resolve_conflict'::text]))),
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
-- Name: knowledge_current; Type: VIEW; Schema: public; Owner: -
--

CREATE VIEW public.knowledge_current WITH (security_invoker='on') AS
 SELECT item.id,
    item.tenant_id,
    item.scope_id,
    item.project_id,
    item.owner_principal_id,
    item.knowledge_type,
    item.origin,
    item.lifecycle_state,
    item.current_revision_id,
    revision.revision_number,
    revision.title,
    revision.body_markdown,
    revision.summary,
    revision.tags,
    revision.sensitivity,
    revision.confidence_permille,
    revision.valid_from,
    revision.valid_to,
    revision.stale_after,
    revision.verification_metadata,
    revision.content_hash,
    revision.metadata,
    revision.created_by AS revision_created_by,
    revision.transaction_time,
    revision.search_document,
    item.created_by,
    item.updated_by,
    item.created_at,
    item.updated_at,
    item.tx_from
   FROM (public.knowledge_items item
     JOIN public.knowledge_revisions revision ON (((revision.tenant_id = item.tenant_id) AND (revision.knowledge_item_id = item.id) AND (revision.id = item.current_revision_id))));


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
-- Name: knowledge_item_versions; Type: VIEW; Schema: public; Owner: -
--

CREATE VIEW public.knowledge_item_versions WITH (security_invoker='on') AS
 SELECT knowledge_items.id,
    knowledge_items.tenant_id,
    knowledge_items.scope_id,
    knowledge_items.project_id,
    knowledge_items.owner_principal_id,
    knowledge_items.knowledge_type,
    knowledge_items.origin,
    knowledge_items.lifecycle_state,
    knowledge_items.current_revision_id,
    knowledge_items.created_by,
    knowledge_items.updated_by,
    knowledge_items.created_at,
    knowledge_items.updated_at,
    knowledge_items.tx_from,
    knowledge_items.tx_to
   FROM public.knowledge_items
UNION ALL
 SELECT knowledge_items_history.id,
    knowledge_items_history.tenant_id,
    knowledge_items_history.scope_id,
    knowledge_items_history.project_id,
    knowledge_items_history.owner_principal_id,
    knowledge_items_history.knowledge_type,
    knowledge_items_history.origin,
    knowledge_items_history.lifecycle_state,
    knowledge_items_history.current_revision_id,
    knowledge_items_history.created_by,
    knowledge_items_history.updated_by,
    knowledge_items_history.created_at,
    knowledge_items_history.updated_at,
    knowledge_items_history.tx_from,
    knowledge_items_history.tx_to
   FROM public.knowledge_items_history;


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
    CONSTRAINT knowledge_sources_content_hash_check CHECK (((content_hash IS NULL) OR (content_hash ~ '^[0-9a-f]{64}$'::text))),
    CONSTRAINT knowledge_sources_created_by_check CHECK (((created_by IS NULL) OR ((btrim(created_by) <> ''::text) AND (char_length(created_by) <= 255)))),
    CONSTRAINT knowledge_sources_locator_check CHECK (((locator IS NULL) OR ((btrim(locator) <> ''::text) AND (char_length(locator) <= 2048)))),
    CONSTRAINT knowledge_sources_metadata_object_check CHECK ((jsonb_typeof(metadata) = 'object'::text)),
    CONSTRAINT knowledge_sources_metadata_size_check CHECK ((octet_length((metadata)::text) <= 16384)),
    CONSTRAINT knowledge_sources_revision_check CHECK (((source_revision IS NULL) OR ((btrim(source_revision) <> ''::text) AND (char_length(source_revision) <= 512)))),
    CONSTRAINT knowledge_sources_shape_check CHECK ((((source_type = 'session_event'::text) AND (session_event_id IS NOT NULL) AND (locator IS NULL)) OR ((source_type = 'manual'::text) AND (session_event_id IS NULL) AND (locator IS NULL) AND (source_revision IS NULL)) OR ((source_type = ANY (ARRAY['document'::text, 'repository'::text, 'url'::text, 'okf'::text, 'system_derived'::text])) AND (session_event_id IS NULL) AND (locator IS NOT NULL)))),
    CONSTRAINT knowledge_sources_type_check CHECK ((source_type = ANY (ARRAY['session_event'::text, 'manual'::text, 'document'::text, 'repository'::text, 'url'::text, 'okf'::text, 'system_derived'::text])))
);

ALTER TABLE ONLY public.knowledge_sources FORCE ROW LEVEL SECURITY;


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
    CONSTRAINT schema_metadata_created_by_version_check CHECK (((length(created_by_version) >= 1) AND (length(created_by_version) <= 64))),
    CONSTRAINT schema_metadata_baseline_revision_check CHECK ((baseline_revision >= 1)),
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
    redactions jsonb,
    CONSTRAINT session_events_client_id_check CHECK (((btrim(client_event_id) <> ''::text) AND (length(client_event_id) <= 200))),
    CONSTRAINT session_events_payload_hash_check CHECK ((payload_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT session_events_payload_object_check CHECK ((jsonb_typeof(payload) = 'object'::text)),
    CONSTRAINT session_events_payload_size_check CHECK ((octet_length((payload)::text) <= 65536)),
    CONSTRAINT session_events_redactions_array_check CHECK (((redactions IS NULL) OR (jsonb_typeof(redactions) = 'array'::text))),
    CONSTRAINT session_events_schema_version_check CHECK (((event_schema_version >= 1) AND (event_schema_version <= 1000))),
    CONSTRAINT session_events_sequence_check CHECK ((sequence >= 1)),
    CONSTRAINT session_events_type_check CHECK ((event_type = ANY (ARRAY['session.started'::text, 'session.ended'::text, 'message.user'::text, 'message.assistant'::text, 'tool.invoked'::text, 'tool.result'::text, 'file.read'::text, 'file.changed'::text, 'command.executed'::text, 'skill.loaded'::text, 'context.requested'::text, 'adapter.warning'::text, 'memory.asserted'::text])))
);

ALTER TABLE ONLY public.session_events FORCE ROW LEVEL SECURITY;


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
    CONSTRAINT tenant_secrets_kind_check CHECK ((kind = ANY (ARRAY['directory'::text, 'tool_server'::text, 'model_provider'::text, 'import_export'::text]))),
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
-- Name: durable_operations durable_operations_pk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.durable_operations
    ADD CONSTRAINT durable_operations_pk PRIMARY KEY (tenant_id, id);


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
-- Name: import_jobs import_jobs_source_digest_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.import_jobs
    ADD CONSTRAINT import_jobs_source_digest_unique UNIQUE (tenant_id, project_id, source_kind, source_locator, bundle_digest);


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
    ADD CONSTRAINT scopes_sibling_slug_unique UNIQUE NULLS NOT DISTINCT (tenant_id, parent_scope_id, slug);


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
-- Name: durable_operations_queue; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX durable_operations_queue ON public.durable_operations USING btree (tenant_id, state, created_at, id) WHERE (state = ANY (ARRAY['pending'::text, 'failed'::text]));


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
-- Name: import_mappings_by_classification; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX import_mappings_by_classification ON public.import_mappings USING btree (tenant_id, job_id, classification, ordinal);


--
-- Name: import_mappings_by_candidate; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX import_mappings_by_candidate ON public.import_mappings USING btree (tenant_id, candidate_id) WHERE (candidate_id IS NOT NULL);


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
-- Name: pending_invites_by_scope; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX pending_invites_by_scope ON public.pending_invites USING btree (tenant_id, scope_id, created_at);


--
-- Name: pending_invites_hash_unique; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX pending_invites_hash_unique ON public.pending_invites USING btree (tenant_id, token_hash);


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
-- Name: scope_grants_unique; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX scope_grants_unique ON public.scope_grants USING btree (tenant_id, scope_id, principal_id, group_id, role_key) NULLS NOT DISTINCT;


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
-- Name: session_events_tenant_id_unique; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX session_events_tenant_id_unique ON public.session_events USING btree (tenant_id, id);


--
-- Name: session_events_tenant_session_id_unique; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX session_events_tenant_session_id_unique ON public.session_events USING btree (tenant_id, session_id, id);


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
-- Name: import_artifacts import_artifacts_append_only; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER import_artifacts_append_only BEFORE DELETE OR UPDATE OR TRUNCATE ON public.import_artifacts FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_import_append_only();


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

CREATE TRIGGER knowledge_relations_append_only BEFORE DELETE OR UPDATE OR TRUNCATE ON public.knowledge_relations FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();


--
-- Name: knowledge_revision_sources knowledge_revision_sources_append_only; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_revision_sources_append_only BEFORE DELETE OR UPDATE OR TRUNCATE ON public.knowledge_revision_sources FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();


--
-- Name: knowledge_revisions knowledge_revisions_append_only; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_revisions_append_only BEFORE DELETE OR UPDATE OR TRUNCATE ON public.knowledge_revisions FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();


--
-- Name: knowledge_revisions knowledge_revisions_require_source; Type: TRIGGER; Schema: public; Owner: -
--

CREATE CONSTRAINT TRIGGER knowledge_revisions_require_source AFTER INSERT ON public.knowledge_revisions DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.synveda_knowledge_revision_has_source();


--
-- Name: knowledge_sources knowledge_sources_append_only; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_sources_append_only BEFORE DELETE OR UPDATE OR TRUNCATE ON public.knowledge_sources FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();


--
-- Name: knowledge_sources knowledge_sources_event_scope; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER knowledge_sources_event_scope BEFORE INSERT ON public.knowledge_sources FOR EACH ROW EXECUTE FUNCTION public.synveda_knowledge_source_event_scope();


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
-- Name: scope_grants scope_grants_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER scope_grants_immutable BEFORE UPDATE ON public.scope_grants FOR EACH ROW EXECUTE FUNCTION public.synveda_grants_are_immutable();


--
-- Name: scopes scopes_immutable_columns; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER scopes_immutable_columns BEFORE UPDATE ON public.scopes FOR EACH ROW EXECUTE FUNCTION public.synveda_scopes_immutable_columns();


--
-- Name: session_context_runs session_context_runs_immutable; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER session_context_runs_immutable BEFORE DELETE OR UPDATE ON public.session_context_runs FOR EACH ROW EXECUTE FUNCTION public.synveda_context_trace_immutable();


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
-- Name: context_selections; Type: ROW SECURITY; Schema: public; Owner: -
--

ALTER TABLE public.context_selections ENABLE ROW LEVEL SECURITY;

--
-- Name: context_selections context_selections_tenant_isolation; Type: POLICY; Schema: public; Owner: -
--

CREATE POLICY context_selections_tenant_isolation ON public.context_selections USING ((tenant_id = public.synveda_current_tenant())) WITH CHECK ((tenant_id = public.synveda_current_tenant()));


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
-- Name: FUNCTION synveda_erase_knowledge(wanted_tenant uuid, wanted_item uuid, wanted_proposal uuid, wanted_operation uuid, wanted_actor_hash text, wanted_reason_hash text); Type: ACL; Schema: public; Owner: -
--

REVOKE ALL ON FUNCTION public.synveda_erase_knowledge(wanted_tenant uuid, wanted_item uuid, wanted_proposal uuid, wanted_operation uuid, wanted_actor_hash text, wanted_reason_hash text) FROM PUBLIC;
GRANT ALL ON FUNCTION public.synveda_erase_knowledge(wanted_tenant uuid, wanted_item uuid, wanted_proposal uuid, wanted_operation uuid, wanted_actor_hash text, wanted_reason_hash text) TO synveda_app;


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
-- Name: COLUMN durable_operations.attempts; Type: ACL; Schema: public; Owner: -
--

GRANT UPDATE(attempts) ON TABLE public.durable_operations TO synveda_app;


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
-- Name: TABLE knowledge_revisions; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.knowledge_revisions TO synveda_app;


--
-- Name: TABLE knowledge_current; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.knowledge_current TO synveda_app;


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
-- Name: TABLE knowledge_items_history; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.knowledge_items_history TO synveda_app;


--
-- Name: TABLE knowledge_item_versions; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT ON TABLE public.knowledge_item_versions TO synveda_app;


--
-- Name: TABLE knowledge_relations; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.knowledge_relations TO synveda_app;


--
-- Name: TABLE knowledge_revision_embeddings; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.knowledge_revision_embeddings TO synveda_app;


--
-- Name: TABLE knowledge_revision_sources; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.knowledge_revision_sources TO synveda_app;


--
-- Name: TABLE knowledge_sources; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT ON TABLE public.knowledge_sources TO synveda_app;


--
-- Name: TABLE pending_invites; Type: ACL; Schema: public; Owner: -
--

GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE public.pending_invites TO synveda_app;


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
