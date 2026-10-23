#!/usr/bin/env sh
# Skill quality evidence.
# Deterministic quality evidence is retained on each exact
# immutable version and recomputed by the non-executing validation sandbox.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "skil3" "Skill quality evidence"
echo "    Quality score/rubric evidence is version-bound and the validation sandbox executes no bundle code."
cargo test -p synveda-gateway --test skills -- --nocapture
demo_finish
