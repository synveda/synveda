-- FLOW-7: releasing a pin (ADR-0036 decision 8).
--
-- A pin is a ref — `pin/{asset}/{channel}` at a scope, pointing at the commit
-- that channel serves — so pinning needs no table: `vedaflow_refs` has been
-- deliberately generic since FLOW-1, and ADR-0031 decision 1 names FLOW-7's
-- pins as one of the things it was left generic for. The commit foreign key
-- already makes a pin unable to name a commit that does not exist, and the
-- primary key already makes a scope hold at most one pin per channel.
--
-- What a pin needs that no ref has needed before is a way to *stop*.
-- Migration 0018 withheld DELETE on this table for a reason that is still
-- right — "a ref is a standing channel pointer ... disposal is TEN-5's" — and
-- a pin is the first ref that is a standing decision rather than a pointer
-- into recorded history. A decision that cannot be reversed is not one this
-- product should write.
--
-- So the grant arrives narrowed twice over, because widening a deletion power
-- on the table that holds every channel pointer is the kind of thing that
-- gets noticed years later:
--
--   * a RESTRICTIVE delete policy, so the application role can never delete a
--     channel ref — the statement is legal and matches nothing;
--   * a before-delete trigger, so anyone who bypasses RLS (a superuser, or an
--     owner who turned it off) gets an exception naming the rule instead of a
--     quiet success.
--
-- That is the same split migration 0018 made for the history tables: RLS is
-- what the product runs under, the trigger is what an attacker has to disable
-- first, and `synveda_vedaflow::verify` is what makes the step visible
-- afterwards.

-- Refs move; channel refs still never disappear.
grant delete on vedaflow_refs to synveda_app;

-- Restrictive: ANDed with vedaflow_refs_tenant_isolation rather than ORed, so
-- a delete must satisfy both. `for delete` leaves select/insert/update exactly
-- as they were.
create policy vedaflow_refs_only_pins_are_deletable on vedaflow_refs
    as restrictive
    for delete
    using (name like 'pin/%');

create function synveda_vedaflow_refs_delete_guard() returns trigger
language plpgsql
as $$
begin
    if old.name not like 'pin/%' then
        raise exception
            'vedaflow_refs.% is a channel pointer; only pins (pin/*) may be deleted (FLOW-7, ADR-0036)',
            old.name;
    end if;
    return old;
end
$$;

create trigger vedaflow_refs_channel_pointers_are_permanent
    before delete on vedaflow_refs
    for each row execute function synveda_vedaflow_refs_delete_guard();

-- A truncate would remove every channel pointer in one statement, which is
-- what the row trigger exists to prevent one row at a time. TRUNCATE is not
-- granted to synveda_app; this covers the owner path, on the migration 0018
-- pattern.
create trigger vedaflow_refs_no_truncate
    before truncate on vedaflow_refs
    execute function synveda_vedaflow_immutable();
