#!/usr/bin/env sh
# GRPH-2: explicit Knowledge relationships.
# CPR-13 re-point: Supports, references, derived-from and supersedes edges connect stable Knowledge aggregates without a record translation layer.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "grph2" "GRPH-2 — explicit Knowledge relationships"
echo "    Supports, references, derived-from and supersedes edges connect stable Knowledge aggregates without a record translation layer."
cargo test -p synveda-store --test knowledge -- --nocapture
demo_finish
