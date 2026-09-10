#!/usr/bin/env sh
# OPS-8 / CPR-45: one-runtime release and install contract.
# The deterministic gate packages and installs the canonical digest-bound
# reference without a source build, then checks the chart and public contract.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "ops8" "OPS-8 — one-runtime release and upgrade contract"
echo "    Compose, the reference archive and Helm share one current runtime contract."
make check-deploy
cargo test -p synveda-store --test epoch -- --nocapture
cargo test -p synveda-gateway --test openapi -- --nocapture
demo_finish
