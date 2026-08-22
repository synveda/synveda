-- CPR-10: the session ledger (ADR-0068 decision 5, ADR-0076).
--
-- What an agent does becomes something this product can name. Three tables:
-- `sessions` (one run), `session_events` (immutable things that happened
-- inside it) and `session_context_runs` (acts of composing context for it).
--
-- There is deliberately **no fourth table for a transcript**. A timeline is a
-- projection over these three, merged and ordered at read time
-- (ADR-0076 decision 9). A materialised transcript would be a second copy of
-- what `session_events` already holds, and the two would disagree the first
-- time one of them was written and the other was not.
--
-- ── This is not `observe_events.session_id` ─────────────────────────────
--
-- That column is an opaque correlation string on somebody else's table. It
-- means a run an agent only *read* in does not exist — which ADPT-8 measured
-- against a headless Claude Code run: three runs, three `inject.ok`, zero
-- `observe.done`. Nothing here reads it and nothing there reads these
-- (ADR-0068 decision 3: no bridge, no synchronisation). Migration 0012 and
-- its column leave with the observe re-cut, which is the next prompt.
--
-- ── The governed scope is derived, never submitted ──────────────────────
--
-- A session names a workspace and optionally a project. The scope it is
-- *decided at* is the project's scope when there is a project and the
-- workspace's when there is not — and that is a row-local fact here rather
-- than a service's discipline. Three columns carry it: `workspace_scope_id`
-- and `project_scope_id`, each pinned to its owner by a composite foreign
-- key, and `scope_id`, held equal to `coalesce(project_scope_id,
-- workspace_scope_id)` by a CHECK. `projects.workspace_scope_id` is the same
-- device one plane up (migration 0041) and exists for the same reason: a rule
-- that can be a row-local key should be one, because a rule that lives in a
-- function holds only for callers who went through that function.
--
-- Where each structural rule is enforced:
--
--   a project session's scope IS its project's   sessions_project_fk
--   a workspace session's scope IS its
--     workspace's                                sessions_workspace_fk
--   the anchor is one of those two, and which     sessions_anchor_check
--     one is decided by whether there is a           + sessions_project_shape_check
--     project
--   the project is in the named workspace        sessions_project_fk (composite)
--   a repository is one of its project's         sessions_repository_fk (composite)
--                                                + sessions_repository_project_check
--   never crosses a tenant                       every FK carries tenant_id
--   a closed session never reopens               synveda_sessions_lifecycle
--   an event is never edited or deleted          no UPDATE/DELETE grant
--   one row per (session, client_event_id)       session_events_client_unique
--   sequence is gapless per session              session_events_sequence_unique
--                                                + the append's own read
--
-- ── Least privilege, and the two tables that get no UPDATE at all ───────
--
-- `session_events` and `session_context_runs` are append-only: the
-- application role holds SELECT and INSERT and nothing else, so "immutable"
-- is a privilege rather than a discipline. `sessions` gets UPDATE, because a
-- session has a lifecycle and an event append stamps `last_observed_at` —
-- and the trigger below is what keeps that UPDATE from being able to rewrite
-- anything else.
--
-- Nothing gets DELETE. A session is what events, candidates, knowledge
-- provenance and audit events name; disposal belongs to the retention plane,
-- which owns the whole tenant's material and its schedule.

-- ── Keys the session anchor needs ────────────────────────────────────────
--
-- 0041's convention: a composite foreign key needs a matching unique key on
-- its target, and the key is created in the migration that exists for it so a
-- reader meets the reason in the same file.

-- (tenant_id, id, workspace_id, scope_id) — the target of sessions_project_fk.
-- One key rather than two, so "this project is in that workspace" and "this
-- project owns that scope" are a single referential fact.
create unique index projects_tenant_id_workspace_scope_unique
    on projects (tenant_id, id, workspace_id, scope_id);

-- (tenant_id, project_id, id) — the target of sessions_repository_fk, so a
-- session's repository is one of *its project's* repositories rather than any
-- repository in the tenant.
create unique index project_repositories_tenant_project_id_unique
    on project_repositories (tenant_id, project_id, id);

-- ── Sessions ─────────────────────────────────────────────────────────────

create table sessions (
    id                     uuid        not null,
    tenant_id              uuid        not null,
    -- The workspace the run happened in. Required: a run happens somewhere.
    workspace_id           uuid        not null,
    -- The project, when the run was against one.
    project_id             uuid,
    -- The two owned scopes, denormalised so the anchor rule below can be a
    -- row-local CHECK over two row-local foreign keys. 0041 carries
    -- `projects.workspace_scope_id` for exactly this reason.
    workspace_scope_id     uuid        not null,
    project_scope_id       uuid,
    -- The governed scope this session is decided at. Derived — the project's
    -- when there is a project, the workspace's when there is not — and held
    -- to that by sessions_anchor_check. Never sent by a client: a client that
    -- could name the scope could name one its workspace is not in.
    scope_id               uuid        not null,
    -- The token subject that opened it. Text, not a foreign key into
    -- `identities`: the PDP's principal is (tenant, subject), and a grant —
    -- or a session — must be able to precede an identity row (ADR-0072).
    principal_id           text        not null,
    -- The agent client as it names itself. A grammar, not a list: seed §2
    -- principle 6 forbids a core change per harness, so `claude-code`,
    -- `zed`, `mcp` and `com.example.agent` are all sayable and a sentence is
    -- not.
    client_name            text        not null,
    client_version         text,
    -- A stable id for this *installation* of that client, when it has one:
    -- what tells two machines running the same client apart. Opaque here.
    client_installation_id text,
    -- The harness's own id for this run. Never an identity in this product
    -- and nothing joins on it — it exists so a stateless hook holding only
    -- the harness's id can find the session it already opened instead of
    -- minting a second one.
    external_session_id    text,
    agent_name             text,
    model_name             text,
    -- The repository the run was against — one of *this project's*, held so
    -- by a composite key rather than by a convention.
    repository_id          uuid,
    branch                 text,
    task_summary           text,
    status                 text        not null default 'active',
    started_at             timestamptz not null default now(),
    ended_at               timestamptz,
    -- The `occurred_at` of the newest event appended. The one column an
    -- event append writes, and what "recently active" sorts by.
    last_observed_at       timestamptz,
    -- The client's labelling bag. Never copied into an audit payload — the
    -- chain records that metadata was present and how large it was, and
    -- nothing else, because an agent's environment is where credentials live
    -- and this is the field a harness would put an environment in.
    metadata               jsonb       not null default '{}'::jsonb,
    created_at             timestamptz not null default now(),
    updated_at             timestamptz not null default now(),

    constraint sessions_pk primary key (id),
    constraint sessions_tenant_fk foreign key (tenant_id) references tenants (id),
    -- The composite target session_events and session_context_runs need.
    constraint sessions_tenant_id_unique unique (tenant_id, id),

    -- The workspace, and the scope it owns.
    constraint sessions_workspace_fk
        foreign key (tenant_id, workspace_id, workspace_scope_id)
        references workspaces (tenant_id, id, scope_id),
    -- The project, the workspace it is in, and the scope it owns — one key,
    -- so a session cannot name workspace A and a project in workspace B.
    -- Vacuous when `project_id` is null (MATCH SIMPLE), which is exactly the
    -- workspace-anchored shape; the CHECK below is what forbids a half-set
    -- pair from reaching that vacuity.
    constraint sessions_project_fk
        foreign key (tenant_id, project_id, workspace_id, project_scope_id)
        references projects (tenant_id, id, workspace_id, scope_id),
    constraint sessions_project_shape_check
        check ((project_id is null) = (project_scope_id is null)),
    -- **The anchor rule**, as a row-local fact rather than a service's
    -- discipline: the scope a session is decided at is the project's when
    -- there is a project and the workspace's when there is not.
    constraint sessions_anchor_check
        check (scope_id = coalesce(project_scope_id, workspace_scope_id)),

    -- A repository belongs to a project, so a session that names one names a
    -- project too — and the composite key holds it to that project's.
    constraint sessions_repository_project_check
        check (repository_id is null or project_id is not null),
    constraint sessions_repository_fk
        foreign key (tenant_id, project_id, repository_id)
        references project_repositories (tenant_id, project_id, id),

    constraint sessions_principal_check
        check (btrim(principal_id) <> '' and length(principal_id) <= 255),
    -- The same grammar `synveda_types::session::validate_client_name`
    -- refuses outside of, so a row written by anything holding a connection
    -- still carries a label rather than a sentence.
    constraint sessions_client_name_check
        check (client_name ~ '^[a-z0-9][a-z0-9.-]{0,63}$'),
    constraint sessions_client_version_check
        check (client_version is null
               or (btrim(client_version) <> '' and length(client_version) <= 200)),
    constraint sessions_client_installation_check
        check (client_installation_id is null
               or (btrim(client_installation_id) <> ''
                   and length(client_installation_id) <= 200)),
    constraint sessions_external_id_check
        check (external_session_id is null
               or (btrim(external_session_id) <> ''
                   and length(external_session_id) <= 200)),
    constraint sessions_agent_name_check
        check (agent_name is null
               or (btrim(agent_name) <> '' and length(agent_name) <= 200)),
    constraint sessions_model_name_check
        check (model_name is null
               or (btrim(model_name) <> '' and length(model_name) <= 200)),
    constraint sessions_branch_check
        check (branch is null
               or (btrim(branch) <> '' and length(branch) <= 200)),
    constraint sessions_task_summary_check
        check (task_summary is null
               or (btrim(task_summary) <> '' and length(task_summary) <= 2000)),
    constraint sessions_status_check
        check (status in ('active', 'ending', 'ended', 'abandoned', 'failed')),
    -- A session is closed exactly when it has an end time. Both directions,
    -- so neither an `ended` row with no timestamp nor an `active` one with a
    -- stale timestamp is representable.
    constraint sessions_ended_shape_check
        check ((status in ('ended', 'abandoned', 'failed')) = (ended_at is not null)),
    constraint sessions_ended_order_check
        check (ended_at is null or ended_at >= started_at),
    constraint sessions_metadata_object_check
        check (jsonb_typeof(metadata) = 'object'),
    -- A backstop rather than the bound (0041's reasoning): synveda-types
    -- refuses over 8 KiB of the caller's encoding, and Postgres renders
    -- jsonb with its own spacing.
    constraint sessions_metadata_size_check
        check (octet_length(metadata::text) <= 8192),
    constraint sessions_updated_check check (updated_at >= created_at)
);

-- The listing's own index: newest first, at or under a scope. Every session
-- listing filters by scope subtree and orders by `started_at desc`, so the
-- index carries the order rather than the sort.
create index sessions_by_scope on sessions (tenant_id, scope_id, started_at desc);

-- "What is running in this workspace" and "what have I been doing", which are
-- the two questions the console and a client actually ask.
create index sessions_by_workspace on sessions (tenant_id, workspace_id, started_at desc);
create index sessions_by_principal on sessions (tenant_id, principal_id, started_at desc);

-- One harness run, one session. Partial, because most sessions carry no
-- external id and NULLs must not collide; keyed by the principal as well as
-- the client so one caller's harness id is not another's — the same reasoning
-- `idempotency_records` is keyed by subject for (ADR-0071 decision 6).
create unique index sessions_external_unique
    on sessions (tenant_id, principal_id, client_name, external_session_id)
    where external_session_id is not null;

-- ── Session events ───────────────────────────────────────────────────────
--
-- Append-only, ordered, idempotent. Three properties, three mechanisms:
-- no UPDATE or DELETE grant; a server-assigned `sequence` unique per session;
-- and `client_event_id` unique per session, which is what makes a redelivered
-- batch append nothing twice.
--
-- `sequence` is the server's rather than the client's because it is what a
-- timeline orders by when two events share a millisecond, and a client that
-- could choose it could interleave itself into another client's ordering of
-- the same session.
--
-- `occurred_at` and `received_at` are both kept and they are different facts:
-- a buffered adapter delivers an hour late, and only one of the two is a
-- clock this deployment controls.

create table session_events (
    id                   uuid        not null,
    tenant_id            uuid        not null,
    session_id           uuid        not null,
    -- The closed vocabulary in `synveda_types::session::SessionEventType`.
    -- A CHECK rather than an enum type: adding a value is then a migration
    -- that alters a constraint, which is reviewable, rather than an
    -- `ALTER TYPE` that cannot run in a transaction with other statements.
    event_type           text        not null,
    -- The client's statement about the shape of `payload`. Not the server's
    -- about the row: an adapter ships separately from the gateway and will
    -- one day have two shapes in flight at once.
    event_schema_version integer     not null default 1,
    -- The idempotency key. Required — an event without one cannot be
    -- redelivered safely, and redelivery is the ordinary case for a hook
    -- that ran while the network was down.
    client_event_id      text        not null,
    -- Position in the session, assigned on append.
    sequence             bigint      not null,
    occurred_at          timestamptz not null,
    received_at          timestamptz not null default now(),
    payload              jsonb       not null default '{}'::jsonb,
    -- BLAKE3-256 of the canonical payload, hex. The server's, computed on
    -- append: what it is for is telling two events sharing a
    -- `client_event_id` apart, and a digest the client supplied could not.
    payload_hash         text        not null,

    constraint session_events_pk primary key (id),
    constraint session_events_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint session_events_session_fk
        foreign key (tenant_id, session_id) references sessions (tenant_id, id),
    constraint session_events_type_check
        check (event_type in (
            'session.started', 'session.ended',
            'message.user', 'message.assistant',
            'tool.invoked', 'tool.result',
            'file.read', 'file.changed',
            'command.executed',
            'skill.loaded',
            'context.requested',
            'adapter.warning'
        )),
    constraint session_events_schema_version_check
        check (event_schema_version between 1 and 1000),
    constraint session_events_client_id_check
        check (btrim(client_event_id) <> '' and length(client_event_id) <= 200),
    constraint session_events_sequence_check check (sequence >= 1),
    -- One row per client event id, per session: the idempotency gate.
    constraint session_events_client_unique
        unique (tenant_id, session_id, client_event_id),
    -- One row per position: what makes the append's `max(sequence) + 1` safe
    -- under concurrency — the loser of a race violates this and retries
    -- rather than writing a duplicate position.
    constraint session_events_sequence_unique
        unique (tenant_id, session_id, sequence),
    constraint session_events_payload_object_check
        check (jsonb_typeof(payload) = 'object'),
    constraint session_events_payload_size_check
        check (octet_length(payload::text) <= 65536),
    constraint session_events_payload_hash_check
        check (payload_hash ~ '^[0-9a-f]{64}$')
);

-- The timeline's read: one session, in order.
create index session_events_by_session
    on session_events (tenant_id, session_id, sequence);

-- ── Context runs ─────────────────────────────────────────────────────────
--
-- One act of composing context for a session. Minimal by intent (ADR-0076
-- decision 7): an identity, what was asked, and the block that came back.
-- Prompt 18 adds the explainability — which scopes were considered, which
-- were denied, why each entry made the cut — **without changing the
-- endpoint**, which is why the endpoint's shape is decided now and its depth
-- later.
--
-- `rendered` holds composed material, which is governed content, which is why
-- this table is tenant-bound and forced like everything else that holds any.

create table session_context_runs (
    id            uuid        not null,
    tenant_id     uuid        not null,
    session_id    uuid        not null,
    -- The scope it was anchored at — the session's, denormalised so the
    -- listing's per-row decision does not need the join.
    scope_id      uuid        not null,
    principal_id  text        not null,
    query         text,
    rendered      text        not null,
    -- BLAKE3 over the composed entries, hex: the block's identity, the same
    -- value the rendered watermark line carries.
    block_hash    text        not null,
    tokens        integer     not null,
    budget_tokens integer     not null,
    entry_count   integer     not null,
    -- Which retrieval legs degraded, if any. A text array rather than a
    -- jsonb bag: it is a closed set of two words today and a client renders
    -- it as a sentence.
    degraded      text[]      not null default '{}',
    created_at    timestamptz not null default now(),

    constraint session_context_runs_pk primary key (id),
    constraint session_context_runs_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint session_context_runs_session_fk
        foreign key (tenant_id, session_id) references sessions (tenant_id, id),
    constraint session_context_runs_scope_fk
        foreign key (tenant_id, scope_id) references scopes (tenant_id, id),
    constraint session_context_runs_principal_check
        check (btrim(principal_id) <> '' and length(principal_id) <= 255),
    constraint session_context_runs_query_check
        check (query is null or (btrim(query) <> '' and length(query) <= 4096)),
    constraint session_context_runs_block_hash_check
        check (block_hash ~ '^[0-9a-f]{1,128}$'),
    constraint session_context_runs_tokens_check
        check (tokens >= 0 and budget_tokens >= 0 and entry_count >= 0)
);

create index session_context_runs_by_session
    on session_context_runs (tenant_id, session_id, created_at);

-- ── Lifecycle and immutability ───────────────────────────────────────────
--
-- 0041's reasoning, applied to a row whose only legal mutation is closing.
-- Forced RLS already stops the application role from moving a row into
-- another tenant; this covers the owner role, which is what migrations,
-- break-glass psql and a restore run as, and which RLS does not constrain.
--
-- The status clause is the other half of ADR-0076's lifecycle: forward only,
-- `active → ending → {ended, abandoned, failed}`, and a closed session never
-- reopens and never changes how it closed. The same rule is in
-- `SessionStatus::may_become`, and it is in both places deliberately — a rule
-- that lives only in a function holds only for callers who went through that
-- function.

create function synveda_sessions_lifecycle() returns trigger
language plpgsql
as $$
begin
    if new.id <> old.id then
        raise exception 'sessions.id is immutable (CPR-10, ADR-0076)';
    end if;
    if new.tenant_id <> old.tenant_id then
        raise exception
            'session % cannot move across tenants (% to %) (CPR-10, ADR-0076)',
            old.id, old.tenant_id, new.tenant_id;
    end if;
    if new.workspace_id <> old.workspace_id
        or new.project_id is distinct from old.project_id
        or new.workspace_scope_id <> old.workspace_scope_id
        or new.project_scope_id is distinct from old.project_scope_id
        or new.scope_id <> old.scope_id then
        raise exception
            'session % cannot move between workspaces, projects or scopes (CPR-10, ADR-0076)',
            old.id;
    end if;
    if new.principal_id <> old.principal_id
        or new.client_name <> old.client_name
        or new.started_at <> old.started_at
        or new.created_at <> old.created_at then
        raise exception 'session % provenance is immutable (CPR-10, ADR-0076)', old.id;
    end if;
    if new.status <> old.status
        and not (
            (old.status = 'active' and new.status = 'ending')
            or (old.status in ('active', 'ending')
                and new.status in ('ended', 'abandoned', 'failed'))
        ) then
        raise exception
            'session % cannot go from % to %; a closed session never reopens (CPR-10, ADR-0076)',
            old.id, old.status, new.status;
    end if;
    -- `last_observed_at` never moves backwards: it is the newest event's
    -- instant, and an out-of-order delivery must not rewind it.
    if new.last_observed_at is not null
        and old.last_observed_at is not null
        and new.last_observed_at < old.last_observed_at then
        raise exception
            'session %.last_observed_at never moves backwards (CPR-10, ADR-0076)', old.id;
    end if;
    return new;
end
$$;

create trigger sessions_lifecycle
    before update on sessions
    for each row execute function synveda_sessions_lifecycle();

-- ── Tenant isolation ─────────────────────────────────────────────────────
--
-- Tenant-scoped tables get forced RLS + policy + least-privilege grants in
-- the same migration (ADR-0009 structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it).

grant select, insert, update on sessions to synveda_app;
grant select, insert on session_events to synveda_app;
grant select, insert on session_context_runs to synveda_app;

alter table sessions enable row level security;
alter table sessions force row level security;
alter table session_events enable row level security;
alter table session_events force row level security;
alter table session_context_runs enable row level security;
alter table session_context_runs force row level security;

create policy sessions_tenant_isolation on sessions
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy session_events_tenant_isolation on session_events
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy session_context_runs_tenant_isolation on session_context_runs
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
