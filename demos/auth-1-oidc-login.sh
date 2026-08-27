#!/usr/bin/env sh
# AUTH-1: OIDC login into a principal scope.
# CPR-13 re-point: A verified login mints the current principal placement and reads the generated /v1/me contract.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "auth1" "AUTH-1 — OIDC login into a principal scope"
echo "    A verified login mints the current principal placement and reads the generated /v1/me contract."
cargo test -p synveda-gateway --test oidc_login -- --nocapture
demo_finish
