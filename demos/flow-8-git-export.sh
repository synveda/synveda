#!/usr/bin/env sh
# FLOW-8: governed export, independent verification and controlled HTTPS push.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "flow8" "FLOW-8 — governed Git export and private remote transport"
cargo test -p synveda-git -- --nocapture
cargo test -p synveda-gateway --test git_bridge -- --nocapture
demo_finish
