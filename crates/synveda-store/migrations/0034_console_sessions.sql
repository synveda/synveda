-- CNSL-1: server-side custody of a browser's credential (ADR-0056).
--
-- One table, and the column it deliberately does **not** have is the
-- design.
--
-- ADR-0056 decision 2 says the cookie *names* a bearer rather than becoming
-- one: a console request loads the access token stored here and hands it to
-- the same `TokenVerifier` a bearer goes through, so the session's authority
-- is the token's authority, re-checked on every request. The first draft of
-- this table carried `tenant_id`, on the reflex that every table does — and
-- that reflex was wrong twice over.
--
-- It was wrong *mechanically*, because the RLS completeness guard
-- (crates/synveda-store/tests/rls.rs) discovers any public base table with a
-- `tenant_id` column and requires forced row security with a tenant
-- predicate. A session is read **before** the tenant scope exists — reading
-- it is one of the steps that establishes the tenant — so that predicate
-- would evaluate against an unset GUC and return zero rows at precisely the
-- moment the row is needed. The obvious dodges (exempt the table, rename the
-- column) both work and both are lies to a guard whose entire value is that
-- nobody can quietly opt out of it.
--
-- It was wrong *architecturally*, which is the half worth keeping. If the
-- row carries no tenant, then no code path can derive a tenant from it, and
-- ADR-0056's invariant stops being a property somebody has to maintain and
-- becomes a property the schema enforces: the tenant comes from the verified
-- token's `tid` claim, exactly as it does for a bearer (TEN-1, ADR-0008), and
-- a corrupted or forged session row cannot move a reviewer into another
-- organisation because there is nowhere in it to write an organisation.
--
-- So this table is not tenant-scoped data. It is credential custody, keyed
-- by a secret, in the same category as the `tenants` table it sits beside on
-- the pre-scope side of RLS — and unlike `tenants`, it holds nothing that
-- reading it would disclose about a tenant.

create table console_sessions (
    -- SHA-256 of the session secret the browser holds in its cookie. The
    -- secret itself is never stored, on the AUD-1 threat model: an attacker
    -- with database credentials who dumps this table must not be able to
    -- mint a cookie from it. 256 bits of `getrandom` entropy needs no
    -- stretching — this is a token lookup, not a password check.
    token_hash          bytea       not null,

    -- The issuer that authenticated the login. Stored because a refresh has
    -- to name the same token endpoint and client (ADR-0027 decision 6's
    -- reasoning, arriving server-side instead of in a CLI's config file).
    issuer              text        not null,

    -- The `/v1` bearer this cookie names. Recoverable by necessity: the
    -- gateway does not check this credential, it *presents* it, so a hash
    -- would defeat the purpose.
    --
    -- This is the first live credential this product stores at rest, and
    -- ADR-0056 records it as an accepted exposure with a named successor:
    -- TEN-4 (per-tenant encryption keys) is where this column and the one
    -- below get a key. Until then the compensating controls are that the
    -- row's usefulness expires with the IdP's own token lifetime, that the
    -- lookup key is hashed, and that the database trust boundary is the one
    -- AUD-1 already documents.
    access_token        text        not null,

    -- When the access token expires, as the IdP reported it. Null for an
    -- issuer that reported no lifetime: the token is then used until the
    -- gateway rejects it, which is the same rule the CLI's credential store
    -- applies (ADPT-1) — the gateway is the authority on that, and refusing
    -- to try would be worse than a 401.
    access_expires_at   timestamptz,

    -- The refresh token, when the issuer granted one. Its absence is what
    -- makes a console session eventually need a fresh login, exactly as it
    -- is for the CLI. **This is the column that justifies the whole table**:
    -- ADR-0027 decision 6 made `SessionResponse` structurally incapable of
    -- carrying a refresh token to a browser, so the only way a console
    -- session can outlive one access token is for the refresh to happen
    -- somewhere the browser cannot reach.
    refresh_token       text,

    created_at          timestamptz not null default now(),

    -- Advanced on use, so an idle session can be reaped without reaping one
    -- somebody is sitting in front of. Written on a coarse cadence rather
    -- than on every request: a review screen polls, and a row updated on
    -- each poll would turn a read path into a write path.
    last_seen_at        timestamptz not null default now(),

    -- The hard cap. A refresh token that an IdP never rotates would
    -- otherwise make a console session immortal, and "log in again
    -- occasionally" is not a feature a governance product should have to be
    -- argued into.
    absolute_expires_at timestamptz not null,

    constraint console_sessions_pk primary key (token_hash),
    constraint console_sessions_hash_check check (octet_length(token_hash) = 32),
    constraint console_sessions_issuer_check check (length(issuer) between 1 and 512),
    constraint console_sessions_access_token_check check (length(access_token) between 1 and 8192),
    constraint console_sessions_refresh_token_check
        check (refresh_token is null or length(refresh_token) between 1 and 8192),
    -- A session whose hard cap precedes its own creation is a row nothing
    -- can ever read back; refuse it here rather than let it sit there
    -- looking like a session.
    constraint console_sessions_expiry_check check (absolute_expires_at > created_at)
);

-- The purge's access path, and the only query on this table that is not a
-- primary-key lookup.
create index console_sessions_expiry on console_sessions (absolute_expires_at);

-- ── Grants ──────────────────────────────────────────────────────────────────
--
-- No RLS, and the header comment is the argument: there is no `tenant_id` to
-- write a policy over, and the row is read before a tenant exists to scope
-- it to. The completeness guard in crates/synveda-store/tests/rls.rs is
-- satisfied structurally rather than by exemption — it discovers tables *by*
-- their `tenant_id` column, and this table has none.
--
-- **DELETE is granted**, which is rare here and worth the contrast.
-- Migration 0033 withheld it from `skill_reviews` because a product that can
-- erase a review is a product whose review trail can be edited. The opposite
-- reasoning applies to a credential: a session that cannot be destroyed is a
-- session that cannot be revoked, and sign-out has to mean something. What
-- is durable about a console session is not this row — it is the audit chain
-- of everything the session did, which is untouched by deleting it (ADR-0056
-- decision 9: the console is not a different actor).
grant select, insert, update, delete on console_sessions to synveda_app;
