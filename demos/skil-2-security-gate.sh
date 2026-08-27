#!/usr/bin/env sh
# SKIL-2: skill bundle security gate.
# CPR-23 re-point: traversal and credential checks run before a change opens;
# exact immutable objects are rebuilt and rescanned before application.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "skil2" "SKIL-2 — skill bundle security gate"
echo "    Exact immutable bundle objects are validated, scanned and policy-gated at open and apply."
cargo test -p synveda-gateway --test skills -- --nocapture
demo_finish
