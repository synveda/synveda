#!/usr/bin/env sh
# Exact-source admission on saved ContextRun replay.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "mem8" "Saved context admission"
cargo test -p synveda-gateway --test context_runs saved_delivery_ \
  -- --test-threads=1 --nocapture
cargo test -p synveda-gateway --test context_runs \
  all_mode_private_lineage_scrubs_forgotten_delivery_copies -- --exact --nocapture
cargo test -p synveda-gateway --test context_packs saved_authored_delivery_ \
  -- --test-threads=1 --nocapture
cargo test -p synveda-gateway --test skills saved_skill_advertisement_ \
  -- --test-threads=1 --nocapture
cargo test -p synveda-gateway --test knowledge_lifecycle typed_retry_claims_ \
  -- --test-threads=1 --nocapture
cargo test -p synveda-gateway --test context_runs \
  conservative_excerpt_keeps_retry_exception_under_tight_budget -- --exact --nocapture
demo_finish
