-- HIER-1: SSD-era planner cost model (ADR-0011).
--
-- The default random_page_cost (4.0) models spinning disks. At the HIER-1
-- AC fixture (10k nodes) it makes the planner prefer a full seq scan +
-- hash join over the closure-driven nested loop for descendant listings
-- (~1.5ms vs ~0.2ms measured), which breaks the <1ms acceptance
-- criterion — and the same misplan would tax every hot read path after
-- this one. Synveda targets SSD/NVMe-backed Postgres (tech plan §1.1);
-- 1.1 is the standard cost setting there.
--
-- Database-scoped so every deployment that applies migrations gets it,
-- SMB compose and enterprise Helm alike. Note the precedence: this
-- overrides postgresql.conf for this database; operators with exotic
-- storage can override it back per role or with a later ALTER DATABASE
-- (OPS-1/OPS-2 own the deployment profiles). Applies to sessions opened
-- after the migration.
do $$
begin
    execute format(
        'alter database %I set random_page_cost = 1.1', current_database()
    );
end
$$;
