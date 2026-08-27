#!/usr/bin/env sh
# TEN-3 — current Knowledge retrieval correctness and measurement boundary.
#
# TEN-3's original Record/HNSW sweep is historical evidence; epoch 3 has no
# Record table or separate sparse sidecar. The supported path embeds immutable
# KnowledgeRevision heads, filters current policy-visible rows, and records the
# lexical/semantic/graph contributions on ContextRun. This executable narrative
# proves that path. The deterministic embedder is lexical-only and is never
# described as semantic; `make eval-retrieval` is the reproducible BGE-M3 gate.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=${DATABASE_URL:-postgres://synveda:synveda-dev@localhost:5432/synveda}
SQLX_OFFLINE=true
export DATABASE_URL SQLX_OFFLINE

echo "==> immutable Knowledge revisions back lexical and semantic candidates"
cargo test -p synveda-gateway --test knowledge_lifecycle \
  public_knowledge_api_is_current_governed_paginated_and_tenant_safe -- --exact

echo "==> ContextRun records budgeted selection and current-head semantics"
cargo test -p synveda-gateway --test context_runs \
  planner_selects_only_current_knowledge_and_feedback_names_one_revision -- --exact

echo "==> bounded graph expansion records its visible evidence path"
cargo test -p synveda-gateway --test context_runs \
  bounded_graph_improves_two_hop_recall_and_denied_endpoints_leave_no_trace -- --exact

echo
echo "TEN-3 current retrieval path passes. Run 'make eval-retrieval' for the"
echo "local BGE-M3 semantic quality and latency measurement."
