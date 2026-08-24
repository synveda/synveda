#!/usr/bin/env sh
# CPR-16 acceptance demo: every Knowledge command opens one VedaFlow change,
# policy decides auto-apply versus review, revisions stay immutable, merge and
# supersession are explicit, and forget leaves content-free durable evidence
# (ADR-0081).
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
DEMO_DB="synveda_cpr16_demo_$$"

cleanup() {
  $COMPOSE exec -T postgres dropdb -U synveda --if-exists --force "$DEMO_DB" >/dev/null 2>&1 || true
}
trap cleanup EXIT HUP INT TERM

$COMPOSE up --detach --wait postgres
$COMPOSE exec -T postgres createdb -U synveda "$DEMO_DB"
$COMPOSE exec -T postgres psql -v ON_ERROR_STOP=1 -U synveda -d "$DEMO_DB" -c \
  "create extension if not exists vector; create extension if not exists age; create extension if not exists pgmq" \
  >/dev/null

DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$DEMO_DB"
SQLX_OFFLINE=true
export DATABASE_URL SQLX_OFFLINE

echo "==> CPR-16 governed Knowledge lifecycle"
cargo test -p synveda-gateway --test knowledge_lifecycle -- --nocapture

echo "==> CPR-16 VedaFlow/PDP vocabulary and forced-RLS inventory"
cargo test -p synveda-policy --test approvals --test packs --test pdp
cargo test -p synveda-store --test rls every_tenant_scoped_table_is_covered_and_forced -- --nocapture

old_records=$($COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DB" -c \
  "select count(*) from records")
changes=$($COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DB" -c \
  "select count(*) from knowledge_changes")
if [ "$old_records" -ne 0 ] || [ "$changes" -eq 0 ]; then
  echo "CPR-16 cutoff failed: records=$old_records Knowledge changes=$changes" >&2
  exit 1
fi

echo ""
echo "CPR-16 lifecycle: $changes governed changes, zero old records; acceptance criteria pass."
