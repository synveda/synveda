#!/usr/bin/env sh
# OPS-9: current beta product walkthrough.
# CPR-13 re-point: The beta narrative is the PulseBoard session-to-Knowledge team loop, with private isolation and explicit supersession.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "ops9" "OPS-9 — current beta product walkthrough"
echo "    The beta narrative is the PulseBoard session-to-Knowledge team loop, with private isolation and explicit supersession."
cargo test -p synveda-gateway --test capture_api pulseboard_cross_session_team_knowledge_loop_is_governed_end_to_end -- --exact --nocapture
demo_finish
