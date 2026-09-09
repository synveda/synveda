#!/usr/bin/env sh
# CPR-17: generated public Knowledge API and browser contract (ADR-0082).
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr17" "CPR-17 — public Knowledge API and browser"

cargo test -p synveda-gateway --test knowledge_lifecycle \
  public_knowledge_api_is_current_governed_paginated_and_tenant_safe \
  -- --exact --nocapture
cargo test -p synveda-gateway --test openapi -- --nocapture
pnpm --filter @synveda/console test

demo_finish
