#!/usr/bin/env sh
# CNSL-2: current scope capability explorer.
# CPR-13 re-point: Capabilities forecast acts over current scopes and grants; every act still decides again at its own seam.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cnsl2" "CNSL-2 — current scope capability explorer"
echo "    Capabilities forecast acts over current scopes and grants; every act still decides again at its own seam."
cargo test -p synveda-gateway --test explorer -- --nocapture
demo_finish
