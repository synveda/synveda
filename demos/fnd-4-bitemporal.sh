#!/usr/bin/env sh
# FND-4 acceptance demo: as-of queries return historical row states, proven by
# the deterministic AC test and a property test over random operation
# histories (crates/synveda-store/tests/bitemporal.rs).
# AC (docs/backlog/FND-4.md): as-of query returns historical row states;
# property tests.
# On Windows, run via Git Bash. Needs only the postgres service, not the full
# dev stack.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL

# The tests apply the sqlx migrations themselves (synveda_store::migrate is
# idempotent), then exercise insert/update/delete/re-insert histories and
# check every as-of and bitemporal invariant.
cargo test -p synveda-store --test bitemporal

echo ""
echo "FND-4 bitemporal base tables: acceptance criteria green."
