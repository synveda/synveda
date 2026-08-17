#!/usr/bin/env bash
# CPR-2 — the fresh schema epoch, its startup guard, and the reset that is the
# only way past it (ADR-0068 decision 3, ADR-0069).
#
# The unit and integration tests cover the library and the readiness route.
# What they cannot reach is the claim the feature is actually about: that a
# **running gateway refuses to start** against a database from before the
# context-platform cut, and that the exact command it prints then works. That
# check lives in `main`, so it is demonstrated here, against a real binary and
# a real database, or it is not demonstrated at all.
#
# What it asserts, in order:
#
#   1. A fresh, empty database bootstraps to the current epoch, and the marker
#      records the epoch, the migration head, the moment and the release.
#   2. The gateway starts against it.
#   3. `/readyz` is ready.
#   4. A database from before the cut — the whole schema, rows in it, no
#      marker — is **refused by the gateway at startup**, which exits non-zero
#      and prints the exact reset command.
#   5. `synveda db migrate` refuses it too, and **writes nothing**: the rows it
#      was pointed at are still exactly there afterwards.
#   6. `synveda reset --database` without `--force` destroys nothing.
#   7. `synveda reset --database --force` builds a working current-epoch
#      database, and **nothing is carried across** — the tenant that was in
#      the old one is gone rather than translated.
#   8. Running the reset again is idempotent.
#   9. The gateway starts against the reset database and is ready.
#
# Usage: demos/cpr-2-schema-epoch.sh
#   KEEP_DB=1      keep the scratch database on the way out
#
# Cost: one scratch database, one port, no network. Under two minutes.
set -euo pipefail

cd "$(dirname "$0")/.."

# Compiling with the checked-in `.sqlx` cache, so a demo needs no database to
# build — ten-2-rls.sh's reasoning, and the same line.
SQLX_OFFLINE=true
export SQLX_OFFLINE

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
DB="synveda_cpr2_demo_$$"
URL="postgres://synveda:synveda-dev@localhost:5432/${DB}"
WORK="$(mktemp -d)"
# Not 8120: a contributor running this may have a deployment on the default
# port, and a demo that fights an operator's own gateway for a socket is a
# demo that fails for a reason it is not about.
PORT=8129
GATEWAY_URL="http://127.0.0.1:${PORT}"
GATEWAY_PID=""

psql_admin() { $COMPOSE exec -T postgres psql -U synveda -d postgres -qtAX -v ON_ERROR_STOP=1 "$@"; }
psql_db() { $COMPOSE exec -T postgres psql -U synveda -d "$DB" -qtAX -v ON_ERROR_STOP=1 "$@"; }

cleanup() {
    [ -n "$GATEWAY_PID" ] && kill "$GATEWAY_PID" 2>/dev/null || true
    if [ "${KEEP_DB:-0}" = "1" ]; then
        echo "keeping ${URL}"
    else
        psql_admin -c "drop database if exists ${DB} with (force)" >/dev/null 2>&1 || true
    fi
    rm -rf "$WORK"
}
trap cleanup EXIT

step() { printf '\n\033[1m== %s\033[0m\n' "$1"; }
ok() { printf '   \033[32mok\033[0m  %s\n' "$1"; }
fail() { printf '   \033[31mFAIL\033[0m %s\n' "$1"; exit 1; }

# Starts the gateway in the background and waits for it, or reports that it
# died. Returns non-zero when it never became healthy — which is the
# *expected* outcome in step 4, so callers decide what that means.
start_gateway() {
    local log="$1"
    "$GATEWAY" >"$log" 2>&1 &
    GATEWAY_PID=$!
    for _ in $(seq 1 60); do
        if ! kill -0 "$GATEWAY_PID" 2>/dev/null; then
            GATEWAY_PID=""
            return 1
        fi
        if curl -fsS "${GATEWAY_URL}/healthz" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.5
    done
    return 1
}

stop_gateway() {
    [ -n "$GATEWAY_PID" ] || return 0
    kill "$GATEWAY_PID" 2>/dev/null || true
    wait "$GATEWAY_PID" 2>/dev/null || true
    GATEWAY_PID=""
}

$COMPOSE ps postgres >/dev/null 2>&1 || { echo "run \`make dev-up\` first"; exit 1; }

step "Building"
cargo build -q -p synveda-cli -p synveda-gateway
BIN="./target/debug/synveda"
GATEWAY="./target/debug/synveda-gateway"

export DATABASE_URL="$URL"
export SYNVEDA_LISTEN_ADDR="127.0.0.1:${PORT}"
export SYNVEDA_PUBLIC_URL="$GATEWAY_URL"
export SYNVEDA_SEARCH_INDEX_DIR="$WORK/search-index"
# No auth mode: every /v1 request is rejected 401, which is fine — this demo
# is about the ops plane and about whether the process starts at all.
unset SYNVEDA_OIDC_ISSUERS SYNVEDA_DEV_JWT_SECRET 2>/dev/null || true

step "1. A fresh, empty database bootstraps to the current epoch"
psql_admin -c "create database ${DB}" >/dev/null
psql_db -c "create extension if not exists vector; create extension if not exists pgmq;" >/dev/null
"$BIN" db migrate >/dev/null 2>&1 || fail "migrating an empty database"
MARKER="$(psql_db -c "select epoch || '|' || migration_head || '|' || created_by_version from schema_metadata")"
EPOCH="${MARKER%%|*}"
REST="${MARKER#*|}"
HEAD="${REST%%|*}"
VERSION="${REST#*|}"
[ "$EPOCH" = "1" ] || fail "expected epoch 1, got '${EPOCH}'"
[ -n "$HEAD" ] || fail "the marker records no migration head"
[ -n "$VERSION" ] || fail "the marker records no creating version"
ROWS="$(psql_db -c "select count(*) from schema_metadata")"
[ "$ROWS" = "1" ] || fail "the marker is a single-row table, found ${ROWS}"
ok "epoch ${EPOCH} at migration ${HEAD}, created by ${VERSION}"

step "2. The gateway starts against it"
start_gateway "$WORK/gateway-1.log" || fail "the gateway did not start: $(tail -3 "$WORK/gateway-1.log")"
grep -q "schema epoch accepted" "$WORK/gateway-1.log" ||
    fail "the gateway did not record accepting the epoch"
ok "started (pid ${GATEWAY_PID})"

step "3. It is ready"
[ "$(curl -fsS "${GATEWAY_URL}/readyz")" = "ready" ] || fail "/readyz did not answer ready"
ok "/readyz: ready"
stop_gateway

# The fixture the whole feature is about: an operator's existing database.
# The schema, rows in it, and no marker — because the marker did not exist
# when it was built. Reproduced by taking the marker away and the migrator's
# record of the migration that creates it, which is precisely the two things a
# pre-cut database lacks.
step "A database from before the cut"
# A KEK so that admitting the tenant provisions its key in one command
# (TEN-4, ADR-0064) rather than printing the note that it could not. Thrown
# away with the scratch database.
SYNVEDA_KMS_KEY="$("$BIN" kms keygen 2>/dev/null)"
export SYNVEDA_KMS_KEY
"$BIN" tenant create --slug "cpr2-demo-$$" --name 'CPR-2 demo' >/dev/null
BEFORE="$(psql_db -c "select count(*) from tenants")"
[ "$BEFORE" = "1" ] || fail "the fixture has no tenant to lose"
psql_db -c "drop table schema_metadata; delete from _sqlx_migrations where version >= 39;" >/dev/null
ok "one tenant, full schema, no epoch marker"

step "4. The gateway refuses to start against it"
if start_gateway "$WORK/gateway-2.log"; then
    stop_gateway
    fail "the gateway started against a database from before the cut"
fi
grep -q "synveda reset --database --force" "$WORK/gateway-2.log" ||
    fail "the refusal does not print the reset command: $(cat "$WORK/gateway-2.log")"
grep -q "hard cut" "$WORK/gateway-2.log" ||
    fail "the refusal does not say why it is a refusal rather than an upgrade"
ok "refused, and said exactly what to run"

step "5. \`db migrate\` refuses it too, and writes nothing"
if "$BIN" db migrate >"$WORK/migrate.log" 2>&1; then
    fail "db migrate advanced a database from before the cut"
fi
grep -q "synveda reset --database --force" "$WORK/migrate.log" ||
    fail "the refusal does not print the reset command: $(cat "$WORK/migrate.log")"
STILL="$(psql_db -c "select to_regclass('public.schema_metadata') is null")"
[ "$STILL" = "t" ] || fail "a refused migration created the marker anyway"
[ "$(psql_db -c "select count(*) from tenants")" = "$BEFORE" ] ||
    fail "a refused migration changed the rows it refused to migrate"
ok "refused before it touched anything; the tenant is still there"

step "6. Without --force nothing is destroyed"
if "$BIN" reset --database >"$WORK/noforce.log" 2>&1; then
    fail "reset destroyed a database without --force"
fi
grep -q -- "--force" "$WORK/noforce.log" || fail "the refusal does not name --force"
[ "$(psql_db -c "select count(*) from tenants")" = "$BEFORE" ] ||
    fail "reset without --force destroyed something"
ok "refused; the database is untouched"

step "7. \`reset --database --force\` builds a working current-epoch database"
"$BIN" reset --database --force >"$WORK/reset.log" 2>&1 ||
    fail "reset failed: $(tail -5 "$WORK/reset.log")"
grep -q "epoch 1" "$WORK/reset.log" || fail "reset did not report the epoch it built"
[ "$(psql_db -c "select epoch from schema_metadata")" = "1" ] ||
    fail "the reset database is not at the current epoch"
CARRIED="$(psql_db -c "select count(*) from tenants")"
[ "$CARRIED" = "0" ] ||
    fail "${CARRIED} row(s) survived the reset — there is no migrator, and this is what says so"
ok "fresh at epoch 1, and nothing was carried across"

step "8. Running it again is idempotent"
"$BIN" reset --database --force >"$WORK/reset-2.log" 2>&1 ||
    fail "the second reset failed: $(tail -5 "$WORK/reset-2.log")"
[ "$(psql_db -c "select epoch from schema_metadata")" = "1" ] ||
    fail "the second reset left a different database"
[ "$(psql_db -c "select count(*) from tenants")" = "0" ] || fail "the second reset left rows"
ok "same database, twice"

step "9. The gateway starts against the reset database and is ready"
start_gateway "$WORK/gateway-3.log" || fail "the gateway did not start after a reset"
[ "$(curl -fsS "${GATEWAY_URL}/readyz")" = "ready" ] || fail "/readyz did not answer ready"
ok "started and ready"
stop_gateway

printf '\n\033[1;32mCPR-2 demo passed.\033[0m\n'
printf 'A pre-cut database is refused at startup, refused by the migrator, and\n'
printf 'destroyed rather than translated by the one command the refusal names.\n'
