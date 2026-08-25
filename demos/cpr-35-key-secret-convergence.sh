#!/usr/bin/env sh
# CPR-35 acceptance demo: stable tenant-secret identity, fail-closed runtime
# references, durable DEK re-encryption and the Knowledge-native hard-cut
# export (ADR-0094).
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
DEMO_DB="synveda_cpr35_demo_$$"

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

echo "==> CPR-35 stable secret identity, AAD isolation and durable re-encryption"
cargo test -p synveda-store --test keys -- --test-threads=1

echo "==> CPR-35 complete Knowledge export projection"
cargo test -p synveda-store --test knowledge \
  sealed_export_projection_contains_complete_knowledge_history_and_provenance -- --nocapture

echo "==> CPR-35 live Tool reference admission, application, rotation and removal"
cargo test -p synveda-gateway --test tools \
  stable_tool_secret_references_fail_closed_rotate_without_rewriting_versions_and_can_be_removed \
  -- --test-threads=1

echo "==> CPR-35 revoked/corrupt directory credentials suppress fallback"
cargo test -p synveda-gateway --test directory_sync \
  an_unusable_stable_credential_never_falls_back_to_deployment_configuration \
  -- --test-threads=1

echo "==> CPR-35 forced-RLS completeness and old archive refusal"
cargo test -p synveda-store --test rls every_tenant_scoped_table_is_covered_and_forced -- --nocapture
cargo test -p synveda-cli keys::tests::the_record_era_archive_magic_is_not_a_compatibility_reader

echo ""
echo "CPR-35 secret convergence: stable references, fail-closed consumers, durable rotation and Knowledge export pass."
