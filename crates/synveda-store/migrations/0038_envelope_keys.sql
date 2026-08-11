-- TEN-4: the key plane, and the first columns sealed by it (ADR-0064).
--
-- Two levels of key. A key-encryption key lives in the KMS and never reaches
-- this database; the data keys below are stored **only** wrapped by it, so a
-- dumped table is a dumped table of ciphertext. That is the whole claim, and
-- it is bounded by the AUD-1 trust boundary: an operator who can read the
-- deployment's configuration can read the KEK, and this defends a stolen
-- artefact rather than that operator.
--
-- ── Two key scopes, and the second one is a finding ──────────────────────
--
-- `tenant_keys` is what "per-tenant encryption keys" names. `deployment_keys`
-- exists because `console_sessions` **cannot have a tenant key** (ADR-0064
-- decision 5), and the reason is worth reading before somebody tidies it
-- away: a session row is read *before* the tenant exists — reading it is one
-- of the steps that establishes the tenant — so selecting a per-tenant key
-- for it would require deriving a tenant from the session row, which is
-- exactly the derivation migration 0034 refuses a `tenant_id` column in order
-- to make impossible.
--
-- The tempting fix is to give that table its tenant back and have one key
-- scope. That trades a real isolation invariant (a forged session row cannot
-- move a reviewer into another organisation, because there is nowhere in it
-- to write an organisation) for a cosmetic one (all key rows look alike). So
-- there are two tables, and `deployment_keys` sits on the pre-scope side of
-- RLS beside `tenants` and `console_sessions`, holding nothing whose
-- disclosure says anything about any tenant.
--
-- ── What is deliberately not sealed ──────────────────────────────────────
--
-- `records.content`, `record_embeddings` and the Tantivy sidecars. There is
-- no BM25 over ciphertext and no HNSW over ciphertext, so sealing them
-- deletes ADR-0024 — both retrieval legs — rather than hardening it.
-- Encryption at rest for the retrieval substrate is the volume's, and
-- ADR-0064 decision 7 scopes seed §10's "all data encrypted at rest" out
-- loud rather than shipping less quietly. The consequence to keep in view:
-- destroying a tenant's key makes its *sealed* data unreadable and leaves
-- its records exactly where they were, so TEN-5's erasure still deletes rows
-- and crypto-shredding is not erasure here.

-- ── The deployment key ───────────────────────────────────────────────────

create table deployment_keys (
    -- The generation. Carried in every envelope header sealed under it, so a
    -- reader peeks the version, selects that key, and opens — which is what
    -- makes rotation lazy instead of stop-the-world (ADR-0064 decision 6).
    version     integer     not null,

    -- The data key, sealed by the KEK. Never the key itself.
    wrapped_dek bytea       not null,

    -- Which KEK wrapped it. Stored per row rather than as a deployment
    -- constant so re-keying is expressible and so an operator can tell which
    -- KEK a row needs — and, for `tenant_keys`, so BYOK is a column value
    -- rather than a redesign (ADR-0064 decision 1).
    kek_ref     text        not null,

    -- Advisory, for the operator query "which keys are still on the old
    -- algorithm". **The envelope header is authoritative** — it is what the
    -- AAD binds and what the reader dispatches on — and this column is a
    -- projection of it for people rather than for code.
    algorithm   text        not null,

    created_at  timestamptz not null default now(),

    -- Stamped when a newer generation supersedes this one. The row stays:
    -- a retired key still opens everything sealed under it, and deleting it
    -- is how lazily-rotated data becomes unreadable by accident.
    retired_at  timestamptz,

    constraint deployment_keys_pk primary key (version),
    constraint deployment_keys_version_check check (version >= 1),
    -- 34 header + 32 key + 16 tag. Exact rather than a range: a wrapped data
    -- key is always the same size, and a column that accepts other sizes is
    -- a column somebody eventually writes something else into.
    constraint deployment_keys_wrapped_check check (octet_length(wrapped_dek) = 82),
    constraint deployment_keys_kek_ref_check check (length(kek_ref) between 1 and 256),
    constraint deployment_keys_algorithm_check
        check (algorithm in ('xchacha20-poly1305')),
    constraint deployment_keys_retired_check
        check (retired_at is null or retired_at >= created_at)
);

-- At most one current key, enforced rather than assumed: rotation inserts the
-- new generation and retires the old one in the same transaction, and a
-- second un-retired row would make "which key seals new payloads" ambiguous
-- at exactly the moment two writers disagree.
create unique index deployment_keys_current
    on deployment_keys ((true)) where retired_at is null;

-- No DELETE and no RLS. The absent policy is structural, not an exemption:
-- this table has no `tenant_id` for a predicate to be written over, which is
-- the same shape `console_sessions` has and the same reason (migration 0034).
grant select, insert, update on deployment_keys to synveda_app;

-- ── Per-tenant keys ──────────────────────────────────────────────────────

create table tenant_keys (
    tenant_id   uuid        not null,
    version     integer     not null,
    wrapped_dek bytea       not null,
    kek_ref     text        not null,
    algorithm   text        not null,
    created_at  timestamptz not null default now(),
    retired_at  timestamptz,

    constraint tenant_keys_pk primary key (tenant_id, version),
    constraint tenant_keys_tenant_fk foreign key (tenant_id) references tenants (id),
    constraint tenant_keys_version_check check (version >= 1),
    constraint tenant_keys_wrapped_check check (octet_length(wrapped_dek) = 82),
    constraint tenant_keys_kek_ref_check check (length(kek_ref) between 1 and 256),
    constraint tenant_keys_algorithm_check check (algorithm in ('xchacha20-poly1305')),
    constraint tenant_keys_retired_check
        check (retired_at is null or retired_at >= created_at)
);

create unique index tenant_keys_current
    on tenant_keys (tenant_id) where retired_at is null;

-- Tenant-scoped table ⇒ forced RLS + policy + least-privilege grants in the
-- same migration (ADR-0009's structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it).
--
-- RLS over a table of *wrapped* keys is not the load-bearing control — the
-- KEK is — and it is here anyway, because the guard's value is that nobody
-- can quietly opt out and because two independent boundaries fail
-- independently: RLS decides which ciphertext a query can fetch, the AAD
-- decides whether it opens (ADR-0064 decision 4).
alter table tenant_keys enable row level security;
alter table tenant_keys force row level security;

create policy tenant_keys_tenant_isolation on tenant_keys
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

-- No DELETE: a key row the application can drop is data it can make
-- permanently unreadable, one statement from a bug. TEN-5's erasure destroys
-- keys deliberately, with a workflow and a destruction certificate around it.
grant select, insert, update on tenant_keys to synveda_app;

-- ── Sealed per-tenant secrets ────────────────────────────────────────────
--
-- The custody ADR-0060 decision 7 deferred to this feature. Its outbound
-- directory credential lives here instead of in deployment configuration,
-- which is what lets one deployment pull two tenants from two directories —
-- the standing limitation AUTH-5 shipped with, deleted.
--
-- Every value is an envelope bound to this tenant, to `name`, and to the
-- purpose the sealing crate names, so a ciphertext moved between tenants or
-- between names fails to open rather than opening.

create table tenant_secrets (
    tenant_id  uuid        not null,

    -- What this secret is. Dotted-lowercase, and it is part of the sealed
    -- payload's AAD — so renaming a secret does not silently re-point a
    -- ciphertext, it makes it unopenable, which is the safe direction.
    --
    -- Not a check-constrained vocabulary: the closed set lives in
    -- `synveda_crypto::Purpose`, where the AAD is composed, and a second copy
    -- in SQL would be a second thing to keep in step for no enforcement the
    -- cipher does not already give.
    name       text        not null,

    -- The envelope. Sealed under this tenant's data key at the generation its
    -- header names.
    sealed     bytea       not null,

    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now(),

    constraint tenant_secrets_pk primary key (tenant_id, name),
    constraint tenant_secrets_tenant_fk foreign key (tenant_id) references tenants (id),
    constraint tenant_secrets_name_check
        check (name ~ '^[a-z][a-z0-9]*(\.[a-z][a-z0-9_]*)*$' and length(name) <= 128),
    -- 34 header + 16 tag + a plaintext bounded like the credentials it holds.
    constraint tenant_secrets_sealed_check
        check (octet_length(sealed) between 51 and 8242)
);

alter table tenant_secrets enable row level security;
alter table tenant_secrets force row level security;

create policy tenant_secrets_tenant_isolation on tenant_secrets
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

-- DELETE **is** granted, and the contrast with `tenant_keys` above is the
-- point: a credential that cannot be destroyed is a credential that cannot be
-- revoked, which is `console_sessions`' reasoning (migration 0034). What is
-- durable about a secret is not this row but the audit chain of everything it
-- did, which deleting it does not touch.
grant select, insert, update, delete on tenant_secrets to synveda_app;

-- ── The console session's tokens, sealed ─────────────────────────────────
--
-- Migration 0034 stored these recoverable and said so: "the first live
-- credential this product stores at rest ... TEN-4 is where this column and
-- the one below get a key". This is that.
--
-- **Existing rows go.** A migration cannot seal a plaintext it can read but
-- has no key for — the KEK is in the application's configuration, not in
-- SQL — so the honest upgrade is that every open console session ends and
-- everyone logs in again. That is a one-time cost on a credential which is
-- already capped, already expiring, and already re-obtainable by a login;
-- the alternative is a nullable sealed column beside a plaintext one, which
-- is a plaintext column that survives forever because nothing forces the
-- last row through.
delete from console_sessions;

alter table console_sessions
    drop column access_token,
    drop column refresh_token,

    -- Sealed under the **deployment** key: see this migration's header.
    add column access_token_sealed   bytea not null,
    add column refresh_token_sealed  bytea;

alter table console_sessions
    -- The old bounds were `length(...) between 1 and 8192` on text. A sealed
    -- payload is 34 header + plaintext + 16 tag, so the same 1..8192 of
    -- plaintext is 51..8242 of envelope. Stated as arithmetic rather than a
    -- round number so the next person can check it against the format.
    add constraint console_sessions_access_token_check
        check (octet_length(access_token_sealed) between 51 and 8242),
    add constraint console_sessions_refresh_token_check
        check (refresh_token_sealed is null
               or octet_length(refresh_token_sealed) between 51 and 8242);
