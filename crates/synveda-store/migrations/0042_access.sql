-- CPR-5: groups, scope grants and invitations (ADR-0068 decisions 1 and 4,
-- ADR-0070, ADR-0072).
--
-- Who may act on a governed scope, and where that authority came from. Four
-- tables and one shape: a person working alone has one row in `scope_grants`
-- (the `owner` grant their first workspace minted) and a company with a
-- directory has fifty thousand, in the same table with the same columns.
-- There is no `personal_members`, no `team_members` and no enterprise variant,
-- and ADR-0068 decision 1 forbids one arriving later.
--
-- ── A role key is a key, and this schema stores nothing else ─────────────
--
-- `scope_grants.role_key` is one of six words and there is **no permission
-- table beside it** (ADR-0072 decision 2). Nothing in this database says what
-- an `owner` may do. The product already has exactly one thing that decides
-- that — the Cedar packs — and a second mapping here would be a second
-- decision point that disagrees with the first the day one of them is edited.
-- So: the key is stored, the policy layer interprets it.
--
-- ── Inheritance is the scope tree, not a fan-out ─────────────────────────
--
-- A grant at a workspace's scope reaches that workspace's projects because a
-- project's scope is *inside* the workspace's — one row, resolved through
-- `scope_closure` at read time. Nothing materialises a per-project copy, so
-- there is no derived set to keep consistent and no window in which the copy
-- and the source disagree.
--
-- The one place inheritance stops is a `principal`-shaped scope, which is
-- somebody's own: no ancestor reaches into it, so a workspace owner does not
-- silently hold their colleagues' private material. That rule lives in the
-- read (`synveda_store::access::members_of`) and in
-- `synveda_types::access::inherits_into`, because it is a property of a walk
-- rather than of a row.
--
-- ── A principal is a token subject ──────────────────────────────────────
--
-- `principal_id` is text, not a foreign key into `identities` — ADR-0015
-- decision 2's reasoning, and one of its own. The PDP's principal is
-- `(tenant, subject)`; a grant that could not precede first login could not be
-- pre-assigned; and, decisively, an `identities` row in this tree still
-- requires a node of the **old** hierarchy (`identities_scope_fk` →
-- `hierarchy_nodes`). A membership model that needed the model it replaces
-- would be a synchronisation between the two, which this programme forbids.
--
-- ── Where each structural rule is enforced ──────────────────────────────
--
-- ADR-0070 decision 2's doctrine again: every rule that can be a database fact
-- is one.
--
--   a grant has exactly one subject          scope_grants_subject_check
--                                            + the two column checks
--   a grant's scope is this tenant's         scope_grants_scope_fk (composite)
--   a grant's group is this tenant's         scope_grants_group_fk (composite)
--   one row per (scope, subject, role)       scope_grants_unique
--   an `invite` grant names its invitation   scope_grants_invite_shape_check
--   a grant is never edited                  synveda_grants_are_immutable
--   an invitation is one-time                pending_invites_status_check
--                                            + synveda_invites_are_terminal
--   an invitation always expires             pending_invites_expiry_check
--   a directory group carries its ref        groups_directory_ref_check
--   group revisions step forward by one      synveda_groups_immutable_columns
--
-- The one rule that is deliberately **not** a database fact is "a
-- directory-managed grant cannot be revoked by hand". The directory adapter
-- and a person hold the same database role, so no constraint can tell them
-- apart; the refusal lives in `synveda_store::access` with a message naming
-- the directory, and is tested there.

-- ── Groups ───────────────────────────────────────────────────────────────
--
-- A named set of principals. It grants nothing by itself: a `scope_grants` row
-- whose subject is the group is what grants, which is what lets a deployment
-- say "engineering" once and then price it differently at three scopes.

create table groups (
    id            uuid        not null,
    tenant_id     uuid        not null,
    slug          text        not null,
    display_name  text        not null,
    description   text,
    -- `direct` (this product's to edit) or `directory` (a directory's).
    source        text        not null default 'direct',
    -- The external id a directory knows the group by. Present exactly when
    -- the group is a directory's, so a sync can find its own rows without a
    -- second table and a person cannot invent one.
    directory_ref text,
    status        text        not null default 'active',
    -- Monotonic; what an update's precondition names (ADR-0071 decision 5,
    -- the same rule one plane over).
    revision      bigint      not null default 1,
    -- The token subject that created it. Text rather than an identity FK, for
    -- the module header's reason.
    created_by    text,
    created_at    timestamptz not null default now(),
    updated_at    timestamptz not null default now(),

    constraint groups_pk primary key (id),
    constraint groups_tenant_fk foreign key (tenant_id) references tenants (id),
    -- The composite target the grant and membership keys need.
    constraint groups_tenant_id_unique unique (tenant_id, id),
    constraint groups_slug_unique unique (tenant_id, slug),
    -- Same grammar as a scope and a workspace slug (ADR-0008, ADR-0070):
    -- URL-, hostname- and CLI-safe, so a group is a thing somebody can type.
    constraint groups_slug_check check (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'),
    constraint groups_display_name_check
        check (btrim(display_name) <> '' and length(display_name) <= 200),
    constraint groups_description_check
        check (description is null
               or (btrim(description) <> '' and length(description) <= 2000)),
    constraint groups_source_check check (source in ('direct', 'directory')),
    constraint groups_directory_ref_check
        check ((source = 'directory') = (directory_ref is not null)),
    constraint groups_directory_ref_shape_check
        check (directory_ref is null
               or (btrim(directory_ref) <> '' and length(directory_ref) <= 255)),
    constraint groups_status_check check (status in ('active', 'archived')),
    constraint groups_revision_check check (revision >= 1),
    constraint groups_created_by_check
        check (created_by is null or length(created_by) between 1 and 255),
    constraint groups_updated_check check (updated_at >= created_at)
);

create index groups_by_tenant on groups (tenant_id, slug);

-- One directory group is one row: a sync that ran twice must not produce two.
create unique index groups_directory_ref_unique
    on groups (tenant_id, directory_ref)
    where directory_ref is not null;

-- ── Group membership ─────────────────────────────────────────────────────
--
-- Resolved at read time rather than expanded into grants: adding somebody to a
-- group gives them everything the group holds, everywhere, with no fan-out to
-- keep consistent and no window where the copy is stale.

create table group_members (
    tenant_id    uuid        not null,
    group_id     uuid        not null,
    principal_id text        not null,
    -- Where the membership came from. The same vocabulary a grant's source
    -- uses, because "why is this person in this group" and "why does this
    -- person have this role" are the same question one level apart, and two
    -- vocabularies for it would be two things to keep aligned.
    source       text        not null default 'direct',
    added_by     text,
    created_at   timestamptz not null default now(),

    constraint group_members_pk primary key (tenant_id, group_id, principal_id),
    constraint group_members_tenant_fk foreign key (tenant_id) references tenants (id),
    -- Membership is the group's content: deleting a group takes it along, and
    -- membership is not governed material in its own right.
    constraint group_members_group_fk
        foreign key (tenant_id, group_id) references groups (tenant_id, id)
        on delete cascade,
    constraint group_members_source_check
        check (source in ('owner', 'direct', 'invite', 'directory', 'automation')),
    constraint group_members_principal_check
        check (btrim(principal_id) <> '' and length(principal_id) <= 255),
    constraint group_members_added_by_check
        check (added_by is null or length(added_by) between 1 and 255)
);

-- "Every group this principal is in" — the read the effective-member
-- resolution makes once per scope.
create index group_members_by_principal on group_members (tenant_id, principal_id);

-- ── Invitations ──────────────────────────────────────────────────────────
--
-- An expiring, one-time bearer credential that mints a grant when somebody
-- redeems it. The token itself is **never here**: what is stored is its
-- SHA-256, and the plaintext exists once, in the response to the request that
-- created it (ADR-0072 decision 5; the shape ADR-0059 decision 13 set for the
-- provisioning credential, and for the same threat model — a database dump
-- must mint nothing).
--
-- Declared before `scope_grants` because a grant produced by an invitation
-- points back at it.

create table pending_invites (
    id          uuid        not null,
    tenant_id   uuid        not null,
    -- The scope the invitation grants at. A generic scope rather than a
    -- workspace id: the route that creates one today is workspace-level, and
    -- the row has no opinion about which product noun owns the scope.
    scope_id    uuid        not null,
    role_key    text        not null,
    -- Who it was meant for, when the inviter said. Optional on purpose: a
    -- deployment with no mail path invites by copying a link, and an address
    -- nobody can send to is a label rather than a requirement.
    email       text,
    -- SHA-256 of the whole presented token, prefix and tenant included.
    token_hash  bytea       not null,
    -- `pending` | `accepted` | `revoked`. There is deliberately no `expired`:
    -- expiry is a property of the decision rather than of a job (ADR-0037
    -- decision 4), so the read derives it from `expires_at` and no sweep has
    -- to have run for an invitation to have stopped working.
    status      text        not null default 'pending',
    expires_at  timestamptz not null,
    created_by  text,
    created_at  timestamptz not null default now(),
    accepted_by text,
    accepted_at timestamptz,
    revoked_by  text,
    revoked_at  timestamptz,

    constraint pending_invites_pk primary key (id),
    constraint pending_invites_tenant_fk foreign key (tenant_id) references tenants (id),
    constraint pending_invites_tenant_id_unique unique (tenant_id, id),
    constraint pending_invites_scope_fk
        foreign key (tenant_id, scope_id) references scopes (tenant_id, id),
    constraint pending_invites_role_check
        check (role_key in ('owner', 'member', 'viewer', 'reviewer', 'curator',
                            'administrator')),
    constraint pending_invites_email_check
        check (email is null or (btrim(email) <> '' and length(email) <= 320)),
    constraint pending_invites_hash_check check (octet_length(token_hash) = 32),
    constraint pending_invites_status_check
        check (status in ('pending', 'accepted', 'revoked')),
    -- A terminal status carries its stamps and an open one carries none, both
    -- ways round — so "accepted" and "who accepted it" cannot disagree.
    constraint pending_invites_accepted_shape_check
        check ((status = 'accepted')
               = (accepted_at is not null and accepted_by is not null)),
    constraint pending_invites_revoked_shape_check
        check ((status = 'revoked') = (revoked_at is not null)),
    -- An invitation that never expires is a key left under the mat (AUTH-3's
    -- lifetime-cap doctrine, ADR-0018 decision 5). One that expired before it
    -- was created is a row nothing can ever redeem; refuse it here rather
    -- than let it sit there looking like an invitation.
    constraint pending_invites_expiry_check check (expires_at > created_at),
    constraint pending_invites_created_by_check
        check (created_by is null or length(created_by) between 1 and 255),
    constraint pending_invites_accepted_by_check
        check (accepted_by is null or length(accepted_by) between 1 and 255),
    constraint pending_invites_revoked_by_check
        check (revoked_by is null or length(revoked_by) between 1 and 255)
);

-- The redeem path: hash lookup inside the caller's own tenant.
create unique index pending_invites_hash_unique on pending_invites (tenant_id, token_hash);

-- The listing (`GET /v1/workspaces/{id}/invites`).
create index pending_invites_by_scope on pending_invites (tenant_id, scope_id, created_at);

-- ── Grants ───────────────────────────────────────────────────────────────
--
-- One subject's one role at one scope. Additive and inherited; there is no
-- deny row and there must not be one — a denial that lives in a membership
-- table is a second policy engine, and this product has one.

create table scope_grants (
    id           uuid        not null,
    tenant_id    uuid        not null,
    scope_id     uuid        not null,
    -- `principal` | `group`, and exactly one of the two columns below.
    subject_kind text        not null,
    principal_id text,
    group_id     uuid,
    role_key     text        not null,
    -- `owner` | `direct` | `invite` | `directory` | `automation` — the whole
    -- of "access-source visibility": "why can this person see my project" is
    -- answerable from the row rather than from an audit search.
    source       text        not null,
    invite_id    uuid,
    granted_by   text,
    created_at   timestamptz not null default now(),

    constraint scope_grants_pk primary key (id),
    constraint scope_grants_tenant_fk foreign key (tenant_id) references tenants (id),
    constraint scope_grants_scope_fk
        foreign key (tenant_id, scope_id) references scopes (tenant_id, id),
    -- A grant to a group dies with the group: a grant whose subject does not
    -- exist grants nothing, and keeping it would be a row that looks like
    -- access and is not.
    constraint scope_grants_group_fk
        foreign key (tenant_id, group_id) references groups (tenant_id, id)
        on delete cascade,
    -- The invitation stays after it is redeemed (it is the provenance the
    -- grant points at), so no cascade here.
    constraint scope_grants_invite_fk
        foreign key (tenant_id, invite_id) references pending_invites (tenant_id, id),
    constraint scope_grants_subject_check
        check (subject_kind in ('principal', 'group')),
    constraint scope_grants_principal_shape_check
        check ((subject_kind = 'principal') = (principal_id is not null)),
    constraint scope_grants_group_shape_check
        check ((subject_kind = 'group') = (group_id is not null)),
    constraint scope_grants_principal_length_check
        check (principal_id is null
               or (btrim(principal_id) <> '' and length(principal_id) <= 255)),
    constraint scope_grants_role_check
        check (role_key in ('owner', 'member', 'viewer', 'reviewer', 'curator',
                            'administrator')),
    constraint scope_grants_source_check
        check (source in ('owner', 'direct', 'invite', 'directory', 'automation')),
    -- An `invite`-sourced grant names the invitation it came from, and no
    -- other source may: provenance that can be claimed without evidence is
    -- not provenance.
    constraint scope_grants_invite_shape_check
        check ((source = 'invite') = (invite_id is not null)),
    constraint scope_grants_granted_by_check
        check (granted_by is null or length(granted_by) between 1 and 255)
);

-- One row per (scope, subject, role). `nulls not distinct` so the rule binds
-- across both subject shapes — the unused column is NULL in every row, and
-- without this two identical principal grants would be "distinct".
create unique index scope_grants_unique
    on scope_grants (tenant_id, scope_id, principal_id, group_id, role_key)
    nulls not distinct;

-- "Every grant at these scopes" — the ancestry read the member resolution
-- makes, and the `GET /v1/admin/grants?scope_id=` filter.
create index scope_grants_by_scope on scope_grants (tenant_id, scope_id);

-- "Everything this principal holds" — the reverse question, which is the one
-- somebody asks when a person leaves.
create index scope_grants_by_principal
    on scope_grants (tenant_id, principal_id)
    where principal_id is not null;

-- ── Immutability ─────────────────────────────────────────────────────────
--
-- 0040's and 0041's reasoning, applied to the access plane. Forced RLS stops
-- the application role from writing another tenant's rows; these cover the
-- owner role, which is what migrations, break-glass psql and a restore run as
-- and which RLS does not bind.

-- A grant is created and revoked, never edited. Changing the role somebody
-- holds is a revoke and a grant, so that the chain records two acts rather
-- than one row quietly meaning something else — and so that `created_at`
-- always answers "since when".
create function synveda_grants_are_immutable() returns trigger
language plpgsql
as $$
begin
    raise exception
        'scope_grants rows are never updated; revoke and grant instead (CPR-5, ADR-0072)';
end
$$;

create trigger scope_grants_immutable
    before update on scope_grants
    for each row execute function synveda_grants_are_immutable();

-- An invitation is one-time: `pending` is the only status anything may leave,
-- and it may only be left once. Without this, "one-time" would be a SELECT
-- ... FOR UPDATE in one function, and anything holding a connection could
-- reopen a redeemed invitation.
create function synveda_invites_are_terminal() returns trigger
language plpgsql
as $$
begin
    if new.id <> old.id or new.tenant_id <> old.tenant_id
        or new.scope_id <> old.scope_id or new.role_key <> old.role_key
        or new.token_hash <> old.token_hash or new.expires_at <> old.expires_at then
        raise exception
            'an invitation''s terms are immutable; revoke it and issue another (CPR-5, ADR-0072)';
    end if;
    if new.created_at <> old.created_at
        or new.created_by is distinct from old.created_by then
        raise exception 'invitation provenance is immutable (CPR-5, ADR-0072)';
    end if;
    if old.status <> 'pending' then
        raise exception
            'invitation % is already %; an invitation is one-time (CPR-5, ADR-0072)',
            old.id, old.status;
    end if;
    return new;
end
$$;

create trigger pending_invites_terminal
    before update on pending_invites
    for each row execute function synveda_invites_are_terminal();

-- A group's handle and provenance are fixed, and its revision steps forward by
-- exactly one — 0041's rule, restated for the table that carries the other
-- `expected_revision` on this plane.
create function synveda_groups_immutable_columns() returns trigger
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
    if new.source <> old.source then
        raise exception
            'a group does not change hands between the product and a directory (CPR-5, ADR-0072)';
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

create trigger groups_immutable_columns
    before update on groups
    for each row execute function synveda_groups_immutable_columns();

-- ── Tenant isolation ─────────────────────────────────────────────────────
--
-- Tenant-scoped tables get forced RLS + policy + least-privilege grants in the
-- same migration (ADR-0009 structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it).
--
-- No UPDATE on `scope_grants` (the trigger above says the same thing twice, on
-- purpose: the grant is what the app role cannot do, the trigger is what
-- nobody can). No DELETE on `groups` — retiring one is a status transition,
-- because a group is what grants and audit events name. `group_members`,
-- `scope_grants` and `pending_invites` all get DELETE, because removing
-- somebody from a group and revoking a grant are the API's own verbs and the
-- rows assert present facts; what they were is in the chain.
--
-- `pending_invites` gets UPDATE for exactly two transitions, both narrowed by
-- the terminal trigger above.

grant select, insert, update on groups to synveda_app;
grant select, insert, delete on group_members to synveda_app;
grant select, insert, delete on scope_grants to synveda_app;
grant select, insert, update, delete on pending_invites to synveda_app;

alter table groups enable row level security;
alter table groups force row level security;
alter table group_members enable row level security;
alter table group_members force row level security;
alter table scope_grants enable row level security;
alter table scope_grants force row level security;
alter table pending_invites enable row level security;
alter table pending_invites force row level security;

create policy groups_tenant_isolation on groups
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy group_members_tenant_isolation on group_members
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy scope_grants_tenant_isolation on scope_grants
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy pending_invites_tenant_isolation on pending_invites
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
