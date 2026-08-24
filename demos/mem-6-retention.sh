#!/usr/bin/env sh
# MEM-6: governed Knowledge erasure.
# CPR-13 re-point: Forget evaluates policy, removes plaintext and owned payloads, invalidates retrieval and leaves only content-free evidence.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "mem6" "MEM-6 — governed Knowledge erasure"
echo "    Forget evaluates policy, removes plaintext and owned payloads, invalidates retrieval and leaves only content-free evidence."
cargo test -p synveda-gateway --test knowledge_lifecycle review_is_live_and_forget_leaves_only_content_free_evidence -- --exact --nocapture
demo_finish
