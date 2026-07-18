-- TEN-2: row-level security as the tenant-isolation backstop (ADR-0009).
--
-- Every tenant-scoped table (structurally: any table with a tenant_id
-- column) gets a FORCED RLS policy keyed to the synveda.tenant_id GUC. The
-- application sets the GUC transaction-locally through
-- synveda_store::rls::begin_tenant_tx; a connection that skipped that sees
-- zero tenant-scoped rows. This is defence in depth against application
-- bugs, not against a principal running arbitrary SQL (any role can set
-- any GUC value) — threat model in the ADR.
--
-- Structural rule: a migration that adds a tenant-scoped table must, in
-- the same migration, ENABLE + FORCE row level security, create the tenant
-- policy, and grant synveda_app its least-privilege DML. The completeness
-- guard in crates/synveda-store/tests/rls.rs fails the build when a table
-- with a tenant_id column lacks any of these.

-- The single reading point for the GUC. Unset or empty => NULL => every
-- policy comparison is false: fail closed. A malformed value makes the
-- query error rather than admit rows: also closed.
create function synveda_current_tenant() returns uuid
language sql stable parallel safe
as $$
    select nullif(current_setting('synveda.tenant_id', true), '')::uuid
$$;

-- The role the data plane runs as: non-superuser, no BYPASSRLS, least
-- privilege. Roles are cluster-global, so guard against one left by a
-- sibling database. NOLOGIN: credentials are provisioned per deployment
-- profile (OPS-1/OPS-2), never by a migration; dev and tests reach it via
-- SET LOCAL ROLE from the compose superuser connection.
do $$
begin
    if not exists (select from pg_roles where rolname = 'synveda_app') then
        create role synveda_app nologin;
    end if;
end
$$;

grant usage on schema public to synveda_app;

-- Control-plane registry: tenant resolution reads it before any tenant
-- context exists (ADR-0008), so it is not GUC-keyed. Read-only for the
-- data plane; admission and lifecycle stay owner-role operations
-- (synveda-cli today, TEN-5 later).
grant select on tenants to synveda_app;

-- Data plane. INSERT on records_history is required because the FND-4
-- archive triggers run with invoker rights; history integrity is guarded
-- by the append-only triggers there and the AUD-1 hash chain later, not by
-- withholding the grant.
grant select, insert, update, delete on records to synveda_app;
grant select, insert on records_history to synveda_app;
grant select on records_versions to synveda_app;

-- FORCE: table owners are not exempt. Superusers still bypass RLS by
-- Postgres semantics — deployment profiles must not connect as one.
alter table records enable row level security;
alter table records force row level security;
alter table records_history enable row level security;
alter table records_history force row level security;

-- One policy per table, all commands, all roles: rows are visible and
-- writable iff their tenant matches the transaction's GUC.
create policy records_tenant_isolation on records
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy records_history_tenant_isolation on records_history
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

-- Without this the as-of surface would evaluate base-table RLS as the view
-- OWNER (which may bypass), silently defeating the backstop for history.
alter view records_versions set (security_invoker = on);
