#!/usr/bin/env sh
# CPR-29 acceptance demo: one executable /v1 catalogue, one generated browser
# client, and ordinary CLI/MCP clients that reach governed state only through
# that public boundary.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr29" "CPR-29 — exact public contract and client boundary"

echo "    Prove the mounted bearer-authenticated route catalogue and OpenAPI agree in both directions."
cargo test -p synveda-gateway --test openapi -- --nocapture

echo "    Exercise service-identity and audit application routes against the isolated database."
cargo test -p synveda-gateway --test service_identities -- --nocapture
cargo test -p synveda-gateway --test audit_query -- --nocapture

echo "    Prove ordinary service/audit and generic MCP modules carry no store authority."
cargo test -p synveda-cli --bin synveda service:: -- --nocapture
cargo test -p synveda-cli --bin synveda audit:: -- --nocapture
cargo test -p synveda-cli --bin synveda mcp:: -- --nocapture --test-threads=1

echo "    Regenerate nothing: the committed OpenAPI-derived console client must already be exact."
make check-api-types check-demos

demo_finish
