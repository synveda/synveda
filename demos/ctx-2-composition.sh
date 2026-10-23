#!/usr/bin/env sh
# Budgeted Knowledge composition.
# A context run records requested and actual budgets, candidate scores, selections and rendered hash.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "ctx2" "Budgeted Knowledge composition"
echo "    A context run records requested and actual budgets, candidate scores, selections and rendered hash."
cargo test -p synveda-gateway --test context_runs planner_selects_only_current_knowledge_and_feedback_names_one_revision -- --exact --nocapture
demo_finish
