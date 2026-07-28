-- AUD-2: the indexes the audit query surface reads through (ADR-0045
-- decision 7).
--
-- `audit_log` has carried exactly one index since AUD-1 — its primary key
-- `(tenant_id, seq)` — because exactly two things read it: `verify`'s
-- forward walk and `tail`'s newest-first page, and both are seq-ordered.
-- Every query AUD-2 adds is not: by actor, by action, by time window, and
-- by a record id buried inside a payload, over a table that is append-only
-- and never pruned before TEN-5.
--
-- **This migration adds indexes and not one column, and that is the whole
-- design.** The table is hashed: `audit_log.hash` covers a canonical
-- serialisation of the row's content (ADR-0019 decision 2), so a new
-- column *inside* that form invalidates every row written since AUD-1, and
-- a new column *outside* it is an audit field the audit chain does not
-- protect — one an app-role attacker could set at will while `verify`
-- stayed green. An index changes no byte any hash covers. That asymmetry
-- is why "add a `scope_id` column and index it" is a rejected option in
-- ADR-0045 rather than the obvious implementation, and why answering
-- "which events concern this scope" is a query problem here and not a
-- schema one.
--
-- All four indexes are tenant-leading, so no plan reaches another tenant's
-- rows even before the RLS policy applies — isolation is the backstop
-- (ADR-0009), never the first line.

create index audit_log_tenant_time_idx
    on audit_log (tenant_id, occurred_at, seq);

create index audit_log_tenant_action_idx
    on audit_log (tenant_id, action, seq);

create index audit_log_tenant_actor_idx
    on audit_log (tenant_id, actor_subject, seq);

-- The disclosure index: "who was served record X" is containment against
-- an entry array, which is what `jsonb_path_ops` is for — it indexes only
-- `@>` and is markedly smaller than the default `jsonb_ops`, and `@>` is
-- the only operator the query uses.
--
-- `btree_gin` buys the tenant-leading column: a GIN index cannot take a
-- uuid key without it, and a payload-only index would be one an
-- unqualified containment scan could enter before the tenant predicate
-- applied. It is a trusted contrib extension; `vector` set the precedent
-- for creating one from a migration (0015).
--
-- **Partial, to the two actions that record a disclosure.** ADR-0045
-- decision 4 defines the answer as `context.injected` and
-- `context.recalled` — the events that record a record being *served* to
-- someone — and those are a small minority of a busy chain, which keeps
-- the largest index on the largest table from being indexed over every
-- payload the product writes. Other actions name record ids too
-- (`memory.superseded`, `memory.expired`, `vedaflow.channel.published`),
-- and none of them has a query yet: shipping an index variant before the
-- query that needs it is what CTX-1 refused (ADR-0024) and what migration
-- 0027 refused again. Widening the predicate is a reviewed diff and
-- rebuilds in place.

create extension if not exists btree_gin;

create index audit_log_disclosure_idx
    on audit_log using gin (tenant_id, payload jsonb_path_ops)
    where action in ('context.injected', 'context.recalled');
