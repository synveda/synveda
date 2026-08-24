#!/usr/bin/env sh
# CPR-15 acceptance demo: stable Knowledge identities, immutable content
# revisions, bitemporal current heads, normalised provenance, explicit
# relations and forced-RLS isolation (ADR-0080).
#
# This package intentionally adds no application route: CPR-16 first wraps
# mutations in VedaFlow and the PDP. The demo therefore exercises the bounded
# persistence acceptance surface and never inserts a competing record or a
# record-to-Knowledge bridge.
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
DEMO_DB="synveda_cpr15_demo_$$"

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

echo "==> CPR-15 immutable Knowledge aggregate acceptance"
cargo test -p synveda-store --test knowledge -- --nocapture

echo "==> CPR-15 forced-RLS and security-invoker completeness"
cargo test -p synveda-store --test rls every_tenant_scoped_table_is_covered_and_forced -- --nocapture

echo ""
echo "CPR-15 Knowledge aggregate: acceptance criteria pass."
