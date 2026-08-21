-- AUTH-2: provisioned identities (ADR-0013), re-anchored on governed scopes
-- by CPR-7 (ADR-0074 decision 3).
--
-- identities binds a verified token subject to its own principal-shaped
-- scope; JIT provisioning inserts exactly one row per (tenant, subject) at
-- first login, minting the scope in the same transaction. An identity's
-- scope is who they are, not where a convention placed them: the
-- `synveda-{dept}-{team}` convention, the `group_mappings` override table
-- this migration once created and the reserved `quarantine` scope are
-- deleted with the hierarchy this table used to point at. "Unmapped" is now
-- *ungranted* — a principal with no grants reaches nothing beyond their own
-- scope, decided per action by the anchor model and the base-layer privacy
-- floor rather than per person by a placement-derived flag.

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
    -- The identity's own scope, in the same tenant: a cross-tenant binding
    -- is unrepresentable (same doctrine as scope parents). No cascade: an
    -- identity pins its scope — leavers are AUTH-4's feature, not a delete
    -- surprise.
    constraint identities_scope_fk
        foreign key (tenant_id, scope_id)
        references scopes (tenant_id, id),
    -- One scope is one identity's own, never two.
    constraint identities_scope_unique unique (tenant_id, scope_id),
    constraint identities_subject_check
        check (length(subject) between 1 and 255)
);

-- Tenant-scoped tables ⇒ forced RLS + policy + least-privilege grants in
-- the same migration (ADR-0009 structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it). Identities are only
-- created for now — subject binding and the mover/leaver lifecycle (AUTH-4/5)
-- bring update with their own migrations.
grant select, insert on identities to synveda_app;

alter table identities enable row level security;
alter table identities force row level security;

create policy identities_tenant_isolation on identities
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
