#!/usr/bin/env sh
# CTX-5 acceptance demo: recall API + MCP tool (ADR-0042).
# AC (docs/backlog/CTX-5.md): MCP client E2E; as-of returns historically
# accurate context (`--as-of` demo).
#
# The demo is shaped around the two claims worth doubting.
#
# THE WIDENING. One corpus, one identity, one pack — the real `standard`
# pack, whose department permit has been unreachable since ADR-0024 fixed
# inject's universe at the caller's chain. Alice on `platform` asks twice:
# `POST /v1/inject` returns nothing of the sibling team's material, because
# it never asks about that scope; `synveda recall --query` returns it,
# because it does. Same PDP, same permit, asked at last.
#
# AS-OF. Alice states a fact and then corrects it, both through `observe`
# and the real extraction pipeline. `synveda recall --as-of <before>`
# returns what was true then; the same command without the flag returns the
# correction. Then the flag is pointed at a moment when alice could read
# the payments runbook, *after* her grant is withdrawn — and it comes back
# empty, because as-of rewinds the corpus and never the authority.
#
# MCP. A real client speaks real JSON-RPC over stdio to the real server —
# `initialize`, `tools/list`, `tools/call` — and the tool answers from the
# live gateway under alice's own bearer. One tool is offered, because "ONE
# MCP tool" is the feature text.
#
# The credential half is ADPT-1's and `demos/adpt-1-claude-code.sh` proves
# it against live Rauthy; here the callers carry dev tokens through
# SYNVEDA_TOKEN (the override ADR-0027 kept for CI and demos), so this demo
# needs only postgres. On Windows, run via Git Bash.
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
$COMPOSE up --detach --wait postgres

# A scratch database per run — the FLOW-4/EVAL-1/MEM-6 discipline: the
# sidecar indexer and the pack refresher visit every active tenant per
# cycle, and on the shared dev database a just-admitted tenant waits
# minutes for its first pass.
CTX5_DB=ctx5_$$
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d synveda \
  -c "create database $CTX5_DB" >/dev/null
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d "$CTX5_DB" \
  -c "create extension if not exists vector;
      create extension if not exists age;
      create extension if not exists pgmq" >/dev/null

DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$CTX5_DB"
export DATABASE_URL
SYNVEDA_SEARCH_INDEX_DIR="./data/ctx5-search-$$"
export SYNVEDA_SEARCH_INDEX_DIR
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG
SYNVEDA_LISTEN_ADDR=127.0.0.1:8150
export SYNVEDA_LISTEN_ADDR
GATEWAY=http://127.0.0.1:8150
SYNVEDA_DEV_JWT_SECRET=ctx-5-demo-secret
export SYNVEDA_DEV_JWT_SECRET
SYNVEDA_EXTRACTOR=deterministic
export SYNVEDA_EXTRACTOR
SYNVEDA_EMBEDDER=deterministic
export SYNVEDA_EMBEDDER
SYNVEDA_EXTRACTION_POLL_MS=300
export SYNVEDA_EXTRACTION_POLL_MS
SYNVEDA_SEARCH_POLL_MS=300
export SYNVEDA_SEARCH_POLL_MS
SYNVEDA_POLICY_REFRESH_SECS=2
export SYNVEDA_POLICY_REFRESH_SECS

cargo build -p synveda-gateway -p synveda-cli
( cd adapters/claude-code && npm run build >/dev/null )

psql_t() {
  $COMPOSE exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d "$CTX5_DB" -tAc "$1"
}

echo "==> migrate + admit a tenant"
./target/debug/synveda db migrate
tenant_id=$(./target/debug/synveda tenant create \
  --slug "ctx5-demo-$$" --name "CTX-5 Demo Tenant" | \
  node -e 'let d="";process.stdin.on("data",c=>d+=c);process.stdin.on("end",()=>console.log(JSON.parse(d).id));')
echo "    tenant: $tenant_id"
admin_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-admin)
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null

cleanup() {
  kill "${GATEWAY_PID:-0}" 2>/dev/null || true
  sleep 1
  $COMPOSE exec -T postgres \
    psql -U synveda -d synveda -c "drop database if exists $CTX5_DB (force)" >/dev/null 2>&1 || true
  rm -rf "$SYNVEDA_SEARCH_INDEX_DIR"
  rm -f "/tmp/ctx5-mcp-$$.mjs"
}
trap cleanup EXIT INT TERM

./target/debug/synveda-gateway &
GATEWAY_PID=$!

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS "$GATEWAY/healthz" >/dev/null 2>&1; do
  tries=$((tries + 1))
  if [ "$tries" -ge 30 ]; then
    echo "demo FAILED: gateway did not become healthy" >&2
    exit 1
  fi
  sleep 1
done

api() {
  tok=$1; method=$2; path=$3; body=${4:-}
  if [ -n "$body" ]; then
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
      -H "Content-Type: application/json" -d "$body" "$GATEWAY$path"
  else
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" "$GATEWAY$path"
  fi
}

field() {
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

records_now() {
  psql_t "select count(*) from records where tenant_id = '$tenant_id'"
}

wait_for_records() {
  want=$1; tries=0
  while :; do
    [ "$(records_now)" = "$want" ] && return 0
    tries=$((tries + 1))
    if [ "$tries" -ge 80 ]; then
      echo "demo FAILED: expected $want records, stuck at $(records_now)" >&2
      exit 1
    fi
    sleep 0.5
  done
}

# The sidecar is a polling replica (ADR-0024 decision 4): a query's lexical
# leg cannot see a record until the indexer's next sweep. The demo waits
# for convergence rather than racing it — and waits by asking the product,
# so what it waits for is exactly what it then asserts.
#
# recall_until <query> <needle> — the recall output once it contains
# <needle>, or a failure that says what it was waiting for.
recall_until() {
  tries=0
  while :; do
    out=$(./target/debug/synveda recall --query "$1" --quiet 2>/dev/null || true)
    if printf '%s' "$out" | grep -q "$2"; then
      printf '%s' "$out"
      return 0
    fi
    tries=$((tries + 1))
    if [ "$tries" -ge 40 ]; then
      echo "demo FAILED: recall never returned '$2' for query '$1'" >&2
      printf '%s\n' "$out" >&2
      exit 1
    fi
    sleep 0.5
  done
}

# seed_user <subject> <parent scope> — a placed user identity, the shape
# JIT provisioning produces on first login (AUTH-2). Written directly here
# for the same reason AUTHZ-5 writes it directly: this demo is about the
# read path, and standing up an IdP to place two readers would be scenery.
seed_user() {
  uid=$(psql_t "select gen_random_uuid()")
  leaf=$(psql_t "select gen_random_uuid()")
  psql_t "begin;
          insert into hierarchy_nodes (id, tenant_id, parent_id, kind, slug, name, depth, path)
          select '$leaf'::uuid, '$tenant_id'::uuid, '$2'::uuid, 'user', 'u-$1', '$1',
                 n.depth + 1, n.path || '/u-$1'
          from hierarchy_nodes n where n.id = '$2';
          insert into hierarchy_closure (tenant_id, ancestor_id, descendant_id, distance)
          select '$tenant_id'::uuid, c.ancestor_id, '$leaf'::uuid, c.distance + 1
          from hierarchy_closure c where c.descendant_id = '$2'
          union all select '$tenant_id'::uuid, '$leaf'::uuid, '$leaf'::uuid, 0;
          insert into identities (id, tenant_id, subject, scope_id, kind)
          values ('$uid', '$tenant_id', '$1', '$leaf', 'user');
          commit;" >/dev/null
}

# seed_record <scope> <owner subject> <sensitivity> <text> — team-scoped
# material, the way AUTHZ-4 and AUTHZ-5 seed theirs. `observe` writes at
# the caller's *home* scope (MEM-1, ADR-0020 decision 4), so a team's
# shelf is stocked either by publication or directly; this demo is about
# the read path, and a promotion pipeline here would be scenery. Alice's
# own material — the half the as-of demo turns on — goes through the real
# observe pipeline below.
seed_record() {
  owner=$(psql_t "select id from identities where tenant_id = '$tenant_id' and subject = '$2'")
  rid=$(psql_t "select gen_random_uuid()")
  psql_t "insert into records (id, tenant_id, scope_id, owner_id, kind, class,
                               content, sensitivity, provenance, valid_from)
          values ('$rid', '$tenant_id', '$1', '$owner', 'derived', 'procedure',
                  '$4', '$3', '{\"source\":\"ctx-5 demo\"}'::jsonb, now());
          insert into record_embeddings (record_id, tenant_id, model, dim, embedding)
          values ('$rid', '$tenant_id', 'hash@1', 4, '[0.25,0.25,0.25,0.25]');" >/dev/null
  printf '%s' "$rid"
}

observe() {
  tok=$1; session=$2; key=$3; text=$4
  body="{\"session_id\":\"$session\",\"events\":[{\"idempotency_key\":\"$key\",
    \"kind\":\"tool_result\",\"payload\":{\"text\":\"$text\"},
    \"occurred_at\":\"$(date -u +%Y-%m-%dT%H:%M:%SZ)\"}]}"
  accepted=$(api "$tok" POST /v1/observe "$body" | field accepted)
  [ "$accepted" = "1" ] || {
    echo "demo FAILED: observe was not accepted ($accepted)" >&2
    exit 1
  }
}

echo
echo "==> the org: two teams under one department, governed by \`standard\`"
org_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
eng_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"department\",\"slug\":\"engineering\",\"name\":\"Engineering\"}" |
  field id)
platform_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$eng_id\",\"kind\":\"team\",\"slug\":\"platform\",\"name\":\"Platform\"}" | field id)
payments_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$eng_id\",\"kind\":\"team\",\"slug\":\"payments\",\"name\":\"Payments\"}" | field id)
api "$admin_token" PUT "/v1/hierarchy/nodes/$org_id/policy" '{"name":"standard"}' >/dev/null
echo "    acme > engineering > {platform, payments}, pack=standard at the org"

# Users, not service identities: AUTH-3 confines a service token to its
# anchor's subtree (ADR-0018 decision 4), which would deny the cross-team
# read this demo exists to show — and would show it for the wrong reason.
# The AUTHZ-5 demo's helper, verbatim.
seed_user alice "$platform_id"
seed_user bea "$payments_id"
alice_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject alice)
bea_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject bea)
echo "    alice on platform, bea on payments"

echo
echo "==> the payments team's shelf, and a note of alice's own"
runbook_id=$(seed_record "$payments_id" bea internal \
  "Settlement mismatch procedure: freeze the reconciliation job, compare the ledger tail against the acquirer statement, then page the payments on-call.")
seed_record "$payments_id" bea confidential \
  "Acquirer escalation contacts and the out-of-hours bridge PIN." >/dev/null
observe "$alice_token" alice-1 alice-1-a \
  "Platform prefers rebase-and-merge for feature branches."
wait_for_records 3
echo "    payments holds an internal runbook and a confidential note;"
echo "    alice's own note came through the real observe pipeline"

echo
echo "==> THE WIDENING: the same identity, the same pack, two surfaces"
block=$(api "$alice_token" POST /v1/inject '{"task":"settlement mismatch acquirer statement"}' | field text)
if printf '%s' "$block" | grep -q "acquirer statement"; then
  echo "demo FAILED: inject's universe is the chain; payments is not on it" >&2
  exit 1
fi
echo "    POST /v1/inject  -> the sibling team's runbook is ABSENT"
echo "                        (ADR-0024 decision 1: inject asks about the chain)"

SYNVEDA_TOKEN=$alice_token
export SYNVEDA_TOKEN
SYNVEDA_GATEWAY=$GATEWAY
export SYNVEDA_GATEWAY

recall_out=$(recall_until "settlement mismatch acquirer statement" "acquirer statement")
printf '%s\n' "$recall_out" | sed 's/^/      /'
echo "    synveda recall -> the runbook IS served"
echo "                        (ADR-0042 decision 2: recall asks about every"
echo "                         scope that could contribute — the department"
echo "                         permit \`standard\` has had since AUTHZ-2, and"
echo "                         which nothing in the product could exercise)"

echo
echo "==> AS-OF: a fact, then its correction"
observe "$alice_token" alice-2 alice-2-a "The deploy freeze runs from December 15th."
wait_for_records 4
before=$(psql_t "select now()")
echo "    stated, and the clock reads $before"

observe "$alice_token" alice-3 alice-3-a "The deploy freeze runs from December 1st."
wait_for_records 5
echo "    corrected"

echo
echo "    synveda recall --query 'deploy freeze'   (now)"
now_out=$(recall_until "deploy freeze December" "December 1st")
printf '%s\n' "$now_out" | sed 's/^/      /'

echo
echo "    synveda recall --as-of '$before'      (no question: the whole"
echo "                                          corpus as it stood then)"
then_out=$(./target/debug/synveda recall \
  --as-of "$(printf '%s' "$before" | sed 's/ /T/; s/+00$/Z/')" --quiet)
printf '%s\n' "$then_out" | sed 's/^/      /'

if ! printf '%s' "$then_out" | grep -q "December 15th"; then
  echo "demo FAILED: as-of must return what was known then" >&2
  exit 1
fi
if printf '%s' "$then_out" | grep -q "December 1st"; then
  echo "demo FAILED: as-of must not return a correction made after it" >&2
  exit 1
fi
echo "    the earlier instant serves the earlier truth, and NOT the"
echo "    correction that had not been made yet — the AC's 'historically"
echo "    accurate context' (ADR-0042 decision 7)."
echo
echo "    the bare instant is its own shape on purpose (decision 14): a"
echo "    *query* as-of ranks over the search indexes, and those hold"
echo "    current truth by construction (ADR-0024 decision 4), so they"
echo "    cannot rank a fact the live corpus has since closed. A sweep"
echo "    reads the corpus itself, which is why it is the complete answer."

echo
echo "==> AS-OF NEVER REWINDS THE AUTHORITY"
# `standard` reaches the department subtree at the *working* tiers only —
# `confidential` is held to explicitly granted scopes under every pack
# (ADR-0038 decision 4). So the confidential note at payments is the tier
# boundary this beat needs, and a curator binding is the grant that crosses
# it.
denied=$(./target/debug/synveda recall --query "acquirer escalation bridge PIN" --quiet)
if printf '%s' "$denied" | grep -q "bridge PIN"; then
  echo "demo FAILED: standard must not reach confidential across the department" >&2
  exit 1
fi
echo "    the confidential note is out of reach: the department permit"
echo "    carries the working tiers, not this one"

./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject alice --scope "$payments_id" --role curator >/dev/null
held_out=$(recall_until "acquirer escalation bridge PIN" "bridge PIN")
printf '%s\n' "$held_out" | sed 's/^/      /'
held=$(psql_t "select now()")
echo "    a curator binding at payments admits it — and marks it"

./target/debug/synveda role unbind --tenant "$tenant_id" \
  --subject alice --scope "$payments_id" --role curator >/dev/null
after=$(./target/debug/synveda recall --query "acquirer escalation bridge PIN" \
  --as-of "$(printf '%s' "$held" | sed 's/ /T/; s/+00$/Z/')" --quiet)
printf '%s\n' "$after" | sed 's/^/      /'
if printf '%s' "$after" | grep -q "bridge PIN"; then
  echo "demo FAILED: an instant must not carry a withdrawn grant back with it" >&2
  exit 1
fi
echo "    the grant is withdrawn, and the SAME instant — one at which alice"
echo "    demonstrably could read it — returns nothing. as-of rewinds the"
echo "    corpus and never the authority (ADR-0042 decision 8): the PDP"
echo "    decides with the roles held now, so a timestamp is not a credential."

echo "==> MCP CLIENT E2E: real JSON-RPC over stdio to the real server"
cat >"/tmp/ctx5-mcp-$$.mjs" <<'MCPCLIENT'
// A minimal MCP client: spawn the server, speak the protocol, assert.
// Deliberately not the adapter's own code — a client that shared the
// server's helpers would prove less than one that does not.
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";

const server = spawn("node", [process.argv[2]], {
  stdio: ["pipe", "pipe", "inherit"],
  env: process.env,
});
const lines = createInterface({ input: server.stdout });
const pending = new Map();
lines.on("line", (line) => {
  if (!line.trim()) return;
  const message = JSON.parse(line);
  const resolve = pending.get(message.id);
  if (resolve) { pending.delete(message.id); resolve(message); }
});
let nextId = 1;
const call = (method, params) =>
  new Promise((resolve) => {
    const id = nextId++;
    pending.set(id, resolve);
    server.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
  });

const fail = (why) => { console.error(`demo FAILED: ${why}`); server.kill(); process.exit(1); };

const init = await call("initialize", {
  protocolVersion: "2025-06-18",
  capabilities: {},
  clientInfo: { name: "ctx-5-demo", version: "0" },
});
if (init.result?.serverInfo?.name !== "synveda") fail("initialize did not name the server");
console.log(`      initialize    -> ${init.result.serverInfo.name}, protocol ${init.result.protocolVersion}`);
server.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" })}\n`);

const listed = await call("tools/list", {});
const tools = listed.result?.tools ?? [];
if (tools.length !== 1 || tools[0].name !== "recall") fail(`expected exactly one tool named recall, got ${JSON.stringify(tools.map((t) => t.name))}`);
console.log(`      tools/list    -> ${tools.length} tool: ${tools[0].name}`);

const called = await call("tools/call", {
  name: "recall",
  arguments: { query: "settlement mismatch acquirer statement" },
});
const text = called.result?.content?.[0]?.text ?? "";
if (called.result?.isError) fail(`the tool returned an error: ${text}`);
if (!text.includes("acquirer statement")) fail(`the tool did not serve the runbook:\n${text}`);
if (!text.includes("Watermark:")) fail("the answer is not watermarked");
console.log("      tools/call    -> served, watermarked, and labelled:");
for (const line of text.split("\n")) console.log(`        ${line}`);

const asOf = await call("tools/call", {
  name: "recall",
  arguments: { query: "deploy freeze December", as_of: process.argv[3] },
});
const historical = asOf.result?.content?.[0]?.text ?? "";
if (!historical.includes("December 15th")) fail(`as-of through MCP did not return the earlier truth:\n${historical}`);
console.log("      tools/call    -> and the same tool answers as-of, historically");

server.kill();
MCPCLIENT

node "/tmp/ctx5-mcp-$$.mjs" \
  "adapters/claude-code/dist/mcp-server.mjs" \
  "$(printf '%s' "$before" | sed 's/ /T/; s/+00$/Z/')"
echo "    the MCP half of the AC, end to end against the live gateway"

echo
echo "==> the trail: one context.recalled per recall, and the chain over it"
psql_t "select count(*) from audit_log
        where tenant_id = '$tenant_id' and action = 'context.recalled'" |
  sed 's/^/    context.recalled events: /'
psql_t "select 'mode=' || (payload ->> 'mode') ||
          '  scopes_decided=' || (payload ->> 'scopes_decided') ||
          '  served=' || (payload ->> 'served') ||
          '  query_hash=' || coalesce(left(payload ->> 'query_hash', 12), 'none')
        from audit_log
        where tenant_id = '$tenant_id' and action = 'context.recalled'
        order by seq desc limit 3" | sed 's/^/    /'
echo "    (the question rides as a hash, never as text — ADR-0021)"
./target/debug/synveda audit verify --tenant "$tenant_id"

echo
echo "==> the AC suites"
cargo test -p synveda-gateway --test recall
cargo test -p synveda-gateway --test tiered
( cd adapters/claude-code && node --test "dist/mcp.test.mjs" )

echo
echo "CTX-5 demo OK: a permit \`standard\` has carried since AUTHZ-2 became"
echo "reachable without widening what it grants, as-of served what the"
echo "database held at an earlier instant while the PDP kept deciding with"
echo "today's roles, one MCP tool answered a real client over stdio, and"
echo "every recall chained one context.recalled the audit log verifies."
