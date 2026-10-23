#!/usr/bin/env sh
# Hash-chained context-platform audit.
# Current scope, grant, session and Knowledge actions append one tenant-bound event and retain tamper evidence.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "aud1" "Hash-chained context-platform audit"
echo "    Current scope, grant, session and Knowledge actions append one tenant-bound event and retain tamper evidence."
cargo test -p synveda-gateway --test audit_events -- --nocapture
cargo test -p synveda-audit --test tamper -- --nocapture
demo_finish
