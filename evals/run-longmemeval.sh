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
# EVAL_LONGMEMEVAL_SEED_TIMEOUT bounds the one wait for the extraction
# pipeline, and defaults to 1800s rather than the 90s EVAL-1 sized for a
# handful of scenario events. Ten instances of longmemeval_s is ~5,000
# turns; the first run against the real corpus gave the pipeline 90s per
# instance, six blocks came back empty, and the suite reported a retrieval
# recall that was really a throughput measurement.
#
# The full supported-API seed plus that post-seed wait can exceed the
# product's default one-hour service-token ceiling. This disposable run mints
# only its lme-* actors with a two-hour lifetime and configures its gateway to
# enforce the same explicit ceiling. Ordinary evaluation and production runs
# retain the one-hour default; no signing material enters the report.
#
# EVAL_LONGMEMEVAL_JUDGED=1 adds the model-judged tier: each block is read
# by SYNVEDA_READER and the answer graded by SYNVEDA_JUDGE, against
# evals/baseline-longmemeval-judged.json. That tier is the published
# figure and gates nothing, so the command still exits successfully when
# one of its bounds is breached.
#
# Exit status is the gate's. It owns one fresh exact-role PostgreSQL fixture
# and needs Docker, node, and the corpus fetched into
# evals/fixtures/longmemeval — see that
# directory's NOTICE.md, and note that the corpus is deliberately not
# committed.
set -eu

cd "$(dirname "$0")/.."

if [ "${SYNVEDA_EVAL_EXACT_DATABASE:-}" != 1 ]; then
  SYNVEDA_DB_TEST_TASK=longmemeval-evaluation
  export SYNVEDA_DB_TEST_TASK
  exec bash scripts/db-test.sh "$@"
fi
. evals/lib.sh

EVAL_LONGMEMEVAL_INSTANCES=${EVAL_LONGMEMEVAL_INSTANCES:-10}
EVAL_LONGMEMEVAL_ACTORS=$EVAL_LONGMEMEVAL_INSTANCES
EVAL_LONGMEMEVAL_TOKEN_TTL_SECS=${EVAL_LONGMEMEVAL_TOKEN_TTL_SECS:-7200}
export EVAL_LONGMEMEVAL_INSTANCES EVAL_LONGMEMEVAL_ACTORS
export EVAL_LONGMEMEVAL_TOKEN_TTL_SECS

trap 'eval_finish $?' EXIT
trap 'eval_finish 130' INT
trap 'eval_finish 143' TERM
eval_up
eval_longmemeval "$@"
