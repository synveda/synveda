#!/usr/bin/env sh
# CPR-20 acceptance demo: public session context selects current immutable
# Knowledge, retains re-authorised explanations, records exact feedback and
# exposes ordinary versus diagnostic scoped query lenses (ADR-0084).
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
DEMO_DB="synveda_cpr20_demo_$$"

cleanup() {
  $COMPOSE exec -T postgres dropdb -U synveda --if-exists --force "$DEMO_DB" >/dev/null 2>&1 || true
}
trap cleanup EXIT HUP INT TERM

$COMPOSE up --detach --wait postgres
$COMPOSE exec -T postgres createdb -U synveda "$DEMO_DB"
$COMPOSE exec -T postgres psql -v ON_ERROR_STOP=1 -U synveda -d "$DEMO_DB" -c \
  "create extension if not exists vector; create extension if not exists btree_gin" \
  >/dev/null

DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$DEMO_DB"
SQLX_OFFLINE=true
export DATABASE_URL SQLX_OFFLINE

echo "==> CPR-20 public Knowledge planning, trace, query and feedback"
cargo test -p synveda-gateway --test context_runs -- --nocapture

echo "==> CPR-20 Knowledge-backed audit query and immutable trace isolation"
cargo test -p synveda-gateway --test audit_query -- --nocapture
cargo test -p synveda-store --test rls \
  the_database_owner_cannot_rewrite_context_trace_history -- --exact --nocapture
cargo test -p synveda-store --test rls \
  every_tenant_scoped_table_is_covered_and_forced -- --exact --nocapture

echo "==> CPR-20 generated contract and public CLI/MCP query clients"
cargo test -p synveda-gateway --test openapi -- --nocapture
cargo test -p synveda-cli recall::tests -- --nocapture
cargo test -p synveda-cli mcp::tests -- --nocapture

knowledge=$($COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DB" -c \
  "select count(*) from knowledge_items")
runs=$($COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DB" -c \
  "select count(*) from session_context_runs")
selections=$($COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DB" -c \
  "select count(*) from context_selections")
feedback=$($COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DB" -c \
  "select count(*) from context_feedback")
retired_table=$($COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DB" -c \
  "select (to_regclass('records') is not null)::int")
if [ "$knowledge" -eq 0 ] || [ "$runs" -eq 0 ] || [ "$selections" -eq 0 ] || \
   [ "$feedback" -eq 0 ] || [ "$retired_table" -ne 0 ]; then
  echo "CPR-20 cutover failed: Knowledge=$knowledge runs=$runs selections=$selections feedback=$feedback retired_table=$retired_table" >&2
  exit 1
fi

echo ""
echo "CPR-20 context: $knowledge Knowledge items, $runs plans, $selections immutable selections, $feedback feedback rows and no retired Record table; acceptance criteria pass."
