-- CPR-31 / ADR-0090: governed, versioned, time-boxed policy relaxations.
--
-- Pre-1.0 hard cut: the AUTHZ-4 lapse projection and mutable pack setting are
-- deleted without translation. A relaxation is a stable aggregate whose
-- immutable versions are effects of typed Policy/apply VedaFlow changes.

drop table policy_lapses;
drop function synveda_policy_lapse_transition();
alter table policy_packs drop column lapse;

alter table vedaflow_proposals drop constraint vedaflow_proposals_apply_asset_check;
alter table vedaflow_proposals add constraint vedaflow_proposals_apply_asset_check
    check (target_channel <> 'apply'
           or asset_kind in ('knowledge', 'skill', 'tool', 'configuration', 'policy'));

create table policy_relaxations (
    id                     uuid        not null,
    tenant_id              uuid        not null,
    governing_scope_id     uuid        not null,
    current_version_id     uuid        not null,
    revision               bigint      not null default 1,
    created_at             timestamptz not null default now(),
    created_by             uuid        not null,
    updated_at             timestamptz not null default now(),
    updated_by             uuid        not null,
    revoked_at             timestamptz,
    revoked_by             uuid,
    revocation_proposal_id uuid,
    revocation_reason      text,
    expiry_recorded_at     timestamptz,

    constraint policy_relaxations_pk primary key (tenant_id, id),
    constraint policy_relaxations_id_unique unique (id),
    constraint policy_relaxations_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint policy_relaxations_scope_fk
        foreign key (tenant_id, governing_scope_id)
        references scopes (tenant_id, id),
    constraint policy_relaxations_created_by_fk
        foreign key (tenant_id, created_by)
        references identities (tenant_id, id),
    constraint policy_relaxations_updated_by_fk
        foreign key (tenant_id, updated_by)
        references identities (tenant_id, id),
    constraint policy_relaxations_revoked_by_fk
        foreign key (tenant_id, revoked_by)
        references identities (tenant_id, id),
    constraint policy_relaxations_revocation_proposal_fk
        foreign key (tenant_id, revocation_proposal_id)
        references vedaflow_proposals (tenant_id, id),
    constraint policy_relaxations_revision_check check (revision > 0),
    constraint policy_relaxations_revocation_shape_check check (
        (revoked_at is null and revoked_by is null
         and revocation_proposal_id is null and revocation_reason is null)
        or
        (revoked_at is not null and revoked_by is not null
         and revocation_proposal_id is not null and revocation_reason is not null)
    ),
    constraint policy_relaxations_revocation_reason_check check (
        revocation_reason is null
        or (btrim(revocation_reason) = revocation_reason
            and char_length(revocation_reason) between 1 and 512)
    )
);

create table policy_relaxation_versions (
    id                       uuid        not null,
    tenant_id                uuid        not null,
    relaxation_id            uuid        not null,
    proposal_id              uuid        not null,
    ordinal                  bigint      not null,
    subject_identity_id      uuid        not null,
    subject_principal_id     text        not null,
    target_scope_id          uuid        not null,
    action                   text        not null,
    max_sensitivity          text        not null,
    requested_start_at       timestamptz not null,
    requested_end_at         timestamptz not null,
    effective_start_at       timestamptz not null,
    hard_expires_at          timestamptz not null,
    reason                   text        not null,
    configuration_version_id uuid,
    configuration_hash       text        not null,
    content_hash             bytea       not null,
    creator_id               uuid        not null,
    approver_ids             uuid[]      not null default '{}',
    auto_applied             boolean     not null,
    created_at               timestamptz not null default now(),

    constraint policy_relaxation_versions_pk primary key (tenant_id, id),
    constraint policy_relaxation_versions_id_unique unique (id),
    constraint policy_relaxation_versions_aggregate_fk
        foreign key (tenant_id, relaxation_id)
        references policy_relaxations (tenant_id, id),
    constraint policy_relaxation_versions_aggregate_id_unique
        unique (tenant_id, relaxation_id, id),
    constraint policy_relaxation_versions_proposal_fk
        foreign key (tenant_id, proposal_id)
        references vedaflow_proposals (tenant_id, id),
    constraint policy_relaxation_versions_proposal_unique
        unique (tenant_id, proposal_id),
    constraint policy_relaxation_versions_subject_fk
        foreign key (tenant_id, subject_identity_id)
        references identities (tenant_id, id),
    constraint policy_relaxation_versions_scope_fk
        foreign key (tenant_id, target_scope_id)
        references scopes (tenant_id, id),
    constraint policy_relaxation_versions_configuration_fk
        foreign key (tenant_id, configuration_version_id)
        references configuration_versions (tenant_id, id),
    constraint policy_relaxation_versions_creator_fk
        foreign key (tenant_id, creator_id)
        references identities (tenant_id, id),
    constraint policy_relaxation_versions_ordinal_unique
        unique (tenant_id, relaxation_id, ordinal),
    constraint policy_relaxation_versions_ordinal_check check (ordinal > 0),
    constraint policy_relaxation_versions_subject_check check (
        btrim(subject_principal_id) = subject_principal_id
        and char_length(subject_principal_id) between 1 and 255
    ),
    constraint policy_relaxation_versions_action_check
        check (action in ('knowledge.read')),
    constraint policy_relaxation_versions_sensitivity_check
        check (max_sensitivity in ('public', 'internal', 'confidential', 'restricted')),
    constraint policy_relaxation_versions_requested_window_check check (
        requested_start_at < requested_end_at
        and requested_end_at <= requested_start_at + interval '90 days'
    ),
    constraint policy_relaxation_versions_effective_window_check check (
        effective_start_at >= created_at
        and effective_start_at >= requested_start_at
        and hard_expires_at > effective_start_at
        and hard_expires_at <= requested_end_at
        and hard_expires_at <= created_at + interval '90 days'
    ),
    constraint policy_relaxation_versions_reason_check check (
        btrim(reason) = reason and char_length(reason) between 1 and 512
    ),
    constraint policy_relaxation_versions_configuration_hash_check
        check (configuration_hash ~ '^[0-9a-f]{64}$'),
    constraint policy_relaxation_versions_content_hash_check
        check (octet_length(content_hash) = 32),
    constraint policy_relaxation_versions_approvers_check
        check (array_position(approver_ids, null) is null),
    constraint policy_relaxation_versions_auto_apply_check
        check ((cardinality(approver_ids) = 0) = auto_applied)
);

alter table policy_relaxations
    add constraint policy_relaxations_current_version_fk
    foreign key (tenant_id, id, current_version_id)
    references policy_relaxation_versions (tenant_id, relaxation_id, id)
    deferrable initially deferred;

create table policy_relaxation_changes (
    tenant_id                  uuid        not null,
    proposal_id                uuid        not null,
    command_kind               text        not null,
    payload                    jsonb       not null,
    payload_hash               text        not null,
    resulting_relaxation_id    uuid,
    resulting_version_id       uuid,
    resulting_revision         bigint,
    applied_at                 timestamptz,
    created_at                 timestamptz not null default now(),

    constraint policy_relaxation_changes_pk primary key (tenant_id, proposal_id),
    constraint policy_relaxation_changes_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint policy_relaxation_changes_proposal_fk
        foreign key (tenant_id, proposal_id)
        references vedaflow_proposals (tenant_id, id),
    constraint policy_relaxation_changes_kind_check
        check (command_kind in ('create', 'revise', 'revoke')),
    constraint policy_relaxation_changes_payload_check
        check (jsonb_typeof(payload) = 'object' and pg_column_size(payload) <= 32768),
    constraint policy_relaxation_changes_payload_hash_check
        check (payload_hash ~ '^[0-9a-f]{64}$'),
    constraint policy_relaxation_changes_result_shape_check check (
        (applied_at is null and resulting_revision is null)
        or
        (applied_at is not null and resulting_relaxation_id is not null
         and resulting_revision is not null and resulting_revision > 0)
    )
);

create index policy_relaxations_listing_idx
    on policy_relaxations (tenant_id, updated_at desc, id desc);
create index policy_relaxation_versions_active_subject_idx
    on policy_relaxation_versions
       (tenant_id, subject_principal_id, effective_start_at, hard_expires_at)
    include (target_scope_id, action, max_sensitivity);
create index policy_relaxations_expiry_idx
    on policy_relaxations (tenant_id, expiry_recorded_at)
    where revoked_at is null and expiry_recorded_at is null;

-- Exact Policy/apply provenance is structural, not merely a handler promise.
create function synveda_policy_relaxation_version_matches_proposal() returns trigger
language plpgsql
as $$
begin
    if not exists (
        select 1 from vedaflow_proposals proposal
         where proposal.tenant_id = new.tenant_id
           and proposal.id = new.proposal_id
           and proposal.asset_kind = 'policy'
           and proposal.target_channel = 'apply'
    ) then
        raise exception 'Relaxation version must bind a Policy/apply proposal'
            using errcode = '23514';
    end if;
    return new;
end
$$;

create trigger policy_relaxation_versions_proposal_shape
    before insert on policy_relaxation_versions
    for each row execute function synveda_policy_relaxation_version_matches_proposal();

create function synveda_policy_relaxation_aggregate_transition() returns trigger
language plpgsql
as $$
begin
    if new.id <> old.id
       or new.tenant_id <> old.tenant_id
       or new.governing_scope_id <> old.governing_scope_id
       or new.created_at <> old.created_at
       or new.created_by <> old.created_by
    then
        raise exception 'a Relaxation aggregate identity is immutable (CPR-31)';
    end if;
    if old.expiry_recorded_at is null
       and new.expiry_recorded_at is not null
       and new.current_version_id = old.current_version_id
       and new.revision = old.revision
       and new.updated_at = old.updated_at
       and new.updated_by = old.updated_by
       and new.revoked_at is not distinct from old.revoked_at
       and new.revoked_by is not distinct from old.revoked_by
       and new.revocation_proposal_id is not distinct from old.revocation_proposal_id
       and new.revocation_reason is not distinct from old.revocation_reason
    then
        return new;
    end if;
    if new.revision <> old.revision + 1 then
        raise exception 'a Relaxation transition must advance revision exactly once (CPR-31)';
    end if;
    if old.revoked_at is not null then
        raise exception 'a revoked Relaxation is terminal (CPR-31)';
    end if;
    if new.current_version_id = old.current_version_id
       and new.revoked_at is null
       and new.expiry_recorded_at is not distinct from old.expiry_recorded_at
    then
        raise exception 'a Relaxation transition must publish, revoke, or record expiry (CPR-31)';
    end if;
    if old.expiry_recorded_at is not null
       and new.expiry_recorded_at is distinct from old.expiry_recorded_at
    then
        raise exception 'a Relaxation expiry may be recorded once (CPR-31)';
    end if;
    return new;
end
$$;

create trigger policy_relaxations_transition
    before update on policy_relaxations
    for each row execute function synveda_policy_relaxation_aggregate_transition();

create function synveda_immutable_relaxation_version() returns trigger
language plpgsql
as $$
begin
    raise exception 'Relaxation versions are immutable (CPR-31)';
end
$$;

create trigger policy_relaxation_versions_immutable
    before update or delete on policy_relaxation_versions
    for each row execute function synveda_immutable_relaxation_version();

create function synveda_policy_relaxation_change_transition() returns trigger
language plpgsql
as $$
begin
    if new.tenant_id <> old.tenant_id
       or new.proposal_id <> old.proposal_id
       or new.command_kind <> old.command_kind
       or new.payload <> old.payload
       or new.payload_hash <> old.payload_hash
       or new.created_at <> old.created_at
    then
        raise exception 'a Relaxation VedaFlow command is immutable (CPR-31)';
    end if;
    if old.applied_at is not null or new.applied_at is null then
        raise exception 'a Relaxation result may be recorded exactly once (CPR-31)';
    end if;
    return new;
end
$$;

create trigger policy_relaxation_changes_transition
    before update on policy_relaxation_changes
    for each row execute function synveda_policy_relaxation_change_transition();

grant select, insert on policy_relaxations to synveda_app;
grant update (current_version_id, revision, updated_at, updated_by,
              revoked_at, revoked_by, revocation_proposal_id,
              revocation_reason, expiry_recorded_at)
    on policy_relaxations to synveda_app;
grant select, insert on policy_relaxation_versions to synveda_app;
grant select, insert on policy_relaxation_changes to synveda_app;
grant update (resulting_relaxation_id, resulting_version_id,
              resulting_revision, applied_at)
    on policy_relaxation_changes to synveda_app;

alter table policy_relaxations enable row level security;
alter table policy_relaxations force row level security;
alter table policy_relaxation_versions enable row level security;
alter table policy_relaxation_versions force row level security;
alter table policy_relaxation_changes enable row level security;
alter table policy_relaxation_changes force row level security;

create policy policy_relaxations_tenant_isolation on policy_relaxations
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy policy_relaxation_versions_tenant_isolation on policy_relaxation_versions
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
create policy policy_relaxation_changes_tenant_isolation on policy_relaxation_changes
    for all using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
