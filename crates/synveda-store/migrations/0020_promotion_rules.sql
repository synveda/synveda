-- FLOW-4: auto-promotion rules (ADR-0033).
--
-- Two columns and two tables. The split is ADR-0033 decision 3: usage is a
-- *summary of facts held under a hash chain elsewhere*, so it gets none of
-- the append-only machinery the governed-history tables carry, and it gets
-- the DELETE grant they deliberately lack — truncating the projection and
-- the watermark and replaying the chain from seq 1 must reproduce it
-- exactly, which is what makes it derived state rather than a record.
--
--   policy_packs.promotion        a stored pack's PromotionConfig, beside
--                                 redaction (0013), composition (0017), and
--                                 approvals (0019). Null means the pack
--                                 configures no rules and nothing
--                                 auto-promotes — unlike `approvals`, where
--                                 null still resolves to the invariant floor.
--                                 Silence is the safe reading for a trigger
--                                 and the unsafe one for a requirement.
--   vedaflow_proposals.evidence   why a rule fired, frozen at open time.
--   memory_usage                  the projection: who recalled what, how
--                                 often, first and last.
--   promotion_watermarks          how far the sweeper has folded, per tenant.

alter table policy_packs add column promotion jsonb;

-- ── Evidence on the proposal row ────────────────────────────────────────────

-- ADR-0033 decision 12: the reviewer's copy, read in the same row read that
-- lists the queue (CNSL-1's inbox, FLOW-6's `proposal show`). Null on a
-- proposal a human opened, which is the honest value — no rule fired.
alter table vedaflow_proposals add column evidence jsonb;

-- Evidence is a fact about why the proposal was opened, so it is immutable
-- exactly like every other non-closure column: recomputing it later against
-- a projection that has since moved would make the reviewer's copy disagree
-- with the audit chain's, and the chain's is the one under a hash.
create or replace function synveda_vedaflow_proposal_transition() returns trigger
language plpgsql
as $$
begin
    if old.state <> 'open' then
        raise exception 'proposal % is already %; closed proposals are history (FLOW-3)',
            old.id, old.state;
    end if;
    if new.state = 'open' then
        raise exception 'proposal % update changed nothing about its state (FLOW-3)', old.id;
    end if;
    if new.tenant_id        <> old.tenant_id
        or new.id               <> old.id
        or new.target_scope_id  <> old.target_scope_id
        or new.source_scope_id  <> old.source_scope_id
        or new.asset_kind       <> old.asset_kind
        or new.target_channel   <> old.target_channel
        or new.commit_hash      <> old.commit_hash
        or new.sensitivity      <> old.sensitivity
        or new.title            <> old.title
        or new.proposer_id      <> old.proposer_id
        or new.proposer_subject <> old.proposer_subject
        or new.created_at       <> old.created_at
        -- FLOW-4 (ADR-0033 decision 12). `is distinct from` because
        -- evidence is null on every manually opened proposal.
        or new.evidence is distinct from old.evidence
    then
        raise exception 'proposal % is immutable except for its closure (FLOW-3)', old.id;
    end if;
    return new;
end
$$;

-- ── The usage projection ────────────────────────────────────────────────────

create table memory_usage (
    tenant_id       uuid        not null,
    -- The recalled record. Deliberately un-FK'd on `records`: the sweeper
    -- folds audit rows forward and the chain outlives the rows it describes,
    -- so an FK would make a since-deleted record's history able to fail the
    -- fold. Evaluation joins to `records` and a missing record drops out
    -- there, which is where "does this still exist" belongs.
    record_id       uuid        not null,
    -- Who recalled it, as the audit chain names them: the token subject.
    -- Distinct members are `count(*)` over this key and total recalls a
    -- `sum` — both facts fall out of the row shape, so neither needs a
    -- maintained aggregate that could drift from the rows under it
    -- (ADR-0033 decision 3).
    subject         text        not null,
    recalls         bigint      not null default 0,
    first_recall_at timestamptz not null,
    last_recall_at  timestamptz not null,

    constraint memory_usage_pk primary key (tenant_id, record_id, subject),
    constraint memory_usage_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint memory_usage_subject_check
        check (length(subject) between 1 and 255),
    constraint memory_usage_recalls_check check (recalls >= 0),
    constraint memory_usage_order_check check (last_recall_at >= first_recall_at)
);

-- ── The sweeper's watermark ─────────────────────────────────────────────────

create table promotion_watermarks (
    tenant_id  uuid        not null,
    -- The last audit_log seq folded into memory_usage. `audit_log.seq` is
    -- 1-based and contiguous per tenant (a gap is a verification failure,
    -- ADR-0019), which is what makes a single integer a cursor with no
    -- ambiguity in it. 0 = nothing folded yet.
    last_seq   bigint      not null default 0,
    updated_at timestamptz not null default now(),

    constraint promotion_watermarks_pk primary key (tenant_id),
    constraint promotion_watermarks_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint promotion_watermarks_seq_check check (last_seq >= 0)
);

-- ── RLS + least-privilege grants ────────────────────────────────────────────

-- Tenant-scoped tables ⇒ forced RLS + policy + least-privilege grants in the
-- same migration (ADR-0009's structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it).

-- Both take DELETE, which every governed-history table in this schema
-- deliberately withholds. That asymmetry is the statement: these hold no
-- governed facts, and the rebuild in ADR-0033 decision 3 has to be something
-- the app role can actually perform.
grant select, insert, update, delete on memory_usage to synveda_app;
grant select, insert, update, delete on promotion_watermarks to synveda_app;

alter table memory_usage enable row level security;
alter table memory_usage force row level security;
alter table promotion_watermarks enable row level security;
alter table promotion_watermarks force row level security;

create policy memory_usage_tenant_isolation on memory_usage
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy promotion_watermarks_tenant_isolation on promotion_watermarks
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
