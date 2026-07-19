-- AUTHZ-3: role bindings (ADR-0015).
--
-- A binding attaches one product role to a token subject at a hierarchy
-- node; the node's whole subtree holds it (the PDP resolves "inherited
-- downward" from the resource's chain at decision time). A null scope
-- binds at the tenant itself — the top of the inheritance chain — which
-- is what makes a fresh tenant governable before any hierarchy exists,
-- and is where the `synveda-admins` convention group lands org-admin.
--
-- Subject-keyed, not an identities FK (ADR-0015 decision 2): the PDP's
-- principal is (tenant, subject); a binding may precede first login
-- (pre-binding), and dev subjects — which never provision — stay
-- bindable.

create table role_bindings (
    tenant_id  uuid        not null,
    subject    text        not null,
    scope_id   uuid,
    role       text        not null,
    updated_at timestamptz not null default now(),

    constraint role_bindings_tenant_fk
        foreign key (tenant_id) references tenants (id),
    -- The bound node must belong to the same tenant: a cross-tenant
    -- binding is unrepresentable (same doctrine as hierarchy parents).
    -- Cascade with the node: HIER-1 deletes are leaf-only, and a deleted
    -- leaf's binding grants nothing.
    constraint role_bindings_scope_fk
        foreign key (tenant_id, scope_id)
        references hierarchy_nodes (tenant_id, id)
        on delete cascade,
    -- The closed product vocabulary (ADR-0015 decision 1); mirrors
    -- synveda_types::Role, which is the in-process guard for the same
    -- rule.
    constraint role_bindings_role_check
        check (role in ('viewer', 'contributor', 'curator', 'steward',
                        'org-admin', 'auditor', 'security-reviewer',
                        'compliance')),
    constraint role_bindings_subject_check
        check (length(subject) between 1 and 255)
);

-- One row per (tenant, subject, node, role); `nulls not distinct` makes
-- the tenant-wide (null-scope) binding unique too, and is the arbiter
-- index the bind upsert infers.
create unique index role_bindings_unique
    on role_bindings (tenant_id, subject, scope_id, role)
    nulls not distinct;

-- The per-node listing (`GET .../nodes/{id}/roles`).
create index role_bindings_by_scope on role_bindings (tenant_id, scope_id);

-- Tenant-scoped tables ⇒ forced RLS + policy + least-privilege grants in
-- the same migration (ADR-0009 structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it).
grant select, insert, update, delete on role_bindings to synveda_app;

alter table role_bindings enable row level security;
alter table role_bindings force row level security;

create policy role_bindings_tenant_isolation on role_bindings
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
