#!/usr/bin/env sh
# Public session context delivery.
# The retired global injection call is now a session-scoped context run with immutable explainable selections.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "ctx3" "Public session context delivery"
echo "    The retired global injection call is now a session-scoped context run with immutable explainable selections."
cargo test -p synveda-gateway --test context_runs planner_selects_only_current_knowledge_and_feedback_names_one_revision -- --exact --nocapture
demo_finish
