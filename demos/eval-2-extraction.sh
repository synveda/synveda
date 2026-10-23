#!/usr/bin/env sh
# Session-event extraction evaluation.
# Extraction evidence is session-event based and its unavailable live-model lens remains an explicit refusal, never a substituted metric.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "eval2" "Session-event extraction evaluation"
echo "    Extraction evidence is session-event based and its unavailable live-model lens remains an explicit refusal, never a substituted metric."
make eval-check
demo_finish
