#!/usr/bin/env sh
# CPR-27 acceptance demo: bounded OKF v0.2 planning, candidate-only
# materialisation, governed acceptance and deterministic provenance export.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr27" "CPR-27 — bounded OKF v0.2 Knowledge exchange"

echo "    Plan inert bundle bytes, review candidates, publish through VedaFlow and export through the public API."
cargo test -p synveda-gateway --test okf_api -- --nocapture

jobs=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from import_jobs")
artifacts=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from import_artifacts")
mappings=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from import_mappings")
candidates=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from capture_candidates where source_kind = 'okf_import'")
knowledge=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from knowledge_items")
sources=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from knowledge_sources where source_type in ('okf', 'url')")

if [ "$jobs" -ne 1 ] || [ "$artifacts" -ne 3 ] || [ "$mappings" -ne 2 ] || \
   [ "$candidates" -ne 2 ] || [ "$knowledge" -ne 1 ] || [ "$sources" -ne 2 ]; then
  echo "CPR-27 state mismatch: jobs=$jobs artifacts=$artifacts mappings=$mappings candidates=$candidates knowledge=$knowledge sources=$sources" >&2
  exit 1
fi

echo ""
echo "CPR-27 OKF: $jobs immutable v0.2 plan, $artifacts artifacts, $mappings dry-run mappings, $candidates review candidates, $knowledge governed Knowledge item and $sources normalised provenance sources; acceptance criteria pass."
