#!/usr/bin/env sh
# CPR-34 acceptance demo: both directory doors project onto the shared
# identity, Group, membership and grant graph without a second group mirror.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr34" "CPR-34 — directory adapter convergence"

echo "    Parse authentic/transcribed Entra and Okta fixtures into stable user/group snapshots."
cargo test -p synveda-identity --test directory_connectors -- --nocapture

echo "    Prove source-qualified identity reconciliation and shared identity-keyed access."
cargo test -p synveda-store --test directory_sync \
  external_ids_never_confuse_tenants_or_directory_sources -- --exact --nocapture
cargo test -p synveda-store --test access \
  a_directory_group_carries_source_identity_and_a_direct_one_does_not \
  -- --exact --nocapture

echo "    Drive SCIM push, complete/partial pull and the PDP-governed assignment API."
cargo test -p synveda-gateway --test scim \
  a_directory_group_becomes_a_governed_group_with_its_members \
  -- --exact --nocapture
cargo test -p synveda-gateway --test directory_sync \
  directory_groups_converge_only_on_complete_snapshots -- --exact --nocapture
cargo test -p synveda-gateway --test access_api \
  a_directory_access_assignment_is_governed_and_source_owned \
  -- --exact --nocapture

echo "    Check the hard-cut schema, forced RLS and generated public contract."
cargo test -p synveda-store --test rls \
  every_tenant_scoped_table_is_covered_and_forced -- --exact --nocapture
cargo test -p synveda-gateway --test openapi -- --nocapture
make check-api-types

echo ""
echo "CPR-34 directory: the acceptance tests proved shared directory groups, chained group transitions, identity-keyed membership, forced RLS and the hard-cut absence of mirror tables."
demo_finish
