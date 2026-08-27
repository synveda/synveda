#!/usr/bin/env sh
# CTX-1: hybrid retrieval over current Knowledge revisions.
# CPR-13 re-point: Lexical and semantic candidates come from active current Knowledge and are filtered before selection.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "ctx1" "CTX-1 — hybrid retrieval over current Knowledge revisions"
echo "    Lexical and semantic candidates come from active current Knowledge and are filtered before selection."
cargo test -p synveda-gateway --test context_runs planner_selects_only_current_knowledge_and_feedback_names_one_revision -- --exact --nocapture
demo_finish
