#!/usr/bin/env sh
# CPR-18: Session evidence becomes reviewable Capture candidates and governed
# Knowledge through the PDP and VedaFlow (ADR-0083).
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr18" "CPR-18 — Session Capture and review"

cargo test -p synveda-gateway --test capture_api -- --nocapture
cargo test -p synveda-ingest --lib extraction
cargo test -p synveda-ingest --test extraction_precision
cargo test -p synveda-store --test rls \
  every_tenant_scoped_table_is_covered_and_forced -- --exact --nocapture
cargo test -p synveda-audit

demo_finish
