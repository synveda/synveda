-- AUTH-4: the directory mirror, the provisioning credential, and the two
-- lifecycle columns identities has been waiting for since migration 0007
-- (ADR-0059).
--
-- Three shapes live here and they are deliberately not one:
--
-- 1. **The mirror** (scim_users, scim_groups, scim_group_members) is the
--    SCIM resource of record — what a conformant `GET /Users/{id}` must
--    answer with, for a person who may never log in. It is not the
--    product's notion of a person; `identities` is, and the reconciler
--    projects one onto the other (ADR-0059 decision 3).
--
-- 2. **The credential** (scim_credentials) is how Entra and Okta
--    authenticate, and it is a static secret because that is the only
--    thing a non-gallery Entra provisioning job can be configured to send
--    (decision 13). Custody rules follow console_sessions (migration
--    0034): the secret is never stored, only its SHA-256.
--
-- 3. **The lifecycle columns** on identities. Migration 0007 said update
--    and delete arrive with this feature; only `update` does. Nothing in
--    AUTH-4 deletes an identity, because every lifecycle end here is a
--    seal (decision 8) — a delete would be the one operation a retention
--    hold exists to prevent.
--
-- ── Why the mirror stores columns rather than a JSON blob ──────────────
--
-- A conformant server declares its schema and answers for what it
-- declares; attributes it never declared are not required to round-trip.
-- So the supported attributes are columns, `/Schemas` is *derived from*
-- them rather than aspirational, and an attribute we do not store is one
-- we do not claim. A `resource jsonb` would have made the endpoint's
-- promises unfalsifiable, and nothing in the product could filter on it
-- without building SQL from strings.
--
-- ── Why there is no `external_id` column on identities ─────────────────
--
-- ADR-0059 decision 4 words the anchor as `identities.external_id`. One
-- source of truth is better: the mirror holds `external_id`, the login
-- path joins through `scim_users.identity_id`, and the reconciler is the
-- only writer of that link. The rule is unchanged — a token subject binds
-- to the identity the directory anchored — and there is no second copy to
-- drift when a customer remaps the attribute.

-- ── The mirror ────────────────────────────────────────────────────────

-- The mirror's foreign keys are composite `(tenant_id, id)` pairs — the
-- same doctrine every other table here follows, so that a cross-tenant
-- link is unrepresentable rather than merely wrong — and a composite FK
-- needs a matching unique constraint to point at. `identities` has one
-- per subject and one per scope, and until now needed no third.
alter table identities
    add constraint identities_tenant_id_unique unique (tenant_id, id);

create table scim_users (
    -- The `id` a SCIM client stores and addresses this resource by,
    -- forever. Distinct from the identity it projects to: a rehire is a
    -- new identity and a new personal scope (ADR-0059 decision 12), and
    -- the client must still be able to fetch the old resource by the id
    -- it holds.
    id           uuid        not null,
    tenant_id    uuid        not null,

    -- The directory's own anchor. Nullable because RFC 7643 makes it
    -- optional, and *mutable* because it is the customer's attribute
    -- mapping rather than a protocol constant — which is exactly why
    -- reconciliation does not depend on it alone (ADR-0059 decision 4).
    external_id  text,

    -- Unique among live rows only. A departed row keeps the address it
    -- had, so a rehire is not 409'd by their own former self
    -- (decision 11).
    user_name    text        not null,

    -- The leaver signal. `false` here is what seals; see identities.status
    -- below for where that lands.
    active       boolean     not null default true,

    display_name text,
    given_name   text,
    family_name  text,
    -- `emails[type eq "work"].value` — the one multi-valued attribute
    -- both AC clients send and the one the reconciler matches on.
    work_email   text,

    -- The identity this row projects onto, once it has one. Null between
    -- a POST and its reconciliation, and for a row whose reconciliation
    -- placed nobody. No cascade: an identity is never deleted (see the
    -- grants below), so a dangling link is unreachable by construction.
    identity_id  uuid,

    -- `meta.version`, the ETag. A counter rather than a timestamp: two
    -- writes inside one clock tick must not share a version, and a
    -- provisioning agent uses this to decide whether to re-send.
    version      bigint      not null default 1,
    created_at   timestamptz not null default now(),
    updated_at   timestamptz not null default now(),

    constraint scim_users_pk primary key (id),
    constraint scim_users_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint scim_users_identity_fk
        foreign key (tenant_id, identity_id)
        references identities (tenant_id, id),
    -- One mirror row per identity: the projection is 1:1 in both
    -- directions, which is what makes "never two identities for one
    -- person" checkable rather than hoped for (ADR-0059 decision 4).
    constraint scim_users_identity_unique unique (tenant_id, identity_id),
    constraint scim_users_user_name_check
        check (length(user_name) between 1 and 255),
    constraint scim_users_external_id_check
        check (external_id is null or length(external_id) between 1 and 255),
    constraint scim_users_version_check check (version > 0)
);

-- The uniqueness RFC 7643 §4.1.1 requires of `userName`, over live rows
-- only (decision 11). Case-insensitive because SCIM says `userName` is
-- caseExact=false and because two accounts differing by case is a
-- collision a directory would never intend.
create unique index scim_users_user_name_live
    on scim_users (tenant_id, lower(user_name))
    where active;

-- Reconciliation's first match, and the filter both clients send.
create unique index scim_users_external_id_live
    on scim_users (tenant_id, external_id)
    where external_id is not null and active;

alter table scim_users
    add constraint scim_users_tenant_id_unique unique (tenant_id, id);

create table scim_groups (
    id           uuid        not null,
    tenant_id    uuid        not null,
    external_id  text,
    -- The name the AUTH-2 mapping resolver sees: matched against
    -- `group_mappings` and then the `synveda-{dept}-{team}` convention,
    -- unchanged (ADR-0013 decision 3, ADR-0059 decision 6).
    display_name text        not null,
    version      bigint      not null default 1,
    created_at   timestamptz not null default now(),
    updated_at   timestamptz not null default now(),

    constraint scim_groups_pk primary key (id),
    constraint scim_groups_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint scim_groups_display_name_unique unique (tenant_id, display_name),
    constraint scim_groups_display_name_check
        check (length(display_name) between 1 and 255),
    constraint scim_groups_external_id_check
        check (external_id is null or length(external_id) between 1 and 255),
    constraint scim_groups_version_check check (version > 0)
);

alter table scim_groups
    add constraint scim_groups_tenant_id_unique unique (tenant_id, id);

create table scim_group_members (
    tenant_id  uuid        not null,
    group_id   uuid        not null,
    user_id    uuid        not null,
    created_at timestamptz not null default now(),

    constraint scim_group_members_pk primary key (tenant_id, group_id, user_id),
    constraint scim_group_members_tenant_fk
        foreign key (tenant_id) references tenants (id),
    -- Membership is the group's content: deleting a group takes its
    -- membership with it, and neither is governed material.
    constraint scim_group_members_group_fk
        foreign key (tenant_id, group_id) references scim_groups (tenant_id, id)
        on delete cascade,
    constraint scim_group_members_user_fk
        foreign key (tenant_id, user_id) references scim_users (tenant_id, id)
        on delete cascade
);

-- The reconciler's own read: every group a user is in, to re-resolve
-- placement.
create index scim_group_members_by_user on scim_group_members (tenant_id, user_id);

-- ── The credential ────────────────────────────────────────────────────
--
-- The presented token is `synveda_scim_v1.<tenant-uuid>.<secret>` and
-- what is hashed is the **whole presented string**, so a secret pasted
-- behind another tenant's prefix hashes to nothing.
--
-- The prefix is why this table can be tenant-scoped at all, and it is a
-- correction to ADR-0059 decision 13's "no tenant-selecting parameter on
-- the wire" (amendment 1). A credential looked up before a tenant exists
-- would have to live on console_sessions' pre-scope side of RLS
-- (migration 0034) — but a SCIM credential *must* name a tenant, so that
-- table would have been tenant data with no tenant policy over it. Naming
-- the tenant in the token instead makes this the same shape as a bearer's
-- `tid` claim (TEN-1, ADR-0008): the caller names the tenant, the secret
-- proves it, the lookup runs inside that tenant's own RLS, and a
-- cross-tenant credential is not denied so much as unrepresentable.

create table scim_credentials (
    id            uuid        not null,
    tenant_id     uuid        not null,

    -- SHA-256 of the whole presented token. The token itself is shown
    -- once, at issuance, and is never stored — the AUD-1 threat model:
    -- an attacker with database credentials must not be able to mint a
    -- provisioning token from a dump.
    token_hash    bytea       not null,

    -- What an operator sees in `synveda scim token list`, so that
    -- rotation is a decision about a named thing rather than about a
    -- uuid.
    label         text        not null,

    -- Required, and capped by the issuing surface (AUTH-3's
    -- lifetime-cap doctrine, ADR-0018 decision 5). A provisioning
    -- credential that never expires is a key to a tenant's directory
    -- plane left under the mat.
    expires_at    timestamptz not null,

    -- Revocation is a stamp rather than a delete: which credential
    -- sealed which identity has to stay answerable from the chain after
    -- the credential is gone (ADR-0059 decision 14).
    revoked_at    timestamptz,

    -- Written on a coarse cadence, console_sessions' rule: a
    -- provisioning agent polls, and a row updated on every poll turns
    -- the directory plane's read path into a write path.
    last_used_at  timestamptz,

    created_at    timestamptz not null default now(),
    -- The subject that issued it. Text rather than an identity FK: the
    -- issuer may be a break-glass operator with no identity row, and
    -- this is a record of who rather than a link to what.
    created_by    text        not null,

    constraint scim_credentials_pk primary key (id),
    constraint scim_credentials_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint scim_credentials_hash_unique unique (token_hash),
    constraint scim_credentials_hash_check check (octet_length(token_hash) = 32),
    constraint scim_credentials_label_check check (length(label) between 1 and 128),
    constraint scim_credentials_created_by_check
        check (length(created_by) between 1 and 255),
    -- A credential that expired before it was created is a row nothing
    -- can ever authenticate with; refuse it here rather than let it sit
    -- there looking like a credential.
    constraint scim_credentials_expiry_check check (expires_at > created_at)
);

-- The authentication path: hash lookup inside the named tenant.
create index scim_credentials_by_tenant on scim_credentials (tenant_id);

-- ── The lifecycle columns ─────────────────────────────────────────────

alter table identities
    -- A directory can create a person on the day they are hired, which is
    -- before any token exists to carry a subject. The existing
    -- `identities_subject_unique (tenant_id, subject)` keeps doing the
    -- work it did: Postgres treats NULLs as distinct in a unique
    -- constraint, so many unbound rows coexist and the JIT first-login
    -- race still resolves on the non-null values (ADR-0059 decision 5).
    alter column subject drop not null,

    -- The seal, stored exactly once (ADR-0059 decision 7). ADR-0013
    -- decision 4 refused a `quarantined` column because placement already
    -- answered that question; departure is answered by nothing else, so
    -- it is stored — and the *scope's* sealed-ness derives from here
    -- through identities_scope_unique, never from a second column on
    -- hierarchy_nodes.
    add column status text not null default 'active'
        constraint identities_status_check check (status in ('active', 'departed')),
    add column departed_at timestamptz,

    -- The two columns say one thing or the schema refuses them.
    add constraint identities_departed_at_check
        check ((status = 'departed') = (departed_at is not null));

-- The seal's own read, and the reconciler's: every departed identity, and
-- the sealed-scope join the chain builder runs per user-kind node.
create index identities_sealed_scopes on identities (tenant_id, scope_id)
    where status = 'departed';

-- ── A stored pack's mover config ──────────────────────────────────────
--
-- Every other pack config has a column here (migration 0025's shape), and
-- a stored pack that could not express this one would be stuck with the
-- default on the one axis where the default is the *strict* reading
-- (ADR-0059 decision 10) — a tenant would have no way to say "our people
-- keep their notes when they change department".
alter table policy_packs add column mover jsonb;

-- ── Grants ────────────────────────────────────────────────────────────
--
-- `update` on identities is the grant migration 0007 promised this
-- feature and the only one it takes: a mover re-binds a subject and
-- re-points a scope, a leaver sets a status, and neither removes a row.
-- The `delete` grant from migration 0010 stays what it was — the store
-- keys it on `kind = 'service'` — and no user row becomes deletable here.
grant update on identities to synveda_app;

-- The mirror is the directory's copy: fully mutable, because the
-- directory is its author and a PATCH that could not remove a group
-- member would make this server unusable. Nothing here is governed
-- material (ADR-0059 decision 2).
grant select, insert, update, delete on scim_users to synveda_app;
grant select, insert, update, delete on scim_groups to synveda_app;
grant select, insert, update, delete on scim_group_members to synveda_app;

-- No DELETE on the credential: revocation is a stamp (see revoked_at),
-- and a provisioning credential that can be erased is one whose use
-- cannot be reconstructed. Migration 0033 withheld DELETE from
-- skill_reviews for the same reason and migration 0034 granted it to
-- console_sessions for the opposite one — a session must be destroyable
-- to be signed out of; a credential's history is the point.
grant select, insert, update on scim_credentials to synveda_app;

-- Tenant-scoped tables ⇒ forced RLS + policy + least-privilege grants in
-- the same migration (ADR-0009 structural rule; the completeness guard in
-- crates/synveda-store/tests/rls.rs enforces it).
alter table scim_users enable row level security;
alter table scim_users force row level security;
alter table scim_groups enable row level security;
alter table scim_groups force row level security;
alter table scim_group_members enable row level security;
alter table scim_group_members force row level security;
alter table scim_credentials enable row level security;
alter table scim_credentials force row level security;

create policy scim_users_tenant_isolation on scim_users
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy scim_groups_tenant_isolation on scim_groups
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy scim_group_members_tenant_isolation on scim_group_members
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());

create policy scim_credentials_tenant_isolation on scim_credentials
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
