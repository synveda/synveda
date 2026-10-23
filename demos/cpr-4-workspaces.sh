#!/usr/bin/env sh
# Workspaces, projects and canonical repository identity (ADR-0071).
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr4" "Workspaces and projects"

cargo test -p synveda-store --test workspaces -- --nocapture
cargo test -p synveda-gateway --test workspaces_api -- --nocapture

demo_finish
