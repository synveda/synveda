-- ADR-0157: every foreign key that closes a table-level cycle among tenant
-- tables, and every self-reference, becomes deferrable but stays initially
-- immediate. Ordinary transactions keep checking each row at statement end;
-- only a restore that sets these constraints deferred may load a tenant's
-- rows in one transaction and have them checked at commit.
ALTER TABLE public.capture_batches
 ALTER CONSTRAINT capture_batches_import_job_fk DEFERRABLE INITIALLY IMMEDIATE;
ALTER TABLE public.import_jobs
 ALTER CONSTRAINT import_jobs_capture_batch_fk DEFERRABLE INITIALLY IMMEDIATE;
ALTER TABLE public.import_jobs
 ALTER CONSTRAINT import_jobs_tenant_id_project_id_managed_source_id_fkey DEFERRABLE INITIALLY IMMEDIATE;
ALTER TABLE public.scopes
 ALTER CONSTRAINT scopes_parent_fk DEFERRABLE INITIALLY IMMEDIATE;
ALTER TABLE public.source_observations
 ALTER CONSTRAINT source_observations_tenant_id_import_job_id_fkey DEFERRABLE INITIALLY IMMEDIATE;
ALTER TABLE public.source_observations
 ALTER CONSTRAINT source_observations_tenant_id_operation_id_fkey DEFERRABLE INITIALLY IMMEDIATE;
ALTER TABLE public.source_observations
 ALTER CONSTRAINT source_observations_tenant_id_previous_id_fkey DEFERRABLE INITIALLY IMMEDIATE;
ALTER TABLE public.source_observations
 ALTER CONSTRAINT source_observations_tenant_id_project_id_source_id_fkey DEFERRABLE INITIALLY IMMEDIATE;
ALTER TABLE public.source_reconcile_heads
 ALTER CONSTRAINT source_reconcile_heads_tenant_id_observation_id_fkey DEFERRABLE INITIALLY IMMEDIATE;
ALTER TABLE public.source_reconcile_requests
 ALTER CONSTRAINT source_reconcile_requests_tenant_id_previous_observation_i_fkey DEFERRABLE INITIALLY IMMEDIATE;
ALTER TABLE public.source_reconcile_requests
 ALTER CONSTRAINT source_reconcile_requests_tenant_id_project_id_source_id_fkey DEFERRABLE INITIALLY IMMEDIATE;
