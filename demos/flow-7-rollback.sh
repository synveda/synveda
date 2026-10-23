#!/usr/bin/env sh
# Immutable rollback and supersession evidence.
# Rollback-like corrections create a new governed revision or explicit supersession and retain prior content history.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "flow7" "Immutable rollback and supersession evidence"
echo "    Rollback-like corrections create a new governed revision or explicit supersession and retain prior content history."
cargo test -p synveda-gateway --test knowledge_lifecycle supersession_and_merge_are_explicit_and_retain_every_source -- --exact --nocapture
demo_finish
