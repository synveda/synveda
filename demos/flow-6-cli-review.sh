#!/usr/bin/env sh
# CLI review through supported application contracts.
# Review commands use VedaFlow identifiers and current scope vocabulary; no hierarchy or direct-record command remains.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "flow6" "CLI review through supported application contracts"
echo "    Review commands use VedaFlow identifiers and current scope vocabulary; no hierarchy or direct-record command remains."
cargo test -p synveda-cli proposal::tests -- --nocapture
demo_finish
