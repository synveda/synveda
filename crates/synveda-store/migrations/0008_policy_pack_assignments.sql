-- AUTHZ-2: per-node policy pack application (ADR-0014).
--
-- Stored packs become named-per-tenant (a tenant may keep several custom
-- packs), and *application* becomes data: a per-node assignment table and
-- a tenant default. The effective pack for a decision is resolved
-- nearest-ancestor-first from these rows at request time — switching a
-- node's pack changes decisions on the very next request. The three
-- product packs (`regulated-strict`, `standard`, `open-collaboration`)
-- are embedded in the binary and have no rows here; their names are
-- reserved so they mean the same thing in every tenant, forever
-- (ADR-0014 decision 6).

-- 1) One row per (tenant, name) instead of one per tenant.
alter table policy_packs drop constraint policy_packs_pk;

-- Rows that used now-reserved names predate the reservation (dev
-- databases only — AUTHZ-1 shipped no admission path for them in
-- production). Rename rather than drop: the content survives, and the
-- tenant-default row created below keeps exactly these semantics in
-- force through the upgrade.
update policy_packs
set name = name || '-legacy'
where name in ('regulated-strict', 'standard', 'open-collaboration', 'bootstrap');

alter table policy_packs
    add constraint policy_packs_pk primary key (tenant_id, name);
alter table policy_packs
    add constraint policy_packs_name_reserved_check
    check (name not in ('regulated-strict', 'standard', 'open-collaboration', 'bootstrap'));

-- 2) The tenant default: what AUTHZ-1's single tenant-wide pack becomes.
-- In force wherever no node on the resource's chain carries an
-- assignment; absent, the embedded `regulated-strict` applies (seed §2.1).
create table policy_pack_defaults (
    tenant_id  uuid        not null,
    pack_name  text        not null,
    updated_at timestamptz not null default now(),

    constraint policy_pack_defaults_pk primary key (tenant_id),
    constraint policy_pack_defaults_tenant_fk
        foreign key (tenant_id) references tenants (id),
    -- Same grammar as pack names (ADR-0008): these surface in logs,
    -- metrics labels, and denial reasons. No reserved-name exclusion:
    -- assigning *to* a product pack is the normal case.
    constraint policy_pack_defaults_name_check
        check (pack_name ~ '^[a-z0-9][a-z0-9-]{0,62}$')
);

-- AUTHZ-1's tenant-wide stored pack keeps deciding for the whole tenant:
-- it becomes the tenant default (one row per tenant existed under the
-- old primary key).
insert into policy_pack_defaults (tenant_id, pack_name)
select tenant_id, name from policy_packs;

-- 3) Per-scope assignments: the scope (and its subtree, until a deeper
-- assignment) runs the named pack. Cascade with the scope: nothing
-- deletes a scope, and an orphaned assignment governs nothing. Re-pointed
-- from `hierarchy_nodes` to `scopes` by CPR-7 (ADR-0074) — the subtree a
-- assignment covers is now `scope_closure`'s.
create table policy_pack_assignments (
    tenant_id  uuid        not null,
    scope_id   uuid        not null,
    pack_name  text        not null,
    updated_at timestamptz not null default now(),

    constraint policy_pack_assignments_pk primary key (tenant_id, scope_id),
    constraint policy_pack_assignments_tenant_fk
        foreign key (tenant_id) references tenants (id),
    -- The scope must belong to the same tenant: a cross-tenant assignment
    -- is unrepresentable (same doctrine as scope parents).
    constraint policy_pack_assignments_scope_fk
        foreign key (tenant_id, scope_id)
        references scopes (tenant_id, id)
        on delete cascade,
    constraint policy_pack_assignments_name_check
        check (pack_name ~ '^[a-z0-9][a-z0-9-]{0,62}$')
);

-- Tenant-scoped tables ⇒ forced RLS + policy + least-privilege grants in
-- the same migration (ADR-0009 structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it).
grant select, insert, update, delete on policy_pack_defaults to synveda_app;
grant select, insert, update, delete on policy_pack_assignments to synveda_app;

alter table policy_pack_defaults enable row level security;
alter table policy_pack_defaults force row level security;
alter table policy_pack_assignments enable row level security;
alter table policy_pack_assignments force row level security;

create policy policy_pack_defaults_tenant_isolation on policy_pack_defaults
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy policy_pack_assignments_tenant_isolation on policy_pack_assignments
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
