-- ADR-0164 Amendment 3: expansion may follow business links, so a retained
-- graph step names either a Knowledge relation or a business link. It holds
-- exactly one of a relation type and a predicate, which hashes-only
-- retention and the erasure scrub both keep, and only that kind's id. A
-- business-link step always supports.
ALTER TABLE public.context_graph_steps
    ADD COLUMN entity_link_id uuid,
    ADD COLUMN predicate text,
    ALTER COLUMN relation_type DROP NOT NULL,
    DROP CONSTRAINT context_graph_steps_address_shape_check,
    DROP CONSTRAINT context_graph_steps_evidence_check,
    DROP CONSTRAINT context_graph_steps_type_check;
ALTER TABLE public.context_graph_steps
    ADD CONSTRAINT context_graph_steps_edge_check CHECK (
        ((relation_type IS NULL) <> (predicate IS NULL))
        AND (relation_type IS NOT NULL OR relation_id IS NULL)
        AND (predicate IS NOT NULL OR entity_link_id IS NULL)),
    ADD CONSTRAINT context_graph_steps_type_check CHECK (
        relation_type IS NULL OR relation_type = ANY (ARRAY['supports'::text, 'contradicts'::text, 'supersedes'::text, 'derived_from'::text, 'references'::text, 'related_to'::text, 'transitions_to'::text])),
    ADD CONSTRAINT context_graph_steps_predicate_check CHECK (predicate IS NULL OR predicate IN ('org:memberOf','org:headOf','org:reportsTo',
        'org:subOrganizationOf','schema:worksFor','schema:owns','prov:wasAttributedTo','dcterms:subject',
        'synveda:customerOf','synveda:dependsOn','synveda:accountableFor')),
    ADD CONSTRAINT context_graph_steps_address_shape_check CHECK (
        ((COALESCE(relation_id, entity_link_id) IS NULL) AND (from_item_id IS NULL)
         AND (from_revision_id IS NULL) AND (to_item_id IS NULL) AND (to_revision_id IS NULL)
         AND (asserting_revision_id IS NULL))
        OR ((COALESCE(relation_id, entity_link_id) IS NOT NULL) AND (from_item_id IS NOT NULL)
         AND (from_revision_id IS NOT NULL) AND (to_item_id IS NOT NULL)
         AND (to_revision_id IS NOT NULL) AND (asserting_revision_id IS NOT NULL))),
    ADD CONSTRAINT context_graph_steps_evidence_check CHECK (
        (supporting
         AND ((relation_type = ANY (ARRAY['supports'::text, 'supersedes'::text, 'derived_from'::text, 'references'::text, 'related_to'::text, 'transitions_to'::text])) OR (predicate IS NOT NULL))
         AND ((edge_weight_micros >= 1) AND (edge_weight_micros <= 1000000)))
        OR ((NOT supporting) AND (relation_type IS NOT NULL) AND (relation_type = 'contradicts'::text)
         AND (edge_weight_micros = 0))),
    ADD CONSTRAINT context_graph_steps_entity_link_fk FOREIGN KEY (tenant_id, entity_link_id)
        REFERENCES public.entity_links(tenant_id, id);
CREATE INDEX context_graph_steps_by_entity_link ON public.context_graph_steps
    USING btree (tenant_id, entity_link_id, created_at) WHERE (entity_link_id IS NOT NULL);

-- The Knowledge erasure scrub now clears a business-link step's link id with
-- its other addresses. The body is otherwise unchanged.
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
               and new_row -> 'entity_link_id' = 'null'::jsonb
               and new_row -> 'from_item_id' = 'null'::jsonb
               and new_row -> 'from_revision_id' = 'null'::jsonb
               and new_row -> 'to_item_id' = 'null'::jsonb
               and new_row -> 'to_revision_id' = 'null'::jsonb
               and new_row -> 'asserting_revision_id' = 'null'::jsonb
               and (new_row - array[
                       'relation_id', 'entity_link_id', 'from_item_id', 'from_revision_id',
                       'to_item_id', 'to_revision_id', 'asserting_revision_id'
                   ])
                   = (old_row - array[
                       'relation_id', 'entity_link_id', 'from_item_id', 'from_revision_id',
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


-- Knowledge erasure deletes an item's links by cascade. A retained step that
-- followed one keeps its hashes and vocabulary and loses every address, as a
-- step over an erased item's relation does; the trace trigger admits that
-- scrub only inside governed Knowledge erasure. Tenant erasure deletes the
-- steps before the links, so it finds none.
CREATE FUNCTION public.synveda_entity_link_trace_scrub() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER SET search_path TO 'public', 'pg_temp'
    AS $$
begin
    update public.context_graph_steps
       set entity_link_id = null,
           from_item_id = null,
           from_revision_id = null,
           to_item_id = null,
           to_revision_id = null,
           asserting_revision_id = null
     where tenant_id = old.tenant_id and entity_link_id = old.id;
    return old;
end
$$;
REVOKE ALL ON FUNCTION public.synveda_entity_link_trace_scrub() FROM PUBLIC;
CREATE TRIGGER entity_links_trace_scrub BEFORE DELETE ON public.entity_links
    FOR EACH ROW EXECUTE FUNCTION public.synveda_entity_link_trace_scrub();
