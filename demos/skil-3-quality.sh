#!/usr/bin/env sh
# SKIL-3: skill quality evidence.
# CPR-13 re-point: Quality scores and review reports are immutable evidence; low scores require distinct governed authority.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "skil3" "SKIL-3 — skill quality evidence"
echo "    Quality scores and review reports are immutable evidence; low scores require distinct governed authority."
cargo test -p synveda-gateway --test skills the_score_is_displayed_at_review_and_in_the_registry -- --exact --nocapture
cargo test -p synveda-gateway --test skills a_low_score_publish_requires_an_override_from_a_second_authority -- --exact --nocapture
demo_finish
