#!/usr/bin/env sh
# Bounded OKF v0.2 planning, candidate-only
# materialisation, governed acceptance and deterministic provenance export.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr27" "Bounded OKF v0.2 Knowledge exchange"

echo "    Plan inert bundle bytes, review candidates, publish through VedaFlow and export through the public API."
cargo test -p synveda-gateway --test okf_api -- --nocapture

echo ""
echo "OKF: the gateway acceptance test proved immutable v0.2 planning, inert artifacts, dry-run mappings, review candidates, governed Knowledge and normalised provenance."
