-- ADR-0164: business links between Knowledge items. A link and its end are
-- separate append-only rows, so ending a link never rewrites it and history
-- stays readable at any transaction time. Both cascade from the items they
-- join, so governed item erasure removes them with the item.
ALTER TABLE public.knowledge_changes DROP CONSTRAINT knowledge_changes_command_check;
ALTER TABLE public.knowledge_changes ADD CONSTRAINT knowledge_changes_command_check CHECK
 (command_kind IN ('create','edit','verify','supersede','merge','archive','restore','forget','resolve_conflict',
  'control_source','control_artifact','link','end_link'));

CREATE TABLE public.entity_links (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    source_item_id uuid NOT NULL,
    target_item_id uuid NOT NULL,
    asserting_revision_id uuid NOT NULL,
    predicate text NOT NULL,
    valid_from timestamp with time zone NOT NULL,
    created_by text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    PRIMARY KEY (id),
    CONSTRAINT entity_links_tenant_id_unique UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id) REFERENCES public.tenants(id),
    CONSTRAINT entity_links_source_fk FOREIGN KEY (tenant_id, source_item_id)
        REFERENCES public.knowledge_items(tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT entity_links_target_fk FOREIGN KEY (tenant_id, target_item_id)
        REFERENCES public.knowledge_items(tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT entity_links_asserting_revision_fk FOREIGN KEY (tenant_id, source_item_id, asserting_revision_id)
        REFERENCES public.knowledge_revisions(tenant_id, knowledge_item_id, id) ON DELETE CASCADE,
    CONSTRAINT entity_links_distinct_items_check CHECK (source_item_id <> target_item_id),
    CONSTRAINT entity_links_created_by_check
        CHECK (created_by IS NULL OR (btrim(created_by) <> '' AND char_length(created_by) <= 255)),
    CONSTRAINT entity_links_predicate_check CHECK (predicate IN ('org:memberOf','org:headOf','org:reportsTo',
        'org:subOrganizationOf','schema:worksFor','schema:owns','prov:wasAttributedTo','dcterms:subject',
        'synveda:customerOf','synveda:dependsOn','synveda:accountableFor'))
);
CREATE INDEX entity_links_from ON public.entity_links (tenant_id, source_item_id, created_at);
CREATE INDEX entity_links_to ON public.entity_links (tenant_id, target_item_id, created_at);
ALTER TABLE public.entity_links ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.entity_links FORCE ROW LEVEL SECURITY;
CREATE POLICY entity_links_tenant_isolation ON public.entity_links
    USING (tenant_id = public.synveda_current_tenant())
    WITH CHECK (tenant_id = public.synveda_current_tenant());
CREATE TRIGGER entity_links_append_only BEFORE DELETE OR UPDATE OR TRUNCATE ON public.entity_links
    FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();
GRANT SELECT, INSERT ON TABLE public.entity_links TO synveda_app;

-- At most one end per link; valid_to is exclusive.
CREATE TABLE public.entity_link_ends (
    tenant_id uuid NOT NULL,
    link_id uuid NOT NULL,
    valid_to timestamp with time zone NOT NULL,
    ended_by text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    PRIMARY KEY (tenant_id, link_id),
    FOREIGN KEY (tenant_id) REFERENCES public.tenants(id),
    CONSTRAINT entity_link_ends_link_fk FOREIGN KEY (tenant_id, link_id)
        REFERENCES public.entity_links(tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT entity_link_ends_ended_by_check
        CHECK (ended_by IS NULL OR (btrim(ended_by) <> '' AND char_length(ended_by) <= 255))
);
ALTER TABLE public.entity_link_ends ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.entity_link_ends FORCE ROW LEVEL SECURITY;
CREATE POLICY entity_link_ends_tenant_isolation ON public.entity_link_ends
    USING (tenant_id = public.synveda_current_tenant())
    WITH CHECK (tenant_id = public.synveda_current_tenant());
CREATE TRIGGER entity_link_ends_append_only BEFORE DELETE OR UPDATE OR TRUNCATE ON public.entity_link_ends
    FOR EACH STATEMENT EXECUTE FUNCTION public.synveda_knowledge_append_only();
GRANT SELECT, INSERT ON TABLE public.entity_link_ends TO synveda_app;
