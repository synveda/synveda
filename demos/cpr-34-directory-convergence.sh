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

retired_mirrors=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from pg_class where relnamespace = 'public'::regnamespace and relname in ('scim_groups', 'scim_group_members')")
identity_membership=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from information_schema.columns where table_schema = 'public' and table_name = 'group_members' and column_name = 'identity_id'")
directory_groups=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from groups where source = 'directory'")
group_audit=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from audit_log where action in ('access.group.created', 'access.group.updated')")

if [ "$retired_mirrors" -ne 0 ] || [ "$identity_membership" -ne 1 ] \
  || [ "$directory_groups" -eq 0 ] || [ "$group_audit" -eq 0 ]; then
  echo "CPR-34 evidence mismatch: retired_mirrors=$retired_mirrors identity_membership=$identity_membership directory_groups=$directory_groups group_audit=$group_audit" >&2
  exit 1
fi

echo ""
echo "CPR-34 directory: $directory_groups shared directory groups, $group_audit chained group transitions, identity-keyed membership and zero mirror tables."
demo_finish
