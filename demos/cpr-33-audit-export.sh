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

echo ""
echo "CPR-33 audit: the acceptance tests proved self-audited export reads, typed artifact evidence, forced RLS and offline verification of frozen bundles."
demo_finish
