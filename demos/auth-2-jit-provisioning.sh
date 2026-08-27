#!/usr/bin/env sh
# AUTH-2: JIT principal placement and operator grant.
# CPR-13 re-point: Directory identities are keyed by external identity, placed in principal scopes and granted through the shared scope tree.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "auth2" "AUTH-2 — JIT principal placement and operator grant"
echo "    Directory identities are keyed by external identity, placed in principal scopes and granted through the shared scope tree."
cargo test -p synveda-gateway --test jit_provisioning -- --nocapture
cargo test -p synveda-gateway --test cli_login -- --nocapture
demo_finish
