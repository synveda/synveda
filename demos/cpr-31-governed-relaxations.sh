#!/usr/bin/env sh
# CPR-31 acceptance demo: one VedaFlow path for personal auto-apply and
# reviewed policy relaxations over current Knowledge.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr31" "CPR-31 — governed policy relaxations"

echo "    Exercise auto-apply, immutable revision, pending review, rejection and exact-subject access through public APIs."
cargo test -p synveda-gateway --test relaxations -- --nocapture

echo ""
echo "CPR-31 relaxations: the gateway acceptance test proved stable aggregates, immutable versions and governed changes; the hard-cut gate proves the retired table remains absent."
demo_finish
