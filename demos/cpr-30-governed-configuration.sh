#!/usr/bin/env sh
# CPR-30 acceptance demo: immutable runtime-configuration versions selected by
# revisioned scope bindings, with every mutation travelling through VedaFlow.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr30" "CPR-30 — governed runtime configuration"

echo "    Create, publish, compare, pin and roll back Configuration through the public API."
cargo test -p synveda-gateway --test configuration_api -- --nocapture

echo ""
echo "CPR-30 Configuration: the gateway acceptance test proved stable artifacts, immutable versions, revisioned bindings and audited applications."
