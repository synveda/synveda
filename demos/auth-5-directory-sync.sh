#!/usr/bin/env sh
# AUTH-5: directory pull convergence on current principals.
# CPR-13 re-point: A pull owns its external identities, converges idempotently and cannot claim a tenant owned by the push adapter.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "auth5" "AUTH-5 — directory pull convergence on current principals"
echo "    A pull owns its external identities, converges idempotently and cannot claim a tenant owned by the push adapter."
cargo test -p synveda-gateway --test directory_sync a_joiner_converges_in_a_single_complete_pass -- --exact --nocapture
cargo test -p synveda-gateway --test directory_sync a_tenant_the_push_plane_owns_is_not_pulled -- --exact --nocapture
demo_finish
