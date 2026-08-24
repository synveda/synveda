#!/usr/bin/env sh
# CPR-25 acceptance demo: immutable MCP discovery evidence, quarantined
# version drift, exact approved project bindings and non-executing tests.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr25" "CPR-25 — trusted MCP catalogue and exact bindings"

echo "    Register, discover, compare, approve and bind through the public API and VedaFlow."
cargo test -p synveda-gateway --test tools -- --nocapture

servers=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from tool_servers")
versions=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from tool_server_versions")
snapshots=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from capability_snapshots")
bindings=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from tool_bindings")
changes=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from tool_changes")
tests=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from tool_test_runs")

if [ "$servers" -ne 1 ] || [ "$versions" -ne 2 ] || [ "$snapshots" -ne 2 ] || \
   [ "$bindings" -ne 1 ] || [ "$changes" -ne 4 ] || [ "$tests" -ne 1 ]; then
  echo "CPR-25 state mismatch: servers=$servers versions=$versions snapshots=$snapshots bindings=$bindings changes=$changes tests=$tests" >&2
  exit 1
fi

echo ""
echo "CPR-25 Tools: $servers stable server, $versions immutable versions/snapshots, $bindings exact binding, $changes governed changes and $tests read-only test report; acceptance criteria pass."
