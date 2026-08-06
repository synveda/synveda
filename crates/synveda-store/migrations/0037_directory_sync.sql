-- AUTH-5: what the pull sync remembers between passes (ADR-0060).
--
-- Two shapes, and between them they hold one idea: **absence is a
-- hypothesis** (ADR-0060 decision 3). On the push plane a leaver is an act —
-- a provisioning agent sends `active: false` and AUTH-4 seals on the spot.
-- A pull sync has no act to react to. It has somebody who was on page 3 last
-- hour and is on no page now, and a throttled response, a truncated page, an
-- expired token, a narrowed assignment filter and a resignation all produce
-- that same nothing.
--
-- Since the seal does not lift (ADR-0059 decision 12), treating that nothing
-- as a departure would let one bad response permanently deprovision everyone
-- it omitted. So absence is *accumulated* here and sealed only when three
-- conditions hold, and the columns below are those conditions made durable:
--
-- 1. **The pass completed** — every page fetched, no error, no partial.
--    `directory_sync_state.last_complete_pass_at` and `passes_completed`
--    are the proof; an incomplete pass advances neither, so it cannot
--    contribute to a conclusion about who is gone.
-- 2. **Absence persisted across N consecutive complete passes** —
--    `scim_users.missing_passes`, reset to zero the moment the directory
--    shows the person again.
-- 3. **The pass would not seal an implausible number of people** — the
--    circuit breaker, recorded in `breaker_tripped_at` /
--    `breaker_would_have_sealed` so that a refusal to seal is a fact an
--    operator is told rather than a silence they have to notice.
--
-- An explicitly deactivated user still seals on the first complete pass that
-- sees `active: false`. That is an act, and it gets an act's treatment. Only
-- absence takes the slow path, and that asymmetry is the whole content of
-- ADR-0060 decision 3.

-- ── The observation columns on the mirror ─────────────────────────────
--
-- These sit on `scim_users` rather than in a table of their own because
-- they are one fact per mirror row and a second table would be a second row
-- per user, its own RLS policy and its own coverage, to hold an integer.
--
-- They are **ours, not the directory's**, and the mirror already carries two
-- of those: `identity_id` is the product's projection link and `version` is
-- an ETag this server mints. The rule they follow is the rule those follow —
-- `/Schemas` publishes what the directory sent us and never these, because
-- what is not published was never promised (migration 0036's note on why the
-- mirror stores columns rather than a blob).
alter table scim_users
    -- When we first failed to see them. Kept for the operator and for the
    -- audit payload — "gone since Tuesday" is the question a human asks —
    -- and never the condition, because passes fail and wall-clock cannot
    -- tell three complete passes from one complete pass and two outages.
    add column missing_since  timestamptz,

    -- The condition: consecutive *complete* passes in which the directory
    -- did not list this user. Zero means present, and the reconciler is
    -- offered nobody until this reaches the configured N.
    add column missing_passes integer not null default 0
        constraint scim_users_missing_passes_check check (missing_passes >= 0),

    -- The two columns say one thing or the schema refuses them
    -- (identities_departed_at_check's shape, migration 0036). Without this
    -- a reset that cleared one and not the other would leave a row that is
    -- present by the counter and missing since Tuesday by the timestamp,
    -- and the disagreement would surface as somebody being sealed or not
    -- sealed for no visible reason.
    add constraint scim_users_missing_pair_check
        check ((missing_passes = 0) = (missing_since is null));

-- The pass's own work list: live rows the directory has stopped listing.
-- Partial because the interesting set is tiny and the table is not — and
-- because a sealed row goes missing forever once the directory drops it,
-- which is a row nothing should keep counting (identities_sealed_scopes'
-- shape, migration 0036).
create index scim_users_missing on scim_users (tenant_id, missing_passes)
    where active and missing_passes > 0;

-- ── The per-tenant pass state ─────────────────────────────────────────

create table directory_sync_state (
    -- One row per tenant: a tenant has one directory authority (ADR-0060
    -- decision 5), which is also why there is no column here saying whether
    -- the pull is yielding to a live SCIM credential. That is answered by
    -- looking for one, and a copy of it would be a second truth about which
    -- plane is in charge — ADR-0013 decision 4's argument, one plane up.
    tenant_id                 uuid        not null,

    -- Which connector wrote this state. Not constrained to a list: the
    -- connector set is a trait's implementations, and a check constraint
    -- here would make adding one a migration. It is stored because a
    -- deployment that re-points a tenant from one directory to another
    -- invalidates every absence count below it — the new connector has
    -- never seen anybody, so nobody it does not list is missing yet.
    connector                 text        not null,

    -- Completed passes only, which is what makes `missing_passes`
    -- countable against something. A pass that failed halfway does not
    -- appear here at all.
    passes_completed          bigint      not null default 0,

    -- Every attempt, including the ones that failed. The gap between this
    -- and `last_complete_pass_at` is how an operator sees a connector that
    -- is running and never finishing — the state in which nobody is being
    -- sealed and nothing looks wrong.
    last_pass_at              timestamptz,
    last_complete_pass_at     timestamptz,

    -- The circuit breaker's last refusal (ADR-0060 decision 3.3). Recorded
    -- rather than enforced here: the breaker re-evaluates every pass and
    -- does not latch, so this is the record of a decision and not the state
    -- of a switch. The absence counts it declined to act on are still
    -- standing on the mirror rows, which is what makes the next pass able
    -- to reach the same conclusion or a different one.
    breaker_tripped_at        timestamptz,
    breaker_would_have_sealed integer,

    created_at                timestamptz not null default now(),
    updated_at                timestamptz not null default now(),

    constraint directory_sync_state_pk primary key (tenant_id),
    constraint directory_sync_state_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint directory_sync_state_connector_check
        check (length(connector) between 1 and 64),
    constraint directory_sync_state_passes_check check (passes_completed >= 0),

    -- A completed-pass timestamp with no completed passes behind it is a
    -- row that would let an incomplete pass masquerade as the completeness
    -- proof decision 3.1 is built on.
    constraint directory_sync_state_complete_pass_check
        check (last_complete_pass_at is null or passes_completed > 0),

    -- The breaker's pair, the same rule as the mirror's pair above.
    constraint directory_sync_state_breaker_pair_check
        check ((breaker_tripped_at is null) = (breaker_would_have_sealed is null)),
    -- A trip that would have sealed nobody is not a trip; it is a pass.
    constraint directory_sync_state_breaker_count_check
        check (breaker_would_have_sealed is null or breaker_would_have_sealed > 0)
);

-- ── What is deliberately not here ─────────────────────────────────────
--
-- **No delta cursor.** Graph's `delta` and Okta's `lastUpdated gt` answer
-- "what changed" and never "what still exists", so neither can carry the
-- completeness proof decision 3.1 needs; full enumeration is the authority
-- for absence (ADR-0060 decision 6). A delta token is an optimisation for
-- *presence*, and the column arrives with the code that earns it rather
-- than sitting here inviting somebody to make it the authority.
--
-- **No `last_error`.** A failed pass is a tracing event and a metric, both
-- of which already carry the endpoint and the status. Storing the error text
-- would put connector-side strings in tenant data on a loop, which is a
-- standing leak surface for the one credential in this product we have to be
-- able to read back (ADR-0060 decision 7) in exchange for nothing the logs
-- do not already say.

-- ── Grants ────────────────────────────────────────────────────────────
--
-- No DELETE: the sync state is how "why has nobody been sealed for three
-- days" stays answerable, and a row the loop can drop is one a failing
-- connector can drop on its way past. Re-pointing a tenant to another
-- connector is an update to `connector`, not a fresh row.
grant select, insert, update on directory_sync_state to synveda_app;

-- Tenant-scoped table ⇒ forced RLS + policy + least-privilege grants in the
-- same migration (ADR-0009 structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it).
alter table directory_sync_state enable row level security;
alter table directory_sync_state force row level security;

create policy directory_sync_state_tenant_isolation on directory_sync_state
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
