-- CPR-2: the schema epoch marker (ADR-0068 decision 3, ADR-0069).
--
-- This product is pre-1.0 and the context-platform redesign is a hard cut:
-- there is no migration from the schema that came before it, no dual read, no
-- compatibility view and no data translator. A database written before the cut
-- is *refused* rather than upgraded, and this table is what makes that a fact
-- the database itself carries rather than something a binary infers.
--
-- ── Why a marker rather than the migration list ──────────────────────────
--
-- `_sqlx_migrations` already records which files have run, and it is the wrong
-- thing to decide an epoch on. It answers "which of this binary's migrations
-- have been applied", which is a question about *this* chain — so a database
-- from before the cut, whose chain a later prompt deletes and replaces, is
-- indistinguishable from a database halfway through the current one. It also
-- moves on every ordinary release, which is exactly what an epoch must not do.
-- The epoch is a separate, deliberate number: it changes when the model
-- underneath changes incompatibly, and never because a migration was added.
--
-- ── One row, and it is the deployment's ──────────────────────────────────
--
-- No `tenant_id`, no RLS. That is structural satisfaction rather than an
-- exemption, the same shape `console_sessions` (migration 0034) and
-- `deployment_keys` (migration 0038) have: there is no tenant column for a
-- policy predicate to be written over, because the schema epoch is a property
-- of the database, not of anybody's data. It holds nothing whose disclosure
-- says anything about any tenant — the guard that reads it runs *before* a
-- tenant is resolved, which is precisely why it cannot be tenant-scoped.
--
-- ── The row is written by Rust, not by this file ─────────────────────────
--
-- `synveda_store::migrate` stamps it after the migrator runs, because two of
-- its four columns are facts only the running binary has: the product version
-- that created the epoch, and the migration head actually reached. A migration
-- carrying a hard-coded version string would be a second place to remember to
-- bump on every release, and it would be wrong in the one direction that
-- matters — the epoch's creator recorded as whichever release happened to
-- write the SQL.
--
-- The refusal of a pre-epoch database lives in Rust too
-- (`synveda_store::epoch::preflight`), and deliberately in *one* place: the
-- message it prints names the reset command, and a `RAISE EXCEPTION` here
-- would be a second copy of that message to keep in step with the CLI.

create table schema_metadata (
    -- Single-row table. `id` is a boolean that must be true, so the primary
    -- key and the CHECK together make a second row unrepresentable rather
    -- than merely unexpected — a marker that can disagree with itself is
    -- worse than no marker.
    id                 boolean     not null default true,

    -- The epoch. Incompatible model changes bump it; ordinary migrations do
    -- not. A database at a different number than the binary serves is
    -- refused in both directions: lower means reset, higher means the
    -- installation is behind and resetting would destroy readable data.
    epoch              integer     not null,

    -- The migration head reached, as its four-digit file prefix. Diagnostic
    -- rather than load-bearing: the guard decides on `epoch` alone, and this
    -- is what the refusal quotes back so an operator can tell two databases
    -- at the same epoch apart.
    migration_head     text        not null,

    -- When the epoch was created — the moment this database became a Synveda
    -- database, which is not the same as when the deployment was installed.
    created_at         timestamptz not null default now(),

    -- The product version that created the epoch. Never rewritten by a later
    -- migration: "which release minted this database" is a fact about the
    -- past, and a column that tracks the current binary would answer a
    -- different question that nobody asked.
    created_by_version text        not null,

    -- Moved every time the migrator advances the head.
    updated_at         timestamptz not null default now(),

    constraint schema_metadata_pk primary key (id),
    constraint schema_metadata_single_row check (id),
    constraint schema_metadata_epoch_check check (epoch >= 1),
    constraint schema_metadata_migration_head_check
        check (length(migration_head) between 1 and 64),
    constraint schema_metadata_created_by_version_check
        check (length(created_by_version) between 1 and 64),
    constraint schema_metadata_updated_check check (updated_at >= created_at)
);

-- Read-only for the data plane. The row is written by whoever runs
-- migrations, which is the owner role — never the application.
grant select on schema_metadata to synveda_app;
