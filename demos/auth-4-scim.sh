#!/usr/bin/env sh
# AUTH-4: SCIM identities and governed groups.
# CPR-13 re-point: SCIM uses the shared principal, group and group-membership substrate and its credential cannot cross into the application plane.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "auth4" "AUTH-4 — SCIM identities and governed groups"
echo "    SCIM uses the shared principal, group and group-membership substrate and its credential cannot cross into the application plane."
cargo test -p synveda-gateway --test scim a_directory_group_becomes_a_governed_group_with_its_members -- --exact --nocapture
cargo test -p synveda-gateway --test scim a_provisioning_credential_reaches_this_plane_and_nothing_else -- --exact --nocapture
demo_finish
