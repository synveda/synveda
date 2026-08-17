-- CPR-4: workspaces, projects and canonical repository identity
-- (ADR-0068 decisions 1 and 4, ADR-0070, ADR-0071).
--
-- Workspaces and projects are **product-level subtypes of a governed scope**,
-- not a second tree. Each owns exactly one scope of the matching shape —
-- `workspace` under the tenant root, `project` under its workspace's scope —
-- and that scope is what policy is assigned to, what a role binding covers,
-- and what every asset attaches to. The subtype row carries what a scope has
-- no opinion about: a description, a lifecycle status, a monotonic revision,
-- and (for a project) the repositories it is about.
--
-- ── There is one shape, for one person and for a bank ────────────────────
--
-- There is no `personal_workspaces` table and no `team_workspaces` table, and
-- ADR-0068 decision 1 forbids one arriving later. A person working alone has
-- one row here; a company has hundreds. What differs is the policy profile
-- assigned to their scopes.
--
-- ── The subtype and its scope are created together or not at all ─────────
--
-- `scope_id` is NOT NULL with a foreign key, so a workspace without a scope
-- is unrepresentable; the services create both in one transaction, so a
-- failure between them leaves neither. See synveda_store::workspaces.
--
-- ── Where each structural rule is enforced ───────────────────────────────
--
-- ADR-0070 decision 2's doctrine, applied again: every rule that can be a
-- database fact is one, because a rule that lives in a function holds only
-- for callers who went through that function.
--
--   a workspace's scope is workspace-shaped   workspaces_scope_fk (composite)
--   a project's scope is project-shaped       projects_scope_fk (composite)
--   a project's scope sits under its
--     workspace's scope                       projects_scope_parent_fk
--   the subtype's slug IS the scope's slug    both composite scope FKs
--   one subtype per scope                     workspaces_scope_unique,
--                                             projects_scope_unique
--   workspace slugs unique per tenant         workspaces_slug_unique
--   project slugs unique per workspace        projects_slug_unique
--   never crosses a tenant                    every FK carries tenant_id,
--                                             plus the immutability triggers
--   revisions only ever step forward by one   the immutability triggers
--   a repository is identified by a canonical
--     URI, never by a path                    project_repositories_uri_check
--                                             + synveda_types::repository
--
-- The composite foreign keys need matching unique keys on `scopes` and on
-- `workspaces`; they are created here rather than in 0040 because they exist
-- for these constraints, and a reader meeting them should meet the reason in
-- the same file.

-- (tenant_id, id, kind, slug) — the target of both subtype scope keys. One
-- key rather than two, so "this scope is workspace-shaped" and "the workspace
-- and its scope share one name" are a single referential fact.
create unique index scopes_tenant_id_kind_slug_unique
    on scopes (tenant_id, id, kind, slug);

-- (tenant_id, id, parent_scope_id) — the target of projects_scope_parent_fk.
-- `parent_scope_id` is nullable on `scopes`, but the referencing column is
-- NOT NULL, so MATCH SIMPLE never satisfies this vacuously.
create unique index scopes_tenant_id_parent_unique
    on scopes (tenant_id, id, parent_scope_id);

-- ── Workspaces ───────────────────────────────────────────────────────────

create table workspaces (
    id            uuid        not null,
    tenant_id     uuid        not null,
    scope_id      uuid        not null,
    -- Denormalised copies of the owned scope's `kind` and `slug`, present so
    -- the two rules above can be row-local foreign keys. Never read by
    -- application code (synveda_store::workspaces reads `slug`, which is the
    -- same column by construction); `scope_kind` is pinned to one value by a
    -- CHECK, and neither can drift because `scopes.kind` and `scopes.slug`
    -- are immutable (0040's trigger) and the key is composite.
    scope_kind    text        not null,
    slug          text        not null,
    display_name  text        not null,
    -- Optional prose. A blank description is refused by synveda-types rather
    -- than stored, so NULL is the only way to say "none".
    description   text,
    status        text        not null default 'active',
    -- Monotonic; what an update's precondition names (ADR-0071 decision 5).
    revision      bigint      not null default 1,
    created_by    uuid,
    created_at    timestamptz not null default now(),
    updated_at    timestamptz not null default now(),

    constraint workspaces_pk primary key (id),
    constraint workspaces_tenant_fk foreign key (tenant_id) references tenants (id),
    -- The composite target projects_workspace_fk needs.
    constraint workspaces_tenant_id_scope_unique unique (tenant_id, id, scope_id),
    -- One workspace per scope, and never another tenant's scope.
    constraint workspaces_scope_unique unique (tenant_id, scope_id),
    constraint workspaces_scope_fk
        foreign key (tenant_id, scope_id, scope_kind, slug)
        references scopes (tenant_id, id, kind, slug),
    constraint workspaces_scope_kind_check check (scope_kind = 'workspace'),
    constraint workspaces_slug_unique unique (tenant_id, slug),
    -- Same grammar as a scope slug (ADR-0008, ADR-0070): URL-, hostname- and
    -- CLI-safe, so a workspace path is a thing somebody can type.
    constraint workspaces_slug_check check (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'),
    constraint workspaces_display_name_check
        check (btrim(display_name) <> '' and length(display_name) <= 200),
    constraint workspaces_description_check
        check (description is null
               or (btrim(description) <> '' and length(description) <= 2000)),
    constraint workspaces_status_check check (status in ('active', 'archived')),
    constraint workspaces_revision_check check (revision >= 1),
    constraint workspaces_updated_check check (updated_at >= created_at)
);

create index workspaces_by_tenant on workspaces (tenant_id, slug);

-- ── Projects ─────────────────────────────────────────────────────────────

create table projects (
    id                 uuid        not null,
    tenant_id          uuid        not null,
    workspace_id       uuid        not null,
    scope_id           uuid        not null,
    scope_kind         text        not null,
    -- The workspace's scope. Denormalised for projects_scope_parent_fk, and
    -- held equal to the workspace's own `scope_id` by projects_workspace_fk —
    -- so "a project's scope is a child of its workspace's scope" is two
    -- foreign keys rather than a rule somebody has to remember.
    workspace_scope_id uuid        not null,
    slug               text        not null,
    display_name       text        not null,
    description        text,
    status             text        not null default 'active',
    revision           bigint      not null default 1,
    created_by         uuid,
    created_at         timestamptz not null default now(),
    updated_at         timestamptz not null default now(),

    constraint projects_pk primary key (id),
    constraint projects_tenant_fk foreign key (tenant_id) references tenants (id),
    constraint projects_tenant_id_unique unique (tenant_id, id),
    constraint projects_workspace_fk
        foreign key (tenant_id, workspace_id, workspace_scope_id)
        references workspaces (tenant_id, id, scope_id),
    constraint projects_scope_unique unique (tenant_id, scope_id),
    constraint projects_scope_fk
        foreign key (tenant_id, scope_id, scope_kind, slug)
        references scopes (tenant_id, id, kind, slug),
    constraint projects_scope_parent_fk
        foreign key (tenant_id, scope_id, workspace_scope_id)
        references scopes (tenant_id, id, parent_scope_id),
    constraint projects_scope_kind_check check (scope_kind = 'project'),
    -- Sibling slugs are unique inside a workspace; across workspaces they are
    -- free, because a project slug is qualified by the workspace it is in —
    -- which is exactly what the scope tree already says.
    constraint projects_slug_unique unique (tenant_id, workspace_id, slug),
    constraint projects_slug_check check (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'),
    constraint projects_display_name_check
        check (btrim(display_name) <> '' and length(display_name) <= 200),
    constraint projects_description_check
        check (description is null
               or (btrim(description) <> '' and length(description) <= 2000)),
    constraint projects_status_check check (status in ('active', 'archived')),
    constraint projects_revision_check check (revision >= 1),
    constraint projects_updated_check check (updated_at >= created_at)
);

create index projects_by_workspace on projects (tenant_id, workspace_id, slug);

-- ── Project repositories ─────────────────────────────────────────────────
--
-- ADR-0071 decision 4: a repository's identity is its **canonical remote
-- URI**, and a local filesystem path is never one — not as a fallback and not
-- when nothing else is available, because a path differs per machine and
-- changes when somebody moves a directory. A repository with no remote is
-- identified by a `git+fingerprint:<hex>` URI built from a stable content id
-- the client computed (a git root-commit object id), which survives every
-- move a path does not.
--
-- The canonicalisation itself lives in `synveda_types::repository` — dropping
-- the transport, the credential, the default port, the `.git` suffix — because
-- it needs to produce messages worth reading. What is enforced here is the
-- shape the canonicalisation must have produced, so a row written by anything
-- holding a connection is still an identity and not a path.

create table project_repositories (
    id                uuid        not null,
    tenant_id         uuid        not null,
    project_id        uuid        not null,
    provider          text        not null,
    -- The identity. Credential-free by construction: `normalise_host` drops
    -- userinfo, which is why this column is safe to store, log and return.
    canonical_uri     text        not null,
    repository_owner  text,
    repository_name   text        not null,
    default_branch    text,
    -- A stable content id, when the client computed one. Identity only when
    -- there is no remote; a hint beside the remote otherwise.
    local_fingerprint text,
    metadata          jsonb       not null default '{}'::jsonb,
    created_by        uuid,
    created_at        timestamptz not null default now(),
    updated_at        timestamptz not null default now(),

    constraint project_repositories_pk primary key (id),
    constraint project_repositories_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint project_repositories_project_fk
        foreign key (tenant_id, project_id) references projects (tenant_id, id),
    constraint project_repositories_provider_check
        check (provider in ('github', 'gitlab', 'bitbucket', 'azure_devops',
                            'generic_git', 'local')),
    -- The identity is one of exactly two shapes, and neither is a path:
    -- `https://host/path` for a remote, `git+fingerprint:<40-128 hex>` for a
    -- repository that has none. No port, no credential, no transport — see
    -- `synveda_types::repository` for why an identity carries none of them.
    constraint project_repositories_uri_check
        check (canonical_uri ~ '^https://[a-z0-9][a-z0-9._-]*/[^[:space:]]+$'
               or canonical_uri ~ '^git\+fingerprint:[0-9a-f]{40,128}$'),
    constraint project_repositories_uri_length_check
        check (length(canonical_uri) between 1 and 512),
    -- `local` and a remote URI are a contradiction, both ways round.
    constraint project_repositories_local_shape_check
        check ((provider = 'local') = (canonical_uri like 'git+fingerprint:%')),
    -- A fingerprint is a content id or it is absent. The CHECK is the same
    -- grammar synveda_types::repository normalises to, so a row cannot hold a
    -- path in the column whose whole purpose is not being one.
    constraint project_repositories_fingerprint_check
        check (local_fingerprint is null
               or local_fingerprint ~ '^[0-9a-f]{40,128}$'),
    constraint project_repositories_name_check
        check (btrim(repository_name) <> '' and length(repository_name) <= 255),
    constraint project_repositories_owner_check
        check (repository_owner is null
               or (btrim(repository_owner) <> '' and length(repository_owner) <= 255)),
    constraint project_repositories_branch_check
        check (default_branch is null
               or (btrim(default_branch) <> '' and length(default_branch) <= 255)),
    constraint project_repositories_metadata_object_check
        check (jsonb_typeof(metadata) = 'object'),
    -- A backstop rather than the bound (0040's reasoning): synveda-types
    -- refuses over 8 KiB of the caller's encoding, and Postgres renders jsonb
    -- with its own spacing.
    constraint project_repositories_metadata_size_check
        check (octet_length(metadata::text) <= 8192),
    constraint project_repositories_updated_check check (updated_at >= created_at)
);

-- One attachment per (project, repository). Case-insensitive, because
-- `Acme/payments` and `acme/payments` are one repository on every host this
-- product knows about — the stored URI keeps the capitalisation the first
-- caller used, and the second caller gets a conflict rather than a duplicate.
create unique index project_repositories_unique
    on project_repositories (tenant_id, project_id, lower(canonical_uri));

create index project_repositories_by_project
    on project_repositories (tenant_id, project_id, created_at);

-- ── Idempotency records ──────────────────────────────────────────────────
--
-- ADR-0071 decision 6. Creation is not naturally idempotent: a client that
-- times out cannot tell a request that never arrived from one that arrived
-- and answered into a dead socket, and retrying the second makes two
-- workspaces. The key is the client's claim that "this is that request
-- again", and this table is what makes the claim answerable.
--
-- Keyed by the **subject** as well as the tenant, so one client's key is not
-- another's: an idempotency key is a token a client mints for itself, and two
-- clients that happen to mint the same one must not collide.
--
-- The digest is what stops a key from being reused for a *different* request.
-- Same key + same digest replays the original resource; same key + different
-- digest is a conflict, because the alternative is silently answering a
-- request the caller did not make with the resource from one they did.

create table idempotency_records (
    tenant_id       uuid        not null,
    subject         text        not null,
    operation       text        not null,
    idempotency_key text        not null,
    -- BLAKE3-256 of the canonical request (route + path parameters + body).
    request_digest  bytea       not null,
    -- The resource the first attempt produced. Untyped on purpose: the
    -- operation column says which table it names, and a per-operation foreign
    -- key would make this table grow a column per governed noun.
    resource_id     uuid        not null,
    created_at      timestamptz not null default now(),

    constraint idempotency_records_pk
        primary key (tenant_id, subject, operation, idempotency_key),
    constraint idempotency_records_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint idempotency_records_key_check
        check (length(idempotency_key) between 1 and 255),
    constraint idempotency_records_subject_check
        check (length(subject) between 1 and 255),
    constraint idempotency_records_operation_check
        check (length(operation) between 1 and 64),
    constraint idempotency_records_digest_check
        check (octet_length(request_digest) = 32)
);

-- Pruning is by age, and the retention plane owns it; the index is here so
-- that when it arrives it is a range scan rather than a table scan.
create index idempotency_records_by_age on idempotency_records (created_at);

-- ── Immutability ─────────────────────────────────────────────────────────
--
-- 0040's reasoning, applied to the subtypes. Forced RLS already stops the
-- application role from moving a row into another tenant; this covers the
-- owner role, which is what migrations, break-glass psql and a restore run
-- as, and which RLS does not constrain.
--
-- The revision clause is the other half of ADR-0071 decision 5. A precondition
-- is only worth anything if the number it names cannot be rewound or skipped,
-- so the trigger — not the store — is what makes every accepted update step it
-- forward by exactly one.

create function synveda_subtype_immutable_columns() returns trigger
language plpgsql
as $$
begin
    if new.id <> old.id then
        raise exception '%.id is immutable (CPR-4, ADR-0071)', tg_table_name;
    end if;
    if new.tenant_id <> old.tenant_id then
        raise exception
            '% % cannot move across tenants (% to %) (CPR-4, ADR-0071)',
            tg_table_name, old.id, old.tenant_id, new.tenant_id;
    end if;
    if new.scope_id <> old.scope_id then
        raise exception
            '% % cannot change the scope it owns (CPR-4, ADR-0071)',
            tg_table_name, old.id;
    end if;
    if new.slug <> old.slug then
        raise exception
            '%.slug is immutable; an update changes display_name (CPR-4, ADR-0071)',
            tg_table_name;
    end if;
    if new.created_at <> old.created_at
        or new.created_by is distinct from old.created_by then
        raise exception '% provenance is immutable (CPR-4, ADR-0071)', tg_table_name;
    end if;
    if new.revision <> old.revision + 1 then
        raise exception
            '%.revision steps forward by one; % to % (CPR-4, ADR-0071)',
            tg_table_name, old.revision, new.revision;
    end if;
    return new;
end
$$;

create trigger workspaces_immutable_columns
    before update on workspaces
    for each row execute function synveda_subtype_immutable_columns();

-- A project additionally never changes workspace: moving one would move its
-- scope across a policy boundary, which is a create and an archive rather
-- than an update.
create function synveda_projects_immutable_workspace() returns trigger
language plpgsql
as $$
begin
    if new.workspace_id <> old.workspace_id
        or new.workspace_scope_id <> old.workspace_scope_id then
        raise exception
            'project % cannot move between workspaces (CPR-4, ADR-0071)', old.id;
    end if;
    return new;
end
$$;

create trigger projects_immutable_columns
    before update on projects
    for each row execute function synveda_subtype_immutable_columns();

create trigger projects_immutable_workspace
    before update on projects
    for each row execute function synveda_projects_immutable_workspace();

-- A repository attachment's identity never changes either: re-pointing a
-- project at a different repository is a detach and an attach, so that the
-- audit chain records two acts rather than one row quietly meaning something
-- else. There is no revision on this table — the API has no update verb for
-- it — so this trigger is its own rather than the shared one.
create function synveda_repository_immutable_columns() returns trigger
language plpgsql
as $$
begin
    if new.id <> old.id
        or new.tenant_id <> old.tenant_id
        or new.project_id <> old.project_id
        or new.canonical_uri <> old.canonical_uri
        or new.provider <> old.provider then
        raise exception
            'project_repositories identity is immutable; detach and attach instead (CPR-4, ADR-0071)';
    end if;
    if new.created_at <> old.created_at
        or new.created_by is distinct from old.created_by then
        raise exception 'repository provenance is immutable (CPR-4, ADR-0071)';
    end if;
    return new;
end
$$;

create trigger project_repositories_immutable_columns
    before update on project_repositories
    for each row execute function synveda_repository_immutable_columns();

-- ── Tenant isolation ─────────────────────────────────────────────────────
--
-- Tenant-scoped tables get forced RLS + policy + least-privilege grants in
-- the same migration (ADR-0009 structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it).
--
-- No DELETE on `workspaces` or `projects`: retiring one is a status
-- transition, because a workspace is what sessions, versions and audit events
-- name. `project_repositories` **does** get DELETE — detaching a repository
-- is the API's own verb, and the row asserts a present fact about a project
-- rather than a historical one; what it was is in the chain.
-- `idempotency_records` gets DELETE for the pruning the retention plane will
-- do, and no UPDATE: a record of what a key already produced is not
-- something a later request revises.

grant select, insert, update on workspaces to synveda_app;
grant select, insert, update on projects to synveda_app;
grant select, insert, update, delete on project_repositories to synveda_app;
grant select, insert, delete on idempotency_records to synveda_app;

alter table workspaces enable row level security;
alter table workspaces force row level security;
alter table projects enable row level security;
alter table projects force row level security;
alter table project_repositories enable row level security;
alter table project_repositories force row level security;
alter table idempotency_records enable row level security;
alter table idempotency_records force row level security;

create policy workspaces_tenant_isolation on workspaces
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy projects_tenant_isolation on projects
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy project_repositories_tenant_isolation on project_repositories
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy idempotency_records_tenant_isolation on idempotency_records
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
