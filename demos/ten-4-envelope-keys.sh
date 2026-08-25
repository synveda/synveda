#!/usr/bin/env bash
# TEN-4 — per-tenant encryption keys (ADR-0064).
#
# The acceptance criterion is one sentence — "tenant export is unreadable
# without that tenant's key" — and a demo that only showed an export being
# written and read back would demonstrate the *easy* half. So this runs the
# claim in both directions: the export opens under the key that made it, and
# the same bytes refuse under every other key this script can construct.
#
# What it asserts, in order:
#
#   1. A tenant is admitted **with** a data key, in one command.
#   2. Its records and audit chain export into one sealed archive.
#   3. The archive's cleartext header names the tenant and the generation —
#      a backup vault full of anonymous blobs is not a backup.
#   4. The archive opens under this deployment's key and the body is the
#      records that went in.
#   5. **It does not open under another deployment's KEK.** The AC.
#   6. **It does not open under another tenant's key.** The AC's other half:
#      "that tenant's" is doing work in the sentence.
#   7. Nothing plaintext reaches the database — the sealed columns are
#      checked from SQL, not from the application that wrote them.
#   8. A rotation mints a new generation and the *old* archive still opens,
#      because the version is in its header (decision 6).
#   9. The key acts are on the audit chain, and the chain still verifies.
#
# Usage: demos/ten-4-envelope-keys.sh
#   DATABASE_URL   the database to run against (defaults to the dev one)
#   KEEP_DB=1      keep the scratch database on the way out
#
# Cost: one scratch database and one loopback gateway, no external network.
# The gateway exists only for the final public audit verification.
set -euo pipefail

cd "$(dirname "$0")/.."

# Compiling with the checked-in `.sqlx` cache, so a demo needs no database to
# build — ten-2-rls.sh's reasoning, and the same line.
SQLX_OFFLINE=true
export SQLX_OFFLINE

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
DB="synveda_ten4_demo_$$"
URL="postgres://synveda:synveda-dev@localhost:5432/${DB}"
WORK="$(mktemp -d)"
PORT=8139
GATEWAY_URL="http://127.0.0.1:${PORT}"
GATEWAY_PID=""

# A **scratch database**, for the reason `make db-test` takes one: this demo
# admits tenants and mints keys, and the deployment key is a per-database
# singleton — running it against the shared dev database would leave that
# database's deployment key wrapped by a KEK this script throws away.
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

$COMPOSE ps postgres >/dev/null 2>&1 || { echo "run \`make dev-up\` first"; exit 1; }

step "Building the CLI and gateway"
cargo build -q -p synveda-cli -p synveda-gateway
BIN="./target/debug/synveda"
GATEWAY="./target/debug/synveda-gateway"

step "A scratch database"
psql_admin -c "create database ${DB}" >/dev/null
psql_db -c "create extension if not exists vector; create extension if not exists btree_gin; create extension if not exists pgmq;" >/dev/null
DATABASE_URL="$URL" "$BIN" db migrate >/dev/null
ok "migrated ${DB}"

# Two KEKs and two tenants. The second of each exists so that steps 5 and 6
# have something real to fail against — a negative asserted against a key
# that was never constructed is not asserted at all.
step "Two key-encryption keys, minted rather than typed"
KEK_OURS="$("$BIN" kms keygen 2>/dev/null)"
KEK_THEIRS="$("$BIN" kms keygen 2>/dev/null)"
[ "${#KEK_OURS}" -eq 64 ] || fail "keygen did not produce 64 hex characters"
[ "$KEK_OURS" != "$KEK_THEIRS" ] || fail "two keygens produced the same key"
ok "two distinct 256-bit KEKs"

export DATABASE_URL="$URL"
export SYNVEDA_KMS_KEY="$KEK_OURS"

step "1. A tenant is admitted with a key, in one command"
CREATED="$("$BIN" tenant create --slug "ten4-demo-$$" --name 'TEN-4 demo')"
TENANT="$(printf '%s' "$CREATED" | python3 -c 'import sys,json; print(json.load(sys.stdin)["id"])')"
VERSION="$(printf '%s' "$CREATED" | python3 -c 'import sys,json; print(json.load(sys.stdin)["encryption_key"]["version"])')"
[ "$VERSION" = "1" ] || fail "a freshly admitted tenant should hold generation 1, got ${VERSION}"
ok "tenant ${TENANT} admitted at key generation 1"

OTHER="$("$BIN" tenant create --slug "ten4-other-$$" --name 'TEN-4 other' \
    | python3 -c 'import sys,json; print(json.load(sys.stdin)["id"])')"
ok "a second tenant ${OTHER}, for the negative in step 6"

step "2. A sealed export"
"$BIN" tenant export --tenant "$TENANT" --out "$WORK/tenant.svexp" >"$WORK/export.json"
BYTES="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["bytes"])' "$WORK/export.json")"
[ "$BYTES" -gt 0 ] || fail "the archive is empty"
ok "wrote ${BYTES} bytes"

step "3. The header is readable without any key"
"$BIN" tenant export-describe --archive "$WORK/tenant.svexp" >"$WORK/header.json"
HEADER_TENANT="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["tenant"])' "$WORK/header.json")"
[ "$HEADER_TENANT" = "$TENANT" ] || fail "the header names ${HEADER_TENANT}, not ${TENANT}"
ok "names its tenant and generation without opening anything"

step "4. It opens under this deployment's key"
"$BIN" tenant export-open --archive "$WORK/tenant.svexp" >"$WORK/body.json"
BODY_TENANT="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["tenant"])' "$WORK/body.json")"
[ "$BODY_TENANT" = "$TENANT" ] || fail "the body is for ${BODY_TENANT}"
ok "opened, and the body is this tenant's"

step "5. It does NOT open under another deployment's KEK — the AC"
if SYNVEDA_KMS_KEY="$KEK_THEIRS" "$BIN" tenant export-open \
        --archive "$WORK/tenant.svexp" >"$WORK/theirs.out" 2>"$WORK/theirs.err"; then
    fail "the archive opened under a KEK that never sealed it"
fi
grep -q "cannot open that export" "$WORK/theirs.err" \
    || fail "refused, but not with a message that says why: $(cat "$WORK/theirs.err")"
ok "refused: $(head -c 120 "$WORK/theirs.err")"

step "5b. And not with no key at all"
if SYNVEDA_KMS_KEY= "$BIN" tenant export-open \
        --archive "$WORK/tenant.svexp" >/dev/null 2>"$WORK/nokey.err"; then
    fail "the archive opened with no key configured"
fi
grep -q "SYNVEDA_KMS_KEY" "$WORK/nokey.err" || fail "the refusal does not name the missing key"
ok "refused, and named the variable an operator has to set"

step "6. It does NOT open under another tenant's key"
# Same deployment, same KEK, different tenant: only the AAD and the data key
# differ, which is what makes this a test of the binding rather than of the
# KEK. The archive is edited to claim the other tenant — exactly what an
# attacker with the file would try.
python3 - "$WORK/tenant.svexp" "$TENANT" "$OTHER" "$WORK/forged.svexp" <<'PY'
import json, struct, sys
raw = open(sys.argv[1], "rb").read()
magic, hlen, klen = raw[:8], struct.unpack(">I", raw[8:12])[0], struct.unpack(">I", raw[12:16])[0]
header = json.loads(raw[16:16 + hlen])
header["tenant"] = sys.argv[3]          # claim the other tenant
body = raw[16 + hlen:]
out = json.dumps(header, separators=(",", ":")).encode()
open(sys.argv[4], "wb").write(magic + struct.pack(">I", len(out)) + struct.pack(">I", klen) + out + body)
PY
if "$BIN" tenant export-open --archive "$WORK/forged.svexp" >/dev/null 2>"$WORK/forged.err"; then
    fail "an archive relabelled for another tenant opened"
fi
ok "refused: a relabelled archive does not open under the tenant it names"

step "7. Nothing plaintext reached the database"
# Asserted from SQL rather than from the application that wrote the rows:
# the claim is about what a dumped table contains.
"$BIN" directory set-credential --tenant "$TENANT" --config - <<'JSON' >/dev/null
{"connector":"okta","org_url":"https://example.okta.com","api_token":"s3cr3t-token-value"}
JSON
LEAKED="$(psql_db -c "select count(*) from tenant_secrets where encode(sealed, 'escape') like '%s3cr3t%'")"
[ "$LEAKED" = "0" ] || fail "the credential is readable in tenant_secrets"
SEALED="$(psql_db -c "select octet_length(sealed) from tenant_secrets where tenant_id = '${TENANT}'")"
[ "$SEALED" -gt 50 ] || fail "the stored value is too short to be an envelope"
ok "the credential is ${SEALED} bytes of envelope and contains no plaintext"

WRAPPED="$(psql_db -c "select octet_length(wrapped_dek) from tenant_keys where tenant_id = '${TENANT}'")"
[ "$WRAPPED" = "82" ] || fail "a wrapped data key should be 82 bytes, got ${WRAPPED}"
ok "the key column holds an 82-byte wrapped key, never a key"

step "8. Rotation, and the old archive still opens"
"$BIN" tenant key rotate --tenant "$TENANT" >"$WORK/rotate.json"
NEW_VERSION="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$WORK/rotate.json")"
[ "$NEW_VERSION" = "2" ] || fail "rotation should reach generation 2, got ${NEW_VERSION}"
"$BIN" tenant export-open --archive "$WORK/tenant.svexp" >/dev/null \
    || fail "an archive sealed under generation 1 stopped opening after a rotation"
ok "generation ${NEW_VERSION} is current, and the generation-1 archive still opens"

CURRENT="$(psql_db -c "select count(*) from tenant_keys where tenant_id = '${TENANT}' and retired_at is null")"
[ "$CURRENT" = "1" ] || fail "expected exactly one current key, found ${CURRENT}"
TOTAL="$(psql_db -c "select count(*) from tenant_keys where tenant_id = '${TENANT}'")"
[ "$TOTAL" = "2" ] || fail "expected the retired generation to be kept, found ${TOTAL} rows"
ok "one current key, one retired and kept — a dropped key is data made unreadable"

step "9. The acts are on the chain, and the chain verifies"
for action in tenant.key.provisioned tenant.exported tenant.key.rotated tenant.secret.stored; do
    COUNT="$(psql_db -c "select count(*) from audit_log where tenant_id = '${TENANT}' and action = '${action}'")"
    [ "$COUNT" -ge 1 ] || fail "no ${action} event on the chain"
done
ok "provisioned, exported, rotated and secret-stored are all chained"

LEAKS="$(psql_db -c "select count(*) from audit_log where tenant_id = '${TENANT}' and payload::text like '%s3cr3t%'")"
[ "$LEAKS" = "0" ] || fail "a credential reached an audit payload"
ok "no credential material in any payload — AUTH-4's sweep, applied here"

# Verification is an ordinary governed read now (CPR-29): start the public
# application boundary, resolve the caller's principal/root, seed only the
# dev-token operator door this scratch tenant cannot obtain from an IdP, and
# ask the API. The key/export commands above remain local custody operations.
export SYNVEDA_DEV_JWT_SECRET="ten4-demo-secret"
export SYNVEDA_LISTEN_ADDR="127.0.0.1:${PORT}"
export SYNVEDA_PUBLIC_URL="$GATEWAY_URL"
export SYNVEDA_SEARCH_INDEX_DIR="$WORK/search-index"
TOKEN="$("$BIN" token issue --tenant "$TENANT" --subject ten4-auditor)"
"$GATEWAY" >"$WORK/gateway.log" 2>&1 &
GATEWAY_PID=$!
for _ in $(seq 1 60); do
    curl -fsS "${GATEWAY_URL}/healthz" >/dev/null 2>&1 && break
    sleep 0.5
done
ME="$(curl -fsS -H "authorization: Bearer ${TOKEN}" "${GATEWAY_URL}/v1/me")" ||
    fail "the gateway did not resolve the audit caller: $(tail -5 "$WORK/gateway.log")"
ROOT_SCOPE="$(printf '%s' "$ME" | python3 -c 'import json,sys; print(json.load(sys.stdin)["onboarding"]["tenant_scope_id"])')"
psql_db -c "insert into scope_grants
              (id, tenant_id, scope_id, subject_kind, principal_id, role_key, source)
            values (gen_random_uuid(), '${TENANT}', '${ROOT_SCOPE}', 'principal',
                    'ten4-auditor', 'administrator', 'automation')" >/dev/null
SYNVEDA_GATEWAY="$GATEWAY_URL" SYNVEDA_TOKEN="$TOKEN" \
    "$BIN" audit verify >"$WORK/verify.txt" 2>&1 \
    || fail "the chain does not verify: $(cat "$WORK/verify.txt")"
ok "$(tr -d '\n' <"$WORK/verify.txt" | head -c 100)"

printf '\n\033[32mTEN-4 demo passed.\033[0m The export is unreadable without that tenant'"'"'s key,\n'
printf 'and "that tenant'"'"'s" is load-bearing: another KEK, no KEK, and another\n'
printf 'tenant'"'"'s key were each refused against the same bytes.\n'
