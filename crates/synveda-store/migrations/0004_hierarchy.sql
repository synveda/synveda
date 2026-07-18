-- HIER-1: the tenancy hierarchy — closure table + materialised path
-- (ADR-0011).
--
-- hierarchy_nodes is the adjacency ground truth (parent_id) plus the
-- human-facing materialised path; hierarchy_closure holds every
-- (ancestor, descendant, distance) pair including distance-0 self-rows and
-- is the structure ancestor/descendant queries scan. Closure maintenance
-- is explicit store code (synveda_store::hierarchy) run inside the
-- caller's transaction — no triggers (ADR-0011 decision 2).
--
-- The kind-rank rule (child kind strictly outranks parent kind) needs the
-- parent row and therefore lives in the store, not in a CHECK. Everything
-- row-local is enforced here.

create table hierarchy_nodes (
    id         uuid        not null,
    tenant_id  uuid        not null,
    parent_id  uuid,
    kind       text        not null,
    slug       text        not null,
    name       text        not null,
    -- Edges from the root; kept in step with the closure by the store.
    depth      integer     not null,
    -- Slug chain from the root (e.g. 'acme/emea/payments'). Display and
    -- ordering only — never an authorisation input (ADR-0011).
    path       text        not null,
    created_at timestamptz not null default now(),

    constraint hierarchy_nodes_pk primary key (id),
    constraint hierarchy_nodes_tenant_fk
        foreign key (tenant_id) references tenants (id),
    -- Composite key target so same-tenant FKs below are expressible.
    constraint hierarchy_nodes_tenant_id_unique unique (tenant_id, id),
    -- A parent must be a node of the same tenant: a cross-tenant edge is
    -- unrepresentable, not merely forbidden.
    constraint hierarchy_nodes_parent_fk
        foreign key (tenant_id, parent_id)
        references hierarchy_nodes (tenant_id, id),
    constraint hierarchy_nodes_kind_check
        check (kind in ('org', 'division', 'department', 'team', 'user')),
    -- The root (and only the root) is the org: both directions, row-local.
    constraint hierarchy_nodes_root_is_org_check
        check ((parent_id is null) = (kind = 'org')),
    -- Same grammar as tenants.slug (ADR-0008): URL/hostname/CLI-safe.
    constraint hierarchy_nodes_slug_check
        check (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'),
    constraint hierarchy_nodes_depth_check check (depth >= 0),
    -- Sibling slugs are unique; NULLS NOT DISTINCT so the rule also binds
    -- root rows (belt to the one-root index's braces).
    constraint hierarchy_nodes_sibling_slug_unique
        unique nulls not distinct (tenant_id, parent_id, slug),
    constraint hierarchy_nodes_path_unique unique (tenant_id, path)
);

-- One root per tenant.
create unique index hierarchy_nodes_one_root_per_tenant
    on hierarchy_nodes (tenant_id)
    where parent_id is null;

-- Children listing and the leaf-only delete check walk the adjacency.
create index hierarchy_nodes_parent_idx on hierarchy_nodes (parent_id);

create table hierarchy_closure (
    tenant_id     uuid    not null,
    ancestor_id   uuid    not null,
    descendant_id uuid    not null,
    -- Edges between the pair; 0 = the self-row.
    distance      integer not null,

    -- Descendant-of-X queries scan this as an index prefix.
    constraint hierarchy_closure_pk primary key (ancestor_id, descendant_id),
    constraint hierarchy_closure_distance_check check (distance >= 0),
    -- Same-tenant by construction; closure rows die with their nodes
    -- (leaf-only delete keeps cascades to the deleted node's own rows).
    constraint hierarchy_closure_ancestor_fk
        foreign key (tenant_id, ancestor_id)
        references hierarchy_nodes (tenant_id, id) on delete cascade,
    constraint hierarchy_closure_descendant_fk
        foreign key (tenant_id, descendant_id)
        references hierarchy_nodes (tenant_id, id) on delete cascade
);

-- Ancestor-of-X queries scan by descendant.
create index hierarchy_closure_descendant_idx
    on hierarchy_closure (descendant_id);

-- Tenant-scoped tables ⇒ forced RLS + policy + least-privilege grants in
-- the same migration (ADR-0009 structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it). Closure rows are only
-- ever inserted and deleted — no UPDATE grant.
grant select, insert, update, delete on hierarchy_nodes to synveda_app;
grant select, insert, delete on hierarchy_closure to synveda_app;

alter table hierarchy_nodes enable row level security;
alter table hierarchy_nodes force row level security;
alter table hierarchy_closure enable row level security;
alter table hierarchy_closure force row level security;

create policy hierarchy_nodes_tenant_isolation on hierarchy_nodes
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy hierarchy_closure_tenant_isolation on hierarchy_closure
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
