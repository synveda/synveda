#!/usr/bin/env sh
# Claude adapter session lifecycle.
# Captured Claude Code 2.1.241 frames cross the built hook, public session API, PDP, capture worker and audit chain.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "adpt1" "Claude adapter session lifecycle"
echo "    Captured Claude Code 2.1.241 frames cross the built hook, public session API, PDP, capture worker and audit chain."
pnpm --filter @synveda/claude-code-adapter build
cargo test -p synveda-gateway --test claude_lifecycle -- --nocapture
demo_finish
