#!/usr/bin/env sh
# CPR-6: ordered current-scope anchors.
# CPR-13 re-point: One gather resolves principal, project, workspace and tenant anchors before every PDP decision.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr6" "CPR-6 — ordered current-scope anchors"
echo "    One gather resolves principal, project, workspace and tenant anchors before every PDP decision."
cargo test -p synveda-gateway --test anchors_api -- --nocapture
demo_finish
