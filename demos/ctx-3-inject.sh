#!/usr/bin/env sh
# CTX-3: public session context delivery.
# CPR-13 re-point: The retired global injection call is now a session-scoped context run with immutable explainable selections.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "ctx3" "CTX-3 — public session context delivery"
echo "    The retired global injection call is now a session-scoped context run with immutable explainable selections."
cargo test -p synveda-gateway --test context_runs planner_selects_only_current_knowledge_and_feedback_names_one_revision -- --exact --nocapture
demo_finish
