#!/usr/bin/env sh
# CPR-39: generated support claims and the honest second-client boundary.
set -eu

echo "CPR-39 — adapter conformance and truthful support levels"

echo "    Validate exact criteria, evidence paths/digests and both generated projections."
node --test scripts/check-adapter-conformance.test.mjs
node scripts/check-adapter-conformance.mjs

echo "    Replay authentic MCP bytes and the vendor-neutral repository-owned launch contract."
cargo test -p synveda-cli --test mcp_corpus -- --nocapture

echo "    Assert the live evidence boundary: Claude verified; Cursor experimental; VS Code configured."
jq -e '
  ([.clients[] | select(.support_level == "verified") | .id] == ["claude-code"]) and
  (.clients[] | select(.id == "cursor") | .support_level == "experimental") and
  (.clients[] | select(.id == "vscode") | .support_level == "configured") and
  ([.clients[] | select(.support_level == "captured") | .id] | sort == ["claude-desktop", "zed"])
' adapters/registry.json >/dev/null

echo "    Cursor live: PENDING — no executable/authenticated client or authentic frame in this environment."
echo "    VS Code fallback: PENDING — no authenticated profile and its Preview contract has no SessionEnd."
echo "demo PASS: CPR-39 support claims derive from checked evidence"
