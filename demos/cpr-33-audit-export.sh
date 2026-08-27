#!/usr/bin/env sh
# CPR-33 acceptance demo: typed context-platform audit questions and one
# frozen, tenant-bound chain export that verifies without database access.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr33" "CPR-33 — context-platform audit query and export"

echo "    Exercise typed artifact/session/context filters, bitemporal Knowledge evidence and frozen export paging."
cargo test -p synveda-gateway --test audit_query -- --nocapture

echo "    Recompute canonical hashes offline and prove the CLI has no ordinary storage authority."
cargo test -p synveda-audit -- --nocapture
cargo test -p synveda-cli --bin synveda audit:: -- --nocapture

echo "    Check the generated public contract and forced-RLS completeness."
cargo test -p synveda-gateway --test openapi -- --nocapture
cargo test -p synveda-store --test rls \
  every_tenant_scoped_table_is_covered_and_forced -- --exact --nocapture
make check-api-types

exports=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from audit_log where action = 'authz.decision' and payload->>'op' = 'export'")
typed=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from audit_log where payload ? 'artifact_references'")
payload_index=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from pg_indexes where indexname = 'audit_log_tenant_payload_idx'")

if [ "$exports" -eq 0 ] || [ "$typed" -eq 0 ] || [ "$payload_index" -ne 1 ]; then
  echo "CPR-33 evidence mismatch: export_reads=$exports typed_events=$typed payload_index=$payload_index" >&2
  exit 1
fi

echo ""
echo "CPR-33 audit: $exports self-audited export reads, $typed typed artifact events and one tenant-leading payload index; frozen bundles verify offline."
demo_finish
