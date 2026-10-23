#!/usr/bin/env sh
# Policy-governed candidate auto-apply.
# Personal auto-apply still creates and executes a VedaFlow change; candidates never promote by a direct write.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "flow4" "Policy-governed candidate auto-apply"
echo "    Personal auto-apply still creates and executes a VedaFlow change; candidates never promote by a direct write."
cargo test -p synveda-gateway --test capture_api candidates_are_reviewable_only_and_every_decision_uses_vedaflow -- --exact --nocapture
demo_finish
