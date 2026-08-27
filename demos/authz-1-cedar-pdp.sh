#!/usr/bin/env sh
# AUTHZ-1: embedded Cedar PDP on governed scopes.
# CPR-13 re-point: The same current scope/grant entities decide every row and adversarial cross-tenant and principal-scope reads remain hidden.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "authz1" "AUTHZ-1 — embedded Cedar PDP on governed scopes"
echo "    The same current scope/grant entities decide every row and adversarial cross-tenant and principal-scope reads remain hidden."
cargo test -p synveda-gateway --test foundation_audit -- --nocapture
cargo test -p synveda-policy --test pdp -- --nocapture
demo_finish
