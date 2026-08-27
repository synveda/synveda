#!/usr/bin/env sh
# OPS-8 / CPR-36: one-runtime release and upgrade contract.
# A release bootstraps the current schema and generated application contract,
# packages no product-data seeder, and replaces its profile without stale files.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "ops8" "OPS-8 — one-runtime release and upgrade contract"
echo "    Compose, Helm and release bundles converge on one current public runtime."
make check-deploy
cargo test -p synveda-store --test epoch -- --nocapture
cargo test -p synveda-gateway --test openapi -- --nocapture
demo_finish
