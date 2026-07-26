-- AUTHZ-4: lapses (ADR-0037).
--
-- One column, one widened check, and one table. The split is ADR-0037
-- decision 1, which is ADR-0032's split applied to a different effect: a
-- lapse's *terms* are an AssetKind::Policy object named by a proposal commit
-- — already immutable, already content-addressed, already reviewed — and a
-- lapse's *grant* is a row the read path consults per request.
--
--   policy_packs.lapse            a stored pack's LapseConfig, beside
--                                 redaction (0013), composition (0017),
--                                 approvals (0019) and promotion (0020).
--                                 Null means the pack configures nothing and
--                                 falls back to the strict 30-day window —
--                                 the `approvals` reading rather than the
--                                 `promotion` one, because a lapse ceiling
--                                 narrows and a missing narrowing must not
--                                 become a missing mechanism.
--   policy_lapses                 the granted proposal's projection in typed
--                                 columns, because parsing an object per
--                                 decision is not a read path.
--
-- No DELETE grant and no hierarchy FK. The second follows vedaflow_proposals
-- (migration 0019), which un-FK'd its scopes for the reason recorded there:
-- recorded governance must neither block a deletion nor be destroyed by one.
-- Correctness does not depend on the FK — a deleted scope is on nobody's
-- chain, so its grants resolve to nothing at the only place they are read.
--
-- **Expiry is not enforced here and deliberately so** (ADR-0037 decision 4).
-- `expires_at` is a column the read predicate compares against `now()`; there
-- is no job, no scheduled statement, and nothing that has to run for a grant
-- to end. The sweep only stamps `expiry_recorded_at` when it has chained the
-- audit event, and nothing consults that column to decide access.

alter table policy_packs add column lapse jsonb;

-- ── The proposal's effect is no longer always a channel ─────────────────────

-- Migration 0019 wrote `check (target_channel = 'published')` when publishing
-- was the only effect a proposal could have. A lapse has no target channel at
-- all: its effect is a grant row. The column now names the proposal's
-- **effect** rather than always a channel, which is a mild tension recorded
-- in ADR-0037 decision 16 rather than papered over by storing 'published' on
-- a row that publishes nothing.
--
-- `lapse` is a literal in this constraint and nothing more. It is not a
-- Channel: no scope has a `policy/lapse` ref, nothing writes one, and
-- GET /v1/channels has nothing new to skip.
alter table vedaflow_proposals drop constraint vedaflow_proposals_channel_check;
alter table vedaflow_proposals add constraint vedaflow_proposals_channel_check
    check (target_channel in ('published', 'lapse'));

-- A lapse proposal is the one kind whose effect is not a publication, so it
-- is the one kind whose asset must be `policy`. Stated here because a
-- 'lapse' effect on a memory proposal would be a code bug that silently
-- produced a proposal nothing could ever run.
alter table vedaflow_proposals add constraint vedaflow_proposals_lapse_asset_check
    check (target_channel <> 'lapse' or asset_kind = 'policy');

-- ── The standing grant ──────────────────────────────────────────────────────

create table policy_lapses (
    tenant_id          uuid        not null,
    id                 uuid        not null,
    -- Where the approvals, the requirement as resolved, and the reviewed
    -- object all live. The FK is the point: a grant that does not name a real
    -- review is unrepresentable, which is what makes "no lapse without dual
    -- approval" a property of the schema rather than of the handler.
    proposal_id        uuid        not null,
    -- Every principal placed at or under this scope gets the access. A single
    -- person is their own personal scope, so one shape covers "team X" and
    -- "just Dana" (ADR-0037 decision 2).
    grantee_scope_id   uuid        not null,
    -- Whose material is disclosed, and the only scope the permit covers
    -- (decision 8). What lives below it reaches the reader through what this
    -- scope published, which is the set the approvers could inspect.
    target_scope_id    uuid        not null,
    -- The closed vocabulary of ADR-0037 decision 2, mirroring
    -- synveda_types::LapseAction. Widening it is a reviewed diff in two
    -- places, which is the point.
    action             text        not null,
    reason             text        not null,
    -- The window starts when the effect ran, never when the proposal opened:
    -- a proposal that sat in a queue for a week must not spend the window it
    -- was approved for (ADR-0037 decision 4).
    granted_at         timestamptz not null default now(),
    expires_at         timestamptz not null,
    granted_by         uuid        not null,
    -- Early revocation (decision 15): narrows only, so it resolves no matrix.
    revoked_at         timestamptz,
    revoked_by         uuid,
    revoke_reason      text,
    -- Bookkeeping for the expiry sweep, and nothing else reads it.
    expiry_recorded_at timestamptz,

    constraint policy_lapses_pk primary key (tenant_id, id),
    constraint policy_lapses_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint policy_lapses_proposal_fk
        foreign key (tenant_id, proposal_id)
        references vedaflow_proposals (tenant_id, id),
    -- A proposal's effect runs at most once. The unique constraint is the
    -- idempotency guard, so a retried or replayed grant is a conflict rather
    -- than a second standing window.
    constraint policy_lapses_proposal_unique unique (tenant_id, proposal_id),
    constraint policy_lapses_action_check check (action in ('memory.read')),
    constraint policy_lapses_reason_check check (length(reason) between 1 and 512),
    -- A lapse from a scope to itself grants nothing: its members already
    -- compose it through their own chain.
    constraint policy_lapses_scopes_check check (grantee_scope_id <> target_scope_id),
    -- A window that ends before it starts is not one. The product ceiling
    -- (90 days) lives in the pack config rather than here, because it is a
    -- policy bound and this is a structural one.
    constraint policy_lapses_window_check check (expires_at > granted_at),
    -- Revoked exactly when it says it is revoked, and a revocation an auditor
    -- cannot read the reason for is not one.
    constraint policy_lapses_revocation_check
        check ((revoked_at is null) = (revoked_by is null)
               and (revoked_at is null) = (revoke_reason is null)),
    constraint policy_lapses_revoke_reason_check
        check (revoke_reason is null or length(revoke_reason) between 1 and 512)
);

-- The read-path query, and the reason this is a table rather than an object:
-- "the active grants whose grantee scope is on this caller's chain", one
-- indexed scan per governed request. Partial on `revoked_at is null` because
-- a revoked grant is history and is read by id.
create index policy_lapses_active_idx
    on policy_lapses (tenant_id, grantee_scope_id, expires_at)
    where revoked_at is null;

-- The expiry sweep's query: grants whose window has closed and whose event
-- has not been chained yet. Partial on both, so the sweep's scan shrinks to
-- nothing on an idle tenant — the FLOW-4 lesson about a pass that pays per
-- tenant just to discover it has nothing to do.
create index policy_lapses_expiry_idx
    on policy_lapses (tenant_id, expires_at)
    where expiry_recorded_at is null and revoked_at is null;

-- The admin listing (`GET /v1/policy/lapses`), newest first.
create index policy_lapses_listing_idx
    on policy_lapses (tenant_id, target_scope_id, granted_at desc);

-- ── The two transitions a grant has ─────────────────────────────────────────

-- A grant is written once and then moves in exactly two ways: it is revoked,
-- or its expiry is recorded. Everything else about it is immutable, and
-- `expires_at` most of all — an UPDATE that pushed it forward would turn a
-- 30-day grant into a permanent one without a second approval, which is the
-- one attack this table exists to make impossible.
--
-- The app path never trips this (its UPDATEs are narrow and carry their own
-- `where` guards); the trigger is what makes the rule true for out-of-band
-- writes, table owner included — the observe_quarantine one-shot pattern
-- (migration 0013) and the proposal transition (migration 0019).
create function synveda_policy_lapse_transition() returns trigger
language plpgsql
as $$
begin
    if new.tenant_id        <> old.tenant_id
        or new.id               <> old.id
        or new.proposal_id      <> old.proposal_id
        or new.grantee_scope_id <> old.grantee_scope_id
        or new.target_scope_id  <> old.target_scope_id
        or new.action           <> old.action
        or new.reason           <> old.reason
        or new.granted_at       <> old.granted_at
        or new.expires_at       <> old.expires_at
        or new.granted_by       <> old.granted_by
    then
        raise exception
            'lapse % is immutable except for its revocation and its recorded expiry '
            '(AUTHZ-4); a moved expires_at would extend a grant without a second approval',
            old.id;
    end if;
    if old.revoked_at is not null and new.revoked_at is distinct from old.revoked_at then
        raise exception 'lapse % is already revoked; a revocation is terminal (AUTHZ-4)', old.id;
    end if;
    if new.revoked_at is null and old.revoked_at is not null then
        raise exception 'lapse % cannot be un-revoked (AUTHZ-4)', old.id;
    end if;
    if old.expiry_recorded_at is not null
        and new.expiry_recorded_at is distinct from old.expiry_recorded_at
    then
        raise exception 'lapse % has already had its expiry chained (AUTHZ-4)', old.id;
    end if;
    return new;
end
$$;

create trigger policy_lapses_transition
    before update on policy_lapses
    for each row execute function synveda_policy_lapse_transition();

-- A grant is the record of a dual-approved decision. It is not history in the
-- vedaflow sense — it is derived from a proposal that is — but deleting one
-- would erase why an inject composed what it composed, so the row stays and
-- the outcome is rendered from it (synveda_types::LapseOutcome).
create trigger policy_lapses_no_delete
    before delete on policy_lapses
    for each row execute function synveda_vedaflow_immutable();
create trigger policy_lapses_no_truncate
    before truncate on policy_lapses
    execute function synveda_vedaflow_immutable();

-- ── RLS + least-privilege grants ────────────────────────────────────────────

-- Tenant-scoped table ⇒ forced RLS + policy + least-privilege grants in the
-- same migration (ADR-0009's structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it).
grant select, insert, update on policy_lapses to synveda_app;

alter table policy_lapses enable row level security;
alter table policy_lapses force row level security;

create policy policy_lapses_tenant_isolation on policy_lapses
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
