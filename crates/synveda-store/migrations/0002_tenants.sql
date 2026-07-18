-- TEN-1: the tenant table — the root isolation boundary (ADR-0008).
--
-- Deliberately plain (not bitemporal): TEN-1 needs admit + resolve only.
-- Lifecycle transitions (suspend/export/delete) are TEN-5; row-level
-- security keyed on tenant_id across all tenant-scoped tables is TEN-2, and
-- referential enforcement from records.tenant_id lands there with it.

create table tenants (
    id         uuid        not null,
    slug       text        not null,
    name       text        not null,
    status     text        not null,
    created_at timestamptz not null default now(),

    constraint tenants_pk primary key (id),
    constraint tenants_slug_unique unique (slug),
    -- Lowercase, hyphenated, starts alphanumeric, at most 63 chars: safe in
    -- URLs, hostnames, and CLI arguments without quoting.
    constraint tenants_slug_check check (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'),
    constraint tenants_status_check check (status in ('active', 'suspended'))
);
