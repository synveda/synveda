-- CPR-34 / ADR-0093: directory facts project once onto shared access rows.
--
-- This is a pre-1.0 hard cut. No row from the old SCIM group/member mirror is
-- copied into the shared Group graph. A database containing those development
-- rows must be reset; a fresh epoch database creates only the target shape.

do $$
begin
    if exists (select 1 from scim_users)
        or exists (select 1 from scim_groups)
        or exists (select 1 from scim_group_members)
        or exists (select 1 from group_members)
    then
        raise exception using
            errcode = '55000',
            message = 'CPR-34 changes the directory/group schema without data migration; reset with `synveda reset --database --force` and bootstrap again',
            hint = 'Reset this pre-1.0 database with `synveda reset --database --force`, then bootstrap it again.';
    end if;
end
$$;

-- A directory principal is adapter state, because SCIM must echo attributes
-- the product identity has no meaning for. Source ownership nevertheless has
-- to be a stored fact rather than an audit inference.
alter table scim_users add column directory_source text;
alter table scim_users alter column directory_source set not null;
alter table scim_users add constraint scim_users_directory_source_check
    check (btrim(directory_source) = directory_source
           and length(directory_source) between 1 and 64);

drop index scim_users_external_id_live;
create unique index scim_users_external_id_live
    on scim_users (tenant_id, directory_source, external_id)
    where external_id is not null and active;

-- A group and its membership are product nouns. The SCIM copies are deleted
-- rather than translated; both push and pull write the shared rows below.
drop table scim_group_members;
drop table scim_groups;

drop index groups_directory_ref_unique;
alter table groups drop constraint groups_directory_ref_shape_check;
alter table groups drop constraint groups_directory_ref_check;
alter table groups drop column directory_ref;
alter table groups
    add column directory_source text,
    add column directory_resource_id text,
    add column directory_external_id text,
    add constraint groups_directory_shape_check check (
        (source = 'directory') =
        (directory_source is not null and directory_resource_id is not null)
        and (source = 'directory' or directory_external_id is null)
    ),
    add constraint groups_directory_source_check check (
        directory_source is null
        or (btrim(directory_source) = directory_source
            and length(directory_source) between 1 and 64)
    ),
    add constraint groups_directory_resource_check check (
        directory_resource_id is null
        or (btrim(directory_resource_id) = directory_resource_id
            and length(directory_resource_id) between 1 and 255)
    ),
    add constraint groups_directory_external_check check (
        directory_external_id is null
        or (btrim(directory_external_id) = directory_external_id
            and length(directory_external_id) between 1 and 255)
    );

create unique index groups_directory_resource_unique
    on groups (tenant_id, directory_source, directory_resource_id)
    where directory_source is not null and directory_resource_id is not null;

-- Stable identity is the principal address available before first login.
-- Recreate rather than alter/cast: no old subject-keyed membership is carried
-- through the hard cut.
drop table group_members;
create table group_members (
    tenant_id  uuid        not null,
    group_id   uuid        not null,
    identity_id uuid       not null,
    source     text        not null default 'direct',
    added_by   text,
    created_at timestamptz not null default now(),

    constraint group_members_pk primary key (tenant_id, group_id, identity_id),
    constraint group_members_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint group_members_group_fk
        foreign key (tenant_id, group_id) references groups (tenant_id, id)
        on delete cascade,
    constraint group_members_identity_fk
        foreign key (tenant_id, identity_id) references identities (tenant_id, id),
    constraint group_members_source_check
        check (source in ('owner', 'direct', 'invite', 'directory', 'automation')),
    constraint group_members_added_by_check
        check (added_by is null or length(added_by) between 1 and 255)
);

create index group_members_by_identity on group_members (tenant_id, identity_id);
grant select, insert, delete on group_members to synveda_app;
alter table group_members enable row level security;
alter table group_members force row level security;
create policy group_members_tenant_isolation on group_members
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

-- A directory access assignment is still the one grant the PDP reads. These
-- columns retain which provider-owned group caused it; no adapter-only role or
-- permission table exists.
alter table scope_grants
    add column directory_source text,
    add column directory_resource_id text,
    add constraint scope_grants_directory_shape_check check (
        (source = 'directory') =
        (directory_source is not null and directory_resource_id is not null)
        and (source <> 'directory' or subject_kind = 'group')
    ),
    add constraint scope_grants_directory_source_check check (
        directory_source is null
        or (btrim(directory_source) = directory_source
            and length(directory_source) between 1 and 64)
    ),
    add constraint scope_grants_directory_resource_check check (
        directory_resource_id is null
        or (btrim(directory_resource_id) = directory_resource_id
            and length(directory_resource_id) between 1 and 255)
    );

-- Extend the existing immutability guard over the directory identity fields.
create or replace function synveda_groups_immutable_columns() returns trigger
language plpgsql
as $$
begin
    if new.id <> old.id then
        raise exception 'groups.id is immutable (CPR-5, ADR-0072)';
    end if;
    if new.tenant_id <> old.tenant_id then
        raise exception
            'group % cannot move across tenants (% to %) (CPR-5, ADR-0072)',
            old.id, old.tenant_id, new.tenant_id;
    end if;
    if new.slug <> old.slug then
        raise exception
            'groups.slug is immutable; an update changes display_name (CPR-5, ADR-0072)';
    end if;
    if new.source <> old.source
        or new.directory_source is distinct from old.directory_source
        or new.directory_resource_id is distinct from old.directory_resource_id then
        raise exception
            'a group does not change hands between the product and a directory (CPR-34, ADR-0093)';
    end if;
    if new.created_at <> old.created_at
        or new.created_by is distinct from old.created_by then
        raise exception 'group provenance is immutable (CPR-5, ADR-0072)';
    end if;
    if new.revision <> old.revision + 1 then
        raise exception
            'groups.revision steps forward by one; % to % (CPR-5, ADR-0072)',
            old.revision, new.revision;
    end if;
    return new;
end
$$;
