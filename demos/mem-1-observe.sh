#!/usr/bin/env sh
# MEM-1: durable session-event ingestion.
# CPR-13 re-point: The old global observation seam is replaced by idempotent ordered event append on a governed session.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "mem1" "MEM-1 — durable session-event ingestion"
echo "    The old global observation seam is replaced by idempotent ordered event append on a governed session."
cargo test -p synveda-gateway --test sessions_api -- --nocapture
cargo test -p synveda-gateway --test session_ingest_load -- --nocapture
demo_finish
