-- AUTHZ-5: ABAC conditions (ADR-0038).
--
-- One column, one widened check, one new one. No new table: a classification
-- is a column on a record that already exists, and a declared ceiling is a
-- column on a grant that already exists (ADR-0038 decision 18).
--
--   policy_lapses.max_sensitivity   the tier the grant declared, which is the
--                                   tier its approval matrix resolved at —
--                                   so a `restricted` grant is one a
--                                   compliance approver signed.
--   vedaflow_proposals              the effect vocabulary grows a third
--                                   member: `classify`.
--
-- What is deliberately *not* here: any enforcement of who may read what.
-- Sensitivity is a policy attribute (ADR-0038 decision 2), decided per scope
-- and per tier by the PDP against the pack in force; a CHECK constraint here
-- would be a second opinion on a question the base layer already answers,
-- and the two could disagree.

-- ── The tier a grant declared ───────────────────────────────────────────────

-- Existing rows mean the working tier, and that is what they granted: the
-- read path composed nothing above `internal` when they were approved, so
-- `internal` is what their approvers consented to. The default is dropped
-- immediately afterwards — a grant written without a declared tier should be
-- a code bug, not a silent `internal`.
alter table policy_lapses add column max_sensitivity text not null default 'internal';
alter table policy_lapses alter column max_sensitivity drop default;

-- The vocabulary, mirroring synveda_types::Sensitivity, in the same
-- reviewed-in-two-places shape as policy_lapses_action_check (migration 0022).
alter table policy_lapses add constraint policy_lapses_sensitivity_check
    check (max_sensitivity in ('public', 'internal', 'confidential', 'restricted'));

-- ── The proposal's effect vocabulary ────────────────────────────────────────

-- Migration 0022 widened this from `= 'published'` to name the proposal's
-- *effect* rather than always a channel. A reclassification is the third
-- effect and the second one that writes no channel: it changes what a record
-- *is*, not where it is published, and a record can be reclassified without
-- ever having crossed the trust boundary (ADR-0038 decision 9).
alter table vedaflow_proposals drop constraint vedaflow_proposals_channel_check;
alter table vedaflow_proposals add constraint vedaflow_proposals_channel_check
    check (target_channel in ('published', 'lapse', 'classify'));

-- A lapse proposal's asset must be `policy` (migration 0022); a
-- reclassification's must be `memory`, and for the mirror-image reason. A
-- `classify` effect on a policy proposal would be a code bug that produced a
-- proposal nothing could ever run — refused here rather than discovered at
-- the effect.
alter table vedaflow_proposals add constraint vedaflow_proposals_classify_asset_check
    check (target_channel <> 'classify' or asset_kind = 'memory');

-- ── The declared tier is immutable, like the window ─────────────────────────

-- ADR-0037's transition trigger made `expires_at` immutable because an UPDATE
-- that pushed it forward would turn a 30-day grant into a permanent one
-- without a second approval. `max_sensitivity` is the same attack in the
-- other dimension: raised after approval, an `internal` grant two stewards
-- approved becomes a `restricted` one no compliance approver ever saw — and
-- the proposal, the approvals and the chain would all still say `internal`.
--
-- Replaces the 0022 function rather than adding a second trigger: one
-- statement of what a grant may do, in one place.
create or replace function synveda_policy_lapse_transition() returns trigger
language plpgsql
as $$
begin
    if new.tenant_id        <> old.tenant_id
        or new.id               <> old.id
        or new.proposal_id      <> old.proposal_id
        or new.grantee_scope_id <> old.grantee_scope_id
        or new.target_scope_id  <> old.target_scope_id
        or new.action           <> old.action
        or new.max_sensitivity  <> old.max_sensitivity
        or new.reason           <> old.reason
        or new.granted_at       <> old.granted_at
        or new.expires_at       <> old.expires_at
        or new.granted_by       <> old.granted_by
    then
        raise exception
            'lapse % is immutable except for its revocation and its recorded expiry '
            '(AUTHZ-4); a moved expires_at would extend a grant without a second approval, '
            'and a raised max_sensitivity would widen one past what its approvers signed',
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
