#!/usr/bin/env sh
# CPR-16: PDP/VedaFlow-governed Knowledge lifecycle (ADR-0081).
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr16" "CPR-16 — governed Knowledge lifecycle"

cargo test -p synveda-gateway --test knowledge_lifecycle -- --nocapture
cargo test -p synveda-policy --test approvals --test packs --test pdp
cargo test -p synveda-store --test rls \
  every_tenant_scoped_table_is_covered_and_forced -- --exact --nocapture

demo_finish
