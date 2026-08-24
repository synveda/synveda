#!/usr/bin/env sh
# SKIL-2: skill bundle security gate.
# CPR-13 re-point: Traversal, credentials and unreviewed content are rejected before a skill can reach a project or session.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "skil2" "SKIL-2 — skill bundle security gate"
echo "    Traversal, credentials and unreviewed content are rejected before a skill can reach a project or session."
cargo test -p synveda-gateway --test skills a_bundle_carrying_a_credential_is_stopped_at_authoring -- --exact --nocapture
cargo test -p synveda-gateway --test skills a_seeded_malicious_skill_cannot_reach_published -- --exact --nocapture
demo_finish
