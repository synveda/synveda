-- AUD-1: hash-chained audit log (ADR-0019).
--
-- One BLAKE3 chain per tenant: audit_log holds the events, keyed
-- (tenant_id, seq); audit_chain_heads holds each chain's tip and is the
-- per-tenant append lock (SELECT ... FOR UPDATE). Event hashes are
-- computed by synveda-audit over a canonical serialisation — never over
-- jsonb's normalised rendering — so verification recomputes them from
-- these columns byte-for-byte (ADR-0019 decision 2).
--
-- Append-only is schema-enforced: synveda_app holds no UPDATE/DELETE on
-- audit_log, and the triggers below raise for every mutation attempt, table
-- owner included. A principal who disables triggers and rewrites history is
-- the AC's simulated attacker: verification names the broken row. The chain
-- proves integrity; anchoring heads externally is AUD-3 (WORM export).
--
-- No foreign key to tenants on either table: the audit trail outlives
-- tenant lifecycle transitions; TEN-5 governs audit disposal explicitly
-- (ADR-0019 decision 3).

create table audit_chain_heads (
    tenant_id uuid   not null,
    -- Number of events in the chain; 0 = genesis, no events yet.
    seq       bigint not null,
    -- Hash of event `seq` (or the genesis hash binding the chain to its
    -- tenant when seq = 0).
    head_hash bytea  not null,

    constraint audit_chain_heads_pk primary key (tenant_id),
    constraint audit_chain_heads_seq_check check (seq >= 0),
    constraint audit_chain_heads_hash_check check (octet_length(head_hash) = 32)
);

create table audit_log (
    tenant_id     uuid        not null,
    -- 1-based, contiguous per tenant; a gap is a verification failure.
    seq           bigint      not null,
    occurred_at   timestamptz not null,
    -- `subject`: an authenticated bearer (user or service — the identities
    -- table knows which, joined at query time by AUD-2). `break_glass`:
    -- store-level CLI access, attributed to the OS user at best
    -- (ADR-0019 decision 7).
    actor_kind    text        not null,
    actor_subject text        not null,
    -- Dotted event taxonomy (`authz.decision`, `hierarchy.node.created`,
    -- ...). Open vocabulary: later features add actions without schema
    -- churn; synveda_audit::AuditAction is the closed in-process list.
    action        text        not null,
    resource      text        not null,
    outcome       text        not null,
    -- Event-specific detail (decision context, pre/post images). Canonical
    -- constraint: no non-integer numbers — enforced at append, where the
    -- hash is computed (ADR-0019 decision 2).
    payload       jsonb       not null default '{}'::jsonb,
    -- OTel trace id when a trace was live at emission; links the chain row
    -- to its request trace.
    trace_id      text,
    prev_hash     bytea       not null,
    hash          bytea       not null,

    constraint audit_log_pk primary key (tenant_id, seq),
    constraint audit_log_seq_check check (seq >= 1),
    constraint audit_log_actor_kind_check
        check (actor_kind in ('subject', 'break_glass')),
    constraint audit_log_actor_subject_check
        check (length(actor_subject) between 1 and 255),
    constraint audit_log_action_check check (length(action) between 1 and 100),
    constraint audit_log_resource_check
        check (length(resource) between 1 and 512),
    constraint audit_log_outcome_check
        check (outcome in ('allow', 'deny', 'success', 'failure')),
    constraint audit_log_prev_hash_check check (octet_length(prev_hash) = 32),
    constraint audit_log_hash_check check (octet_length(hash) = 32)
);

-- Mutating history raises, whoever asks — the table owner included (FORCED
-- RLS does not cover superusers, but these triggers fire for them too
-- unless triggers are disabled, which is the tamper test's attacker move).
create function synveda_audit_log_immutable() returns trigger
language plpgsql
as $$
begin
    raise exception 'audit_log is append-only (AUD-1, ADR-0019)';
end
$$;

create trigger audit_log_no_update
    before update on audit_log
    for each row execute function synveda_audit_log_immutable();

create trigger audit_log_no_delete
    before delete on audit_log
    for each row execute function synveda_audit_log_immutable();

create trigger audit_log_no_truncate
    before truncate on audit_log
    execute function synveda_audit_log_immutable();

-- Tenant-scoped tables ⇒ forced RLS + policy + least-privilege grants in
-- the same migration (ADR-0009 structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it). audit_log deliberately
-- gets no UPDATE/DELETE grant; the head row advances, so it gets UPDATE
-- but no DELETE.
grant select, insert on audit_log to synveda_app;
grant select, insert, update on audit_chain_heads to synveda_app;

alter table audit_log enable row level security;
alter table audit_log force row level security;
alter table audit_chain_heads enable row level security;
alter table audit_chain_heads force row level security;

create policy audit_log_tenant_isolation on audit_log
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy audit_chain_heads_tenant_isolation on audit_chain_heads
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
