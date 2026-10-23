#!/usr/bin/env sh
# Authorised skill distribution.
# Project/principal bindings select exact immutable versions;
# PDP visibility, not declared tool metadata, controls advertisement.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "skil4" "Authorised skill distribution"
echo "    Enabled bindings select a PDP-visible exact version; declared tools grant no authority."
cargo test -p synveda-gateway --test skills -- --nocapture
demo_finish
