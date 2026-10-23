#!/usr/bin/env sh
# Service identities on current anchors.
# Services receive principal scopes and explicit grants; registration, lifetime and revocation all decide through the PDP.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "auth3" "Service identities on current anchors"
echo "    Services receive principal scopes and explicit grants; registration, lifetime and revocation all decide through the PDP."
cargo test -p synveda-gateway --test service_identities -- --nocapture
demo_finish
