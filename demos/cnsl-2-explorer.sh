#!/usr/bin/env sh
# Current scope capability explorer.
# Capabilities forecast acts over current scopes and grants; every act still decides again at its own seam.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cnsl2" "Current scope capability explorer"
echo "    Capabilities forecast acts over current scopes and grants; every act still decides again at its own seam."
cargo test -p synveda-gateway --test explorer -- --nocapture
demo_finish
