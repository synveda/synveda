#!/usr/bin/env sh
# CPR-18 acceptance demo: session events freeze into restart-safe capture
# batches, extraction creates reviewable candidates only, and every accepted
# candidate reaches Knowledge through the PDP and VedaFlow (ADR-0083).
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
DEMO_DB="synveda_cpr18_demo_$$"

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

echo "==> CPR-18 public capture, review, provenance and isolation path"
cargo test -p synveda-gateway --test capture_api -- --nocapture

echo "==> CPR-18 extractor contract and candidate-quality fixture"
cargo test -p synveda-ingest --lib extraction
cargo test -p synveda-ingest --test extraction_precision

echo "==> CPR-18 forced-RLS and audit vocabulary gates"
cargo test -p synveda-store --test rls every_tenant_scoped_table_is_covered_and_forced -- --nocapture
cargo test -p synveda-audit

retired_table=$($COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DB" -c \
  "select (to_regclass('records') is not null)::int")
candidates=$($COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DB" -c \
  "select count(*) from capture_candidates")
changes=$($COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DB" -c \
  "select count(*) from knowledge_changes")
unexpected_extensions=$($COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DB" -c \
  "select coalesce(string_agg(extname, ',' order by extname), '') from pg_extension where extname not in ('plpgsql', 'btree_gin', 'vector')")
if [ "$retired_table" -ne 0 ] || [ "$candidates" -eq 0 ] || [ "$changes" -eq 0 ] || [ -n "$unexpected_extensions" ]; then
  echo "CPR-18 cutover failed: retired_table=$retired_table candidates=$candidates changes=$changes unexpected_extensions=$unexpected_extensions" >&2
  exit 1
fi

echo ""
echo "CPR-18 capture: $candidates reviewable candidates, $changes governed changes, and no retired Record table or queue extension; acceptance criteria pass."
