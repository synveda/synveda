#!/usr/bin/env sh
# CPR-32 acceptance demo: one typed, revision-aware VedaFlow lifecycle for
# every governed context-platform artifact family.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr32" "CPR-32 — unified governed-artifact reviews"

echo "    Exercise typed references, exact-commit verdicts, separation, cancellation and execution across the common review plane."
cargo test -p synveda-gateway \
  --test knowledge_lifecycle \
  --test skills \
  --test tools \
  --test configuration_api \
  --test relaxations \
  --test okf_api \
  -- --nocapture

echo ""
echo "CPR-32 reviews: the gateway acceptance tests proved typed proposals across governed families, exact-commit verdicts, separated regulated effects, stale-verdict refusal and content-free audit metadata."
demo_finish
