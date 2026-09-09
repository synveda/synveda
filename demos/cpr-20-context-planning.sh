#!/usr/bin/env sh
# CPR-20: explainable, immutable and re-authorised Knowledge context planning
# through public API, CLI and MCP boundaries (ADR-0084).
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr20" "CPR-20 — explainable Knowledge context planning"

cargo test -p synveda-gateway --test context_runs -- --nocapture
cargo test -p synveda-gateway --test audit_query -- --nocapture
cargo test -p synveda-store --test rls \
  every_tenant_scoped_table_is_covered_and_forced -- --exact --nocapture
cargo test -p synveda-gateway --test openapi -- --nocapture
cargo test -p synveda-cli recall::tests -- --nocapture
cargo test -p synveda-cli mcp::tests -- --nocapture

demo_finish
