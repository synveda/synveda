#!/usr/bin/env sh
# OPS-8: current epoch install contract.
# CPR-13 re-point: A release bootstraps the current schema and generated application contract without legacy routes or setup commands.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "ops8" "OPS-8 — current epoch install contract"
echo "    A release bootstraps the current schema and generated application contract without legacy routes or setup commands."
cargo test -p synveda-store --test epoch -- --nocapture
cargo test -p synveda-gateway --test openapi -- --nocapture
demo_finish
