-- FLOW-1: the VedaFlow object store (ADR-0030; substrate settled by ADR-0003).
--
-- Six tables migrated here with the rest of the schema — this crate owns the
-- one embedded migrator — while their semantics live in `synveda-vedaflow`.
-- Same split as the AUD-1 audit tables (migration 0011): the sibling crate
-- computes the hashes, this migration makes the invariants true.
--
-- Named `vedaflow_*` (ADR-0030 decision 13): tech plan §2.1 sketches them as
-- objects/trees/commits/refs, but unqualified in a schema that also holds
-- records, identities, and policy_packs, three of those four are ambiguous —
-- `commits` in particular reads as a database concept.
--
-- Content addressing (ADR-0030 decision 2): every hash is
-- BLAKE3(domain ‖ length-prefixed fields), computed in Rust. The tenant is
-- deliberately NOT in the hash, so an auditor holding the bytes — or the
-- FLOW-8 git mirror holding the exported object — recomputes the same
-- address with no access to this schema. Storage is per-tenant regardless:
-- primary keys are (tenant_id, hash), so identical content dedups inside a
-- tenant and never across one. A shared row is the arrangement forced RLS
-- cannot express, and `on conflict do nothing` over it would answer
-- "already present" for content the caller never wrote — an oracle for
-- another tenant's knowledge (ADR-0030 decision 3).
--
-- Immutability (ADR-0030 decision 6) is schema-enforced on the audit_log
-- pattern: the five history tables grant synveda_app SELECT and INSERT only
-- and raise on every UPDATE/DELETE/TRUNCATE, table owner included.
-- vedaflow_refs is the one mutable table — a ref is a pointer and moving it
-- is the point — and it holds no DELETE grant either.
--
-- No foreign key to identities on author/updated_by, and none to
-- hierarchy_nodes on scope: a service identity's revocation deletes its
-- identity row and personal leaf (ADR-0018 decision 2), and recorded history
-- must neither block that deletion nor be destroyed by it — the AUD-1 /
-- MEM-1 doctrine. An `on delete cascade` from hierarchy_nodes would also be
-- a ref-deletion path around the withheld DELETE grant. The tenant foreign
-- keys stay: knowledge history does not outlive its tenant, and TEN-5 owns
-- its disposal alongside the observe buffer and the CTX-1 sidecars.

-- ── Objects: immutable content-addressed blobs ──────────────────────────────

create table vedaflow_objects (
    tenant_id  uuid        not null,
    -- BLAKE3 over ("synveda-vedaflow-object-v1" ‖ len(kind) ‖ kind ‖
    -- len(content) ‖ content). `kind` is inside the address: identical bytes
    -- registered as a prompt and as a skill are two different objects,
    -- because FLOW-3 resolves required approvals from asset type and a skill
    -- is executable where a prompt is not (ADR-0030 decision 4).
    hash       bytea       not null,
    kind       text        not null,
    -- bytea, not text: skill bundles (SKIL-1) carry files that are not
    -- necessarily UTF-8, and hashing bytes leaves no encoding question.
    content    bytea       not null,
    size_bytes integer     not null,
    -- First-write time. Not hashed, not updatable; provenance for the
    -- packing/GC story ADR-0003 anticipated and ADR-0030 leaves open.
    created_at timestamptz not null default now(),

    constraint vedaflow_objects_pk primary key (tenant_id, hash),
    constraint vedaflow_objects_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint vedaflow_objects_hash_check check (octet_length(hash) = 32),
    -- The four managed asset types (seed §4.3) plus policy, which tech plan
    -- §2.3 makes an asset: "policy packs and lapses are themselves assets
    -- flowing through VedaFlow".
    constraint vedaflow_objects_kind_check
        check (kind in ('memory', 'prompt', 'skill', 'context-pack', 'policy')),
    -- size_bytes is the content's own length, never an independent claim.
    -- The 8 MiB cap is deliberate and is a reviewed diff to raise (ADR-0030
    -- reversal trigger b): a governed store that accepts arbitrary blobs is
    -- a file server with an approval workflow.
    constraint vedaflow_objects_size_check
        check (size_bytes = octet_length(content) and size_bytes <= 8388608)
);

-- ── Trees: named groupings, per scope ───────────────────────────────────────

create table vedaflow_trees (
    tenant_id  uuid        not null,
    -- BLAKE3 over ("synveda-vedaflow-tree-v1" ‖ entry count ‖ per entry:
    -- len(name) ‖ name ‖ target tag ‖ target hash), entries sorted bytewise
    -- by name. A tree's hash covers its children's hashes, so a tree cannot
    -- contain itself without a preimage attack — cycles are impossible by
    -- construction, not by a check (ADR-0030 decision 5).
    hash       bytea       not null,
    created_at timestamptz not null default now(),

    constraint vedaflow_trees_pk primary key (tenant_id, hash),
    constraint vedaflow_trees_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint vedaflow_trees_hash_check check (octet_length(hash) = 32)
);

-- One row per entry rather than an `entries[]` column (ADR-0030 decision 5):
-- the foreign keys are the point. A tree entry pointing at an object that
-- does not exist is unrepresentable here, where with an array column it
-- would be a dangling reference some later batch job might find.
--
-- The two target columns are how a conditional foreign key is spelled in
-- Postgres: exactly one is non-null, each carries its own FK, and MATCH
-- SIMPLE (the default) leaves a composite FK satisfied when a component is
-- null. No ordinal column: entries are canonically sorted by name, so an
-- ordinal would be a redundant column that could disagree with the hash.
create table vedaflow_tree_entries (
    tenant_id    uuid  not null,
    tree_hash    bytea not null,
    name         text  not null,
    object_hash  bytea,
    subtree_hash bytea,

    constraint vedaflow_tree_entries_pk primary key (tenant_id, tree_hash, name),
    constraint vedaflow_tree_entries_tree_fk
        foreign key (tenant_id, tree_hash)
        references vedaflow_trees (tenant_id, hash),
    constraint vedaflow_tree_entries_object_fk
        foreign key (tenant_id, object_hash)
        references vedaflow_objects (tenant_id, hash),
    constraint vedaflow_tree_entries_subtree_fk
        foreign key (tenant_id, subtree_hash)
        references vedaflow_trees (tenant_id, hash),
    constraint vedaflow_tree_entries_target_check
        check ((object_hash is null) <> (subtree_hash is null)),
    constraint vedaflow_tree_entries_name_check
        check (length(name) between 1 and 255)
);

-- ── Commits: the governed history ───────────────────────────────────────────

create table vedaflow_commits (
    tenant_id            uuid        not null,
    -- BLAKE3 over ("synveda-vedaflow-commit-v1" ‖ tree ‖ parent count ‖
    -- parents in order ‖ author ‖ committed_at ‖ len(message) ‖ message ‖
    -- policy snapshot).
    hash                 bytea       not null,
    tree_hash            bytea       not null,
    -- The authoring identity. Deliberately un-FK'd; see the header.
    author_id            uuid        not null,
    message              text        not null,
    -- Hashed as RFC 3339 UTC truncated to microseconds — the AUD-1 canonical
    -- timestamp rule (ADR-0019 decision 2), so recomputation from this
    -- column is byte-exact.
    committed_at         timestamptz not null,
    -- Which policy pack, at which version, with which configuration, was in
    -- force when this commit was made (ADR-0003's compliance claim;
    -- ADR-0030 decision 8). BLAKE3 over the caller's canonical snapshot.
    policy_snapshot_hash bytea       not null,
    -- Over `hash`, which already covers every field above (ADR-0030
    -- decision 9). NULL means nobody signed it — the honest default, rather
    -- than a signature over nothing.
    signature            bytea,
    signer_key_id        text,
    created_at           timestamptz not null default now(),

    constraint vedaflow_commits_pk primary key (tenant_id, hash),
    constraint vedaflow_commits_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint vedaflow_commits_tree_fk
        foreign key (tenant_id, tree_hash)
        references vedaflow_trees (tenant_id, hash),
    constraint vedaflow_commits_hash_check check (octet_length(hash) = 32),
    constraint vedaflow_commits_policy_snapshot_check
        check (octet_length(policy_snapshot_hash) = 32),
    constraint vedaflow_commits_message_check
        check (length(message) between 1 and 4096),
    -- A signature with no key id is unverifiable; a key id with no signature
    -- is noise. Length is a range, not 64: pinning Ed25519 in the schema
    -- would defeat the point of the signer seam.
    constraint vedaflow_commits_signature_pairing_check
        check ((signature is null) = (signer_key_id is null)),
    constraint vedaflow_commits_signature_size_check
        check (signature is null or octet_length(signature) between 1 and 1024),
    constraint vedaflow_commits_signer_key_check
        check (signer_key_id is null or length(signer_key_id) between 1 and 128)
);

-- Ordered parents as rows (ADR-0030 decision 5). First parent is the
-- mainline, as in git, so ordinal is semantic here and is kept — unlike the
-- tree entries, whose order is derivable from their names. Both foreign keys
-- point back at vedaflow_commits, which is what closes the DAG: a commit
-- claiming a parent that does not exist cannot be inserted.
create table vedaflow_commit_parents (
    tenant_id   uuid    not null,
    commit_hash bytea   not null,
    ordinal     integer not null,
    parent_hash bytea   not null,

    constraint vedaflow_commit_parents_pk
        primary key (tenant_id, commit_hash, ordinal),
    constraint vedaflow_commit_parents_unique
        unique (tenant_id, commit_hash, parent_hash),
    constraint vedaflow_commit_parents_commit_fk
        foreign key (tenant_id, commit_hash)
        references vedaflow_commits (tenant_id, hash),
    constraint vedaflow_commit_parents_parent_fk
        foreign key (tenant_id, parent_hash)
        references vedaflow_commits (tenant_id, hash),
    constraint vedaflow_commit_parents_ordinal_check check (ordinal >= 0)
);

-- ── Refs: the only mutable table ────────────────────────────────────────────

-- FLOW-2 gives names meaning (derived/staged/published per scope per asset
-- type); FLOW-1 leaves the vocabulary open on purpose — a CHECK here would
-- have to be guessed now and migrated then.
--
-- Updates are compare-and-swap in the application (ADR-0030 decision 10):
-- `update ... where commit_hash = $expected`, with zero affected rows
-- reported to the caller as a race to retry. No DELETE grant: a ref is a
-- standing channel pointer, created once per scope per asset type, and
-- disposal is TEN-5's.
create table vedaflow_refs (
    tenant_id   uuid        not null,
    scope_id    uuid        not null,
    name        text        not null,
    commit_hash bytea       not null,
    updated_at  timestamptz not null default now(),
    -- The identity that last moved this ref. Un-FK'd; see the header.
    updated_by  uuid        not null,

    constraint vedaflow_refs_pk primary key (tenant_id, scope_id, name),
    constraint vedaflow_refs_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint vedaflow_refs_commit_fk
        foreign key (tenant_id, commit_hash)
        references vedaflow_commits (tenant_id, hash),
    constraint vedaflow_refs_name_check check (length(name) between 1 and 200)
);

-- ── Immutability ────────────────────────────────────────────────────────────

-- Mutating recorded history raises, whoever asks — the table owner included.
-- This is not proof against a principal who disables triggers; nothing at
-- this layer is. It makes tampering require that step, and
-- `synveda_vedaflow::verify` recomputes every hash from the stored columns
-- to make the step visible afterwards (the AUD-1 shape, ADR-0030 decision 6).
create function synveda_vedaflow_immutable() returns trigger
language plpgsql
as $$
begin
    raise exception '% is append-only (FLOW-1, ADR-0030)', tg_table_name;
end
$$;

create trigger vedaflow_objects_no_update
    before update on vedaflow_objects
    for each row execute function synveda_vedaflow_immutable();
create trigger vedaflow_objects_no_delete
    before delete on vedaflow_objects
    for each row execute function synveda_vedaflow_immutable();
create trigger vedaflow_objects_no_truncate
    before truncate on vedaflow_objects
    execute function synveda_vedaflow_immutable();

create trigger vedaflow_trees_no_update
    before update on vedaflow_trees
    for each row execute function synveda_vedaflow_immutable();
create trigger vedaflow_trees_no_delete
    before delete on vedaflow_trees
    for each row execute function synveda_vedaflow_immutable();
create trigger vedaflow_trees_no_truncate
    before truncate on vedaflow_trees
    execute function synveda_vedaflow_immutable();

create trigger vedaflow_tree_entries_no_update
    before update on vedaflow_tree_entries
    for each row execute function synveda_vedaflow_immutable();
create trigger vedaflow_tree_entries_no_delete
    before delete on vedaflow_tree_entries
    for each row execute function synveda_vedaflow_immutable();
create trigger vedaflow_tree_entries_no_truncate
    before truncate on vedaflow_tree_entries
    execute function synveda_vedaflow_immutable();

create trigger vedaflow_commits_no_update
    before update on vedaflow_commits
    for each row execute function synveda_vedaflow_immutable();
create trigger vedaflow_commits_no_delete
    before delete on vedaflow_commits
    for each row execute function synveda_vedaflow_immutable();
create trigger vedaflow_commits_no_truncate
    before truncate on vedaflow_commits
    execute function synveda_vedaflow_immutable();

create trigger vedaflow_commit_parents_no_update
    before update on vedaflow_commit_parents
    for each row execute function synveda_vedaflow_immutable();
create trigger vedaflow_commit_parents_no_delete
    before delete on vedaflow_commit_parents
    for each row execute function synveda_vedaflow_immutable();
create trigger vedaflow_commit_parents_no_truncate
    before truncate on vedaflow_commit_parents
    execute function synveda_vedaflow_immutable();

-- ── RLS + least-privilege grants ────────────────────────────────────────────

-- Tenant-scoped tables ⇒ forced RLS + policy + least-privilege grants in the
-- same migration (ADR-0009's structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it).

grant select, insert on vedaflow_objects to synveda_app;
grant select, insert on vedaflow_trees to synveda_app;
grant select, insert on vedaflow_tree_entries to synveda_app;
grant select, insert on vedaflow_commits to synveda_app;
grant select, insert on vedaflow_commit_parents to synveda_app;
-- Refs move; they never disappear.
grant select, insert, update on vedaflow_refs to synveda_app;

alter table vedaflow_objects enable row level security;
alter table vedaflow_objects force row level security;
alter table vedaflow_trees enable row level security;
alter table vedaflow_trees force row level security;
alter table vedaflow_tree_entries enable row level security;
alter table vedaflow_tree_entries force row level security;
alter table vedaflow_commits enable row level security;
alter table vedaflow_commits force row level security;
alter table vedaflow_commit_parents enable row level security;
alter table vedaflow_commit_parents force row level security;
alter table vedaflow_refs enable row level security;
alter table vedaflow_refs force row level security;

create policy vedaflow_objects_tenant_isolation on vedaflow_objects
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy vedaflow_trees_tenant_isolation on vedaflow_trees
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy vedaflow_tree_entries_tenant_isolation on vedaflow_tree_entries
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy vedaflow_commits_tenant_isolation on vedaflow_commits
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy vedaflow_commit_parents_tenant_isolation on vedaflow_commit_parents
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy vedaflow_refs_tenant_isolation on vedaflow_refs
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
