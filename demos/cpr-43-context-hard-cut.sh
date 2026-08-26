#!/usr/bin/env sh
# CPR-43: one epoch-3 schema and one current public product surface. This demo
# exercises the refusal boundaries as executable evidence; it never creates an
# old-to-new translator or calls a retired route.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr43" "CPR-43 — final context-platform hard cut"

echo "    Prove the repository has one current baseline and no active compatibility residue."
node --test scripts/check-context-hard-cut.test.mjs
node scripts/check-context-hard-cut.mjs

echo "    Bootstrap epoch 3, refuse markerless/epoch-2 databases, and reset safely."
cargo test -p synveda-store --test epoch -- --nocapture --test-threads=1

echo "    Discover every tenant table and require enabled, forced RLS plus policy coverage."
cargo test -p synveda-store --test rls \
  every_tenant_scoped_table_is_covered_and_forced \
  -- --exact --nocapture --test-threads=1

echo "    Match executable routes to OpenAPI and reject retired CLI shapes."
cargo test -p synveda-gateway --test openapi -- --nocapture
cargo test -p synveda-cli hard_cut_tests -- --nocapture

demo_finish
