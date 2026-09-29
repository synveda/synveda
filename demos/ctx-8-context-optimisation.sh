#!/usr/bin/env sh
# CTX-8: governed off/conservative preview, required-revision and detail fetch.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "ctx8" "CTX-8 — governed context optimisation"
echo "    Compares the same authorised fixture snapshot in off and conservative modes."
echo "    Asserts exact local rendered-text counts, required facts, preview side effects and detail fetch."
cargo test -p synveda-gateway --test context_runs \
  conservative_preview_is_not_delivery_and_required_revisions_fail_closed \
  -- --exact --nocapture
echo "    Checks paired authentication, configuration, code and multilingual required-fact cases."
cargo test -p synveda-gateway --test context_runs \
  conservative_required_fact_matrix_preserves_exact_task_evidence \
  -- --exact --nocapture
echo "    Confirms equal source bytes across distinct grants retain their separate revisions."
cargo test -p synveda-gateway --test context_runs \
  conservative_dedup_preserves_same_body_across_distinct_authority \
  -- --exact --nocapture
demo_finish
