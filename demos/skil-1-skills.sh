#!/usr/bin/env sh
# SKIL-1: governed Agent Skills registry.
# CPR-13 re-point: Skill bundles retain content-addressed files, scan evidence and policy-gated publication on current scopes.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "skil1" "SKIL-1 — governed Agent Skills registry"
echo "    Skill bundles retain content-addressed files, scan evidence and policy-gated publication on current scopes."
cargo test -p synveda-gateway --test skills -- --nocapture
demo_finish
