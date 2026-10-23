#!/usr/bin/env sh
# Governed Agent Skills registry.
# Stable skills retain immutable content-addressed versions,
# provenance and scan evidence; every install/update is a VedaFlow change.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "skil1" "Governed Agent Skills registry"
echo "    Stable Skills retain immutable content-addressed versions, provenance and scan evidence."
cargo test -p synveda-gateway --test skills -- --nocapture
demo_finish
