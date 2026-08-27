#!/usr/bin/env sh
# MEM-2: session-event redaction.
# CPR-13 re-point: Redaction occurs before durable event payload storage and the timeline exposes only safe summaries.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "mem2" "MEM-2 — session-event redaction"
echo "    Redaction occurs before durable event payload storage and the timeline exposes only safe summaries."
cargo test -p synveda-gateway --test session_redaction -- --nocapture
demo_finish
