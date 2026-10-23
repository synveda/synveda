#!/usr/bin/env sh
# Policy-authorised Knowledge and context audit queries.
# Audit reads resolve current scopes and expose identifiers, decisions and hashes without retired record payloads.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "aud2" "Policy-authorised Knowledge and context audit queries"
echo "    Audit reads resolve current scopes and expose identifiers, decisions and hashes without retired record payloads."
cargo test -p synveda-gateway --test audit_query -- --nocapture
demo_finish
