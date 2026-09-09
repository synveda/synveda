#!/usr/bin/env sh
# CPR-22: one public-API team loop from Session evidence through Capture and
# Knowledge into clean-session Context.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr22" "CPR-22 — individual and small-team MVP"

cargo test -p synveda-gateway --test capture_api \
  pulseboard_cross_session_team_knowledge_loop_is_governed_end_to_end \
  -- --exact --nocapture
pnpm --filter @synveda/console test

demo_finish
