#!/usr/bin/env sh
# CPR-31 acceptance demo: one VedaFlow path for personal auto-apply and
# reviewed policy relaxations over current Knowledge.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr31" "CPR-31 — governed policy relaxations"

echo "    Exercise auto-apply, immutable revision, pending review, rejection and exact-subject access through public APIs."
cargo test -p synveda-gateway --test relaxations -- --nocapture

aggregates=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from policy_relaxations")
versions=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from policy_relaxation_versions")
changes=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from policy_relaxation_changes")
old_table=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from pg_class where relnamespace = 'public'::regnamespace and relname = 'policy_lapses'")

if [ "$aggregates" -lt 2 ] || [ "$versions" -lt 3 ] || \
   [ "$changes" -lt 5 ] || [ "$old_table" -ne 0 ]; then
  echo "CPR-31 state mismatch: aggregates=$aggregates versions=$versions changes=$changes old_table=$old_table" >&2
  exit 1
fi

echo ""
echo "CPR-31 relaxations: $aggregates stable aggregates, $versions immutable versions and $changes governed changes; the retired table remains absent."
demo_finish
