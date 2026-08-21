-- CPR-3: the generic governed scope substrate (ADR-0068 decision 4,
-- ADR-0070). Moved to this slot by CPR-7 (ADR-0074): this migration file
-- created `hierarchy_nodes` + `hierarchy_closure` until the hierarchy
-- cutover deleted them, and the scope substrate — which had arrived as
-- migration 0040, after the identities and pack assignments that must
-- reference it — takes their place at the head of the chain. A fresh
-- database is the only database this chain accepts (CPR-2, ADR-0069); the
-- epoch was bumped with it, so a pre-cutover database is refused with the
-- reset instruction rather than migrated.
--
-- A scope is a named node with a parent and a subtree. `scopes` is the
-- adjacency ground truth; `scope_closure` holds every (ancestor, descendant,
-- distance) pair including the distance-0 self-row, and is what ancestor and
-- descendant queries scan. Closure maintenance is explicit store code
-- (synveda_store::scopes) run inside the caller's transaction — no triggers,
-- the rule ADR-0011 decision 2 set and this substrate keeps.
--
-- ── What this is not ─────────────────────────────────────────────────────
--
-- It is not the fixed tenancy hierarchy this slot used to create. That model
-- encoded a five-value rank vocabulary (org, division, department, team,
-- user) in a CHECK, a root-must-be-an-org CHECK, and a store-side rule that
-- a child must strictly outrank its parent — so an individual was required
-- to declare an organisation containing a team before this product would
-- hold a record. `kind` here is a **shape**, not a rank: it decides only
-- which kinds may be a scope's parent, `org_unit` nests inside itself to
-- arbitrary depth, and nothing anywhere compares two kinds for order. There
-- is one tree and this is it (ADR-0074 decision 1).
--
-- ── Where each structural rule is enforced ───────────────────────────────
--
-- Every rule that can be a database fact is one, because the alternative is
-- a rule that holds only for callers who remembered:
--
--   tenant has no parent                 CHECK scopes_root_shape_check
--   one tenant-root per tenant           UNIQUE INDEX scopes_one_root_per_tenant
--   org_unit under tenant|org_unit       CHECK scopes_placement_check + the parent FK
--   workspace under tenant|org_unit      "
--   project under workspace              "
--   principal under tenant|org_unit|workspace  "
--   sibling slugs unique under a parent  UNIQUE scopes_sibling_slug_unique
--   never moves across tenants           composite parent FK + the immutability
--                                        trigger + forced RLS
--   cycles are impossible                CHECK scope_closure_self_row_check
--                                        (see below — this one is worth reading)
--
-- Only two rules need the parent row and therefore also live in the store:
-- the placement rule's *error message* (the CHECK refuses it; the store says
-- which kinds would have been legal) and the descendant check a move makes
-- before it starts, so a cycle is refused as an error rather than as an
-- aborted transaction.

-- ── The placement rule as a foreign key ──────────────────────────────────
--
-- `parent_kind` is a denormalised copy of the parent's `kind`, and it exists
-- so the placement rule can be a row-local CHECK. The copy cannot drift: the
-- parent foreign key is composite over (tenant_id, parent_scope_id,
-- parent_kind) and targets (tenant_id, id, kind), so a row whose
-- `parent_kind` disagrees with its parent's `kind` has no referent and cannot
-- be written. `kind` is immutable (the trigger below), so the reference
-- cannot be invalidated from the other end either.
--
-- Carrying the tenant in that same key is what makes "a scope can never move
-- across tenants" structural rather than procedural: a parent in another
-- tenant is not forbidden, it is unrepresentable.

create table scopes (
    id               uuid        not null,
    tenant_id        uuid        not null,
    kind             text        not null,
    parent_scope_id  uuid,
    -- The parent's kind. Never read by application code; it exists for the
    -- placement CHECK below. Kept honest by the composite FK, not by a
    -- trigger and not by the store.
    parent_kind      text,
    -- Human-stable handle, unique among siblings, immutable. Same grammar as
    -- a tenant slug (ADR-0008): URL-, hostname- and CLI-safe, so a scope path
    -- is a thing somebody can type.
    slug             text        not null,
    display_name     text        not null,
    status           text        not null default 'active',
    -- Open labelling bag: what a deployment means by a scope is the
    -- deployment's to say. Never an authorisation input.
    attributes       jsonb       not null default '{}'::jsonb,
    -- The identity that created the scope. Nullable, and deliberately not a
    -- foreign key: a tenant root minted at admission has no author.
    created_by       uuid,
    created_at       timestamptz not null default now(),
    updated_at       timestamptz not null default now(),

    constraint scopes_pk primary key (id),
    constraint scopes_tenant_fk foreign key (tenant_id) references tenants (id),
    -- Composite key targets, so the same-tenant foreign keys below and in
    -- scope_closure are expressible.
    constraint scopes_tenant_id_unique unique (tenant_id, id),
    constraint scopes_tenant_id_kind_unique unique (tenant_id, id, kind),
    constraint scopes_parent_fk
        foreign key (tenant_id, parent_scope_id, parent_kind)
        references scopes (tenant_id, id, kind),

    constraint scopes_kind_check
        check (kind in ('tenant', 'org_unit', 'workspace', 'project', 'principal')),
    constraint scopes_status_check
        check (status in ('active', 'archived')),
    -- The tenant root, and only the tenant root, has no parent: both
    -- directions, row-local.
    constraint scopes_root_shape_check
        check ((parent_scope_id is null) = (kind = 'tenant')),
    -- `parent_kind` is present exactly when a parent is. Without this the
    -- composite FK would be satisfied vacuously by a null `parent_kind` on a
    -- row that names a parent (MATCH SIMPLE), and the placement rule would
    -- have a hole the size of one null.
    constraint scopes_parent_kind_present_check
        check ((parent_scope_id is null) = (parent_kind is null)),
    -- The placement rule. `else false` rather than `else null`: an unknown
    -- kind must fail this CHECK, not pass it — a CHECK that evaluates to NULL
    -- is satisfied.
    --
    -- A `principal` nests under `tenant`, `org_unit` or `workspace`
    -- (ADR-0074 decision 3): a person's own scope hangs at the root when
    -- login mints it, and a service identity's scope hangs under the scope an
    -- operator registered it at — which is ADR-0018 decision 4's confinement
    -- anchor, expressed as tree position rather than as a derived lookup.
    constraint scopes_placement_check
        check (
            case kind
                when 'tenant'    then parent_kind is null
                when 'org_unit'  then parent_kind in ('tenant', 'org_unit')
                when 'workspace' then parent_kind in ('tenant', 'org_unit')
                when 'project'   then parent_kind = 'workspace'
                when 'principal' then parent_kind in ('tenant', 'org_unit', 'workspace')
                else false
            end
        ),
    constraint scopes_slug_check
        check (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'),
    constraint scopes_display_name_check
        check (btrim(display_name) <> '' and length(display_name) <= 200),
    constraint scopes_attributes_object_check
        check (jsonb_typeof(attributes) = 'object'),
    -- A backstop rather than the bound: `synveda_types::scope` refuses over
    -- 16 KiB of the *caller's* encoding, and Postgres renders jsonb with its
    -- own spacing, so the two measurements are close rather than identical.
    -- Deliberately: what a bound on a governed row read on every chain walk
    -- has to stop is a blob, and both numbers stop one.
    constraint scopes_attributes_size_check
        check (octet_length(attributes::text) <= 16384),
    constraint scopes_updated_check check (updated_at >= created_at),
    -- Sibling slugs are unique. NULLS NOT DISTINCT so the rule also binds
    -- root rows, which is belt to the one-root index's braces.
    constraint scopes_sibling_slug_unique
        unique nulls not distinct (tenant_id, parent_scope_id, slug)
);

-- One tenant root per tenant.
create unique index scopes_one_root_per_tenant
    on scopes (tenant_id)
    where parent_scope_id is null;

-- Children listing and the parent lock walk the adjacency.
create index scopes_parent_idx on scopes (tenant_id, parent_scope_id);

-- ── Cycles are impossible, and the closure is where that is decided ──────
--
-- A cycle needs a scope to be its own ancestor at a distance greater than
-- zero. `scope_closure_self_row_check` says a row where ancestor and
-- descendant are the same scope has distance 0, and the primary key says
-- that row exists exactly once — so the row a cycle would require cannot be
-- written. This is not a belt-and-braces restatement of the store's
-- descendant check: a move's relink cross-joins the destination's ancestry
-- with the moved subtree, and if the destination is inside that subtree the
-- product *contains* (X, X, distance > 0). The transaction aborts. The store
-- checks first so that the ordinary refusal is an error with a sentence in
-- it; this is what holds when something reaches these tables another way.

create table scope_closure (
    tenant_id     uuid    not null,
    ancestor_id   uuid    not null,
    descendant_id uuid    not null,
    -- Edges between the pair; 0 = the self-row.
    distance      integer not null,

    -- Descendant-of-X queries scan this as an index prefix.
    constraint scope_closure_pk primary key (ancestor_id, descendant_id),
    constraint scope_closure_distance_check check (distance >= 0),
    constraint scope_closure_self_row_check
        check ((ancestor_id = descendant_id) = (distance = 0)),
    -- Same-tenant by construction; closure rows die with their scope.
    constraint scope_closure_ancestor_fk
        foreign key (tenant_id, ancestor_id)
        references scopes (tenant_id, id) on delete cascade,
    constraint scope_closure_descendant_fk
        foreign key (tenant_id, descendant_id)
        references scopes (tenant_id, id) on delete cascade
);

-- Ancestor-of-X queries scan by descendant.
create index scope_closure_descendant_idx
    on scope_closure (descendant_id);

-- ── Immutability ─────────────────────────────────────────────────────────
--
-- Forced RLS already stops the application role from moving a row into
-- another tenant: the policy's WITH CHECK refuses a `tenant_id` that is not
-- the transaction's. This covers the other role — the owner, which is what
-- migrations, break-glass psql and a restore run as, and which RLS does not
-- constrain. "A scope can never move across tenants" is one of this
-- substrate's structural rules, so it holds for everybody or it is not
-- structural.
--
-- `kind` rides along because the parent FK's integrity depends on it, `slug`
-- because a path somebody wrote down is half slugs, and `id`, `created_at`
-- and `created_by` because a stable aggregate id whose provenance can be
-- rewritten is not stable. What remains updatable is exactly what the
-- mutating services change: `display_name` (rename), `parent_scope_id` +
-- `parent_kind` (move), `status`, `attributes` and `updated_at`.

create function synveda_scopes_immutable_columns() returns trigger
language plpgsql
as $$
begin
    if new.id <> old.id then
        raise exception 'scopes.id is immutable (CPR-3, ADR-0070)';
    end if;
    if new.tenant_id <> old.tenant_id then
        raise exception
            'scope % cannot move across tenants (% to %) (CPR-3, ADR-0070)',
            old.id, old.tenant_id, new.tenant_id;
    end if;
    if new.kind <> old.kind then
        raise exception
            'scopes.kind is immutable; scope % is a % (CPR-3, ADR-0070)',
            old.id, old.kind;
    end if;
    if new.slug <> old.slug then
        raise exception
            'scopes.slug is immutable; rename changes display_name (CPR-3, ADR-0070)';
    end if;
    if new.created_at <> old.created_at
        or new.created_by is distinct from old.created_by then
        raise exception 'scope provenance is immutable (CPR-3, ADR-0070)';
    end if;
    return new;
end
$$;

create trigger scopes_immutable_columns
    before update on scopes
    for each row execute function synveda_scopes_immutable_columns();

-- ── Tenant isolation ─────────────────────────────────────────────────────
--
-- Tenant-scoped tables get forced RLS + policy + least-privilege grants in
-- the same migration (ADR-0009 structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it).
--
-- No DELETE on `scopes`: nothing deletes a scope. There is no delete service
-- in this substrate — a scope is what audit events, versions and grants
-- name, so retiring one is a status transition rather than a row that stops
-- existing — and a grant for a path that does not exist is a grant nobody
-- reviewed. Closure rows are only ever inserted and deleted, so they get no
-- UPDATE.

grant select, insert, update on scopes to synveda_app;
grant select, insert, delete on scope_closure to synveda_app;

alter table scopes enable row level security;
alter table scopes force row level security;
alter table scope_closure enable row level security;
alter table scope_closure force row level security;

create policy scopes_tenant_isolation on scopes
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy scope_closure_tenant_isolation on scope_closure
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
