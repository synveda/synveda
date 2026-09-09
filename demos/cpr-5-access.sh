#!/usr/bin/env sh
# CPR-5: membership, groups, grants and invitations (ADR-0072).
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr5" "CPR-5 — governed team access"

cargo test -p synveda-store --test access -- --nocapture
cargo test -p synveda-gateway --test access_api -- --nocapture

demo_finish
