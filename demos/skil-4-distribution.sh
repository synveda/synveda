#!/usr/bin/env sh
# SKIL-4: authorised skill distribution.
# CPR-23 re-point: project/principal bindings select exact immutable versions;
# PDP visibility, not declared tool metadata, controls advertisement.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "skil4" "SKIL-4 — authorised skill distribution"
echo "    Enabled bindings select a PDP-visible exact version; declared tools grant no authority."
cargo test -p synveda-gateway --test skills -- --nocapture
demo_finish
