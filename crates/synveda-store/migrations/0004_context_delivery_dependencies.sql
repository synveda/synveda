-- MEM-8 / ADR-0144: private lifecycle proof is independent of trace retention.
-- Apply with application writers stopped. Published 0001-0003 remain unchanged.

CREATE TABLE public.context_run_delivery_dependencies (
    tenant_id uuid NOT NULL,
    context_run_id uuid NOT NULL,
    dependency_version integer NOT NULL,
    manifest_hash text NOT NULL,
    manifest jsonb NOT NULL,
    created_at timestamptz DEFAULT now() NOT NULL,
    PRIMARY KEY (tenant_id, context_run_id),
    CONSTRAINT context_delivery_run_fk FOREIGN KEY (tenant_id, context_run_id)
        REFERENCES public.session_context_runs (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT context_delivery_version_check CHECK (dependency_version = 1),
    CONSTRAINT context_delivery_hash_check CHECK (manifest_hash ~ '^[0-9a-f]{64}$'),
    CONSTRAINT context_delivery_manifest_check CHECK (
        jsonb_typeof(manifest) = 'object' AND octet_length(manifest::text) <= 524288
    )
);
ALTER TABLE public.context_run_delivery_dependencies ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.context_run_delivery_dependencies FORCE ROW LEVEL SECURITY;
CREATE POLICY context_delivery_tenant ON public.context_run_delivery_dependencies
    USING (tenant_id = public.synveda_current_tenant())
    WITH CHECK (tenant_id = public.synveda_current_tenant());
GRANT SELECT, INSERT ON public.context_run_delivery_dependencies TO synveda_app;
CREATE INDEX context_delivery_manifest ON public.context_run_delivery_dependencies
    USING gin (manifest jsonb_path_ops);
CREATE TRIGGER context_delivery_append_only
    BEFORE UPDATE OR DELETE ON public.context_run_delivery_dependencies
    FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_capture_append_only();

-- The released erasure routine deletes revisions only after validating the
-- running governed operation and inserting its content-free tombstone. The
-- trigger adds all-mode delivery scrubbing without granting application DML.
CREATE FUNCTION public.synveda_context_delivery_knowledge_erasure() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER SET search_path TO 'public', 'pg_temp'
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
REVOKE ALL ON FUNCTION public.synveda_context_delivery_knowledge_erasure() FROM PUBLIC;
CREATE TRIGGER knowledge_revision_delivery_erasure
    AFTER DELETE ON public.knowledge_revisions FOR EACH ROW
    EXECUTE FUNCTION public.synveda_context_delivery_knowledge_erasure();
