-- CPR-12: the observe cutover (ADR-0078).
--
-- `session_events` becomes the product's only runtime write seam. The three
-- things `observe_events` did that were worth keeping move onto it — the
-- scan's finding summary, the work signal, and the quarantine review that
-- withholds one — and `observe_events`, `observe_quarantine` and the
-- `observe` queue are dropped whole.
--
-- ── Why the event row gains a column and not a state ────────────────────
--
-- `session_events` has SELECT and INSERT and no UPDATE (migration 0044), and
-- that is load-bearing: immutability there is a privilege rather than a
-- discipline. So a quarantined event is inserted like any other and simply
-- gets **no work signal**; the review state lives in its own table and a
-- release enqueues the signal that admission withheld.
--
-- `redactions` is different and does belong on the row: it is the scan's
-- finding summary (rule ids, categories, counts — never matched text),
-- decided once at admission and true of the payload forever after. It is
-- immutable provenance, which is exactly what an append-only row is for.
--
-- ── The queue is renamed rather than reused ─────────────────────────────
--
-- A queue called `observe` whose messages name session events is a trap for
-- whoever reads it next. `pgmq.drop_queue` takes the old one and its archive
-- with it; nothing is in flight across this migration because a pre-cut
-- database is refused at startup by the epoch guard (ADR-0069) and a post-cut
-- one has never had an `observe` producer.

-- ── The vocabulary gains its thirteenth name ─────────────────────────────
--
-- `memory.asserted`: a fact a model composed and chose to store, arriving
-- because it called a write tool rather than because a hook observed a run.
-- It carried `ObserveKind::Assertion` until this migration and it survives the
-- cut for ADR-0057 decision 8's reason — the distinction between a model's
-- claim and a host's observation is epistemic and cannot be recovered once the
-- two share a name.
--
-- Widening a CHECK is expand-only and needs no backfill: every existing row
-- holds one of the twelve and stays valid, and `session_events` has no UPDATE
-- grant, so no path could rewrite an old row into the new value either.

alter table session_events
    drop constraint session_events_type_check;

alter table session_events
    add constraint session_events_type_check
        check (event_type in (
            'session.started', 'session.ended',
            'message.user', 'message.assistant',
            'tool.invoked', 'tool.result',
            'file.read', 'file.changed',
            'command.executed',
            'skill.loaded',
            'context.requested',
            'adapter.warning',
            'memory.asserted'
        ));

-- ── The scan's finding summary ───────────────────────────────────────────

alter table session_events add column redactions jsonb;

-- Shape only: the contents are `[{rule, category, count}]`, and what matters
-- structurally is that it is an array and never an object holding text.
alter table session_events
    add constraint session_events_redactions_array_check
        check (redactions is null or jsonb_typeof(redactions) = 'array');

-- The composite target the quarantine table's event foreign key needs.
-- `session_events_pk` is on `id` alone, so `(tenant_id, id)` is a separate
-- referential fact and is created here, in the migration that exists for it
-- (0041's convention). It must precede the table that references it.
create unique index session_events_tenant_id_unique
    on session_events (tenant_id, id);

-- ── The work-signal queue ────────────────────────────────────────────────
--
-- Content-free by design (ADR-0020 decision 1, carried forward): messages
-- carry `{tenant_id, event_id}` and nothing else, so the queue lives outside
-- the RLS discipline without ever holding anything RLS protects.

select pgmq.create('session_events');

grant select, insert, update, delete on pgmq.q_session_events to synveda_app;
grant select, insert on pgmq.a_session_events to synveda_app;
grant usage, select on all sequences in schema pgmq to synveda_app;

-- ── The quarantine review plane ──────────────────────────────────────────
--
-- Migration 0013's table, re-anchored on session events. Two things differ
-- and both are improvements the session model paid for:
--
--   * `scope_id` is a real foreign key into `scopes`. The old table could not
--     have one — a staging row's scope was the submitter's home leaf and had
--     to survive that identity's revocation. A session's scope is derived
--     from its workspace and project and outlives everybody.
--   * The event foreign key is composite, so a review row can never name an
--     event from another tenant.

create table session_event_quarantine (
    event_id         uuid        not null,
    tenant_id        uuid        not null,
    -- The session the event belongs to, denormalised so the review listing
    -- can say which run without a join.
    session_id       uuid        not null,
    -- The governed scope the run was decided at — what a subtree-filtered
    -- review queue narrows by.
    scope_id         uuid        not null,
    -- Finding summary: [{rule, category, count}] — never matched text.
    findings         jsonb       not null,
    state            text        not null default 'pending',
    created_at       timestamptz not null default now(),
    reviewer_subject text,
    reviewed_at      timestamptz,
    review_reason    text,

    constraint session_event_quarantine_pk primary key (event_id),
    constraint session_event_quarantine_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint session_event_quarantine_event_fk
        foreign key (tenant_id, event_id)
        references session_events (tenant_id, id),
    constraint session_event_quarantine_session_fk
        foreign key (tenant_id, session_id) references sessions (tenant_id, id),
    constraint session_event_quarantine_scope_fk
        foreign key (tenant_id, scope_id) references scopes (tenant_id, id),
    constraint session_event_quarantine_state_check
        check (state in ('pending', 'released', 'rejected')),
    constraint session_event_quarantine_findings_array_check
        check (jsonb_typeof(findings) = 'array'),
    -- A review names its reviewer and time; a pending row has neither.
    constraint session_event_quarantine_review_check
        check (
            (state = 'pending')
                = (reviewer_subject is null and reviewed_at is null)
        ),
    constraint session_event_quarantine_reviewer_check
        check (reviewer_subject is null
               or length(reviewer_subject) between 1 and 255),
    constraint session_event_quarantine_reason_check
        check (review_reason is null
               or length(review_reason) between 1 and 1000)
);

-- The review queue read: pending rows per tenant, oldest first.
create index session_event_quarantine_pending_idx
    on session_event_quarantine (tenant_id, created_at)
    where state = 'pending';

-- Review is one-shot and reviews only: the only representable change is
-- pending → released | rejected with provenance columns untouched. The app
-- path never reaches this trigger (its UPDATE carries `where state =
-- 'pending'`); this guards out-of-band writes, table owner included.
create function synveda_session_event_quarantine_transition() returns trigger
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
        or new.session_id <> old.session_id
        or new.scope_id <> old.scope_id
        or new.findings <> old.findings
        or new.created_at <> old.created_at
    then
        raise exception 'quarantine provenance columns are immutable';
    end if;
    return new;
end
$$;

create trigger session_event_quarantine_review_transition
    before update on session_event_quarantine
    for each row execute function synveda_session_event_quarantine_transition();

-- Deletes come from retention disposal alone, and disposal declares itself
-- with the same transaction-local flag every other append-only surface
-- retention is allowed through uses (migration 0025). Everything else is
-- refused, table owner included, and TRUNCATE always.
create function synveda_session_event_quarantine_immutable() returns trigger
language plpgsql
as $$
begin
    if tg_op = 'DELETE'
       and coalesce(current_setting('synveda.retention_purge', true), 'off') = 'on'
    then
        return old;
    end if;
    raise exception
        'session_event_quarantine rows are retired by retention disposal (MEM-6), never deleted';
end
$$;

create trigger session_event_quarantine_guarded_delete
    before delete on session_event_quarantine
    for each row execute function synveda_session_event_quarantine_immutable();

create trigger session_event_quarantine_no_truncate
    before truncate on session_event_quarantine
    execute function synveda_session_event_quarantine_immutable();

-- The same guard on the events themselves. Migration 0044 gave
-- `session_events` no DELETE at all on the reasoning that disposal belongs to
-- the retention plane; this is the retention plane arriving, so the grant
-- that sentence promised comes with the flag that bounds it.
create function synveda_session_events_immutable() returns trigger
language plpgsql
as $$
begin
    if tg_op = 'DELETE'
       and coalesce(current_setting('synveda.retention_purge', true), 'off') = 'on'
    then
        return old;
    end if;
    raise exception
        'session_events rows are retired by retention disposal (MEM-6), never deleted';
end
$$;

create trigger session_events_guarded_delete
    before delete on session_events
    for each row execute function synveda_session_events_immutable();

create trigger session_events_no_truncate
    before truncate on session_events
    execute function synveda_session_events_immutable();

-- Tenant-scoped tables ⇒ forced RLS + policy + least-privilege grants in the
-- same migration (ADR-0009 structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it). UPDATE is column-level: the
-- app can review, never rewrite findings or provenance.
grant select, insert, delete on session_event_quarantine to synveda_app;
grant update (state, reviewer_subject, reviewed_at, review_reason)
    on session_event_quarantine to synveda_app;
grant delete on session_events to synveda_app;

alter table session_event_quarantine enable row level security;
alter table session_event_quarantine force row level security;

create policy session_event_quarantine_tenant_isolation on session_event_quarantine
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

-- ── A context run cites the skills it advertised ─────────────────────────
--
-- ADR-0054 decision 8: the citation rides the response rather than the token
-- budget, so an adapter can materialise exactly what was advertised without
-- asking twice. `/v1/inject` carried it and this endpoint is what replaced
-- that route, so it carries it too.
--
-- **Stored, not computed at read time**, because this endpoint is idempotent:
-- a replay must serve the body the original served, and a citation recomputed
-- against a channel that has since moved would quietly serve a different one.

alter table session_context_runs
    add column skills jsonb not null default '[]'::jsonb;

alter table session_context_runs
    add constraint session_context_runs_skills_array_check
        check (jsonb_typeof(skills) = 'array');

-- ── The disclosure index reaches the surface that replaced inject ────────
--
-- Migration 0028's index is **partial**, to the actions that record a
-- disclosure, and the audit plane's query has to match that predicate exactly
-- or it stops using the index. `/v1/inject` and `/v1/recall` are deleted, so a
-- context run is now the only way material reaches an agent — and a disclosure
-- query that did not count it would answer "nobody was served anything" about
-- every deployment on the new plane.
--
-- Rebuilt rather than widened in place: a partial index's predicate is part of
-- its definition, so there is no ALTER that adds a value to it.

drop index if exists audit_log_disclosure_idx;

create index audit_log_disclosure_idx
    on audit_log using gin (tenant_id, payload jsonb_path_ops)
    where action in ('context.injected', 'context.recalled',
                     'session.context.composed');

-- ── The old plane, dropped whole ─────────────────────────────────────────
--
-- No data migration, no compatibility view, nothing read across (ADR-0068
-- decision 3). A database that ran the pre-cut chain is refused at startup
-- with a reset instruction and never reaches this statement.

drop table if exists observe_quarantine;
drop table if exists observe_events;

drop function if exists synveda_observe_quarantine_transition();
drop function if exists synveda_observe_quarantine_immutable();

select pgmq.drop_queue('observe');
