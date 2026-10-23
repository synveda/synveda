#!/usr/bin/env sh
# Policy packs assigned to current scopes.
# Pack inheritance follows the one scope tree and each governed object is decided under its own chain.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "authz2" "Policy packs assigned to current scopes"
echo "    Pack inheritance follows the one scope tree and each governed object is decided under its own chain."
cargo test -p synveda-gateway --test policy_routes -- --nocapture
demo_finish
