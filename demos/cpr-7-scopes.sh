#!/usr/bin/env sh
# CPR-7: one scope tree and one role vocabulary.
# CPR-13 re-point: Admin scope routes, principal placement and subtree grants replace the fixed hierarchy and role bindings whole.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr7" "CPR-7 — one scope tree and one role vocabulary"
echo "    Admin scope routes, principal placement and subtree grants replace the fixed hierarchy and role bindings whole."
cargo test -p synveda-gateway --test admin_scopes_api -- --nocapture
cargo test -p synveda-gateway --test foundation_audit -- --nocapture
demo_finish
