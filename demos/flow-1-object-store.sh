#!/usr/bin/env sh
# FLOW-1: content-addressed VedaFlow objects and immutable concurrent history
# (ADR-0030).
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "flow1" "FLOW-1 — VedaFlow object store"

cargo test -p synveda-vedaflow --test object_store -- --nocapture --test-threads=1
cargo test -p synveda-store --test rls -- --nocapture

demo_finish
