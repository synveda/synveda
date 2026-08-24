#!/usr/bin/env sh
# CPR-17 acceptance demo: the generated public Knowledge API searches current
# immutable revisions, governs every read and write, and drives the Knowledge
# Browser without a raw-record translation seam (ADR-0082).
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
DEMO_DB="synveda_cpr17_demo_$$"

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

echo "==> CPR-17 public Knowledge API, search and hard-cut acceptance"
cargo test -p synveda-gateway --test knowledge_lifecycle \
  public_knowledge_api_is_current_governed_paginated_and_tenant_safe \
  -- --exact --nocapture

echo "==> CPR-17 generated OpenAPI and Knowledge Browser contract"
cargo test -p synveda-gateway --test openapi -- --nocapture
pnpm --filter @synveda/console test

knowledge=$($COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DB" -c \
  "select count(*) from knowledge_items")
old_records=$($COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DB" -c \
  "select count(*) from records")
if [ "$knowledge" -eq 0 ] || [ "$old_records" -ne 0 ]; then
  echo "CPR-17 noun cutover failed: Knowledge=$knowledge old records=$old_records" >&2
  exit 1
fi

echo ""
echo "CPR-17 browser: $knowledge Knowledge items, zero old records; acceptance criteria pass."
