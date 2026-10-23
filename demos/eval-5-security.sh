#!/usr/bin/env sh
# Current-scope security evaluation.
# Security scenarios name current project, principal and tenant boundaries and require zero denied-content leakage.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "eval5" "Current-scope security evaluation"
echo "    Security scenarios name current project, principal and tenant boundaries and require zero denied-content leakage."
make eval-check
demo_finish
