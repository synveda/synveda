#!/usr/bin/env sh
# CPR-22 MVP acceptance: one public-API PulseBoard loop from session evidence
# through reviewable capture and VedaFlow Knowledge into clean teammate context,
# explicit supersession and the generated Context Inspector contract.
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
DEMO_DB="synveda_cpr22_demo_$$"

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

echo "==> CPR-22 PulseBoard cross-session team Knowledge loop"
cargo test -p synveda-gateway --test capture_api \
  pulseboard_cross_session_team_knowledge_loop_is_governed_end_to_end \
  -- --exact --nocapture

echo "==> CPR-22 New Learnings and Context Inspector rendering"
pnpm --filter @synveda/console test

sessions=$($COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DB" -c \
  "select count(*) from sessions")
candidates=$($COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DB" -c \
  "select count(*) from capture_candidates")
changes=$($COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DB" -c \
  "select count(*) from knowledge_changes")
active=$($COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DB" -c \
  "select count(*) from knowledge_items where lifecycle_state = 'active'")
superseded=$($COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DB" -c \
  "select count(*) from knowledge_items where lifecycle_state = 'superseded'")
runs=$($COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DB" -c \
  "select count(*) from session_context_runs")
selections=$($COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DB" -c \
  "select count(*) from context_selections")
retired_table=$($COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DB" -c \
  "select (to_regclass('records') is not null)::int")

if [ "$sessions" -ne 3 ] || [ "$candidates" -ne 5 ] || [ "$changes" -ne 4 ] || \
   [ "$active" -ne 3 ] || [ "$superseded" -ne 1 ] || [ "$runs" -ne 2 ] || \
   [ "$selections" -eq 0 ] || [ "$retired_table" -ne 0 ]; then
  echo "CPR-22 loop failed: sessions=$sessions candidates=$candidates changes=$changes active=$active superseded=$superseded runs=$runs selections=$selections retired_table=$retired_table" >&2
  exit 1
fi

echo ""
echo "CPR-22 MVP: $sessions clean sessions, $candidates reviewed candidates, $active current Knowledge items, $superseded explicit supersession, $runs explainable context runs, $selections immutable selections and no retired Record table; acceptance criteria pass."
