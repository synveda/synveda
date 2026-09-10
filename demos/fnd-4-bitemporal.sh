#!/usr/bin/env sh
# FND-4: historical row states remain queryable across current bitemporal
# tables.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "fnd4" "FND-4 — bitemporal persistence"

cargo test -p synveda-store --test knowledge \
  revisions_are_immutable_and_current_projection_is_bitemporal \
  -- --exact --nocapture

demo_finish
