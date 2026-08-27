#!/usr/bin/env sh
# GRPH-2: explicit Knowledge relationships.
# CPR-13 re-point: Supports, references, derived-from and supersedes edges connect stable Knowledge aggregates without a record translation layer.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "grph2" "GRPH-2 — explicit Knowledge relationships"
echo "    KnowledgeRelation is the one graph: governed support claims feed a bounded, PDP-filtered ContextRun path."
cargo test -p synveda-gateway --test context_runs \
  bounded_graph_improves_two_hop_recall_and_denied_endpoints_leave_no_trace -- --nocapture
demo_finish
