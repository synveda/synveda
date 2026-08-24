#!/usr/bin/env sh
# AUTHZ-4: time-bounded policy evidence on current scopes.
# CPR-13 re-point: The surviving lapse plane is fail-closed and cannot relax Knowledge reads; governed relaxations replace it later.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "authz4" "AUTHZ-4 — time-bounded policy evidence on current scopes"
echo "    The surviving lapse plane is fail-closed and cannot relax Knowledge reads; governed relaxations replace it later."
cargo test -p synveda-gateway --test lapses -- --nocapture
demo_finish
