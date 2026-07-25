#!/usr/bin/env sh
# One eval run against a fresh stack (EVAL-1, ADR-0028). This is what
# `make eval` runs and what the nightly workflow runs.
#
#   evals/run.sh                     gate against evals/baseline.json
#   evals/run.sh --update-baseline   rewrite the baseline from this run
#
# Exit status is the gate's: 0 when it held, non-zero when a metric
# breached its bound or the run could not complete. Needs the dev compose
# (postgres only) and node.
set -eu

cd "$(dirname "$0")/.."
. evals/lib.sh

trap eval_down EXIT INT TERM
eval_up
eval_run "$@"
