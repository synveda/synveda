#!/usr/bin/env sh
# LongMemEval's deterministic retrieval tier against a fresh stack
# (EVAL-3, ADR-0061 decision 5). This is what `make eval-longmemeval` runs.
#
#   evals/run-longmemeval.sh                     gate against the baseline
#   evals/run-longmemeval.sh --update-baseline   rewrite it from this run
#
# EVAL_LONGMEMEVAL_INSTANCES sets the declared slice, and the actor pool
# is sized to match it — one actor per instance is what keeps the
# haystacks apart (decision 8), so the two numbers are one number.
#
# Exit status is the gate's. Needs the dev compose (postgres), node, and
# the corpus fetched into evals/fixtures/longmemeval — see that
# directory's NOTICE.md, and note that the corpus is deliberately not
# committed.
set -eu

cd "$(dirname "$0")/.."
. evals/lib.sh

EVAL_LONGMEMEVAL_INSTANCES=${EVAL_LONGMEMEVAL_INSTANCES:-10}
EVAL_LONGMEMEVAL_ACTORS=$EVAL_LONGMEMEVAL_INSTANCES
export EVAL_LONGMEMEVAL_INSTANCES EVAL_LONGMEMEVAL_ACTORS

trap eval_down EXIT INT TERM
eval_up
eval_longmemeval "$@"
