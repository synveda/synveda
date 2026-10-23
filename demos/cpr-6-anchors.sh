#!/usr/bin/env sh
# Ordered current-scope anchors.
# One gather resolves principal, project, workspace and tenant anchors before every PDP decision.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr6" "Ordered current-scope anchors"
echo "    One gather resolves principal, project, workspace and tenant anchors before every PDP decision."
cargo test -p synveda-gateway --test anchors_api -- --nocapture
demo_finish
