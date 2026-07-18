-- FND-4: bitemporal base tables for memory records (ADR-0006).
--
-- Pattern: a current/history table pair per bitemporal entity. `records`
-- holds exactly the live versions; closed versions are archived into
-- `records_history` by the triggers below. Transaction time (tx_from/tx_to)
-- is written ONLY by triggers — application SQL never sets it and cannot
-- forge it. Valid time (valid_from/valid_to) is ordinary application data.
-- Open-ended bounds are NULL.
--
-- Structural rule: a migration that alters `records` must make the identical
-- change to `records_history`, to `records_versions`, and to the explicit
-- column lists in the archive trigger functions, in the same migration.

create table records (
    id          uuid        not null,
    tenant_id   uuid        not null,
    scope_id    uuid        not null,
    owner_id    uuid        not null,
    kind        text        not null,
    class       text        not null,
    content     text        not null,
    sensitivity text        not null,
    provenance  jsonb       not null,
    valid_from  timestamptz not null,
    valid_to    timestamptz,          -- null = no known end of validity
    tx_from     timestamptz not null, -- written by triggers only
    tx_to       timestamptz,          -- null on every current row, by check

    constraint records_pk primary key (id),
    constraint records_kind_check
        check (kind in ('derived', 'pinned')),
    constraint records_class_check
        check (class in ('fact', 'decision', 'preference', 'procedure', 'entity', 'episode')),
    constraint records_sensitivity_check
        check (sensitivity in ('public', 'internal', 'confidential', 'restricted')),
    constraint records_valid_period_check
        check (valid_to is null or valid_from < valid_to),
    constraint records_tx_to_is_null_check
        check (tx_to is null)
);

create table records_history (
    id          uuid        not null,
    tenant_id   uuid        not null,
    scope_id    uuid        not null,
    owner_id    uuid        not null,
    kind        text        not null,
    class       text        not null,
    content     text        not null,
    sensitivity text        not null,
    provenance  jsonb       not null,
    valid_from  timestamptz not null,
    valid_to    timestamptz,
    tx_from     timestamptz not null,
    tx_to       timestamptz not null, -- history rows are always closed

    constraint records_history_pk primary key (id, tx_from),
    constraint records_history_kind_check
        check (kind in ('derived', 'pinned')),
    constraint records_history_class_check
        check (class in ('fact', 'decision', 'preference', 'procedure', 'entity', 'episode')),
    constraint records_history_sensitivity_check
        check (sensitivity in ('public', 'internal', 'confidential', 'restricted')),
    constraint records_history_valid_period_check
        check (valid_to is null or valid_from < valid_to),
    constraint records_history_tx_period_check
        check (tx_from < tx_to)
);

-- Every version the database has ever known — the as-of query surface.
create view records_versions as
select id, tenant_id, scope_id, owner_id, kind, class, content, sensitivity,
       provenance, valid_from, valid_to, tx_from, tx_to
from records
union all
select id, tenant_id, scope_id, owner_id, kind, class, content, sensitivity,
       provenance, valid_from, valid_to, tx_from, tx_to
from records_history;

-- ── Transaction-time maintenance ─────────────────────────────────────────────

create function records_tx_insert() returns trigger
language plpgsql as $$
begin
    -- Transaction time is server truth; anything the application supplied
    -- is overwritten.
    new.tx_from := now();
    new.tx_to := null;
    return new;
end;
$$;

create function records_tx_update() returns trigger
language plpgsql as $$
begin
    if new.id is distinct from old.id then
        raise exception 'records.id is immutable; delete and re-insert instead';
    end if;
    if new.tenant_id is distinct from old.tenant_id then
        raise exception 'records.tenant_id is immutable; a record never changes tenant';
    end if;
    if old.tx_from > now() then
        -- A concurrent transaction with a later clock already committed this
        -- version; closing it "before it began" would record a
        -- negative-length period. Fail like a serialization conflict.
        raise exception 'transaction-time clock anomaly on records.id=%: version began at %, now() is %; retry',
            old.id, old.tx_from, now()
            using errcode = 'serialization_failure';
    end if;
    if old.tx_from < now() then
        insert into records_history
            (id, tenant_id, scope_id, owner_id, kind, class, content,
             sensitivity, provenance, valid_from, valid_to, tx_from, tx_to)
        values
            (old.id, old.tenant_id, old.scope_id, old.owner_id, old.kind,
             old.class, old.content, old.sensitivity, old.provenance,
             old.valid_from, old.valid_to, old.tx_from, now());
    end if;
    -- When old.tx_from = now() the replaced version's transaction period is
    -- empty — it never existed in transaction time — so no history row.
    new.tx_from := now();
    new.tx_to := null;
    return new;
end;
$$;

create function records_tx_delete() returns trigger
language plpgsql as $$
begin
    if old.tx_from > now() then
        raise exception 'transaction-time clock anomaly on records.id=%: version began at %, now() is %; retry',
            old.id, old.tx_from, now()
            using errcode = 'serialization_failure';
    end if;
    if old.tx_from < now() then
        insert into records_history
            (id, tenant_id, scope_id, owner_id, kind, class, content,
             sensitivity, provenance, valid_from, valid_to, tx_from, tx_to)
        values
            (old.id, old.tenant_id, old.scope_id, old.owner_id, old.kind,
             old.class, old.content, old.sensitivity, old.provenance,
             old.valid_from, old.valid_to, old.tx_from, now());
    end if;
    return old;
end;
$$;

create trigger records_tx_insert before insert on records
    for each row execute function records_tx_insert();
create trigger records_tx_update before update on records
    for each row execute function records_tx_update();
create trigger records_tx_delete before delete on records
    for each row execute function records_tx_delete();

-- ── Guard rails ──────────────────────────────────────────────────────────────
-- Not a security boundary (a superuser can drop triggers) — defence in depth
-- against application bugs, complementary to the AUD-1 hash chain.

create function records_history_append_only() returns trigger
language plpgsql as $$
begin
    raise exception 'records_history is append-only (% attempted)', tg_op;
end;
$$;

create trigger records_history_append_only
    before update or delete on records_history
    for each row execute function records_history_append_only();
create trigger records_history_no_truncate
    before truncate on records_history
    for each statement execute function records_history_append_only();

create function records_block_truncate() returns trigger
language plpgsql as $$
begin
    raise exception 'truncate on records would bypass history archiving; delete rows instead';
end;
$$;

create trigger records_no_truncate
    before truncate on records
    for each statement execute function records_block_truncate();
