#!/usr/bin/env bash
# `make db-test` — the workspace suite against a database of its own.
#
# # Why a scratch database
#
# The suite used to run against the long-lived dev database, and every test
# that admits a tenant left it there: 12,838 tenants across three days when
# this was written, ~4,000 a day, from `flow*`, `ctx*`, `mem*`, `skil*`,
# `rls-*`, `aud*` and `auth*` — the test families, not the demos. Nothing
# reaped them and nothing ever had.
#
# That is not only untidy. The sidecar indexer, the pack refresher, the
# promotion sweep and the retention sweep all visit **every active tenant
# per cycle**, which is why `demos/ctx-5-recall.sh` took a scratch database
# and said so: "on the shared dev database a just-admitted tenant waits
# minutes for its first pass". (That demo is deleted — CPR-12 — but the
# measurement below is why this script still does the same thing.) A suite whose fixtures wait on a sweep gets
# slower as the database fills, and eventually flaky. Measured before this
# change, same tests either side: `synveda-store --test hierarchy` 6.49s
# shared against 4.54s fresh, `--test rls` 0.90s against 0.47s.
#
# # Why this is a small change
#
# The tests were always written for their own database — 50 of them call
# `synveda_store::migrate` themselves, and the whole workspace (98 binaries)
# passes against an empty one. `db-test` simply never gave them one. So
# nothing here touches a test; the target hands them a fresh database and
# takes it away again.
#
# Cost: one `CREATE DATABASE`, three extensions, and ~0.7s of migration
# inside the first test that runs. Against a suite that takes minutes.
#
# # What it keeps
#
# A failed run keeps its database and prints the URL, because a suite that
# destroys the evidence on the way out is a suite you debug by re-running
# with a patch to stop it. `KEEP_TEST_DB=1` keeps it either way.
set -euo pipefail

cd "$(dirname "$0")/.."

COMPOSE=${COMPOSE:-docker compose -f deploy/compose/docker-compose.yml}
# The server, credentials and port come from the caller's DATABASE_URL so an
# override still points at the right postgres; only the database name is
# ours. Everything runs through `compose exec` because psql is not assumed
# on the host — the same reason every demo in this repo does.
SOURCE_URL=${DATABASE_URL:-postgres://synveda:synveda-dev@localhost:5432/synveda}
TEST_DB=${TEST_DB:-synveda_test_$$}
# Swap the database name, keep everything else. Parameter expansion rather
# than sed: the first attempt used a BSD-incompatible pattern and failed on
# macOS, where this is mostly run.
url_head=${SOURCE_URL%%\?*}
url_query=${SOURCE_URL#"$url_head"}
TEST_URL="${url_head%/*}/$TEST_DB$url_query"

psql_admin() {
  # shellcheck disable=SC2086
  $COMPOSE exec -T postgres psql -v ON_ERROR_STOP=1 -U synveda -d synveda "$@"
}
psql_test() {
  # shellcheck disable=SC2086
  $COMPOSE exec -T postgres psql -v ON_ERROR_STOP=1 -U synveda -d "$TEST_DB" "$@"
}

drop_test_db() {
  psql_admin -c "drop database if exists $TEST_DB with (force)" >/dev/null 2>&1 || true
}

# shellcheck disable=SC2086
$COMPOSE up --detach --wait postgres >/dev/null

# A leftover from a killed run would be migrated already and full of another
# run's tenants, which is the state this script exists to prevent.
drop_test_db

# Kept databases are evidence somebody may still want, so they are counted
# rather than reaped — but counted out loud, because this whole script
# exists because nobody was watching an unbounded pile grow.
kept=$(psql_admin -tAc \
  "select count(*) from pg_database where datname like 'synveda\\_test\\_%'" 2>/dev/null | tr -d ' ')
if [ "${kept:-0}" -gt 0 ]; then
  echo "db-test: $kept scratch database(s) kept from earlier failed runs; drop with"
  echo "  $COMPOSE exec -T postgres psql -U synveda -d synveda -tAc \\"
  echo "    \"select 'drop database ' || datname || ' with (force);' from pg_database \\"
  echo "     where datname like 'synveda\\_test\\_%'\" | $COMPOSE exec -T postgres psql -U synveda -d synveda"
fi
psql_admin -c "create database $TEST_DB" >/dev/null
# The extensions are the database's, not the migrations': `synveda db
# migrate` assumes they exist, exactly as the compose bootstrap provides
# them for the dev database.
psql_test -c "create extension if not exists vector;
              create extension if not exists age;
              create extension if not exists pgmq" >/dev/null

# Migrate once, here, rather than leaving it to the tests.
#
# Most of them call `synveda_store::migrate` themselves and are idempotent
# about it, but not all — `crates/synveda-gateway/tests/audit_events.rs`
# assumes the schema exists and failed with `relation "tenants" does not
# exist` the first time this ran against a bare database. It had been
# relying on the shared dev database having been migrated at some point in
# the past, which is a dependency nobody wrote down and nothing checked.
# Doing it up front also removes the race the alternative has: `cargo test
# --workspace` runs binaries in parallel, so fifty concurrent first-run
# migrations against an empty database is a thing to avoid rather than
# survive. Every demo in this repo migrates before it starts, for the same
# reason.
#
# `SQLX_OFFLINE=true` for this build and this build only, and it is not a
# convenience: the scratch database is **empty at this moment**, so any
# crate that has to recompile here would expand its `sqlx::query!` macros
# against a schema that does not exist yet and fail with `relation
# "audit_chain_heads" does not exist`. It went unnoticed for as long as the
# workspace happened to be built already — the migrate step then compiled
# nothing — and surfaced the first time a change to a low crate forced a
# rebuild inside this window. The checked-in `.sqlx` cache is exactly the
# right answer to "compile without a database", and it is what `make ci`
# uses for the same reason.
SQLX_OFFLINE=true DATABASE_URL="$TEST_URL" \
  cargo run -q -p synveda-cli --bin synveda -- db migrate
echo "db-test: $TEST_DB (scratch, migrated)"

# On interrupt the database goes; a Ctrl-C is not a failure worth keeping
# evidence from, and leaving one behind per interrupted run is how the
# accumulation started.
trap 'drop_test_db' INT TERM

status=0
DATABASE_URL="$TEST_URL" cargo test --workspace "$@" || status=$?

if [ "$status" -eq 0 ] && [ -z "${KEEP_TEST_DB:-}" ]; then
  drop_test_db
else
  echo
  echo "db-test: kept $TEST_DB for inspection"
  echo "  DATABASE_URL=$TEST_URL"
  echo "  drop it with: $COMPOSE exec -T postgres \\"
  echo "    psql -U synveda -d synveda -c 'drop database $TEST_DB with (force)'"
fi
exit "$status"
