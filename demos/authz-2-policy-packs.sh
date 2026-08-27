#!/usr/bin/env sh
# AUTHZ-2: policy packs assigned to current scopes.
# CPR-13 re-point: Pack inheritance follows the one scope tree and each governed object is decided under its own chain.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "authz2" "AUTHZ-2 — policy packs assigned to current scopes"
echo "    Pack inheritance follows the one scope tree and each governed object is decided under its own chain."
cargo test -p synveda-gateway --test policy_routes -- --nocapture
demo_finish
