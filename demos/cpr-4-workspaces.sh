#!/usr/bin/env sh
# CPR-4: workspaces, projects and canonical repository identity (ADR-0071).
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr4" "CPR-4 — workspaces and projects"

cargo test -p synveda-store --test workspaces -- --nocapture
cargo test -p synveda-gateway --test workspaces_api -- --nocapture

demo_finish
