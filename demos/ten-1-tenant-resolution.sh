#!/usr/bin/env sh
# TEN-1: tenant resolution through the current identity contract.
# CPR-13 re-point: Verified subjects resolve one tenant and principal scope; missing, suspended and cross-tenant claims fail closed.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "ten1" "TEN-1 — tenant resolution through the current identity contract"
echo "    Verified subjects resolve one tenant and principal scope; missing, suspended and cross-tenant claims fail closed."
cargo test -p synveda-gateway --test tenant_resolution -- --nocapture
demo_finish
