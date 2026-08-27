#!/usr/bin/env sh
# CPR-37: deterministic conflict evidence, governed current-state resolution,
# bitemporal Knowledge queries and type-aware freshness (ADR-0096).
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr37" "CPR-37 — conflict, supersession and freshness"

cargo test -p synveda-types --lib knowledge::tests -- --nocapture
cargo test -p synveda-gateway --test knowledge_lifecycle \
  conflicts_are_transitional_governed_and_temporally_queryable \
  -- --exact --nocapture
cargo test -p synveda-gateway --test knowledge_lifecycle -- --nocapture
cargo test -p synveda-store --test rls \
  every_tenant_scoped_table_is_covered_and_forced -- --exact --nocapture
pnpm --filter @synveda/console test

demo_finish
