-- CPR-37 / ADR-0096: durable conflict evidence, transitional Knowledge and
-- governed resolution. FreshnessPolicy remains a projection of immutable
-- governed Configuration, so this migration deliberately creates no second
-- settings table.

alter table knowledge_items
    drop constraint knowledge_items_lifecycle_check;
alter table knowledge_items
    add constraint knowledge_items_lifecycle_check
    check (lifecycle_state in (
        'active', 'stale', 'transitional', 'superseded', 'archived',
        'erasure_pending', 'erased'
    ));

alter table knowledge_items_history
    drop constraint knowledge_items_history_lifecycle_check;
alter table knowledge_items_history
    add constraint knowledge_items_history_lifecycle_check
    check (lifecycle_state in (
        'active', 'stale', 'transitional', 'superseded', 'archived',
        'erasure_pending', 'erased'
    ));

alter table knowledge_changes
    drop constraint knowledge_changes_command_check;
alter table knowledge_changes
    add constraint knowledge_changes_command_check
    check (command_kind in (
        'create', 'edit', 'verify', 'supersede', 'merge',
        'archive', 'restore', 'forget', 'resolve_conflict'
    ));

alter table capture_candidate_matches
    drop constraint capture_candidate_matches_kind_check;
alter table capture_candidate_matches
    add constraint capture_candidate_matches_kind_check
    check (match_kind in (
        'duplicate', 'support', 'contradiction', 'supersession', 'transition'
    ));

-- Temporal collection/search resolves one aggregate head at a transaction
-- instant before applying valid time. Current and closed sides each retain a
-- tenant-leading index so the security-invoker union view stays bounded.
create index knowledge_items_as_known
    on knowledge_items (tenant_id, tx_from, id);
create index knowledge_items_history_as_known_scan
    on knowledge_items_history (tenant_id, tx_from, tx_to, id);

create table knowledge_conflict_sets (
    id                     uuid        not null,
    tenant_id              uuid        not null,
    scope_id               uuid        not null,
    project_id             uuid,
    classification         text        not null,
    status                 text        not null default 'open',
    revision               bigint      not null default 1,
    capture_candidate_id   uuid,
    resolution_change_id   uuid,
    resolution             text,
    created_by             text        not null,
    resolved_by            text,
    created_at             timestamptz not null default now(),
    updated_at             timestamptz not null default now(),
    resolved_at            timestamptz,

    constraint knowledge_conflict_sets_pk primary key (tenant_id, id),
    constraint knowledge_conflict_sets_id_unique unique (id),
    constraint knowledge_conflict_sets_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint knowledge_conflict_sets_scope_fk
        foreign key (tenant_id, scope_id) references scopes (tenant_id, id),
    constraint knowledge_conflict_sets_project_fk
        foreign key (tenant_id, project_id) references projects (tenant_id, id),
    constraint knowledge_conflict_sets_candidate_fk
        foreign key (tenant_id, capture_candidate_id)
        references capture_candidates (tenant_id, id),
    constraint knowledge_conflict_sets_change_fk
        foreign key (tenant_id, resolution_change_id)
        references knowledge_changes (tenant_id, proposal_id)
        deferrable initially deferred,
    constraint knowledge_conflict_sets_classification_check
        check (classification in (
            'duplicate', 'support', 'contradiction', 'supersession', 'transition'
        )),
    constraint knowledge_conflict_sets_status_check
        check (status in ('open', 'pending_review', 'resolved', 'dismissed')),
    constraint knowledge_conflict_sets_resolution_check
        check (resolution is null or resolution in (
            'keep_separate', 'support', 'duplicate', 'supersede', 'transition', 'archive'
        )),
    constraint knowledge_conflict_sets_revision_check check (revision > 0),
    constraint knowledge_conflict_sets_actor_check check (
        btrim(created_by) <> '' and length(created_by) <= 255
        and (resolved_by is null or
             (btrim(resolved_by) <> '' and length(resolved_by) <= 255))
    ),
    constraint knowledge_conflict_sets_time_check
        check (updated_at >= created_at and
               (resolved_at is null or resolved_at >= created_at)),
    constraint knowledge_conflict_sets_resolution_shape_check check (
        (status = 'open' and resolution_change_id is null and resolution is null
         and resolved_by is null and resolved_at is null)
        or
        (status = 'pending_review' and resolution_change_id is not null
         and resolution is not null and resolved_by is not null and resolved_at is null)
        or
        (status in ('resolved', 'dismissed') and resolution_change_id is not null
         and resolution is not null and resolved_by is not null and resolved_at is not null)
    )
);

create unique index knowledge_conflict_sets_candidate_open
    on knowledge_conflict_sets (tenant_id, capture_candidate_id)
    where capture_candidate_id is not null and status in ('open', 'pending_review');
create index knowledge_conflict_sets_queue
    on knowledge_conflict_sets (tenant_id, status, updated_at desc, id desc);
create index knowledge_conflict_sets_scope_queue
    on knowledge_conflict_sets (tenant_id, scope_id, status, updated_at desc, id desc);
create index knowledge_conflict_sets_project_queue
    on knowledge_conflict_sets (tenant_id, project_id, status, updated_at desc, id desc)
    where project_id is not null;

create function synveda_knowledge_conflict_set_transition() returns trigger
language plpgsql
as $$
begin
    if new.id <> old.id
       or new.tenant_id <> old.tenant_id
       or new.scope_id <> old.scope_id
       or new.project_id is distinct from old.project_id
       or new.classification <> old.classification
       or new.capture_candidate_id is distinct from old.capture_candidate_id
       or new.created_by <> old.created_by
       or new.created_at <> old.created_at then
        raise exception 'Knowledge conflict identity and evidence address are immutable'
            using errcode = '23514';
    end if;
    if new.revision <> old.revision + 1 or new.updated_at <= old.updated_at then
        raise exception 'Knowledge conflict revisions advance exactly once'
            using errcode = '23514';
    end if;
    if old.status not in ('open', 'pending_review')
       or (old.status = 'pending_review' and not (
           (new.status in ('resolved', 'dismissed')
            and new.resolution_change_id = old.resolution_change_id
            and new.resolution = old.resolution
            and new.resolved_by = old.resolved_by)
           or
           (new.status = 'open'
            and new.resolution_change_id is null
            and new.resolution is null
            and new.resolved_by is null
            and new.resolved_at is null)
       )) then
        raise exception 'a Knowledge conflict resolution has an invalid transition'
            using errcode = '23514';
    end if;
    return new;
end
$$;

create trigger knowledge_conflict_set_transition
before update on knowledge_conflict_sets
for each row execute function synveda_knowledge_conflict_set_transition();

create table knowledge_conflict_members (
    id                       uuid        not null,
    tenant_id                uuid        not null,
    conflict_set_id          uuid        not null,
    role                     text        not null,
    knowledge_item_id        uuid,
    knowledge_revision_id    uuid,
    capture_candidate_id     uuid,
    classification           text        not null,
    similarity_permille      integer     not null,
    reason_code              text        not null,
    created_at               timestamptz not null default now(),

    constraint knowledge_conflict_members_pk primary key (tenant_id, id),
    constraint knowledge_conflict_members_id_unique unique (id),
    constraint knowledge_conflict_members_set_fk
        foreign key (tenant_id, conflict_set_id)
        references knowledge_conflict_sets (tenant_id, id),
    constraint knowledge_conflict_members_item_fk
        foreign key (tenant_id, knowledge_item_id, knowledge_revision_id)
        references knowledge_revisions (tenant_id, knowledge_item_id, id),
    constraint knowledge_conflict_members_candidate_fk
        foreign key (tenant_id, capture_candidate_id)
        references capture_candidates (tenant_id, id),
    constraint knowledge_conflict_members_role_check
        check (role in ('challenger', 'current')),
    constraint knowledge_conflict_members_classification_check
        check (classification in (
            'duplicate', 'support', 'contradiction', 'supersession', 'transition'
        )),
    constraint knowledge_conflict_members_similarity_check
        check (similarity_permille between 0 and 1000),
    constraint knowledge_conflict_members_reason_check
        check (reason_code ~ '^[a-z][a-z0-9_]*$' and length(reason_code) <= 64),
    constraint knowledge_conflict_members_shape_check check (
        (knowledge_item_id is not null and knowledge_revision_id is not null
         and capture_candidate_id is null)
        or
        (knowledge_item_id is null and knowledge_revision_id is null
         and capture_candidate_id is not null and role = 'challenger')
    )
);

create unique index knowledge_conflict_members_one_challenger
    on knowledge_conflict_members (tenant_id, conflict_set_id)
    where role = 'challenger';
create unique index knowledge_conflict_members_unique_knowledge
    on knowledge_conflict_members
       (tenant_id, conflict_set_id, knowledge_item_id, knowledge_revision_id)
    where knowledge_item_id is not null;
create unique index knowledge_conflict_members_unique_candidate
    on knowledge_conflict_members (tenant_id, conflict_set_id, capture_candidate_id)
    where capture_candidate_id is not null;
create index knowledge_conflict_members_by_item
    on knowledge_conflict_members
       (tenant_id, knowledge_item_id, knowledge_revision_id, conflict_set_id)
    where knowledge_item_id is not null;
create index knowledge_conflict_members_by_candidate
    on knowledge_conflict_members (tenant_id, capture_candidate_id, conflict_set_id)
    where capture_candidate_id is not null;

create function synveda_knowledge_conflict_member_immutable() returns trigger
language plpgsql
as $$
begin
    raise exception 'Knowledge conflict members are immutable'
        using errcode = '23514';
end
$$;

create trigger knowledge_conflict_member_immutable
before update or delete or truncate on knowledge_conflict_members
for each statement execute function synveda_knowledge_conflict_member_immutable();

alter table knowledge_conflict_sets enable row level security;
alter table knowledge_conflict_sets force row level security;
create policy knowledge_conflict_sets_tenant_isolation on knowledge_conflict_sets
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

alter table knowledge_conflict_members enable row level security;
alter table knowledge_conflict_members force row level security;
create policy knowledge_conflict_members_tenant_isolation on knowledge_conflict_members
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

grant select, insert, update on knowledge_conflict_sets to synveda_app;
grant select, insert on knowledge_conflict_members to synveda_app;
