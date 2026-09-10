#!/usr/bin/env sh
# CPR-25 acceptance demo: immutable MCP discovery evidence, quarantined
# version drift, exact approved project bindings and non-executing tests.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr25" "CPR-25 — trusted MCP catalogue and exact bindings"

echo "    Register, discover, compare, approve and bind through the public API and VedaFlow."
cargo test -p synveda-gateway --test tools -- --nocapture

echo ""
echo "CPR-25 Tools: the gateway acceptance test proved stable servers, immutable discovery snapshots, exact bindings, governed drift and read-only test reports."
