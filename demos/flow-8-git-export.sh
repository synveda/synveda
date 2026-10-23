#!/usr/bin/env sh
# Governed export, independent verification and controlled HTTPS push.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "flow8" "Governed Git export and private remote transport"
cargo test -p synveda-git -- --nocapture
cargo test -p synveda-gateway --test git_bridge -- --nocapture
demo_finish
