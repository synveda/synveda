#!/usr/bin/env sh
# Sessions as the runtime root.
# Runs, immutable ordered session events and two-phase close replace client-invented session strings.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr10" "Sessions as the runtime root"
echo "    Runs, immutable ordered session events and two-phase close replace client-invented session strings."
cargo test -p synveda-gateway --test sessions_api -- --nocapture
demo_finish
