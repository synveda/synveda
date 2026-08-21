-- FLOW-3: proposals & the approval matrix (ADR-0032).
--
-- Two tables and one column. The split is ADR-0032 decision 1, which is
-- ADR-0030's split restated: a proposal's *content* is a commit — already
-- immutable, already content-addressed, already in vedaflow_commits — and a
-- proposal's *workflow* is a row that moves. So:
--
--   vedaflow_proposals           mutable lifecycle, SELECT/INSERT/UPDATE, no
--                                DELETE, with a trigger that permits exactly
--                                the open → closed transition and nothing else.
--   vedaflow_proposal_approvals  governed history, SELECT/INSERT only, with
--                                the ADR-0019 append-only triggers. A review
--                                log that can be edited is not one.
--   policy_packs.approvals       a stored pack's ApprovalMatrix, beside
--                                redaction (0013) and composition (0017).
--                                Null means the pack configures nothing —
--                                which still resolves to the invariant floor
--                                (ADR-0032 decision 4), never to "no review".
--
-- No new ref names and no table for curator files: a curator file is an
-- AssetKind::Policy object under a ref named `curators` at its scope
-- (ADR-0032 decision 14), so it needs no schema of its own — it inherits
-- content addressing, immutable history, and the pack snapshot on every
-- change from migration 0018.
--
-- No ref per proposal either (ADR-0032 decision 1): vedaflow_refs holds no
-- DELETE grant by design, so one ref per proposal would leave a permanent
-- pointer per closed proposal that nothing follows.

alter table policy_packs add column approvals jsonb;

-- ── Proposals: the workflow row ─────────────────────────────────────────────

create table vedaflow_proposals (
    tenant_id        uuid        not null,
    id               uuid        not null,
    -- The scope whose channel would move. Requirements resolve here: the
    -- effective pack at this node, this node's kind, and the nearest
    -- curator file on this node's chain (ADR-0032 decision 3).
    target_scope_id  uuid        not null,
    -- Where the material lives now. FLOW-3 requires it to equal the target
    -- (ADR-0032 decision 17) and enforces that in code rather than here:
    -- FLOW-5's cross-scope climb relaxes exactly this, and a CHECK written
    -- now would have to be migrated then.
    source_scope_id  uuid        not null,
    asset_kind       text        not null,
    target_channel   text        not null,
    -- The reviewed content: a commit whose tree names every member at the
    -- object address of the version proposed. The FK is the point —
    -- a proposal that does not name real history is unrepresentable.
    commit_hash      bytea       not null,
    -- The maximum sensitivity over the members. A set is reviewed as a set,
    -- so it is governed by its most sensitive element (ADR-0032 decision 3).
    sensitivity      text        not null,
    -- Only what happened (ADR-0032 decision 11). `approved` is deliberately
    -- absent: whether an open proposal has enough approvals is computed live
    -- from the approvals below against the live requirement, because a pack
    -- switch governs the very next request (ADR-0014 decision 3).
    state            text        not null default 'open',
    title            text        not null,
    -- The proposing identity and its token subject. Un-FK'd on identities,
    -- the AUD-1 / ADR-0030 doctrine: a service identity's revocation deletes
    -- its identity row, and recorded governance must neither block that nor
    -- be destroyed by it.
    proposer_id      uuid        not null,
    proposer_subject text        not null,
    created_at       timestamptz not null default now(),
    updated_at       timestamptz not null default now(),
    -- Set together, exactly when the proposal closes.
    closed_at        timestamptz,
    closed_by        uuid,
    close_reason     text,

    constraint vedaflow_proposals_pk primary key (tenant_id, id),
    constraint vedaflow_proposals_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint vedaflow_proposals_commit_fk
        foreign key (tenant_id, commit_hash)
        references vedaflow_commits (tenant_id, hash),
    constraint vedaflow_proposals_asset_check
        check (asset_kind in ('memory', 'prompt', 'skill', 'context-pack', 'policy')),
    -- Published is the only channel a proposal targets. Derived is written by
    -- the pipeline and needs no review to reach; staged has no writer at all
    -- (ADR-0032 decision 2). Widening this is a reviewed diff.
    constraint vedaflow_proposals_channel_check
        check (target_channel = 'published'),
    constraint vedaflow_proposals_sensitivity_check
        check (sensitivity in ('public', 'internal', 'confidential', 'restricted')),
    constraint vedaflow_proposals_state_check
        check (state in ('open', 'rejected', 'withdrawn', 'published')),
    constraint vedaflow_proposals_title_check
        check (length(title) between 1 and 500),
    constraint vedaflow_proposals_subject_check
        check (length(proposer_subject) between 1 and 255),
    -- Closed exactly when it says it is closed.
    constraint vedaflow_proposals_closure_check
        check ((state = 'open') = (closed_at is null and closed_by is null)),
    -- A rejection an auditor cannot read the reason for is not a review.
    constraint vedaflow_proposals_reject_reason_check
        check (state <> 'rejected' or close_reason is not null),
    constraint vedaflow_proposals_reason_check
        check (close_reason is null
               or (state <> 'open' and length(close_reason) between 1 and 1000))
);

-- The review queue read (CNSL-1's inbox, FLOW-6's `proposal list`): what is
-- open at a scope, oldest first. Partial, because closed proposals are
-- history and are read by id.
create index vedaflow_proposals_open_idx
    on vedaflow_proposals (tenant_id, target_scope_id, created_at)
    where state = 'open';

-- The tenant-wide listing, newest first.
create index vedaflow_proposals_listing_idx
    on vedaflow_proposals (tenant_id, created_at desc);

-- ── Approvals: governed history ─────────────────────────────────────────────

create table vedaflow_proposal_approvals (
    tenant_id        uuid        not null,
    proposal_id      uuid        not null,
    approver_id      uuid        not null,
    -- What was approved. Approvals bind bytes (ADR-0032 decision 6): the
    -- commit is in the key, so an approval is evidence about one exact
    -- content set and can never be inherited by another.
    commit_hash      bytea       not null,
    verdict          text        not null,
    -- The approver's effective roles at the target scope when they cast it.
    -- Recorded rather than re-derived: an approval is evidence of the
    -- authority that existed then, not a claim re-checked against bindings
    -- that may since have changed (ADR-0032 decision 5).
    roles            text[]      not null,
    approver_subject text        not null,
    comment          text,
    created_at       timestamptz not null default now(),

    constraint vedaflow_proposal_approvals_pk
        primary key (tenant_id, proposal_id, approver_id, commit_hash),
    constraint vedaflow_proposal_approvals_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint vedaflow_proposal_approvals_proposal_fk
        foreign key (tenant_id, proposal_id)
        references vedaflow_proposals (tenant_id, id),
    constraint vedaflow_proposal_approvals_commit_fk
        foreign key (tenant_id, commit_hash)
        references vedaflow_commits (tenant_id, hash),
    constraint vedaflow_proposal_approvals_verdict_check
        check (verdict in ('approve', 'reject')),
    -- The closed grant-key vocabulary (ADR-0072, re-vocabularied onto the
    -- approval plane by CPR-7/ADR-0074 decision 6), in the schema for the
    -- same reason every other vocabulary is: a stored value outside it
    -- means code and schema drifted, and the reader should say so rather
    -- than shrug.
    constraint vedaflow_proposal_approvals_roles_check
        check (roles <@ array['owner', 'member', 'viewer', 'reviewer',
                              'curator', 'administrator']::text[]),
    constraint vedaflow_proposal_approvals_subject_check
        check (length(approver_subject) between 1 and 255),
    constraint vedaflow_proposal_approvals_comment_check
        check (comment is null or length(comment) between 1 and 1000)
);

-- ── Immutability and the one permitted transition ───────────────────────────

-- Approvals are recorded history, on the audit_log / vedaflow pattern.
create trigger vedaflow_proposal_approvals_no_update
    before update on vedaflow_proposal_approvals
    for each row execute function synveda_vedaflow_immutable();
create trigger vedaflow_proposal_approvals_no_delete
    before delete on vedaflow_proposal_approvals
    for each row execute function synveda_vedaflow_immutable();
create trigger vedaflow_proposal_approvals_no_truncate
    before truncate on vedaflow_proposal_approvals
    execute function synveda_vedaflow_immutable();

-- A proposal row moves exactly once, open → closed, and nothing else about it
-- ever changes. The app path never trips this (its UPDATE carries
-- `where state = 'open'` and sets only these columns); the trigger is what
-- makes the rule true for out-of-band writes, table owner included — the
-- observe_quarantine one-shot pattern (migration 0013).
create function synveda_vedaflow_proposal_transition() returns trigger
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
    then
        raise exception 'proposal % is immutable except for its closure (FLOW-3)', old.id;
    end if;
    return new;
end
$$;

create trigger vedaflow_proposals_transition
    before update on vedaflow_proposals
    for each row execute function synveda_vedaflow_proposal_transition();

create trigger vedaflow_proposals_no_delete
    before delete on vedaflow_proposals
    for each row execute function synveda_vedaflow_immutable();
create trigger vedaflow_proposals_no_truncate
    before truncate on vedaflow_proposals
    execute function synveda_vedaflow_immutable();

-- ── RLS + least-privilege grants ────────────────────────────────────────────

-- Tenant-scoped tables ⇒ forced RLS + policy + least-privilege grants in the
-- same migration (ADR-0009's structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it).

-- The lifecycle column moves; the row never disappears.
grant select, insert, update on vedaflow_proposals to synveda_app;
grant select, insert on vedaflow_proposal_approvals to synveda_app;

alter table vedaflow_proposals enable row level security;
alter table vedaflow_proposals force row level security;
alter table vedaflow_proposal_approvals enable row level security;
alter table vedaflow_proposal_approvals force row level security;

create policy vedaflow_proposals_tenant_isolation on vedaflow_proposals
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy vedaflow_proposal_approvals_tenant_isolation on vedaflow_proposal_approvals
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
