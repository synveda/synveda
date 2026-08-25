-- CPR-32 / ADR-0091: one typed approval lifecycle across artifact families.
--
-- The common proposal row gains a bounded immutable artifact index. There is
-- deliberately no backfill: this is the pre-1.0 epoch-2 hard cut, so a database
-- carrying pre-CPR-32 proposals is refused here and must be reset. A fresh
-- chain has no proposal rows when this migration runs.

create function synveda_valid_artifact_references(candidate jsonb) returns boolean
language sql
immutable
as $$
    select case
      when jsonb_typeof(candidate) <> 'array' then false
      else jsonb_array_length(candidate) between 1 and 200
       and not exists (
            select 1
            from jsonb_array_elements(candidate) as entry(value)
            where jsonb_typeof(value) <> 'object'
               or not (value ?& array['family', 'artifact_id', 'operation', 'version'])
               or value - array['family', 'artifact_id', 'operation', 'version',
                                'expected_revision']::text[] <> '{}'::jsonb
               or jsonb_typeof(value -> 'family') <> 'string'
               or value ->> 'family' not in (
                    'knowledge', 'skill', 'tool_server', 'tool_binding',
                    'configuration', 'policy_relaxation', 'okf_import',
                    'prompt', 'context_pack', 'memory'
               )
               or jsonb_typeof(value -> 'artifact_id') <> 'string'
               or char_length(value ->> 'artifact_id') not between 1 and 1024
               or value ->> 'artifact_id' ~ '[[:cntrl:]]'
               or jsonb_typeof(value -> 'operation') <> 'string'
               or char_length(value ->> 'operation') not between 1 and 64
               or value ->> 'operation' ~ '[[:cntrl:]]'
               or jsonb_typeof(value -> 'version') <> 'string'
               or char_length(value ->> 'version') not between 1 and 512
               or value ->> 'version' ~ '[[:cntrl:]]'
               or (
                    value ? 'expected_revision'
                    and (
                        jsonb_typeof(value -> 'expected_revision') <> 'string'
                        or char_length(value ->> 'expected_revision') not between 1 and 512
                        or value ->> 'expected_revision' ~ '[[:cntrl:]]'
                    )
               )
       )
       and jsonb_array_length(candidate) = (
            select count(distinct value)
            from jsonb_array_elements(candidate) as entry(value)
       )
      end
$$;

alter table vedaflow_proposals
    add column artifact_references jsonb not null;

alter table vedaflow_proposals
    add constraint vedaflow_proposals_artifact_references_check
    check (synveda_valid_artifact_references(artifact_references));

create index vedaflow_proposals_artifact_references_idx
    on vedaflow_proposals using gin (artifact_references jsonb_path_ops);

-- The lifecycle row remains immutable except for its one closure transition;
-- replace the guard so privileged/out-of-band writes cannot rewrite the typed
-- artifact address under approvals that bound the old one.
create or replace function synveda_vedaflow_proposal_transition() returns trigger
language plpgsql
as $$
begin
    if old.state <> 'open' then
        raise exception 'proposal % is already %; closed proposals are history (FLOW-3)',
            old.id, old.state;
    end if;
    if new.state = 'open' then
        raise exception 'proposal % update changed nothing about its state (FLOW-3)', old.id;
    end if;
    if new.tenant_id             <> old.tenant_id
        or new.id                    <> old.id
        or new.target_scope_id       <> old.target_scope_id
        or new.source_scope_id       <> old.source_scope_id
        or new.asset_kind            <> old.asset_kind
        or new.target_channel        <> old.target_channel
        or new.commit_hash           <> old.commit_hash
        or new.sensitivity           <> old.sensitivity
        or new.title                 <> old.title
        or new.proposer_id           <> old.proposer_id
        or new.proposer_subject      <> old.proposer_subject
        or new.artifact_references   <> old.artifact_references
        or new.created_at            <> old.created_at
        or new.evidence is distinct from old.evidence
    then
        raise exception 'proposal % is immutable except for its closure (FLOW-3)', old.id;
    end if;
    return new;
end
$$;
