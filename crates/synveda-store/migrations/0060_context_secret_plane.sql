-- CPR-35: stable tenant-secret references and durable DEK re-encryption jobs
-- (ADR-0094).
--
-- This is a pre-1.0 hard cut. The TEN-4 table was keyed by a mutable name and
-- its AAD used that name. There is no honest SQL translation to a UUID-bound
-- envelope because PostgreSQL has neither the deployment KEK nor plaintext.
-- Refuse a database carrying one and give the one supported transition.

do $$
begin
    if exists (select 1 from tenant_secrets limit 1) then
        raise exception 'the context secret-plane cut cannot translate name-bound tenant secrets; run `synveda reset --database --force`'
            using errcode = 'P0001';
    end if;
end
$$;

drop table tenant_secrets;

create table tenant_secrets (
    id             uuid        not null,
    tenant_id      uuid        not null,
    scope_id       uuid        not null,
    kind           text        not null,
    label          text        not null,
    provider       text,
    state          text        not null default 'active',
    value_revision bigint      not null default 1,
    key_version    integer,
    sealed         bytea,
    created_at     timestamptz not null default now(),
    rotated_at     timestamptz not null default now(),
    updated_at     timestamptz not null default now(),
    revoked_at     timestamptz,

    constraint tenant_secrets_pk primary key (tenant_id, id),
    constraint tenant_secrets_id_unique unique (id),
    constraint tenant_secrets_tenant_fk foreign key (tenant_id) references tenants (id),
    constraint tenant_secrets_scope_fk foreign key (tenant_id, scope_id)
        references scopes (tenant_id, id),
    constraint tenant_secrets_key_fk foreign key (tenant_id, key_version)
        references tenant_keys (tenant_id, version),
    constraint tenant_secrets_label_unique unique (tenant_id, kind, label),
    constraint tenant_secrets_kind_check
        check (kind in ('directory', 'tool_server', 'model_provider', 'import_export')),
    constraint tenant_secrets_label_check
        check (label ~ '^[a-z][a-z0-9]*(\.[a-z][a-z0-9_-]*)*$'
               and length(label) <= 128),
    constraint tenant_secrets_provider_check
        check (provider is null or
               (provider ~ '^[a-z][a-z0-9_-]*$' and length(provider) <= 64)),
    constraint tenant_secrets_state_check check (state in ('active', 'revoked')),
    constraint tenant_secrets_revision_check check (value_revision > 0),
    constraint tenant_secrets_shape_check check (
        (state = 'active' and sealed is not null and key_version > 0 and revoked_at is null)
        or
        (state = 'revoked' and sealed is null and key_version is null and revoked_at is not null)
    ),
    -- 34 header + 16 tag + a plaintext bounded like the credentials it holds.
    constraint tenant_secrets_sealed_check
        check (sealed is null or octet_length(sealed) between 51 and 65586)
);

create index tenant_secrets_scope_state
    on tenant_secrets (tenant_id, scope_id, kind, state, id);
create index tenant_secrets_key_generation
    on tenant_secrets (tenant_id, key_version, id)
    where state = 'active';

create function synveda_tenant_secret_transition_guard() returns trigger
language plpgsql
as $$
begin
    if new.id <> old.id
       or new.tenant_id <> old.tenant_id
       or new.scope_id <> old.scope_id
       or new.kind <> old.kind
       or new.label <> old.label
       or new.provider is distinct from old.provider
       or new.created_at <> old.created_at then
        raise exception 'tenant-secret identity and credential-free ownership metadata are immutable'
            using errcode = '23514';
    end if;

    if new.value_revision = old.value_revision then
        -- A DEK re-encryption changes only envelope/generation/update time.
        if old.state <> 'active' or new.state <> 'active'
           or new.key_version = old.key_version
           or new.sealed = old.sealed
           or new.rotated_at <> old.rotated_at
           or new.revoked_at is distinct from old.revoked_at then
            raise exception 'same-revision tenant-secret updates are DEK re-encryption only'
                using errcode = '23514';
        end if;
    elsif new.value_revision = old.value_revision + 1 then
        -- A logical rotate, revoke or reactivation advances exactly once.
        if new.updated_at <= old.updated_at then
            raise exception 'tenant-secret logical transitions must advance updated_at'
                using errcode = '23514';
        end if;
    else
        raise exception 'tenant-secret value revisions advance exactly once'
            using errcode = '23514';
    end if;
    return new;
end
$$;

create trigger tenant_secret_transition_guard
before update on tenant_secrets
for each row execute function synveda_tenant_secret_transition_guard();

alter table tenant_secrets enable row level security;
alter table tenant_secrets force row level security;
create policy tenant_secrets_tenant_isolation on tenant_secrets
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
grant select, insert, update on tenant_secrets to synveda_app;

-- A key rotation owns a durable retry address. Old generations are retained;
-- this job advances only database-owned active secret envelopes.
create table tenant_secret_reencryption_jobs (
    id                  uuid        not null,
    tenant_id           uuid        not null,
    from_key_version    integer     not null,
    to_key_version      integer     not null,
    state               text        not null default 'pending',
    secrets_total       bigint      not null default 0,
    secrets_reencrypted bigint      not null default 0,
    attempt             bigint      not null default 0,
    failure_code        text,
    created_at          timestamptz not null default now(),
    started_at          timestamptz,
    completed_at        timestamptz,
    updated_at          timestamptz not null default now(),

    constraint tenant_secret_reencrypt_jobs_pk primary key (tenant_id, id),
    constraint tenant_secret_reencrypt_jobs_id_unique unique (id),
    constraint tenant_secret_reencrypt_jobs_tenant_fk
        foreign key (tenant_id) references tenants (id),
    constraint tenant_secret_reencrypt_jobs_versions_check
        check (from_key_version > 0 and to_key_version > from_key_version),
    constraint tenant_secret_reencrypt_jobs_unique
        unique (tenant_id, from_key_version, to_key_version),
    constraint tenant_secret_reencrypt_jobs_state_check
        check (state in ('pending', 'running', 'completed', 'failed')),
    constraint tenant_secret_reencrypt_jobs_counts_check
        check (secrets_total >= 0 and secrets_reencrypted >= 0
               and secrets_reencrypted <= secrets_total and attempt >= 0),
    constraint tenant_secret_reencrypt_jobs_failure_check
        check (failure_code is null or
               (failure_code ~ '^[a-z][a-z0-9_]*$' and length(failure_code) <= 64)),
    constraint tenant_secret_reencrypt_jobs_shape_check check (
        (state = 'pending' and started_at is null and completed_at is null and failure_code is null)
        or (state = 'running' and started_at is not null and completed_at is null and failure_code is null)
        or (state = 'completed' and started_at is not null and completed_at is not null and failure_code is null
            and secrets_reencrypted = secrets_total)
        or (state = 'failed' and started_at is not null and completed_at is not null and failure_code is not null)
    )
);

create index tenant_secret_reencrypt_jobs_state
    on tenant_secret_reencryption_jobs (tenant_id, state, created_at, id);

alter table tenant_secret_reencryption_jobs enable row level security;
alter table tenant_secret_reencryption_jobs force row level security;
create policy tenant_secret_reencrypt_jobs_tenant_isolation
    on tenant_secret_reencryption_jobs
    for all
    using (tenant_id = synveda_current_tenant())
    with check (tenant_id = synveda_current_tenant());
grant select, insert, update on tenant_secret_reencryption_jobs to synveda_app;
