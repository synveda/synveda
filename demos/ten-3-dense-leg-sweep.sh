#!/usr/bin/env sh
# TEN-3: current Knowledge retrieval correctness and semantic measurement seam.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "ten3" "TEN-3 — current Knowledge retrieval"

cargo test -p synveda-gateway --test knowledge_lifecycle \
  public_knowledge_api_is_current_governed_paginated_and_tenant_safe \
  -- --exact --nocapture
cargo test -p synveda-gateway --test context_runs \
  planner_selects_only_current_knowledge_and_feedback_names_one_revision \
  -- --exact --nocapture
cargo test -p synveda-gateway --test context_runs \
  bounded_graph_improves_two_hop_recall_and_denied_endpoints_leave_no_trace \
  -- --exact --nocapture

echo "    Run make eval-retrieval for the isolated BGE-M3 measurement."
demo_finish
