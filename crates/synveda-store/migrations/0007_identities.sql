-- AUTH-2: provisioned identities and group-mapping overrides (ADR-0013).
--
-- identities binds a verified token subject to its personal user-kind
-- scope node; JIT provisioning inserts exactly one row per (tenant,
-- subject) at first login. Quarantined status is derived from placement
-- (the user node's parent is the tenant's reserved `quarantine` scope),
-- never stored — no flag to drift (ADR-0013 decision 4).
--
-- group_mappings overrides the `synveda-{dept}-{team}` convention: an
-- exact IdP group name maps to any non-user scope. Managed at the store
-- level for now, like policy packs pre-AUTHZ-2 (ADR-0013 decision 3).

create table identities (
    id           uuid        not null,
    tenant_id    uuid        not null,
    subject      text        not null,
    email        text,
    display_name text,
    scope_id     uuid        not null,
    created_at   timestamptz not null default now(),

    constraint identities_pk primary key (id),
    constraint identities_tenant_fk
        foreign key (tenant_id) references tenants (id),
    -- One identity per subject per tenant; the JIT race resolves here
    -- (the losing login retries and adopts the winner's identity).
    constraint identities_subject_unique unique (tenant_id, subject),
    -- The personal scope must be a node of the same tenant: a cross-tenant
    -- binding is unrepresentable (same doctrine as hierarchy parents). No
    -- cascade: an identity pins its node — leavers are AUTH-4's feature,
    -- not a delete surprise.
    constraint identities_scope_fk
        foreign key (tenant_id, scope_id)
        references hierarchy_nodes (tenant_id, id),
    -- One node is one person's personal scope, never two.
    constraint identities_scope_unique unique (tenant_id, scope_id),
    constraint identities_subject_check
        check (length(subject) between 1 and 255)
);

create table group_mappings (
    tenant_id  uuid        not null,
    group_name text        not null,
    scope_id   uuid        not null,
    created_at timestamptz not null default now(),

    -- Exact-match lookup by group name; one target per name.
    constraint group_mappings_pk primary key (tenant_id, group_name),
    constraint group_mappings_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint group_mappings_scope_fk
        foreign key (tenant_id, scope_id)
        references hierarchy_nodes (tenant_id, id),
    constraint group_mappings_name_check
        check (length(group_name) between 1 and 255)
);

-- Tenant-scoped tables ⇒ forced RLS + policy + least-privilege grants in
-- the same migration (ADR-0009 structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it). Identities are only
-- created for now — movers/leavers (AUTH-4/5) bring update/delete with
-- their own migrations; mappings are admin-curated.
grant select, insert on identities to synveda_app;
grant select, insert, update, delete on group_mappings to synveda_app;

alter table identities enable row level security;
alter table identities force row level security;
alter table group_mappings enable row level security;
alter table group_mappings force row level security;

create policy identities_tenant_isolation on identities
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy group_mappings_tenant_isolation on group_mappings
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
