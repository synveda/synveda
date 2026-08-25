-- CPR-33: structured context-platform audit filters and frozen export
-- (ADR-0092).
--
-- `audit_log` is canonical hash input. Adding a query column either rewrites
-- every historical hash or leaves an unsigned field beside the chain; both
-- are forbidden by ADR-0045. Typed artifact, session and context-run
-- addresses already live inside the hashed payload, so this migration adds
-- only a tenant-leading containment index. It changes no event byte and old
-- chain heads continue to verify.
--
-- Unlike the partial disclosure index, the typed query spans VedaFlow,
-- Knowledge, context, Skills, Tools, Configuration and relaxation actions.
-- A whole-payload index is therefore the honest bounded shape; every query
-- still carries an exact tenant predicate before JSON containment.

create index audit_log_tenant_payload_idx
    on audit_log using gin (tenant_id, payload jsonb_path_ops);
