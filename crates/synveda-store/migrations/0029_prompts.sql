-- PRMT-1: the prompt registry's draft table (ADR-0049).
--
-- One table, and it holds the one thing VedaFlow cannot: **the authoring
-- state**. Everything else about a prompt is already expressible —
-- `vedaflow_objects` addresses its bytes, `vedaflow_proposals` reviews them,
-- `vedaflow_refs` publishes them, and the channel's first-parent line is its
-- version history (ADR-0049 decision 1).
--
-- Why the draft is not a channel (decision 2): ADR-0032 decision 2 kept
-- `staged` unwritten because "a set channel cannot express withdrawal", and
-- an author replacing a draft is exactly that withdrawal. A row can be
-- overwritten, which is what authoring is. So there is deliberately no
-- `prompt/staged` ref and nothing writes one.
--
-- The row is mutable *in its content* and immutable in its identity: the
-- trigger below refuses a moved scope, a renamed prompt, and a rewritten
-- creation. Renaming is not an edit — it is a different prompt, and a
-- published channel entry that silently followed one would serve content
-- under a name nobody reviewed it as.

create table prompts (
    tenant_id     uuid        not null,
    -- The scope that stands behind it. Part of the key, because the *same*
    -- name at a nearer scope is how a team overrides the org's version
    -- (decision 8) — two rows, two prompts, one name.
    scope_id      uuid        not null,
    -- Path-shaped, lower-case, ≤128 characters: synveda_types::PromptName.
    -- This column's bound is the schema's half of that vocabulary; the type
    -- refuses the shapes a CHECK cannot describe, and the unit tests pin the
    -- two together.
    name          text        not null,
    description   text        not null,
    template      text        not null,
    -- synveda_types::PromptVariable[]. Stored as the schema the author
    -- declared, sorted by name so the served order and the addressed order
    -- are the same list (the canonical form sorts too, and is idempotent).
    variables     jsonb       not null,
    -- Never 'restricted' (decision 5). The only mechanism in the product
    -- that mints that tier is a classification proposal over *records*
    -- (ADR-0038 decision 8), and PRMT-1 ships no classify effect for
    -- authored assets — so a restricted prompt would be a row nothing could
    -- have created and nothing could read back. The CHECK is where that
    -- becomes structural rather than a handler's good manners.
    sensitivity   text        not null,
    -- The address of exactly these bytes. The FK is the point: a draft whose
    -- content is not in the object store is unrepresentable, so "the bytes a
    -- proposal will bind are already stored" is a property of the schema.
    object_hash   bytea       not null,
    created_at    timestamptz not null default now(),
    created_by    uuid        not null,
    updated_at    timestamptz not null default now(),
    -- Who last authored it. Deliberately *not* in the object address: a
    -- handover is not an edit, and demoting a published prompt for one would
    -- be a surprise nobody could act on (ADR-0049, prompts module header).
    updated_by    uuid        not null,

    constraint prompts_pk primary key (tenant_id, scope_id, name),
    constraint prompts_tenant_fk foreign key (tenant_id) references tenants (id),
    constraint prompts_object_fk
        foreign key (tenant_id, object_hash) references vedaflow_objects (tenant_id, hash),
    constraint prompts_name_check check (length(name) between 1 and 128),
    constraint prompts_description_check check (length(description) between 0 and 512),
    constraint prompts_template_check check (length(template) between 1 and 32768),
    constraint prompts_variables_check check (jsonb_typeof(variables) = 'array'),
    constraint prompts_sensitivity_check
        check (sensitivity in ('public', 'internal', 'confidential'))
);

-- No hierarchy FK, on migration 0019's rule: recorded governance must
-- neither block a scope deletion nor be destroyed by one. A draft at a
-- deleted scope is on nobody's chain, so it resolves to nothing at the only
-- place it is read, and TEN-5's erasure disposes of it with the rest.

-- ── What a draft may and may not become ─────────────────────────────────────

-- A draft is content that changes; that is the whole point of it. Its
-- *identity* does not. A moved scope_id would relocate authored material
-- past the PromptWrite decision that admitted it; a renamed prompt would
-- keep a published entry pointing at a name nobody reviewed it under; and a
-- rewritten created_at/created_by would erase who started it.
create function synveda_prompt_transition() returns trigger
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
            'prompt %/% is identified by its tenant, scope and name (PRMT-1); '
            'renaming or moving one is a new prompt, not an edit',
            old.scope_id, old.name;
    end if;
    return new;
end
$$;

create trigger prompts_transition
    before update on prompts
    for each row execute function synveda_prompt_transition();

-- ── RLS + least-privilege grants ────────────────────────────────────────────

-- Tenant-scoped table ⇒ forced RLS + policy + least-privilege grants in the
-- same migration (ADR-0009's structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it).
--
-- No DELETE grant, and that is a decision rather than an omission (ADR-0049,
-- deferred): retracting a *published* prompt is FLOW-7's rewind, which works
-- for `prompt/published` the moment PromptRead exists, and replacing a draft
-- is an overwrite. Nothing in the product needs to remove the row, so
-- nothing is granted the statement that could.
grant select, insert, update on prompts to synveda_app;

alter table prompts enable row level security;
alter table prompts force row level security;

create policy prompts_tenant_isolation on prompts
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
