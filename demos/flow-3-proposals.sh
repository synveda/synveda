#!/usr/bin/env sh
# FLOW-3: VedaFlow Knowledge proposals.
# CPR-13 re-point: Create, review and apply use one change ledger and immutable Knowledge revisions under the current scope tree.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "flow3" "FLOW-3 — VedaFlow Knowledge proposals"
echo "    Create, review and apply use one change ledger and immutable Knowledge revisions under the current scope tree."
cargo test -p synveda-gateway --test knowledge_lifecycle -- --nocapture
demo_finish
