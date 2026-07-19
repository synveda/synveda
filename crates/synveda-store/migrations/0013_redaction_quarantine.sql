-- MEM-2: redaction & secret scanning (ADR-0021).
--
-- Scanning runs in the observe ack path, before any insert, so staging
-- rows only ever hold redacted content. Three additions here:
--
--   1. observe_events.redactions — the finding summary (rule ids,
--      categories, counts — never matched text), stamped at insert;
--      append-only is untouched.
--   2. policy_packs.redaction — a stored pack's optional
--      RedactionConfig; null means the strict default (fail safe,
--      ADR-0021 decision 3). Embedded product packs carry compiled-in
--      configs and no row.
--   3. observe_quarantine — the review queue. A quarantined event's
--      staging row exists (idempotency stays single-point) but no work
--      signal was sent; this row gates it. Review is one-shot
--      (pending → released | rejected) and schema-enforced: column-level
--      UPDATE grants cover only the review columns, and the transition
--      trigger raises for everything else, table owner included (the
--      AUD-1 doctrine).

alter table observe_events add column redactions jsonb;

alter table policy_packs add column redaction jsonb;

create table observe_quarantine (
    -- The gated staging row. The FK is deliberate (unlike the staging
    -- table's missing scope/owner FKs): a quarantine marker without its
    -- event is meaningless, and disposal (MEM-6/TEN-5) retires both at
    -- the same horizon.
    event_id         uuid        not null,
    tenant_id        uuid        not null,
    -- The event's home scope, denormalised for subtree-filtered queue
    -- listings; no FK, same provenance doctrine as observe_events.
    scope_id         uuid        not null,
    -- Finding summary: [{rule, category, count}] — never matched text.
    findings         jsonb       not null,
    state            text        not null default 'pending',
    created_at       timestamptz not null default now(),
    reviewer_subject text,
    reviewed_at      timestamptz,
    review_reason    text,

    constraint observe_quarantine_pk primary key (event_id),
    constraint observe_quarantine_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint observe_quarantine_event_fk
        foreign key (event_id) references observe_events (id),
    constraint observe_quarantine_state_check
        check (state in ('pending', 'released', 'rejected')),
    -- A review names its reviewer and time; a pending row has neither.
    constraint observe_quarantine_review_check
        check (
            (state = 'pending')
                = (reviewer_subject is null and reviewed_at is null)
        ),
    constraint observe_quarantine_reviewer_check
        check (reviewer_subject is null
               or length(reviewer_subject) between 1 and 255),
    constraint observe_quarantine_reason_check
        check (review_reason is null
               or length(review_reason) between 1 and 1000)
);

-- The review queue read: pending rows per tenant, oldest first.
create index observe_quarantine_pending_idx
    on observe_quarantine (tenant_id, created_at)
    where state = 'pending';

-- Review is one-shot and reviews only: the only representable change is
-- pending → released | rejected with provenance columns untouched. The
-- app path never reaches this trigger (its UPDATE carries
-- `where state = 'pending'`); this guards out-of-band writes.
create function synveda_observe_quarantine_transition() returns trigger
language plpgsql
as $$
begin
    if old.state <> 'pending' then
        raise exception 'quarantine review is one-shot: event % is already %',
            old.event_id, old.state;
    end if;
    if new.state = 'pending' then
        raise exception 'a quarantine review cannot return to pending';
    end if;
    if new.event_id <> old.event_id
        or new.tenant_id <> old.tenant_id
        or new.scope_id <> old.scope_id
        or new.findings <> old.findings
        or new.created_at <> old.created_at
    then
        raise exception 'quarantine provenance columns are immutable';
    end if;
    return new;
end
$$;

create trigger observe_quarantine_review_transition
    before update on observe_quarantine
    for each row execute function synveda_observe_quarantine_transition();

create function synveda_observe_quarantine_immutable() returns trigger
language plpgsql
as $$
begin
    raise exception
        'observe_quarantine rows are retired by disposal (MEM-6/TEN-5), never deleted';
end
$$;

create trigger observe_quarantine_no_delete
    before delete on observe_quarantine
    for each row execute function synveda_observe_quarantine_immutable();

create trigger observe_quarantine_no_truncate
    before truncate on observe_quarantine
    execute function synveda_observe_quarantine_immutable();

-- Tenant-scoped tables ⇒ forced RLS + policy + least-privilege grants in
-- the same migration (ADR-0009 structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it). UPDATE is column-level:
-- the app can review, never rewrite findings or provenance.
grant select, insert on observe_quarantine to synveda_app;
grant update (state, reviewer_subject, reviewed_at, review_reason)
    on observe_quarantine to synveda_app;

alter table observe_quarantine enable row level security;
alter table observe_quarantine force row level security;

create policy observe_quarantine_tenant_isolation on observe_quarantine
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
