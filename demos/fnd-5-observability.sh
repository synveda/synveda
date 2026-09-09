#!/usr/bin/env sh
# FND-5: health, metrics and distributed-trace context across gateway, core and
# store boundaries (ADR-0007).
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "fnd5" "FND-5 — OpenTelemetry observability"

cargo test -p synveda-gateway --test observability -- --nocapture
node --test scripts/check-runtime-smoke.test.mjs

demo_finish
