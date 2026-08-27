#!/usr/bin/env sh
# OPS-1: one personal and team runtime.
# CPR-13 re-point: The PulseBoard loop uses the same workspace, project, session, capture, Knowledge and context plane for an individual and teammate.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "ops1" "OPS-1 — one personal and team runtime"
echo "    The PulseBoard loop uses the same workspace, project, session, capture, Knowledge and context plane for an individual and teammate."
cargo test -p synveda-gateway --test capture_api pulseboard_cross_session_team_knowledge_loop_is_governed_end_to_end -- --exact --nocapture
demo_finish
