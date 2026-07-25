#!/usr/bin/env sh
# GRPH-4 acceptance demo: the AGE traversal gate (ADR-0029).
# AC (docs/backlog/GRPH-4.md): report with traversal benchmarks at 1M/10M
# edges; go/no-go criteria recorded as an ADR.
#
# The load-bearing part of this demo is the ORDER. ADR-0029 was written
# and its six criteria fixed before the harness existed, because a spike
# that picks its thresholds after seeing the numbers has measured nothing.
# So this script shows the criteria first, then runs the benchmark that
# is judged by them, then shows the verdict — and the git history is what
# proves the sequence rather than the script's own say-so.
#
# Flow: the pre-registered criteria -> a scratch database -> 1M and 10M
# edge graphs seeded into AGE and into a plain adjacency table ->
# traversal, write, and tenant-cost measurements on both -> the three
# query-shape traps -> the transactionality and RLS checks -> the verdict.
#
# Needs postgres from the dev compose. No IdP, no model server, no
# gateway: this measures a database extension, not the product.
# Takes about three minutes, most of it seeding 11M edges.
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
SPIKE_DB=${SPIKE_DB:-grph4_demo_$$}

spike_psql() {
  $COMPOSE exec -T postgres psql -v ON_ERROR_STOP=1 -U synveda -d "$1" -tAc "$2"
}

cleanup() {
  $COMPOSE exec -T postgres psql -U synveda -d synveda \
    -c "drop database if exists $SPIKE_DB with (force)" >/dev/null 2>&1 || true
  return 0
}
trap cleanup EXIT INT TERM

echo "==> the criteria, recorded before anything was measured (ADR-0029)"
sed -n '/^### The criteria/,/^### The verdict rule/p' \
  docs/adr/adr-0029-graph-traversal-gate.md | sed '$d' | sed 's/^/    /'

echo "==> a scratch database with AGE ($SPIKE_DB)"
$COMPOSE up --detach --wait postgres >/dev/null
$COMPOSE exec -T postgres psql -v ON_ERROR_STOP=1 -U synveda -d synveda \
  -c "create database $SPIKE_DB" >/dev/null
spike_psql "$SPIKE_DB" "create extension if not exists age" >/dev/null
echo "    AGE $(spike_psql "$SPIKE_DB" "select extversion from pg_extension where extname='age'")" \
     "on $(spike_psql "$SPIKE_DB" "select current_setting('server_version')")"

echo
echo "==> the benchmark: 1M and 10M edges, AGE against plain adjacency"
echo "    (seeding 11M edges takes a couple of minutes)"
DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$SPIKE_DB" \
SQLX_OFFLINE=true \
  cargo test -q -p synveda-store --test graph_spike -- --ignored --nocapture

echo "==> G6: does a cypher write roll back with its transaction, and does"
echo "    forced RLS reach graph data? (the tenant backstop, seed 2.2/TEN-2)"
$COMPOSE exec -T postgres psql -v ON_ERROR_STOP=1 -U synveda -d "$SPIKE_DB" <<'SQL' | sed 's/^/    /'
LOAD 'age'; SET search_path = ag_catalog, "$user", public;
SELECT create_graph('g6probe'); SELECT create_vlabel('g6probe','Entity');
INSERT INTO g6probe."Entity" (id, properties)
SELECT _graphid(_label_id('g6probe','Entity')::int, n),
       ('{"eid": ' || n || ', "tenant_id": "'
         || CASE WHEN n <= 50 THEN 'tenant-a' ELSE 'tenant-b' END || '"}')::agtype
FROM generate_series(1,100) n;
-- bulk loading through the label tables leaves the label sequence behind,
-- so the next cypher CREATE would collide on the primary key. Keeping the
-- sequences in step is the obligation the report records for GRPH-2.
SELECT setval('g6probe."Entity_id_seq"', 100);

\echo '-- a cypher CREATE inside BEGIN..ROLLBACK leaves nothing behind:'
BEGIN;
SELECT * FROM cypher('g6probe', $$ CREATE (:Entity {eid: 999999}) $$) AS (v agtype);
SELECT count(*) AS inside_txn FROM g6probe."Entity" WHERE properties @> '{"eid": 999999}';
ROLLBACK;
SELECT count(*) AS after_rollback FROM g6probe."Entity" WHERE properties @> '{"eid": 999999}';

\echo '-- forced RLS keyed to the TEN-2 GUC, honoured by cypher, failing closed:'
ALTER TABLE g6probe."Entity" ENABLE ROW LEVEL SECURITY;
ALTER TABLE g6probe."Entity" FORCE ROW LEVEL SECURITY;
CREATE POLICY g6_tenant ON g6probe."Entity"
  USING (properties -> '"tenant_id"'
         = ('"' || current_setting('synveda.tenant_id', true) || '"')::agtype);
DO $$ BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'synveda_app') THEN
    CREATE ROLE synveda_app NOSUPERUSER NOBYPASSRLS LOGIN;
  END IF;
END $$;
GRANT USAGE ON SCHEMA ag_catalog, g6probe TO synveda_app;
GRANT SELECT ON ALL TABLES IN SCHEMA ag_catalog, g6probe TO synveda_app;

BEGIN;
  SET LOCAL ROLE synveda_app; SET LOCAL search_path = ag_catalog, public;
  SELECT set_config('synveda.tenant_id', 'tenant-a', true) AS acting_as;
  SELECT count(*) AS tenant_a_sees FROM cypher('g6probe', $$ MATCH (a:Entity) RETURN a $$) AS (v agtype);
COMMIT;
BEGIN;
  SET LOCAL ROLE synveda_app; SET LOCAL search_path = ag_catalog, public;
  SELECT set_config('synveda.tenant_id', 'tenant-b', true) AS acting_as;
  SELECT count(*) AS tenant_b_sees FROM cypher('g6probe', $$ MATCH (a:Entity) RETURN a $$) AS (v agtype);
COMMIT;
BEGIN;
  SET LOCAL ROLE synveda_app; SET LOCAL search_path = ag_catalog, public;
  SELECT count(*) AS no_guc_sees FROM cypher('g6probe', $$ MATCH (a:Entity) RETURN a $$) AS (v agtype);
COMMIT;
SQL

echo
echo "==> G5: what AGE will not accept as a parameter — the reason a"
echo "    per-tenant graph name can only reach the statement as text"
$COMPOSE exec -T postgres psql -U synveda -d "$SPIKE_DB" 2>&1 <<'SQL' | grep -E "ERROR" | sed 's/^/    /'
LOAD 'age'; SET search_path = ag_catalog, "$user", public;
PREPARE by_name (text, agtype) AS
  SELECT * FROM cypher($1, $$ MATCH (a:Entity {eid: $seed}) RETURN a $$, $2) AS (v agtype);
PREPARE by_cast (text) AS
  SELECT * FROM cypher('g6probe', $$ MATCH (a:Entity {eid: $seed}) RETURN a $$, $1::agtype) AS (v agtype);
SQL

echo
echo "==> the verdict, against the criteria fixed before the run"
sed -n '/^## Verdict/,$p' docs/adr/adr-0029-graph-traversal-gate.md | sed 's/^/    /'

echo
echo "==> GRPH-4 done. Full report: docs/spikes/grph-4-age-traversal.md"
