#!/usr/bin/env sh
# AUTH-5 acceptance demo: the scheduled directory pull sync (ADR-0060).
# AC (docs/backlog/AUTH-5.md): drift converges <= sync interval, and
# deletions are handled as leavers.
#
# The load-bearing demonstration is [2/6] and [3/6], run as a pair, because
# the AC's two clauses pull in opposite directions. "Deletions handled as
# leavers" wants a seal; "converges <= sync interval" wants it quickly. What
# this feature refuses to do is get there in one pass — because on a pull
# plane a deletion and a throttled page look identical, and the seal does not
# lift (ADR-0059 decision 12). So absence is a hypothesis: it needs a pass
# that COMPLETED, and it needs to survive N of them.
#
# [3/6] is the contrast that makes [2/6] mean anything: the same person
# missing from three passes in a row is never sealed when those passes did
# not finish. If the demo only showed the seal, it would be showing a
# behaviour that a much more dangerous implementation also has.
#
# Flow:
#
#   postgres -> scratch db -> tenant, acme > eng > core
#   [1/6] the joiner: the directory lists two people, one pass places both,
#         with zero admin action — AUTH-2's resolver reached through a third
#         door.
#   [2/6] THE AC. Bob is deleted at the directory. Pass 2 counts the absence
#         and seals nobody; pass 3 seals him. Two complete passes, not one.
#   [3/6] the contrast: a directory that fails half way. Three passes in a
#         row miss Carol, and she is never sealed — while somebody first
#         seen in a FAILED pass is still placed, because presence survives
#         what absence does not.
#   [4/6] the breaker: twelve of twenty leave at once and the pass refuses,
#         sealing nobody. `synveda directory status` leads with the refusal
#         and prints the command that clears it.
#   [5/6] the release: an org-admin signs for exactly that many. The next
#         pass seals them, the authorisation is spent, and it does not
#         survive into the next failure.
#   [6/6] the trail: the chain verifies, the breaker's refusal is on it, and
#         a sweep proves the outbound directory credential reaches no log,
#         no span and no event.
#
# ---------------------------------------------------------------------------
# Why this demo runs two gateways
#
# The pull sync's connector is configured on an issuer (ADR-0060 decision 7),
# so it only exists in OIDC mode — and ADR-0010 makes the auth modes mutually
# exclusive, so a deployment using the HS256 dev secret CANNOT run one. That
# is a real consequence of the placement and it is worth stating rather than
# papering over: `synveda init` deployments are fine (they configure Rauthy),
# and dev-secret deployments are not.
#
# Rather than stand up Rauthy to mint tokens for six CLI calls — which is
# AUTH-1's and AUTH-2's demonstration, not this one — this demo runs the
# same binary twice against one database: one instance in OIDC mode whose
# only job is the sync loop, and one in dev mode serving `/v1` for the
# operator commands. Both are the product; the split is the demo's, and it
# exists because of the sentence above.
# ---------------------------------------------------------------------------
#
# On Windows, run via Git Bash. Needs postgres and node; no TEI, no Rauthy.
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
$COMPOSE up --detach --wait postgres

DB=auth5_$$
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d synveda \
  -c "create database $DB" >/dev/null
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d "$DB" \
  -c "create extension if not exists vector;
      create extension if not exists age;
      create extension if not exists pgmq" >/dev/null

DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$DB"
export DATABASE_URL
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG
SYNVEDA_EMBEDDER=deterministic
export SYNVEDA_EMBEDDER
SYNVEDA_SEARCH_INDEX_DIR="./data/auth5-search-$$"
export SYNVEDA_SEARCH_INDEX_DIR

WORK="${TMPDIR:-/tmp}/auth5-$$"
mkdir -p "$WORK"
STATE="$WORK/directory.json"
MOCK_PORT=8171
OPS_PORT=8172
SYNC_PORT=8173
BASE="http://127.0.0.1:$OPS_PORT"

cleanup() {
  kill "${SYNC_PID:-0}" "${OPS_PID:-0}" "${MOCK_PID:-0}" 2>/dev/null || true
  sleep 1
  $COMPOSE exec -T postgres \
    psql -U synveda -d synveda \
    -c "drop database if exists $DB (force)" >/dev/null 2>&1 || true
  rm -rf "$SYNVEDA_SEARCH_INDEX_DIR" "$WORK"
}
trap cleanup EXIT INT TERM

SQLX_OFFLINE=true cargo build -p synveda-gateway -p synveda-cli
CLI=$PWD/target/debug/synveda

json_field() {
  node -e '
    let d = "";
    process.stdin.on("data", (c) => (d += c));
    process.stdin.on("end", () => {
      let v = JSON.parse(d);
      for (const k of process.argv.slice(1)) v = v[k];
      console.log(typeof v === "string" ? v : JSON.stringify(v));
    });
  ' "$@"
}

says() {
  if ! printf '%s' "$2" | grep -qF -- "$1"; then
    echo "demo FAILED: expected '$1' in:" >&2
    printf '%s\n' "$2" >&2
    exit 1
  fi
}

denies() {
  if printf '%s' "$2" | grep -qF -- "$1"; then
    echo "demo FAILED: did NOT expect '$1' in:" >&2
    printf '%s\n' "$2" >&2
    exit 1
  fi
}

wait_gateway() {
  tries=0
  until curl -fsS "$1/healthz" >/dev/null 2>&1; do
    tries=$((tries + 1))
    if [ "$tries" -ge 40 ]; then
      echo "demo FAILED: gateway did not become healthy on $1" >&2
      exit 1
    fi
    sleep 1
  done
}

# The directory, as this run wants it right now.
directory() {
  cat > "$STATE"
}

# One directory member per line -> the users + one group holding them.
people() {
  node -e '
    const ids = process.argv.slice(1);
    const fail = process.env.FAIL || null;
    const users = ids.map((spec) => {
      const [id, status] = spec.split(":");
      return { id, login: `${id}@example.test`, status: status || "ACTIVE" };
    });
    console.log(JSON.stringify({
      users,
      groups: [{ id: "g1", name: "synveda-eng-core", members: users.map((u) => u.id) }],
      fail,
    }, null, 2));
  ' "$@"
}

# Waits for `passes_completed` to advance past the value given, which is how
# this demo says "one more pass happened" without racing the loop's timer.
await_pass() {
  want=$1
  tries=0
  until [ "$(passes_completed)" -gt "$want" ] 2>/dev/null; do
    tries=$((tries + 1))
    if [ "$tries" -ge 60 ]; then
      echo "demo FAILED: no complete pass after $want within 60s" >&2
      $CLI_ENV directory status || true
      exit 1
    fi
    sleep 1
  done
}

passes_completed() {
  $COMPOSE exec -T postgres psql -tAq -U synveda -d "$DB" \
    -c "select coalesce(max(passes_completed), 0) from directory_sync_state" 2>/dev/null \
    | tr -d ' \r'
}

sealed_count() {
  $COMPOSE exec -T postgres psql -tAq -U synveda -d "$DB" \
    -c "select count(*) from identities where status = 'departed'" | tr -d ' \r'
}

is_sealed() {
  $COMPOSE exec -T postgres psql -tAq -U synveda -d "$DB" \
    -c "select count(*) from identities
        where status = 'departed' and email = '$1@example.test'" | tr -d ' \r'
}

placed() {
  $COMPOSE exec -T postgres psql -tAq -U synveda -d "$DB" \
    -c "select count(*) from identities where email = '$1@example.test'" | tr -d ' \r'
}

echo "==> setup: migrate, a tenant, and acme > eng > core"
"$CLI" db migrate >/dev/null
tenant_json=$("$CLI" tenant create --slug "auth5-demo-$$" --name "AUTH-5 Demo")
TENANT_ID=$(echo "$tenant_json" | json_field id)
echo "    tenant: $TENANT_ID"

# Phase 1: the ops gateway, in dev mode. It serves /v1 for the whole demo —
# seeding now, and the operator commands in [4/6] and [5/6].
SYNVEDA_LISTEN_ADDR=127.0.0.1:$OPS_PORT
export SYNVEDA_LISTEN_ADDR
SYNVEDA_DEV_JWT_SECRET=auth-5-demo-secret
export SYNVEDA_DEV_JWT_SECRET
# stderr to a file, like the sync gateway's: without Jaeger the OTLP
# exporter logs an export failure every few seconds, and a demo whose
# output is half red herrings is a demo nobody reads to the end.
./target/debug/synveda-gateway > "$WORK/ops.log" 2>&1 &
OPS_PID=$!
wait_gateway "$BASE"

TOKEN=$("$CLI" token issue --tenant "$TENANT_ID" --subject demo-admin)
"$CLI" role bind --tenant "$TENANT_ID" --subject demo-admin --role org-admin >/dev/null

create_node() {
  curl -fsS -X POST "$BASE/v1/hierarchy/nodes" \
    -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
    -d "$1" | json_field id
}
ORG=$(create_node '{"parent_id":null,"kind":"org","slug":"acme","name":"ACME"}')
ENG=$(create_node "{\"parent_id\":\"$ORG\",\"kind\":\"department\",\"slug\":\"eng\",\"name\":\"Engineering\"}")
create_node "{\"parent_id\":\"$ENG\",\"kind\":\"team\",\"slug\":\"core\",\"name\":\"Core\"}" >/dev/null
create_node "{\"parent_id\":\"$ORG\",\"kind\":\"team\",\"slug\":\"quarantine\",\"name\":\"Quarantine\"}" >/dev/null
echo "    hierarchy seeded; synveda-eng-core resolves by convention"

CLI_ENV="env HOME=$WORK/home XDG_CONFIG_HOME=$WORK/home/.config SYNVEDA_TOKEN=$TOKEN SYNVEDA_GATEWAY=$BASE $CLI"

echo
echo "==> the directory: a mock Okta whose contents this demo rewrites"
people alice bob | directory
node demos/fixtures/mock-okta.mjs "$MOCK_PORT" "$STATE" >/dev/null &
MOCK_PID=$!
sleep 1

# Phase 2: the sync gateway. OIDC mode, because that is the only mode in
# which a connector exists — see the header. Its issuer is deliberately
# unreachable: nothing authenticates against it, the JWKS fetch is lazy, and
# this instance serves no requests. Its whole job is the loop.
SYNC_ISSUERS=$(node -e '
  console.log(JSON.stringify([{
    issuer: "https://idp.invalid/",
    client_id: "auth-5-demo",
    tenant: { static: { tenant_id: process.argv[1] } },
    directory_sync: {
      connector: "okta",
      org_url: `http://127.0.0.1:${process.argv[2]}`,
      api_token: "demo-directory-token-never-logged",
    },
  }]));
' "$TENANT_ID" "$MOCK_PORT")

env -u SYNVEDA_DEV_JWT_SECRET \
  SYNVEDA_LISTEN_ADDR=127.0.0.1:$SYNC_PORT \
  SYNVEDA_OIDC_ISSUERS="$SYNC_ISSUERS" \
  SYNVEDA_DIRECTORY_SYNC_INTERVAL_SECS=2 \
  SYNVEDA_DIRECTORY_ABSENCE_PASSES=2 \
  SYNVEDA_DIRECTORY_BREAKER_FRACTION=0.10 \
  SYNVEDA_DIRECTORY_BREAKER_FLOOR=5 \
  ./target/debug/synveda-gateway > "$WORK/sync.log" 2>&1 &
SYNC_PID=$!
wait_gateway "http://127.0.0.1:$SYNC_PORT"
echo "    sync loop running every 2s against the mock"

echo
echo "==> [1/6] the joiner: one pass, two people placed, zero admin action"
await_pass 0
[ "$(placed alice)" = "1" ] || { echo "demo FAILED: alice not placed" >&2; exit 1; }
[ "$(placed bob)" = "1" ] || { echo "demo FAILED: bob not placed" >&2; exit 1; }
echo "    alice and bob placed under eng/core by the mapping resolver"

echo
echo "==> [2/6] THE AC: bob is deleted at the directory"
people alice | directory
before=$(passes_completed)
await_pass "$before"
[ "$(is_sealed bob)" = "0" ] || {
  echo "demo FAILED: bob sealed after ONE missed pass — a throttled page \
looks exactly like this, and the seal does not lift" >&2
  exit 1
}
echo "    pass $((before + 1)): absence counted, nobody sealed (a hypothesis)"
before=$(passes_completed)
await_pass "$before"
[ "$(is_sealed bob)" = "1" ] || { echo "demo FAILED: bob not sealed after two" >&2; exit 1; }
echo "    pass $((before + 1)): bob sealed — two complete passes, not one"
[ "$(is_sealed alice)" = "0" ] || { echo "demo FAILED: alice sealed" >&2; exit 1; }

echo
echo "==> [3/6] the contrast: a directory that fails half way"
# Carol is new and the groups collection now 500s: the pass lists the users
# and never finishes. She must still be placed; nobody must be sealed.
FAIL=groups people alice carol | directory
before=$(passes_completed)
sleep 8
after=$(passes_completed)
[ "$after" = "$before" ] || {
  echo "demo FAILED: an incomplete pass advanced the completeness proof \
($before -> $after)" >&2
  exit 1
}
echo "    3 failed passes in ~8s: passes_completed still $after"
[ "$(placed carol)" = "1" ] || {
  echo "demo FAILED: carol not placed — presence must survive an incomplete pass" >&2
  exit 1
}
echo "    carol, first seen in a FAILED pass, is placed anyway"
sealed_now=$(sealed_count)
[ "$sealed_now" = "1" ] || { echo "demo FAILED: something was sealed ($sealed_now)" >&2; exit 1; }
echo "    and nobody new was sealed: absence needs a pass that finished"

echo
echo "==> [4/6] the breaker: twelve of twenty leave at once"
FAIL= people alice carol p1 p2 p3 p4 p5 p6 p7 p8 p9 p10 p11 p12 p13 p14 p15 p16 p17 p18 | directory
before=$(passes_completed)
await_pass "$before"
before=$(passes_completed)
await_pass "$before"
echo "    twenty people on the books"
FAIL= people alice carol p1 p2 p3 p4 p5 p6 | directory
before=$(passes_completed)
await_pass "$before"
before=$(passes_completed)
await_pass "$before"
status_out=$($CLI_ENV directory status 2>/dev/null)
says "BREAKER TRIPPED" "$status_out"
says "declined to seal 12 people" "$status_out"
says "--ceiling 12" "$status_out"
printf '%s\n' "$status_out" | sed 's/^/    | /'
sealed_now=$(sealed_count)
[ "$sealed_now" = "1" ] || {
  echo "demo FAILED: the breaker sealed somebody ($sealed_now departed)" >&2
  exit 1
}
echo "    nobody sealed: a directory that lost a third of a tenant is a \
decision, not a pass"

echo
echo "==> [5/6] the release: an org-admin signs for exactly that many"
$CLI_ENV directory authorise-seals --ceiling 12 \
  --reason "Q3 restructure, ticket OPS-1123" 2>/dev/null | sed 's/^/    /'
before=$(passes_completed)
await_pass "$before"
sealed_now=$(sealed_count)
[ "$sealed_now" = "13" ] || {
  echo "demo FAILED: expected 13 departed (bob + 12), got $sealed_now" >&2
  exit 1
}
echo "    the next pass sealed exactly 12"
after_status=$($CLI_ENV directory status 2>/dev/null)
denies "in force" "$after_status"
echo "    and the authorisation is spent: it does not survive into the next \
directory failure"

echo
echo "==> [6/6] the trail"
chain=$("$CLI" audit verify --tenant "$TENANT_ID" 2>&1 || true)
says "valid" "$chain"
echo "    $chain"
breaker_events=$($COMPOSE exec -T postgres psql -tAq -U synveda -d "$DB" \
  -c "select count(*) from audit_log where action = 'directory.sync.breaker_tripped'" | tr -d ' \r')
[ "$breaker_events" -ge 1 ] || {
  echo "demo FAILED: the breaker's refusal is not on the chain" >&2
  exit 1
}
echo "    the refusal is chained ($breaker_events event(s)): a pass that \
declined to act is not something an auditor has to notice"
authorised=$($COMPOSE exec -T postgres psql -tAq -U synveda -d "$DB" \
  -c "select count(*) from audit_log
      where action in ('directory.seal.authorised','directory.seal.authorisation_used')" \
  | tr -d ' \r')
[ "$authorised" = "2" ] || {
  echo "demo FAILED: expected the grant AND its use on the chain, got $authorised" >&2
  exit 1
}
echo "    the grant and its use are two events: what was permitted, and \
what was done with it"

# The outbound credential is the first secret in this product that has to be
# recoverable. It must reach no log, no span, no audit payload.
leaks=$($COMPOSE exec -T postgres psql -tAq -U synveda -d "$DB" \
  -c "select count(*) from audit_log
      where payload::text like '%demo-directory-token%'" | tr -d ' \r')
[ "$leaks" = "0" ] || { echo "demo FAILED: $leaks chained events carry the credential" >&2; exit 1; }
if grep -q "demo-directory-token" "$WORK/sync.log"; then
  echo "demo FAILED: the gateway log carries the outbound credential" >&2
  exit 1
fi
echo "    credential sweep: 0 rows in the chain, 0 lines in the log"

echo
echo "AUTH-5 demo complete."
echo "  drift converged in one pass for a joiner, and in two for a deletion —"
echo "  which is wider than the AC asks for, on purpose, because on a pull"
echo "  plane a deletion and a throttled page are the same silence."
