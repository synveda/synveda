-- AUTHZ-1: the per-tenant policy pack store (ADR-0012 decision 5).
--
-- One active pack row per tenant: Cedar source plus a monotonically
-- bumped version (the store's `apply` upsert owns the bump). The gateway's
-- refresher reads this table per tenant and hot-swaps compiled packs;
-- a tenant with no row runs the embedded `bootstrap` pack. History and
-- per-node application arrive with AUTHZ-2, and VedaFlow eventually makes
-- packs governed assets (tech plan §2.3).

create table policy_packs (
    tenant_id  uuid        not null,
    name       text        not null,
    version    bigint      not null,
    source     text        not null,
    updated_at timestamptz not null default now(),

    constraint policy_packs_pk primary key (tenant_id),
    constraint policy_packs_tenant_fk
        foreign key (tenant_id) references tenants (id),
    -- Same grammar as tenant and hierarchy slugs (ADR-0008): pack names
    -- surface in logs, metrics labels, and denial reasons.
    constraint policy_packs_name_check
        check (name ~ '^[a-z0-9][a-z0-9-]{0,62}$'),
    constraint policy_packs_version_check check (version >= 1),
    constraint policy_packs_source_check check (length(source) > 0)
);

-- Tenant-scoped tables ⇒ forced RLS + policy + least-privilege grants in
-- the same migration (ADR-0009 structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it).
grant select, insert, update, delete on policy_packs to synveda_app;

alter table policy_packs enable row level security;
alter table policy_packs force row level security;

create policy policy_packs_tenant_isolation on policy_packs
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
