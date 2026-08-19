-- CPR-6: whose scope a `principal`-shaped scope is (ADR-0073 decision 2).
--
-- CPR-3 gave the scope tree a `principal` shape and CPR-5 made a principal a
-- **token subject** rather than an `identities` row (ADR-0072 decision 4), but
-- nothing yet said *which* subject a principal-shaped scope belongs to. The
-- anchor resolver this feature adds has to start from "the authenticated
-- principal's own scope", and it must be able to find it without a convention.
--
-- A convention is what this column refuses. The alternatives were a slug
-- grammar (`principal-<hash>`, which every reader would have to parse and no
-- constraint would check) or the `attributes` bag (which ADR-0070 states is
-- never an authorisation input, and this *is* one). So it is a column, with
-- the two rules that make it trustworthy structural:
--
--   present exactly on a principal scope   CHECK scopes_principal_id_shape_check
--   at most one scope per subject          UNIQUE INDEX scopes_one_per_principal
--
-- and immutable, through the trigger CPR-3 already installed — extended here
-- rather than duplicated, because "a scope never changes whose it is" is the
-- same class of rule as "a scope never changes tenant".

alter table scopes
    add column principal_id text;

alter table scopes
    add constraint scopes_principal_id_shape_check
        check ((principal_id is null) <> (kind = 'principal'));

alter table scopes
    add constraint scopes_principal_id_check
        check (principal_id is null
               or (btrim(principal_id) <> '' and length(principal_id) <= 255));

-- One scope per subject per tenant. Partial rather than a plain UNIQUE so the
-- (many) null rows of every other shape are not asked to be distinct.
create unique index scopes_one_per_principal
    on scopes (tenant_id, principal_id)
    where principal_id is not null;

-- The resolver's own lookup: "the scope of this subject, in this tenant".
-- Covered by the unique index above; named here so the intent survives a
-- future change to it.

create or replace function synveda_scopes_immutable_columns() returns trigger
language plpgsql
as $$
begin
    if new.id <> old.id then
        raise exception 'scopes.id is immutable (CPR-3, ADR-0070)';
    end if;
    if new.tenant_id <> old.tenant_id then
        raise exception
            'scope % cannot move across tenants (% to %) (CPR-3, ADR-0070)',
            old.id, old.tenant_id, new.tenant_id;
    end if;
    if new.kind <> old.kind then
        raise exception
            'scopes.kind is immutable; scope % is a % (CPR-3, ADR-0070)',
            old.id, old.kind;
    end if;
    if new.slug <> old.slug then
        raise exception
            'scopes.slug is immutable; rename changes display_name (CPR-3, ADR-0070)';
    end if;
    -- A principal scope is somebody's own, and whose it is cannot be edited:
    -- re-pointing one would hand somebody else's private material to a new
    -- subject without a single grant row changing (CPR-6, ADR-0073).
    if new.principal_id is distinct from old.principal_id then
        raise exception
            'scopes.principal_id is immutable; scope % belongs to one subject (CPR-6, ADR-0073)',
            old.id;
    end if;
    if new.created_at <> old.created_at
        or new.created_by is distinct from old.created_by then
        raise exception 'scope provenance is immutable (CPR-3, ADR-0070)';
    end if;
    return new;
end
$$;
