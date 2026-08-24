#!/usr/bin/env sh
# SKIL-4: authorised skill distribution.
# CPR-13 re-point: A session sees only scope-visible published skill content, pinned by digest, with no inferred tool authority.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "skil4" "SKIL-4 — authorised skill distribution"
echo "    A session sees only scope-visible published skill content, pinned by digest, with no inferred tool authority."
cargo test -p synveda-gateway --test skills a_skill_reaches_a_client_only_through_review_under_every_pack -- --exact --nocapture
cargo test -p synveda-gateway --test skills every_served_file_hashes_to_the_address_the_commit_named -- --exact --nocapture
demo_finish
