#!/usr/bin/env sh
# CPR-30 acceptance demo: immutable runtime-configuration versions selected by
# revisioned scope bindings, with every mutation travelling through VedaFlow.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr30" "CPR-30 — governed runtime configuration"

echo "    Create, publish, compare, pin and roll back Configuration through the public API."
cargo test -p synveda-gateway --test configuration_api -- --nocapture

artifacts=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from configuration_artifacts")
versions=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from configuration_versions")
bindings=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from configuration_bindings")
applied=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from audit_log where action = 'configuration.change.applied'")
retired_tables=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from pg_class where relnamespace = 'public'::regnamespace and relname in ('policy_pack_defaults','policy_pack_assignments')")

if [ "$artifacts" -lt 2 ] || [ "$versions" -lt 3 ] || \
   [ "$bindings" -lt 2 ] || [ "$applied" -lt 5 ] || [ "$retired_tables" -ne 0 ]; then
  echo "CPR-30 state mismatch: artifacts=$artifacts versions=$versions bindings=$bindings applied=$applied retired_tables=$retired_tables" >&2
  exit 1
fi

echo ""
echo "CPR-30 Configuration: $artifacts stable artifacts, $versions immutable versions, $bindings revisioned bindings and $applied audited applications; deleted assignment tables remain absent."
