#!/usr/bin/env sh
# CNSL-1: advanced governed review surface.
# CPR-13 re-point: Complex VedaFlow review remains an authorised advanced surface; ordinary captured learnings use New Learnings.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cnsl1" "CNSL-1 — advanced governed review surface"
echo "    Complex VedaFlow review remains an authorised advanced surface; ordinary captured learnings use New Learnings."
cargo test -p synveda-gateway --test console_session --test console_serving -- --nocapture
demo_finish
