#!/usr/bin/env sh
# CTX-4: retention-aware context evidence.
# CPR-13 re-point: Full, redacted, hashes-only and disabled traces disclose exactly the detail their governed mode permits.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "ctx4" "CTX-4 — retention-aware context evidence"
echo "    Full, redacted, hashes-only and disabled traces disclose exactly the detail their governed mode permits."
cargo test -p synveda-gateway --test context_runs retention_modes_and_diagnostic_query_have_distinct_disclosure -- --exact --nocapture
demo_finish
