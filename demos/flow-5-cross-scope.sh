#!/usr/bin/env sh
# FLOW-5: cross-scope governed publication.
# CPR-13 re-point: Candidate scope choices are capability-filtered and stricter profiles retain a pending review rather than bypassing policy.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "flow5" "FLOW-5 — cross-scope governed publication"
echo "    Candidate scope choices are capability-filtered and stricter profiles retain a pending review rather than bypassing policy."
cargo test -p synveda-gateway --test capture_api strict_profile_retains_a_pending_review_instead_of_publishing -- --exact --nocapture
demo_finish
