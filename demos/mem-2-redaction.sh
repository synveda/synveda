#!/usr/bin/env sh
# Session-event redaction.
# Redaction occurs before durable event payload storage and the timeline exposes only safe summaries.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "mem2" "Session-event redaction"
echo "    Redaction occurs before durable event payload storage and the timeline exposes only safe summaries."
cargo test -p synveda-gateway --test session_redaction -- --nocapture
demo_finish
