-- OPS-11 / CPR-45, ADR-0152: converge the published lineage on the
-- PostgreSQL 14 compatible schema that fresh installations receive from the
-- portable baseline. Prior migration bytes remain immutable.

-- Knowledge reads join the forced-RLS tables in caller-executed SQL, so no
-- view can evaluate row security as its owner.
DROP VIEW public.knowledge_current;
DROP VIEW public.knowledge_item_versions;

-- scopes_one_root_per_tenant already admits one parentless scope per tenant,
-- so a NULL parent cannot repeat a slug.
ALTER TABLE public.scopes DROP CONSTRAINT scopes_sibling_slug_unique;
ALTER TABLE public.scopes ADD CONSTRAINT scopes_sibling_slug_unique
 UNIQUE (tenant_id, parent_scope_id, slug);

-- The subject shape checks leave exactly one of principal_id and group_id set.
DROP INDEX public.scope_grants_unique;
CREATE UNIQUE INDEX scope_grants_principal_unique ON public.scope_grants
 USING btree (tenant_id, scope_id, principal_id, role_key) WHERE (group_id IS NULL);
CREATE UNIQUE INDEX scope_grants_group_unique ON public.scope_grants
 USING btree (tenant_id, scope_id, group_id, role_key) WHERE (principal_id IS NULL);

-- The empty string cannot collide: import_job_repository_shape and
-- import_managed_source_shape require a 64-digit hex digest or a non-empty
-- source slug whenever these columns are set.
ALTER TABLE public.import_jobs DROP CONSTRAINT import_jobs_source_digest_unique;
CREATE UNIQUE INDEX import_jobs_source_digest_unique ON public.import_jobs
 USING btree (tenant_id, project_id, source_kind, source_locator, bundle_digest,
  (COALESCE(repository_identity_digest, ''::text)), (COALESCE(managed_source_id, ''::text)),
  (COALESCE(managed_review_digest, ''::text)));

-- Equivalent to uuid_extract_version(id) = 7, which is NULL, and so passes,
-- for UUIDs outside the RFC 9562 variant.
ALTER TABLE public.context_pack_chunks DROP CONSTRAINT context_pack_chunks_id_v7;
ALTER TABLE public.context_pack_chunks ADD CONSTRAINT context_pack_chunks_id_v7
 CHECK (((get_byte(uuid_send(id), 8) & 192) <> 128) OR ((get_byte(uuid_send(id), 6) >> 4) = 7));
