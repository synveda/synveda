#!/usr/bin/env sh
# Governed prompt artifacts on current scopes.
# Prompt publication resolves current scope anchors and remains a VedaFlow-reviewed artifact rather than session payload.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "prmt1" "Governed prompt artifacts on current scopes"
echo "    Prompt publication resolves current scope anchors and remains a VedaFlow-reviewed artifact rather than session payload."
cargo test -p synveda-gateway --test prompts -- --nocapture
demo_finish
