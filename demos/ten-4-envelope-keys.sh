#!/usr/bin/env sh
# TEN-4: tenant-bound envelope keys, rotation and unreadable cross-key material
# (ADR-0064).
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "ten4" "TEN-4 — per-tenant envelope keys"

cargo test -p synveda-store --test keys -- --nocapture --test-threads=1
cargo test -p synveda-cli keys::tests -- --nocapture
cargo test -p synveda-gateway --test audit_query \
  no_knowledge_content_reaches_any_audit_answer -- --exact --nocapture

demo_finish
