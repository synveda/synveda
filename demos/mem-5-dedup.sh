#!/usr/bin/env sh
# MEM-5: candidate duplicate and conflict matches.
# CPR-13 re-point: Repeated session extraction is idempotent and candidate comparisons are re-authorised before disclosure.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "mem5" "MEM-5 — candidate duplicate and conflict matches"
echo "    Repeated session extraction is idempotent and candidate comparisons are re-authorised before disclosure."
cargo test -p synveda-gateway --test capture_api candidate_matches_are_reauthorised_and_foreign_tenants_see_404 -- --exact --nocapture
demo_finish
