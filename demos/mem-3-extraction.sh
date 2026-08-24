#!/usr/bin/env sh
# MEM-3: reviewable session capture.
# CPR-13 re-point: Extraction reads real session events and creates candidates only; acceptance calls the Knowledge command layer and VedaFlow.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "mem3" "MEM-3 — reviewable session capture"
echo "    Extraction reads real session events and creates candidates only; acceptance calls the Knowledge command layer and VedaFlow."
cargo test -p synveda-gateway --test capture_api -- --nocapture
demo_finish
