#!/usr/bin/env sh
# Dense Knowledge plan stability and retrieval cost at 1024 dimensions.
# Runs the plan-stability regression tests, then seeds tenants into the shared
# HNSW index and reports exact ground truth, each planner/pgvector arm and six
# executions of the product statement, then full ContextRun and Knowledge
# search latency over the same corpora. SYNVEDA_DENSE_PLAN_STEPS selects any of
# `tests`, `benchmark` and `context` (default: all three). The context step
# times the gateway in-process, so it builds with SYNVEDA_DENSE_PLAN_PROFILE
# (default `ingest-load`, an optimised test profile); the others use `dev`.
set -eu

# Refuse an unknown or malformed step before the demo enters a database
# fixture, so a typo can never skip a step and still finish successfully.
steps=${SYNVEDA_DENSE_PLAN_STEPS:-tests,benchmark,context}
old_ifs=$IFS
IFS=,
for step in $steps; do
  case "$step" in
    tests|benchmark|context) ;;
    *)
      echo "ctx7: unknown SYNVEDA_DENSE_PLAN_STEPS entry '$step' (expected tests, benchmark or context)" >&2
      exit 64
      ;;
  esac
done
IFS=$old_ifs

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "ctx7" "Dense Knowledge plan stability"

case ",$steps," in
  *,tests,*)
    cargo test -p synveda-gateway --test dense_plans -- --test-threads=1
    ;;
esac
case ",$steps," in
  *,benchmark,*)
    cargo test -p synveda-gateway --test dense_plans \
      dense_leg_plans_recall_and_latency_at_1024_dimensions \
      -- --ignored --exact --nocapture
    echo "    Plan report: ${SYNVEDA_DENSE_PLAN_REPORT:-not written (set SYNVEDA_DENSE_PLAN_REPORT)}"
    ;;
esac
case ",$steps," in
  *,context,*)
    cargo test --profile "${SYNVEDA_DENSE_PLAN_PROFILE:-ingest-load}" \
      -p synveda-gateway --test dense_plans \
      context_runs_and_search_at_corpus_scale \
      -- --ignored --exact --nocapture
    echo "    ContextRun report: ${SYNVEDA_DENSE_PLAN_CONTEXT_REPORT:-not written (set SYNVEDA_DENSE_PLAN_CONTEXT_REPORT)}"
    ;;
esac
demo_finish
