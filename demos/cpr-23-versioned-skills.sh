#!/usr/bin/env sh
# One public VedaFlow path installs and updates a
# stable Agent Skill, retains two immutable versions, binds/rolls back the
# project, records exact-version usage and validates without executing code.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr23" "Immutable Skill versions and governed bindings"

echo "    Install, update, bind and rollback through the generated public API and VedaFlow."
cargo test -p synveda-gateway --test skills -- --nocapture

echo ""
echo "Skills: the gateway acceptance test proved stable aggregation, immutable versions, a revisioned binding, idempotent usage and non-executing validation through governed runtime paths."
