-- AUTH-3: service identities (ADR-0018).
--
-- A service identity is an identity row of kind 'service', placed exactly
-- like a user: a personal user-kind scope node under its anchor. Reusing
-- the identities table keeps subject uniqueness, the placement FK/unique
-- pair, quarantine derivation, and RLS coverage unchanged (ADR-0018
-- decision 2 / option 5).

alter table identities
    add column kind text not null default 'user'
        constraint identities_kind_check check (kind in ('user', 'service'));

-- Revoking a service identity deletes its row (and its personal node via
-- the hierarchy plane). User rows still gain no update path here —
-- movers/leavers stay AUTH-4/5's feature (migration 0007's note stands);
-- the store deletes only kind = 'service' rows.
grant delete on identities to synveda_app;
