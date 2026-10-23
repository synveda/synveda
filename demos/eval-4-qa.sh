#!/usr/bin/env sh
# Knowledge query evaluation.
# The QA corpus targets the scoped Knowledge query lens and keeps budgeted context composition separate from enumeration.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "eval4" "Knowledge query evaluation"
echo "    The QA corpus targets the scoped Knowledge query lens and keeps budgeted context composition separate from enumeration."
make eval-check
demo_finish
