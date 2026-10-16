#!/usr/bin/env sh
# CPR-41 / CPR-40: owned real Git, public review, actual CLI handoff and removal.
set -eu
. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr41-iris" "CPR-41 — Iris domain context lifecycle"
cargo build -p synveda-cli --bin synveda
cargo test -p synveda-gateway --test source_reconciliation \
  iris_domain_context_lifecycle_is_governed_end_to_end -- --exact --nocapture --test-threads=1
node --test evals/product/health-diagnosis.test.mjs
demo_finish
