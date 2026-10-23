#!/usr/bin/env sh
# Durable session-event ingestion.
# The old global observation seam is replaced by idempotent ordered event append on a governed session.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "mem1" "Durable session-event ingestion"
echo "    The old global observation seam is replaced by idempotent ordered event append on a governed session."
cargo test -p synveda-gateway --test sessions_api -- --nocapture
cargo test --profile ingest-load -p synveda-gateway --test session_ingest_load -- --ignored --nocapture
demo_finish
