#!/usr/bin/env sh
# SKIL-3: skill quality evidence.
# CPR-23 re-point: deterministic quality evidence is retained on each exact
# immutable version and recomputed by the non-executing validation sandbox.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "skil3" "SKIL-3 — skill quality evidence"
echo "    Quality score/rubric evidence is version-bound and the validation sandbox executes no bundle code."
cargo test -p synveda-gateway --test skills -- --nocapture
demo_finish
