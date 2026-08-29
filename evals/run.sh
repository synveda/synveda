#!/usr/bin/env sh
# One eval run against a fresh stack (EVAL-1, ADR-0028). This is what
# `make eval` runs and what the nightly workflow runs.
#
#   evals/run.sh                     gate against evals/baseline.json
#   evals/run.sh --update-baseline   rewrite the baseline from this run
#
# Exit status is the gate's: 0 when it held, non-zero when a metric
# breached its bound or the run could not complete. It owns one fresh
# exact-role PostgreSQL fixture and needs Docker plus node.
set -eu

cd "$(dirname "$0")/.."

# The evaluator never provisions or migrates through the retained contributor
# database. Its outer invocation enters the same fresh exact-role fixture as
# `make db-test`; the inner invocation receives only role-scoped URL files.
if [ "${SYNVEDA_EVAL_EXACT_DATABASE:-}" != 1 ]; then
  SYNVEDA_DB_TEST_TASK=evaluation
  export SYNVEDA_DB_TEST_TASK
  exec bash scripts/db-test.sh "$@"
fi
. evals/lib.sh

trap 'eval_finish $?' EXIT
trap 'eval_finish 130' INT
trap 'eval_finish 143' TERM
eval_up
eval_run "$@"
