#!/usr/bin/env sh
# Attribute-aware decisions without scope confusion.
# Cedar entities are gathered from the current scope/grant model and denied objects reveal no cross-tenant identity.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "authz5" "Attribute-aware decisions without scope confusion"
echo "    Cedar entities are gathered from the current scope/grant model and denied objects reveal no cross-tenant identity."
cargo test -p synveda-gateway --test cedar_entity_sync -- --nocapture
cargo test -p synveda-gateway --test foundation_audit -- --nocapture
demo_finish
