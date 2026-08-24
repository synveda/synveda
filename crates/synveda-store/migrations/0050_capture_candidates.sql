-- CPR-18: session evidence becomes reviewable capture candidates (ADR-0083).
--
-- Extraction is not publication. A batch freezes the exact ordered event set
-- it read; candidates retain those source links and can become Knowledge only
-- through the CPR-16 command/VedaFlow seam. This migration also removes the
-- PGMQ signal queue whose retired worker wrote active records directly.

-- Composite targets used to prove that every link stays inside one tenant,
-- session and batch. The existing keys prove the smaller facts separately;
-- these prove the joined facts without relying on service discipline.
create unique index session_events_tenant_session_id_unique
    on session_events (tenant_id, session_id, id);

-- 0041 exposes the stronger `(tenant, id, scope)` workspace fact because
-- most consumers need its scope too. Capture keeps the scope that belongs to
-- the session (a project's when present), so its workspace FK needs the
-- smaller tenant-safe target.
create unique index workspaces_tenant_id_unique
    on workspaces (tenant_id, id);

create table capture_batches (
    id                  uuid        not null,
    tenant_id           uuid        not null,
    session_id          uuid        not null,
    scope_id            uuid        not null,
    workspace_id        uuid        not null,
    project_id          uuid,
    principal_id        text        not null,
    input_hash          text        not null,
    event_count         integer     not null,
    state               text        not null default 'pending',
    extractor_method    text,
    model_version       text,
    attempts            integer     not null default 0,
    lease_owner         text,
    lease_expires_at    timestamptz,
    candidate_count     integer     not null default 0,
    error_code          text,
    created_at          timestamptz not null default now(),
    started_at          timestamptz,
    completed_at        timestamptz,
    updated_at          timestamptz not null default now(),

    constraint capture_batches_pk primary key (id),
    constraint capture_batches_tenant_id_unique unique (tenant_id, id),
    constraint capture_batches_tenant_session_id_unique
        unique (tenant_id, session_id, id),
    constraint capture_batches_snapshot_unique
        unique (tenant_id, session_id, input_hash),
    constraint capture_batches_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint capture_batches_session_fk
        foreign key (tenant_id, session_id) references sessions (tenant_id, id),
    constraint capture_batches_scope_fk
        foreign key (tenant_id, scope_id) references scopes (tenant_id, id),
    constraint capture_batches_workspace_fk
        foreign key (tenant_id, workspace_id) references workspaces (tenant_id, id),
    constraint capture_batches_project_fk
        foreign key (tenant_id, project_id) references projects (tenant_id, id),
    constraint capture_batches_principal_check
        check (btrim(principal_id) <> '' and char_length(principal_id) <= 255),
    constraint capture_batches_hash_check check (input_hash ~ '^[0-9a-f]{64}$'),
    constraint capture_batches_event_count_check check (event_count >= 0),
    constraint capture_batches_state_check
        check (state in ('pending', 'running', 'completed', 'failed')),
    constraint capture_batches_method_check
        check (extractor_method is null
               or (btrim(extractor_method) <> '' and char_length(extractor_method) <= 100)),
    constraint capture_batches_model_check
        check (model_version is null
               or (btrim(model_version) <> '' and char_length(model_version) <= 512)),
    constraint capture_batches_attempts_check check (attempts between 0 and 5),
    constraint capture_batches_lease_check
        check ((lease_owner is null) = (lease_expires_at is null)
               and (lease_owner is null
                    or (btrim(lease_owner) <> '' and char_length(lease_owner) <= 255))),
    constraint capture_batches_candidate_count_check check (candidate_count >= 0),
    constraint capture_batches_error_check
        check (error_code is null
               or (btrim(error_code) <> '' and char_length(error_code) <= 100)),
    constraint capture_batches_state_shape_check
        check (
            (state = 'pending' and completed_at is null
             and lease_owner is null and lease_expires_at is null)
            or
            (state = 'running' and started_at is not null
             and completed_at is null and lease_owner is not null)
            or
            (state in ('completed', 'failed') and started_at is not null
             and completed_at is not null
             and lease_owner is null and lease_expires_at is null)
        ),
    constraint capture_batches_time_check
        check (updated_at >= created_at
               and (started_at is null or started_at >= created_at)
               and (completed_at is null or completed_at >= started_at))
);

-- The separate foreign keys prove existence; this trigger proves the frozen
-- batch copied the session's exact anchor and principal, including the
-- nullable project comparison that a MATCH SIMPLE composite FK cannot hold.
create function synveda_capture_batch_session_identity() returns trigger
language plpgsql
as $$
begin
    if not exists (
        select 1 from sessions session
        where session.tenant_id = new.tenant_id
          and session.id = new.session_id
          and session.scope_id = new.scope_id
          and session.workspace_id = new.workspace_id
          and session.project_id is not distinct from new.project_id
          and session.principal_id = new.principal_id
    ) then
        raise exception 'capture batch identity must match its session'
            using errcode = '23514';
    end if;
    return new;
end
$$;

create trigger capture_batches_session_identity
    before insert on capture_batches
    for each row execute function synveda_capture_batch_session_identity();

create index capture_batches_pending
    on capture_batches (tenant_id, created_at, id)
    where state = 'pending';
create index capture_batches_by_session
    on capture_batches (tenant_id, session_id, created_at desc, id desc);
create index capture_batches_by_scope
    on capture_batches (tenant_id, scope_id, created_at desc, id desc);
create index capture_batches_by_project
    on capture_batches (tenant_id, project_id, created_at desc, id desc)
    where project_id is not null;

create table capture_batch_events (
    tenant_id      uuid        not null,
    batch_id       uuid        not null,
    session_id     uuid        not null,
    event_id       uuid        not null,
    ordinal        integer     not null,
    linked_at      timestamptz not null default now(),

    constraint capture_batch_events_pk primary key (tenant_id, batch_id, event_id),
    constraint capture_batch_events_ordinal_unique
        unique (tenant_id, batch_id, ordinal),
    constraint capture_batch_events_batch_event_unique
        unique (tenant_id, batch_id, event_id),
    constraint capture_batch_events_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint capture_batch_events_batch_fk
        foreign key (tenant_id, session_id, batch_id)
        references capture_batches (tenant_id, session_id, id),
    constraint capture_batch_events_event_fk
        foreign key (tenant_id, session_id, event_id)
        references session_events (tenant_id, session_id, id),
    constraint capture_batch_events_ordinal_check check (ordinal >= 1)
);

create table capture_candidates (
    id                            uuid        not null,
    tenant_id                     uuid        not null,
    batch_id                      uuid        not null,
    session_id                    uuid        not null,
    ordinal                       integer     not null,
    proposed_scope_id             uuid        not null,
    proposed_project_id           uuid,
    proposed_owner_principal_id   text,
    knowledge_type                text        not null,
    origin                        text        not null,
    title                         text        not null,
    body_markdown                 text        not null,
    summary                       text        not null,
    tags                          text[]      not null default '{}'::text[],
    sensitivity                   text        not null,
    confidence_permille           integer     not null,
    valid_from                    timestamptz not null,
    valid_to                      timestamptz,
    stale_after                   timestamptz,
    verification_metadata         jsonb       not null default '{}'::jsonb,
    metadata                      jsonb       not null default '{}'::jsonb,
    content_hash                  text        not null,
    state                         text        not null default 'pending',
    resulting_change_id           uuid,
    resulting_outcome             text,
    resulting_knowledge_item_id   uuid,
    resulting_revision_id         uuid,
    decided_by                    text,
    decision_reason               text,
    decided_at                    timestamptz,
    content_erased                boolean     not null default false,
    created_at                    timestamptz not null default now(),

    constraint capture_candidates_pk primary key (id),
    constraint capture_candidates_tenant_id_unique unique (tenant_id, id),
    constraint capture_candidates_batch_id_unique unique (tenant_id, batch_id, id),
    constraint capture_candidates_ordinal_unique unique (tenant_id, batch_id, ordinal),
    constraint capture_candidates_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint capture_candidates_batch_fk
        foreign key (tenant_id, session_id, batch_id)
        references capture_batches (tenant_id, session_id, id),
    constraint capture_candidates_scope_fk
        foreign key (tenant_id, proposed_scope_id) references scopes (tenant_id, id),
    constraint capture_candidates_project_fk
        foreign key (tenant_id, proposed_project_id) references projects (tenant_id, id),
    constraint capture_candidates_owner_check
        check (proposed_owner_principal_id is null
               or (btrim(proposed_owner_principal_id) <> ''
                   and char_length(proposed_owner_principal_id) <= 255)),
    constraint capture_candidates_type_check
        check (knowledge_type in (
            'fact', 'decision', 'preference', 'procedure', 'entity',
            'episode', 'convention', 'warning', 'reference'
        )),
    constraint capture_candidates_origin_check
        check (origin in ('observed', 'asserted', 'authored', 'imported')),
    constraint capture_candidates_title_check
        check ((not content_erased and btrim(title) <> '' and char_length(title) <= 300)
               or (content_erased and title = '')),
    constraint capture_candidates_body_check
        check ((not content_erased and btrim(body_markdown) <> ''
               and octet_length(body_markdown) <= 131072)
               or (content_erased and body_markdown = '')),
    constraint capture_candidates_summary_check
        check ((not content_erased and btrim(summary) <> ''
               and char_length(summary) <= 2000)
               or (content_erased and summary = '')),
    constraint capture_candidates_tags_check
        check ((not content_erased and synveda_knowledge_tags_canonical(tags))
               or (content_erased and tags = '{}'::text[])),
    constraint capture_candidates_sensitivity_check
        check (sensitivity in ('public', 'internal', 'confidential', 'restricted')),
    constraint capture_candidates_confidence_check
        check (confidence_permille between 0 and 1000),
    constraint capture_candidates_valid_time_check
        check (valid_to is null or valid_to > valid_from),
    constraint capture_candidates_stale_time_check
        check (stale_after is null
               or (stale_after > valid_from and (valid_to is null or stale_after <= valid_to))),
    constraint capture_candidates_verification_check
        check (jsonb_typeof(verification_metadata) = 'object'
               and octet_length(verification_metadata::text) <= 16384),
    constraint capture_candidates_metadata_check
        check (jsonb_typeof(metadata) = 'object'
               and octet_length(metadata::text) <= 16384),
    constraint capture_candidates_hash_check check (content_hash ~ '^[0-9a-f]{64}$'),
    constraint capture_candidates_state_check
        check (state in (
            'pending', 'accepted', 'edited_and_accepted', 'merged',
            'replaced', 'dismissed', 'failed'
        )),
    constraint capture_candidates_outcome_check
        check (resulting_outcome is null
               or resulting_outcome in ('applied', 'pending_review', 'rejected')),
    constraint capture_candidates_decider_check
        check (decided_by is null
               or (btrim(decided_by) <> '' and char_length(decided_by) <= 255)),
    constraint capture_candidates_reason_check
        check (decision_reason is null
               or (btrim(decision_reason) <> '' and char_length(decision_reason) <= 1000)),
    constraint capture_candidates_decision_shape_check
        check ((state = 'pending') =
               (decided_by is null and decided_at is null
                and resulting_change_id is null and resulting_outcome is null
                and resulting_knowledge_item_id is null and resulting_revision_id is null)),
    constraint capture_candidates_erasure_metadata_check
        check (not content_erased
               or (verification_metadata = '{}'::jsonb and metadata = '{}'::jsonb))
);

create index capture_candidates_pending
    on capture_candidates (tenant_id, proposed_scope_id, created_at, id)
    where state = 'pending';
create index capture_candidates_by_session
    on capture_candidates (tenant_id, session_id, created_at desc, id desc);
create index capture_candidates_by_batch
    on capture_candidates (tenant_id, batch_id, ordinal);
create index capture_candidates_by_result
    on capture_candidates (tenant_id, resulting_knowledge_item_id)
    where resulting_knowledge_item_id is not null;
create index capture_candidates_by_hash
    on capture_candidates (tenant_id, content_hash);

create table capture_candidate_events (
    tenant_id       uuid        not null,
    candidate_id    uuid        not null,
    batch_id        uuid        not null,
    event_id        uuid        not null,
    ordinal         integer     not null,
    linked_at       timestamptz not null default now(),

    constraint capture_candidate_events_pk
        primary key (tenant_id, candidate_id, event_id),
    constraint capture_candidate_events_ordinal_unique
        unique (tenant_id, candidate_id, ordinal),
    constraint capture_candidate_events_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint capture_candidate_events_candidate_fk
        foreign key (tenant_id, batch_id, candidate_id)
        references capture_candidates (tenant_id, batch_id, id),
    constraint capture_candidate_events_frozen_event_fk
        foreign key (tenant_id, batch_id, event_id)
        references capture_batch_events (tenant_id, batch_id, event_id),
    constraint capture_candidate_events_ordinal_check check (ordinal >= 1)
);

create table capture_candidate_matches (
    tenant_id                    uuid        not null,
    candidate_id                 uuid        not null,
    knowledge_item_id            uuid        not null,
    knowledge_revision_id        uuid        not null,
    match_kind                   text        not null,
    similarity_permille          integer     not null,
    reason_code                  text        not null,
    created_at                   timestamptz not null default now(),

    constraint capture_candidate_matches_pk
        primary key (tenant_id, candidate_id, knowledge_item_id, match_kind),
    constraint capture_candidate_matches_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint capture_candidate_matches_candidate_fk
        foreign key (tenant_id, candidate_id)
        references capture_candidates (tenant_id, id),
    constraint capture_candidate_matches_knowledge_fk
        foreign key (tenant_id, knowledge_item_id, knowledge_revision_id)
        references knowledge_revisions (tenant_id, knowledge_item_id, id)
        on delete cascade,
    constraint capture_candidate_matches_kind_check
        check (match_kind in ('duplicate', 'conflict', 'possible_supersession')),
    constraint capture_candidate_matches_similarity_check
        check (similarity_permille between 0 and 1000),
    constraint capture_candidate_matches_reason_check
        check (btrim(reason_code) <> '' and char_length(reason_code) <= 100)
);

create index capture_candidate_matches_by_knowledge
    on capture_candidate_matches (tenant_id, knowledge_item_id, candidate_id);

create table capture_candidate_decisions (
    id                            uuid        not null,
    tenant_id                     uuid        not null,
    candidate_id                  uuid        not null,
    action                        text        not null,
    state                         text        not null default 'running',
    actor_subject                 text        not null,
    idempotency_key               text        not null,
    request_hash                  text        not null,
    payload                       jsonb,
    payload_hash                  text        not null,
    resulting_change_id           uuid,
    resulting_outcome             text,
    resulting_knowledge_item_id   uuid,
    resulting_revision_id         uuid,
    error_code                    text,
    created_at                    timestamptz not null default now(),
    completed_at                  timestamptz,

    constraint capture_candidate_decisions_pk primary key (id),
    constraint capture_candidate_decisions_tenant_id_unique unique (tenant_id, id),
    constraint capture_candidate_decisions_candidate_unique unique (tenant_id, candidate_id),
    constraint capture_candidate_decisions_key_unique
        unique (tenant_id, actor_subject, idempotency_key),
    constraint capture_candidate_decisions_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint capture_candidate_decisions_candidate_fk
        foreign key (tenant_id, candidate_id) references capture_candidates (tenant_id, id),
    constraint capture_candidate_decisions_action_check
        check (action in ('accept', 'edit_and_accept', 'merge', 'replace', 'dismiss')),
    constraint capture_candidate_decisions_state_check
        check (state in ('running', 'succeeded', 'failed')),
    constraint capture_candidate_decisions_actor_check
        check (btrim(actor_subject) <> '' and char_length(actor_subject) <= 255),
    constraint capture_candidate_decisions_key_check
        check (btrim(idempotency_key) <> '' and char_length(idempotency_key) <= 255),
    constraint capture_candidate_decisions_request_hash_check
        check (request_hash ~ '^[0-9a-f]{64}$'),
    constraint capture_candidate_decisions_payload_check
        check (payload is null
               or (jsonb_typeof(payload) = 'object' and octet_length(payload::text) <= 147456)),
    constraint capture_candidate_decisions_payload_hash_check
        check (payload_hash ~ '^[0-9a-f]{64}$'),
    constraint capture_candidate_decisions_outcome_check
        check (resulting_outcome is null
               or resulting_outcome in ('applied', 'pending_review', 'rejected')),
    constraint capture_candidate_decisions_error_check
        check (error_code is null
               or (btrim(error_code) <> '' and char_length(error_code) <= 100)),
    constraint capture_candidate_decisions_state_shape_check
        check ((state = 'running' and completed_at is null
                and resulting_change_id is null and resulting_outcome is null
                and error_code is null)
               or
               (state = 'succeeded' and completed_at is not null and error_code is null)
               or
               (state = 'failed' and completed_at is not null and error_code is not null))
);

-- Batch input and candidate content are immutable; only the explicit job and
-- decision result columns move. The triggers protect owner connections as
-- well as the column-level application grants below.
create function synveda_capture_batch_transition() returns trigger
language plpgsql
as $$
begin
    if new.id <> old.id or new.tenant_id <> old.tenant_id
       or new.session_id <> old.session_id or new.scope_id <> old.scope_id
       or new.workspace_id <> old.workspace_id
       or new.project_id is distinct from old.project_id
       or new.principal_id <> old.principal_id or new.input_hash <> old.input_hash
       or new.event_count <> old.event_count or new.created_at <> old.created_at then
        raise exception 'capture batch evidence is immutable';
    end if;
    if not (
        (old.state = 'pending' and new.state = 'running')
        or (old.state = 'running' and new.state in ('pending', 'completed', 'failed'))
        or (old.state = new.state)
    ) then
        raise exception 'invalid capture batch transition: % -> %', old.state, new.state;
    end if;
    return new;
end
$$;

create trigger capture_batches_transition
    before update on capture_batches
    for each row execute function synveda_capture_batch_transition();

create function synveda_capture_candidate_transition() returns trigger
language plpgsql
as $$
begin
    if new.id <> old.id or new.tenant_id <> old.tenant_id
       or new.batch_id <> old.batch_id or new.session_id <> old.session_id
       or new.ordinal <> old.ordinal or new.proposed_scope_id <> old.proposed_scope_id
       or new.proposed_project_id is distinct from old.proposed_project_id
       or new.proposed_owner_principal_id is distinct from old.proposed_owner_principal_id
       or new.knowledge_type <> old.knowledge_type or new.origin <> old.origin
       or new.content_hash <> old.content_hash or new.created_at <> old.created_at then
        raise exception 'capture candidate identity and proposal are immutable';
    end if;
    if current_setting('synveda.knowledge_erasure', true) = 'on' then
        if not new.content_erased or old.content_erased then
            raise exception 'capture candidate erasure is one-way';
        end if;
        return new;
    end if;
    if new.title <> old.title or new.body_markdown <> old.body_markdown
       or new.summary <> old.summary or new.tags <> old.tags
       or new.sensitivity <> old.sensitivity
       or new.confidence_permille <> old.confidence_permille
       or new.valid_from <> old.valid_from or new.valid_to is distinct from old.valid_to
       or new.stale_after is distinct from old.stale_after
       or new.verification_metadata <> old.verification_metadata
       or new.metadata <> old.metadata or new.content_erased <> old.content_erased then
        raise exception 'capture candidate content is immutable';
    end if;
    if old.state <> 'pending' and new is distinct from old then
        raise exception 'capture candidate decision is terminal';
    end if;
    return new;
end
$$;

create trigger capture_candidates_transition
    before update on capture_candidates
    for each row execute function synveda_capture_candidate_transition();

create function synveda_capture_decision_transition() returns trigger
language plpgsql
as $$
begin
    if new.id <> old.id or new.tenant_id <> old.tenant_id
       or new.candidate_id <> old.candidate_id or new.action <> old.action
       or new.actor_subject <> old.actor_subject
       or new.idempotency_key <> old.idempotency_key
       or new.request_hash <> old.request_hash
       or new.payload_hash <> old.payload_hash or new.created_at <> old.created_at then
        raise exception 'capture decision intent is immutable';
    end if;
    if current_setting('synveda.knowledge_erasure', true) = 'on' then
        if new.payload is not null then
            raise exception 'capture decision erasure may only clear payload';
        end if;
        return new;
    end if;
    if new.payload is distinct from old.payload then
        raise exception 'capture decision payload is immutable';
    end if;
    if old.state <> 'running' and new is distinct from old then
        raise exception 'capture decision result is terminal';
    end if;
    return new;
end
$$;

create trigger capture_candidate_decisions_transition
    before update on capture_candidate_decisions
    for each row execute function synveda_capture_decision_transition();

create function synveda_capture_append_only() returns trigger
language plpgsql
as $$
begin
    if tg_op = 'DELETE'
       and (current_setting('synveda.knowledge_erasure', true) = 'on'
            or current_setting('synveda.retention_purge', true) = 'on') then
        -- This is a statement trigger: returning NULL allows the statement
        -- to proceed and avoids reading an unassigned OLD record.
        return null;
    end if;
    raise exception '% is append-only (CPR-18, ADR-0083)', tg_table_name;
end
$$;

create trigger capture_batch_events_append_only
    before update or delete or truncate on capture_batch_events
    for each statement execute function synveda_capture_append_only();
create trigger capture_candidate_events_append_only
    before update or delete or truncate on capture_candidate_events
    for each statement execute function synveda_capture_append_only();
create trigger capture_candidate_matches_append_only
    before update or delete or truncate on capture_candidate_matches
    for each statement execute function synveda_capture_append_only();

-- When the governed erasure primitive removes a resulting Knowledge item,
-- scrub the review copy and its decision payload before the item/revisions
-- leave. This trigger runs inside that security-definer transaction and only
-- while its transaction-local erasure flag is on.
create function synveda_capture_scrub_for_knowledge() returns trigger
language plpgsql
as $$
begin
    if current_setting('synveda.knowledge_erasure', true) <> 'on' then
        return old;
    end if;
    update capture_candidate_decisions decision
       set payload = null
      from capture_candidates candidate
     where candidate.tenant_id = old.tenant_id
       and candidate.resulting_knowledge_item_id = old.id
       and decision.tenant_id = candidate.tenant_id
       and decision.candidate_id = candidate.id
       and decision.payload is not null;
    update capture_candidates
       set title = '', body_markdown = '', summary = '', tags = '{}'::text[],
           verification_metadata = '{}'::jsonb, metadata = '{}'::jsonb,
           content_erased = true
     where tenant_id = old.tenant_id
       and resulting_knowledge_item_id = old.id
       and not content_erased;
    return old;
end
$$;

create trigger knowledge_items_scrub_capture
    before delete on knowledge_items
    for each row execute function synveda_capture_scrub_for_knowledge();

-- Least privilege: evidence/link tables have no update path; job/result rows
-- expose only their explicit transition columns.
grant select, insert on capture_batches to synveda_app;
grant update (state, extractor_method, model_version, attempts, lease_owner,
              lease_expires_at, candidate_count, error_code, started_at,
              completed_at, updated_at) on capture_batches to synveda_app;
grant select, insert on capture_batch_events to synveda_app;
grant select, insert on capture_candidates to synveda_app;
grant update (state, resulting_change_id, resulting_outcome,
              resulting_knowledge_item_id, resulting_revision_id, decided_by,
              decision_reason, decided_at, title, body_markdown, summary, tags,
              verification_metadata, metadata, content_erased)
    on capture_candidates to synveda_app;
grant select, insert on capture_candidate_events to synveda_app;
grant select, insert, delete on capture_candidate_matches to synveda_app;
grant select, insert on capture_candidate_decisions to synveda_app;
grant update (state, payload, resulting_change_id, resulting_outcome,
              resulting_knowledge_item_id, resulting_revision_id, error_code,
              completed_at) on capture_candidate_decisions to synveda_app;

alter table capture_batches enable row level security;
alter table capture_batches force row level security;
alter table capture_batch_events enable row level security;
alter table capture_batch_events force row level security;
alter table capture_candidates enable row level security;
alter table capture_candidates force row level security;
alter table capture_candidate_events enable row level security;
alter table capture_candidate_events force row level security;
alter table capture_candidate_matches enable row level security;
alter table capture_candidate_matches force row level security;
alter table capture_candidate_decisions enable row level security;
alter table capture_candidate_decisions force row level security;

create policy capture_batches_tenant_isolation on capture_batches
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy capture_batch_events_tenant_isolation on capture_batch_events
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy capture_candidates_tenant_isolation on capture_candidates
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy capture_candidate_events_tenant_isolation on capture_candidate_events
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy capture_candidate_matches_tenant_isolation on capture_candidate_matches
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy capture_candidate_decisions_tenant_isolation on capture_candidate_decisions
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

-- Nothing consumes per-event work signals after this cut. The queue carries
-- no domain data and no pre-epoch data is migrated.
select pgmq.drop_queue('session_events');
