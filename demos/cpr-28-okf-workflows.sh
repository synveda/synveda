#!/usr/bin/env sh
# CPR-28 acceptance demo: the filesystem-owning CLI applies the pinned adapter,
# then the public gateway lifecycle proves dry-run, candidate-only publication
# and deterministic round-trip preservation on an isolated current database.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr28" "CPR-28 — OKF CLI and project product workflows"

FIXTURE="demos/fixtures/cpr-28-okf-v02"
SYNVEDA_CLI="target/debug/synveda"
export SYNVEDA_CLI

echo "    Build the public client, then validate and inspect one real local directory without a gateway call."
cargo build --quiet -p synveda-cli --bin synveda
"$SYNVEDA_CLI" okf validate "$FIXTURE" --json
"$SYNVEDA_CLI" okf inspect "$FIXTURE" --source-revision pulseboard-release-42 --json

echo "    Exercise the public import, materialise, accept and export operations against the isolated database."
cargo test -p synveda-gateway --test okf_api -- --nocapture

echo "    Render the generated-contract project console and its exact request envelopes."
pnpm --filter @synveda/console test

demo_finish
