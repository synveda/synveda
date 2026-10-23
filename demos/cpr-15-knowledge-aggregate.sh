#!/usr/bin/env sh
# Stable Knowledge identities, immutable revisions, provenance,
# relations and forced-RLS isolation (ADR-0080).
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr15" "Immutable Knowledge aggregate"

cargo test -p synveda-store --test knowledge -- --nocapture
cargo test -p synveda-store --test rls \
  every_tenant_scoped_table_is_covered_and_forced -- --exact --nocapture

demo_finish
