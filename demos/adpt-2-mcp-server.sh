#!/usr/bin/env sh
# ADPT-2: generic MCP adapter contract.
# CPR-13 re-point: The generic MCP surface is checked against the supported CLI/public-API contract; it has no direct store path.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "adpt2" "ADPT-2 — generic MCP adapter contract"
echo "    The generic MCP surface is checked against the supported CLI/public-API contract; it has no direct store path."
cargo test -p synveda-cli --test mcp_corpus -- --nocapture
cargo test -p synveda-cli mcp::tests -- --nocapture
demo_finish
