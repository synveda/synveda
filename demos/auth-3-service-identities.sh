#!/usr/bin/env sh
# AUTH-3: service identities on current anchors.
# CPR-13 re-point: Services receive principal scopes and explicit grants; registration, lifetime and revocation all decide through the PDP.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "auth3" "AUTH-3 — service identities on current anchors"
echo "    Services receive principal scopes and explicit grants; registration, lifetime and revocation all decide through the PDP."
cargo test -p synveda-gateway --test service_identities -- --nocapture
demo_finish
