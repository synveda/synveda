#!/usr/bin/env sh
# FLOW-2: governed context channels on current scopes.
# CPR-13 re-point: Context-pack and channel policy follows project/workspace scopes; publishing never writes active Knowledge directly.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "flow2" "FLOW-2 — governed context channels on current scopes"
echo "    Context-pack and channel policy follows project/workspace scopes; publishing never writes active Knowledge directly."
cargo test -p synveda-gateway --test context_packs -- --nocapture
demo_finish
