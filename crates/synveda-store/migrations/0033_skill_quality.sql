-- SKIL-3: the reviewer's checklist, and the registry's score cache
-- (ADR-0053).
--
-- One table and two columns, and the split between them is the whole
-- design. A skill's quality score has two halves with **opposite
-- durability** (decision 1):
--
--   * the automated rubric is a pure function of (file bytes, rubric
--     version), recomputable anywhere the bytes are, and therefore stored
--     nowhere a decision reads — ADR-0052 decision 6, inherited whole;
--   * the reviewer's checklist is a person's judgement on a particular
--     afternoon, which nothing can recompute, so it is the thing that
--     needs a table.
--
-- The feature text says "stored on the version". That is true of exactly
-- one of the two, and this migration is that one.

-- ── The checklist ───────────────────────────────────────────────────────────

-- One reviewer's answers about one bundle.
--
-- **The key is a digest of the bundle's bytes**, and that is decision 4 and
-- the reason this table needs no invalidation logic at all. A checklist
-- keyed by proposal id, or by skill name, would survive an edit beneath it:
-- a reviewer answers "yes, somebody ran it", the author pushes a new
-- scripts/run.py, and the answer sits there describing a bundle that no
-- longer exists. That is precisely the laundering ADR-0032 decision 6's
-- "approvals bind bytes" exists to prevent, arriving in the one review
-- artefact that had no address check of its own.
--
-- Keyed by the digest, an edited bundle is simply a bundle for which no
-- checklist is found. Nothing is invalidated, nothing goes stale, and the
-- old answers remain attached to the bytes they were true of — which is
-- what makes the trail readable backwards.
create table skill_reviews (
    tenant_id     uuid        not null,
    -- The scope whose published channel this bundle is headed for. Part of
    -- the key because the same bytes proposed at two scopes are two
    -- reviews: "does it belong at this scope" is one of the questions.
    scope_id      uuid        not null,
    -- The bundle's name, denormalised so a listing and an audit query can
    -- find a skill's reviews without resolving a digest first. Not a FK:
    -- migration 0019's rule (recorded governance must neither block a
    -- deletion nor be destroyed by one) applies with more force here than
    -- to skill_files, because a review outlives the draft it was about.
    skill_name    text        not null,
    -- BLAKE3 over the domain-separated, path-sorted (member name, object
    -- address) pairs — a tree hash by another name.
    --
    -- Over *addresses* rather than raw file bytes, deliberately: ADR-0051
    -- decision 2 put the governed context (scope, skill, sensitivity, path)
    -- inside each object's address, so reclassifying a bundle from
    -- 'internal' to 'confidential' re-keys its checklist. That is correct —
    -- a reviewer who signed off on an internal skill did not sign off on a
    -- confidential one.
    bundle_digest bytea       not null,
    -- {item: verdict}, with the wire names synveda_types::ChecklistItem and
    -- ChecklistVerdict serialise to. A jsonb object rather than a row per
    -- answer, because a checklist is read and written whole and nothing
    -- queries one item across bundles.
    answers       jsonb       not null,
    -- Whatever the reviewer wanted to say. Passed through MEM-2's scanner
    -- before it is written, like every other author-supplied prose in the
    -- product.
    note          text,
    -- Which rubric was rendered beside the reviewer when they answered.
    -- Not used to invalidate anything — the checklist is about the bundle,
    -- not about the rubric — but an auditor asking "what did they see"
    -- needs it, and it costs four bytes.
    rubric_version integer    not null,
    reviewed_at   timestamptz not null default now(),
    reviewed_by   uuid        not null,

    constraint skill_reviews_pk primary key (tenant_id, scope_id, skill_name, bundle_digest),
    constraint skill_reviews_tenant_fk foreign key (tenant_id) references tenants (id),
    constraint skill_reviews_digest_check check (octet_length(bundle_digest) = 32),
    constraint skill_reviews_name_check check (length(skill_name) between 1 and 64),
    constraint skill_reviews_note_check check (note is null or length(note) between 1 and 2000),
    -- An answers object that is not an object is a row nothing can read
    -- back. The *vocabulary* is the type's to enforce (the role_bindings
    -- discipline, ADR-0015); this is the shape only.
    constraint skill_reviews_answers_check
        check (jsonb_typeof(answers) = 'object' and answers <> '{}'::jsonb)
);

-- An auditor, and CNSL-1's inbox, both ask "what has been reviewed at this
-- scope lately" rather than "what was said about this digest".
create index skill_reviews_by_scope on skill_reviews (tenant_id, scope_id, reviewed_at desc);

-- ── The registry's score cache ──────────────────────────────────────────────

-- The automated half, denormalised onto the draft row — and **a cache,
-- never a truth** (decision 3). This is ADR-0052 reversal trigger (e)
-- arriving exactly as it was written: "if SKIL-3 brings that table anyway,
-- a cached scan may join it — as a cache, keyed by ruleset version, never
-- as the truth."
--
-- It exists because a registry listing at a scope with forty skills would
-- otherwise read every object of every bundle to draw one column.
--
-- Two rules keep it honest, and they are the whole of what makes a cache
-- safe here:
--
--   1. **No gate reads these columns.** The publish seam recomputes from
--      the bytes it is about to publish, always. A cache a decision reads
--      is not a cache.
--   2. **A row whose rubric_version is not the compiled-in one renders as
--      stale**, not as current. The rubric moves; a number that did not say
--      which table produced it could not be compared with one taken at
--      review time (ADR-0052 force 4).
--
-- Nullable because every skill authored before this migration has no score
-- and must not be given a fabricated one: `null` reads as "not scored yet",
-- which is true, and re-authoring fills it in.
alter table skills add column quality_score  smallint;
alter table skills add column rubric_version integer;

alter table skills add constraint skills_quality_score_check
    check (quality_score is null or quality_score between 0 and 100);
-- The pair is meaningless apart: a score without the rubric that produced
-- it cannot be told stale from current, and a version without a score names
-- nothing.
alter table skills add constraint skills_quality_pair_check
    check ((quality_score is null) = (rubric_version is null));

-- ── RLS + least-privilege grants ────────────────────────────────────────────

-- Tenant-scoped table ⇒ forced RLS + policy + least-privilege grants in the
-- same migration (ADR-0009's structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it).
--
-- **No DELETE**, and the contrast with migration 0031 is the argument.
-- skill_files has one because a bundle is authored whole and a file the
-- author dropped must not be published back onto a laptop. A checklist is a
-- record that a person judged something on a day; a product that can erase
-- one is a product whose review trail can be edited. UPDATE is granted
-- because re-answering is an ordinary act — and every submission chains
-- skill.checklist.recorded, so the row is last-writer-wins while the audit
-- chain is every writer.
--
-- The two new columns on `skills` need no grant of their own: a column
-- inherits its table's RLS policy, its forced flag and its grants, which is
-- why every config column since ADR-0025 has arrived the same way.
grant select, insert, update on skill_reviews to synveda_app;

alter table skill_reviews enable row level security;
alter table skill_reviews force row level security;

create policy skill_reviews_tenant_isolation on skill_reviews
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
