#!/usr/bin/env sh
# MEM-4: semantic indexing of current Knowledge.
# CPR-13 re-point: Only accepted current Knowledge revisions are searchable; selected revision IDs and index versions remain explainable.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "mem4" "MEM-4 — semantic indexing of current Knowledge"
echo "    Only accepted current Knowledge revisions are searchable; selected revision IDs and index versions remain explainable."
cargo test -p synveda-gateway --test context_runs planner_selects_only_current_knowledge_and_feedback_names_one_revision -- --exact --nocapture
demo_finish
