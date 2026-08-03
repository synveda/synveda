-- SKIL-1: the skills registry (ADR-0051).
--
-- Two tables, and neither is a new idea: `skills` is migration 0030's bundle
-- row and `skill_files` is its documents. Everything else about a skill is
-- already expressible — `vedaflow_objects` addresses each file's bytes,
-- `vedaflow_proposals` reviews them, `vedaflow_refs` publishes them, and the
-- channel's first-parent line is its version history (ADR-0051 decision 1).
--
-- What is deliberately **absent** is the third table PRMT-2 needed. A context
-- pack's published content becomes `records`, so it needed a mapping from a
-- row to the document it was cut from. A skill's content becomes nothing: it
-- is fetched by name and materialised into a client's own skills directory,
-- and the client's progressive disclosure is the loader (ADR-0051
-- decision 9). Nothing here touches the read path's corpus, and that absence
-- is the shape of the decision rather than an oversight.
--
-- Why the drafts are not channels (decision 1, on ADR-0049 decision 2's
-- reasoning): ADR-0032 decision 2 kept `staged` unwritten because "a set
-- channel cannot express withdrawal", and an author replacing a file is
-- exactly that withdrawal. So there is deliberately no `skill/staged` ref and
-- nothing writes one.

create table skills (
    tenant_id     uuid        not null,
    -- The scope that stands behind it. Part of the key, because the *same*
    -- skill name at a nearer scope is how a team overrides the org's — two
    -- rows, two skills, one name. For skills that override is also a
    -- *filesystem* fact: the name is the installed directory name and a
    -- client's skills root is flat, so only one of them can exist on disk
    -- (ADR-0051 decision 6).
    scope_id      uuid        not null,
    -- One segment, lower-case letters, digits and '-', ≤64 characters:
    -- synveda_types::SkillName, which is the agentskills.io grammar and is
    -- deliberately stricter than the product's own prompt and pack names.
    -- This column's bound is the schema's half of that vocabulary; the type
    -- refuses the shapes a CHECK cannot describe.
    name          text        not null,
    -- The `description` from SKILL.md's frontmatter, denormalised so a
    -- listing does not read an object per row. The bytes remain the object's;
    -- this is a copy of what the parse found, rewritten on every author.
    description   text        not null,
    -- Never 'restricted' (decision 11), for migrations 0029 and 0030's
    -- reason: the only mechanism in the product that mints that tier is a
    -- classification proposal over *records*, and this feature ships no
    -- classify effect for authored assets — so a restricted skill would be a
    -- row nothing could have created and nothing could read back.
    --
    -- Declared per *skill* rather than per file, unlike a context pack's
    -- documents (decision 11): a client loads a bundle whole, so a bundle
    -- whose SKILL.md is `internal` and whose script is `confidential` is a
    -- bundle that cannot be half-loaded.
    sensitivity   text        not null,
    created_at    timestamptz not null default now(),
    created_by    uuid        not null,
    updated_at    timestamptz not null default now(),
    -- Who last authored into it. Deliberately *not* in any file's object
    -- address: a handover is not an edit (migrations 0029 and 0030's rule,
    -- unchanged).
    updated_by    uuid        not null,

    constraint skills_pk primary key (tenant_id, scope_id, name),
    constraint skills_tenant_fk foreign key (tenant_id) references tenants (id),
    constraint skills_name_check check (length(name) between 1 and 64),
    constraint skills_description_check check (length(description) between 1 and 1024),
    constraint skills_sensitivity_check
        check (sensitivity in ('public', 'internal', 'confidential'))
);

-- One file of one bundle: what an author uploads, what a reviewer reads, and
-- what an install writes to disk byte for byte.
create table skill_files (
    tenant_id   uuid        not null,
    scope_id    uuid        not null,
    skill_name  text        not null,
    -- Relative, '/'-separated, ≤4 segments, ≤128 characters:
    -- synveda_types::SkillFilePath, which refuses '..', absolute forms,
    -- backslashes, colons, reserved device stems, trailing dots and
    -- non-ASCII. With the skill's 64 that bounds the tree entry name
    -- `skill/path` at 193 — inside vedaflow_tree_entries.name (255) and
    -- inside vedaflow_refs.name (200), which is what a curator glob matches
    -- (ADR-0032).
    --
    -- The one rule that cannot live here is the case-fold collision:
    -- `scripts/Run.py` and `scripts/run.py` are two legal paths and two
    -- distinct objects, and it takes seeing *both* to know a filesystem would
    -- write one file. That is SkillBundle::validate's, at authoring.
    path        text        not null,
    -- The address of exactly these bytes. The FK is the point: a file whose
    -- content is not in the object store is unrepresentable, so "the bytes a
    -- proposal will bind are already stored" is a property of the schema
    -- rather than of a handler.
    object_hash bytea       not null,
    created_at  timestamptz not null default now(),
    created_by  uuid        not null,
    updated_at  timestamptz not null default now(),
    updated_by  uuid        not null,

    constraint skill_files_pk primary key (tenant_id, scope_id, skill_name, path),
    constraint skill_files_skill_fk
        foreign key (tenant_id, scope_id, skill_name)
        references skills (tenant_id, scope_id, name),
    constraint skill_files_object_fk
        foreign key (tenant_id, object_hash) references vedaflow_objects (tenant_id, hash),
    constraint skill_files_path_check check (length(path) between 1 and 128)
);

-- The listing and the resolve both read a whole bundle at once, in path
-- order, which is the order an install writes it in.
create index skill_files_by_skill on skill_files (tenant_id, scope_id, skill_name, path);

-- No hierarchy FK on either, on migration 0019's rule: recorded governance
-- must neither block a scope deletion nor be destroyed by one. A draft at a
-- deleted scope is on nobody's chain, so it resolves to nothing at the only
-- place it is read, and TEN-5's erasure disposes of it with the rest.

-- ── What a draft may and may not become ─────────────────────────────────────

-- A draft is content that changes; that is the whole point of it. Its
-- *identity* does not. Migration 0030's trigger, applied to both tables: a
-- moved scope_id would relocate authored material past the SkillWrite
-- decision that admitted it; a renamed skill or file would keep a published
-- entry pointing at a name nobody reviewed it under; and a rewritten
-- created_at/created_by would erase who started it.
--
-- The rename rule bites harder here than anywhere else in the product,
-- because the skill's name is inside its own SKILL.md: the open spec requires
-- the frontmatter `name` to match the directory. Renaming is therefore an
-- edit to the artefact AND a new row, and the surface refuses the pair rather
-- than letting a text edit silently mint a second skill (decision 5).
create function synveda_skill_transition() returns trigger
language plpgsql
as $$
begin
    if new.tenant_id  <> old.tenant_id
        or new.scope_id   <> old.scope_id
        or new.name       <> old.name
        or new.created_at <> old.created_at
        or new.created_by <> old.created_by
    then
        raise exception
            'skill %/% is identified by its tenant, scope and name (SKIL-1); '
            'renaming or moving one is a new skill, not an edit',
            old.scope_id, old.name;
    end if;
    return new;
end
$$;

create trigger skills_transition
    before update on skills
    for each row execute function synveda_skill_transition();

create function synveda_skill_file_transition() returns trigger
language plpgsql
as $$
begin
    if new.tenant_id  <> old.tenant_id
        or new.scope_id   <> old.scope_id
        or new.skill_name <> old.skill_name
        or new.path       <> old.path
        or new.created_at <> old.created_at
        or new.created_by <> old.created_by
    then
        raise exception
            'skill file %/%/% is identified by its tenant, scope, skill and path '
            '(SKIL-1); renaming or moving one is a new file, not an edit',
            old.scope_id, old.skill_name, old.path;
    end if;
    return new;
end
$$;

create trigger skill_files_transition
    before update on skill_files
    for each row execute function synveda_skill_file_transition();

-- ── RLS + least-privilege grants ────────────────────────────────────────────

-- Tenant-scoped tables ⇒ forced RLS + policy + least-privilege grants in the
-- same migration (ADR-0009's structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it).
--
-- DELETE is granted on `skill_files` and on nothing else, which is the one
-- place this registry differs from the prompt and pack ones (ADR-0049 and
-- ADR-0050 both refuse DELETE outright). A bundle is a *set* of files and
-- removing one is an ordinary edit to the artefact a client loads — a skill
-- that shipped `scripts/old.py` and no longer should must be authorable
-- without a rename. Removing the *skill* is still FLOW-7's rewind, and a
-- delete here cannot reach a published version: the tree names object
-- addresses, which are append-only, so what a channel serves is unaffected
-- by which draft rows exist.
grant select, insert, update on skills to synveda_app;
grant select, insert, update, delete on skill_files to synveda_app;

alter table skills enable row level security;
alter table skills force row level security;
alter table skill_files enable row level security;
alter table skill_files force row level security;

create policy skills_tenant_isolation on skills
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy skill_files_tenant_isolation on skill_files
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
