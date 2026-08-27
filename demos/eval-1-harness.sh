#!/usr/bin/env sh
# EVAL-1: deterministic context-platform evaluation harness.
# CPR-13 re-point: Evaluation parses committed scenarios and baselines without calling retired hierarchy or global runtime routes.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "eval1" "EVAL-1 — deterministic context-platform evaluation harness"
echo "    Evaluation parses committed scenarios and baselines without calling retired hierarchy or global runtime routes."
make eval-check
demo_finish
