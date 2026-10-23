#!/usr/bin/env sh
# Immutable MCP discovery evidence, quarantined
# version drift, exact approved project bindings and non-executing tests.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr25" "Trusted MCP catalogue and exact bindings"

echo "    Register, discover, compare, approve and bind through the public API and VedaFlow."
cargo test -p synveda-gateway --test tools -- --nocapture

echo ""
echo "Tools: the gateway acceptance test proved stable servers, immutable discovery snapshots, exact bindings, governed drift and read-only test reports."
