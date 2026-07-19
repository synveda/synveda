-- MEM-1: observe ingestion buffer (ADR-0020).
--
-- observe_events is the staging table: event content lands here, under
-- forced RLS, inside the caller's tenant transaction. The PGMQ queue
-- `observe` carries content-free work signals ({tenant_id, event_id});
-- the pipeline (MEM-2/3) reads the signal, opens a tenant transaction,
-- and loads the row. Content never lives outside the RLS backstop
-- (ADR-0009); the queue does delivery, nothing else.
--
-- Idempotency is the unique (tenant_id, idempotency_key): admission is
-- ON CONFLICT DO NOTHING, and only newly-inserted rows are enqueued, so
-- a redelivered event can never enter the pipeline twice (the AC's
-- "duplicate delivery does not duplicate memories", discharged
-- structurally at the buffer).
--
-- No scope/owner foreign keys, deliberately: staging rows are provenance
-- records. A service identity's revocation deletes its identity row and
-- personal leaf (ADR-0018 decision 2); buffered observations must
-- neither block that delete nor be destroyed by it — same doctrine as
-- audit_log's missing FKs (ADR-0019 decision 3). The tenant FK stays:
-- buffered events do not outlive their tenant (TEN-5 governs disposal).

create table observe_events (
    id              uuid        not null,
    tenant_id       uuid        not null,
    -- The caller's personal (home) scope — the only write target in
    -- MEM-1 (ADR-0020 decision 4).
    scope_id        uuid        not null,
    -- The submitting identity (user or service).
    owner_id        uuid        not null,
    -- Opaque harness session identifier; groups a session's deltas.
    session_id      text        not null,
    idempotency_key text        not null,
    kind            text        not null,
    payload         jsonb       not null,
    -- Client-asserted event time; received_at is the server's.
    occurred_at     timestamptz not null,
    received_at     timestamptz not null default now(),

    constraint observe_events_pk primary key (id),
    constraint observe_events_tenant_fk
        foreign key (tenant_id) references tenants (id),
    -- The admission gate: one key, one event, per tenant.
    constraint observe_events_idempotency_unique
        unique (tenant_id, idempotency_key),
    constraint observe_events_kind_check
        check (kind in ('transcript_delta', 'tool_result', 'decision')),
    constraint observe_events_session_check
        check (length(session_id) between 1 and 200),
    constraint observe_events_key_check
        check (length(idempotency_key) between 1 and 200)
);

-- Tenant-scoped tables ⇒ forced RLS + policy + least-privilege grants in
-- the same migration (ADR-0009 structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it). SELECT + INSERT only:
-- the application cannot rewrite what was observed. Delivery state lives
-- in PGMQ; content disposal is MEM-6/TEN-5 territory and brings its own
-- grants.
grant select, insert on observe_events to synveda_app;

alter table observe_events enable row level security;
alter table observe_events force row level security;

create policy observe_events_tenant_isolation on observe_events
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

-- The work-signal queue. pgmq tables have PGMQ's own shape (no tenant_id
-- column — outside the RLS discipline by design, which is why messages
-- carry ids only, never content).
select pgmq.create('observe');

-- pgmq functions are SECURITY INVOKER: table privileges gate effects.
-- send needs INSERT (and the msg_id identity's sequence); read/pop/archive
-- need SELECT/UPDATE/DELETE on the queue and INSERT on the archive.
-- EXECUTE alone confers no DDL power (create_queue would still fail on
-- schema CREATE rights).
grant usage on schema pgmq to synveda_app;
grant execute on all functions in schema pgmq to synveda_app;
grant select, insert, update, delete on pgmq.q_observe to synveda_app;
grant select, insert on pgmq.a_observe to synveda_app;
grant usage, select on all sequences in schema pgmq to synveda_app;
