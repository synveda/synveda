#!/usr/bin/env sh
# AUD-1: hash-chained context-platform audit.
# CPR-13 re-point: Current scope, grant, session and Knowledge actions append one tenant-bound event and retain tamper evidence.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "aud1" "AUD-1 — hash-chained context-platform audit"
echo "    Current scope, grant, session and Knowledge actions append one tenant-bound event and retain tamper evidence."
cargo test -p synveda-gateway --test audit_events -- --nocapture
cargo test -p synveda-audit --test tamper -- --nocapture
demo_finish
