#!/usr/bin/env sh
# CPR-23 acceptance demo: one public VedaFlow path installs and updates a
# stable Agent Skill, retains two immutable versions, binds/rolls back the
# project, records exact-version usage and validates without executing code.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr23" "CPR-23 — immutable Skill versions and governed bindings"

echo "    Install, update, bind and rollback through the generated public API and VedaFlow."
cargo test -p synveda-gateway --test skills -- --nocapture

skills=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from skills")
versions=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from skill_versions")
bindings=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from skill_bindings")
usage=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from skill_usage_events")
tests=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from skill_test_runs")

if [ "$skills" -ne 1 ] || [ "$versions" -ne 2 ] || [ "$bindings" -ne 1 ] || \
   [ "$usage" -ne 1 ] || [ "$tests" -ne 1 ]; then
  echo "CPR-23 state mismatch: skills=$skills versions=$versions bindings=$bindings usage=$usage tests=$tests" >&2
  exit 1
fi

echo ""
echo "CPR-23 Skills: $skills stable aggregate, $versions immutable versions, $bindings revisioned binding, $usage idempotent usage event and $tests non-executing validation run; acceptance criteria pass."
