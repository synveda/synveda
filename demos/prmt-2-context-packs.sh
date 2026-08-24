#!/usr/bin/env sh
# PRMT-2: project-scoped context packs.
# CPR-13 re-point: Context packs bind through current project/workspace scopes and are decided independently from Knowledge selection.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "prmt2" "PRMT-2 — project-scoped context packs"
echo "    Context packs bind through current project/workspace scopes and are decided independently from Knowledge selection."
cargo test -p synveda-gateway --test context_packs -- --nocapture
demo_finish
