#!/usr/bin/env sh
# GRPH-1 acceptance demo: the multi-graph schema (ADR-0043).
# AC (docs/backlog/GRPH-1.md, amended 2026-07-27): an edge written through
# the store API reads back through the traversal API with its kind,
# endpoints and validity intact; a supersession closes the prior edge's
# window with both versions readable as-of; the shipped statements' plans
# contain no sequential scan over the edge table.
#
# The demo is arranged so each clause is shown twice — once through the
# product's own API, and once in raw SQL that never touches it. That
# duplication is the point. ADR-0043 decision 3 claims the bitemporal
# behaviour is a property of the SCHEMA rather than of the code that
# writes it, and the only way to show that is to bypass the code: the
# psql section below closes a window with a plain UPDATE and the history
# row appears anyway, because the trigger put it there.
#
# Flow: a scratch database -> the AC suite over the real store API ->
# the same three properties in raw SQL, including the plans -> the
# traversal re-measured on the schema as built (ADR-0043 decision 15),
# against the spike's own numbers.
#
# Needs postgres from the dev compose. No IdP, no model server, no
# gateway: this is a storage feature, and nothing above the store reads
# the graph until GRPH-2/GRPH-3. Takes about a minute, most of it
# seeding a million edges for the measurement.
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
$COMPOSE up --detach --wait postgres

# A scratch database per run — the FLOW-4/EVAL-1 discipline. Here it also
# buys the measurement a table whose planner statistics describe this
# run's corpus and nothing else.
GRPH1_DB=grph1_$$
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d synveda \
  -c "create database $GRPH1_DB" >/dev/null
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d "$GRPH1_DB" \
  -c "create extension if not exists vector;
      create extension if not exists age;
      create extension if not exists pgmq" >/dev/null

cleanup() {
  $COMPOSE exec -T postgres psql -U synveda -d synveda \
    -c "drop database if exists $GRPH1_DB with (force)" >/dev/null 2>&1 || true
  return 0
}
trap cleanup EXIT INT TERM

DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$GRPH1_DB"
export DATABASE_URL
# The scratch database is empty until the test harness migrates it at
# runtime, so the compile-time query checks read `.sqlx` instead — the
# GRPH-4 demo's shape, for the same reason.
SQLX_OFFLINE=true
export SQLX_OFFLINE
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG

psql_demo() {
  $COMPOSE exec -T postgres psql -v ON_ERROR_STOP=1 -U synveda -d "$GRPH1_DB" "$@"
}

echo
echo "==> the engine question, settled before any of this was built"
echo "    (ADR-0043 decision 1; the evidence is GRPH-4's own spike)"
sed -n '/^\*\*The graph layer is indexed adjacency/,/^Decisions, specifically/p' \
  docs/adr/adr-0043-graph-schema.md | sed '$d' | sed 's/^/    /'
echo "    AGE is installed in this database and called by nothing:"
printf '    cypher() calls in the whole workspace: '
grep -rl "cypher(" crates/*/src/ 2>/dev/null | wc -l | tr -d ' '

echo
echo "==> the AC suite, over the real store API"
echo "    (migrations run on first connect, so the schema arrives here too)"
cargo test -q -p synveda-store --test graph -- --nocapture 2>&1 \
  | grep -Ev "^\s*$" | sed 's/^/    /'

echo
echo "==> the same three properties, in SQL that never touches that API"
psql_demo <<'SQL' | sed 's/^/    /'
\pset border 2
\echo '-- one claim, written by hand: Ada reports to Grace, from 2026-01-01'
insert into tenants (id, slug, name, status)
values ('00000000-0000-7000-8000-000000000001', 'grph1-demo', 'GRPH-1 demo', 'active');
insert into graph_vertices (id, tenant_id, graph, kind, key, label) values
  ('00000000-0000-7000-8000-00000000000a', '00000000-0000-7000-8000-000000000001',
   'entity', 'person', 'ada', 'Ada'),
  ('00000000-0000-7000-8000-00000000000b', '00000000-0000-7000-8000-000000000001',
   'entity', 'person', 'grace', 'Grace'),
  ('00000000-0000-7000-8000-00000000000c', '00000000-0000-7000-8000-000000000001',
   'entity', 'person', 'alan', 'Alan');
insert into graph_edges
  (id, tenant_id, graph, kind, src_id, dst_id, method, confidence_permille,
   valid_from, tx_from)
values
  ('00000000-0000-7000-8000-0000000000e1', '00000000-0000-7000-8000-000000000001',
   'entity', 'reports_to', '00000000-0000-7000-8000-00000000000a',
   '00000000-0000-7000-8000-00000000000b', 'demo', 900, '2026-01-01', now());

\echo
\echo '-- the claim as stored. tx_from was written by the trigger, not by us:'
select e.kind, src.label as src, dst.label as dst, e.valid_from::date, e.valid_to,
       (e.tx_from is not null) as tx_stamped, e.tx_to
from graph_edges e
join graph_vertices src on src.id = e.src_id
join graph_vertices dst on dst.id = e.dst_id
where e.id = '00000000-0000-7000-8000-0000000000e1';

\echo '-- Ada moves. A plain UPDATE closes the window; nothing tells the'
\echo '-- database this is a supersession, and history appears regardless:'
update graph_edges set valid_to = '2026-02-01'
where id = '00000000-0000-7000-8000-0000000000e1';
insert into graph_edges
  (id, tenant_id, graph, kind, src_id, dst_id, method, confidence_permille,
   valid_from, tx_from)
values
  ('00000000-0000-7000-8000-0000000000e2', '00000000-0000-7000-8000-000000000001',
   'entity', 'reports_to', '00000000-0000-7000-8000-00000000000a',
   '00000000-0000-7000-8000-00000000000c', 'demo', 900, '2026-02-01', now());

\echo
\echo '-- every version the database has ever known of that one claim:'
select valid_from::date, valid_to::date,
       case when tx_to is null then 'current' else 'archived' end as tx_period,
       dst.label as reports_to
from graph_edges_versions v
join graph_vertices dst on dst.id = v.dst_id
where v.id = '00000000-0000-7000-8000-0000000000e1'
order by tx_from;
SQL

echo "    -- and the trigger refuses the rewrite that would have lost it:"
echo "    -- (changing an endpoint is a different claim, not a new version)"
$COMPOSE exec -T postgres psql -U synveda -d "$GRPH1_DB" 2>&1 <<'SQL' | grep -E "ERROR" | sed 's/^/    /'
update graph_edges set dst_id = '00000000-0000-7000-8000-00000000000b'
where id = '00000000-0000-7000-8000-0000000000e2';
SQL

echo
echo "==> where the plans above came from: the AC suite explained the"
echo "    statements this crate SHIPS, found in src/graph.rs by the marker"
echo "    each carries — not a copy of them, which is the failure mode a"
echo "    plan guard has to survive (ADR-0043 decision 9)"
grep -n -- "-- shipped-traversal:" crates/synveda-store/src/graph.rs | sed 's/^/    /'
echo "    Every leg of all four planned as an Index Scan; the same test"
echo "    fails the build on a Seq Scan over either edge table."

echo
echo "==> decision 15: the traversal re-measured on the schema as built —"
echo "    RLS predicate applied, bitemporal columns, composite foreign keys"
echo "    — at the spike's own shape, and printed beside the spike's numbers"
cargo test -q -p synveda-store --test graph traversal_medians \
  -- --ignored --nocapture 2>&1 | grep -Ev "^\s*$" | sed 's/^/    /'

echo
echo "==> GRPH-1 done. Schema: crates/synveda-store/migrations/0026_graph_schema.sql"
echo "    API: crates/synveda-store/src/graph.rs · Decision: docs/adr/adr-0043-graph-schema.md"
