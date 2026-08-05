#!/usr/bin/env sh
# ADPT-2 acceptance demo: the generic MCP server (ADR-0057, as amended).
# AC (docs/backlog/ADPT-2.md): recall (+ policy-gated write) for any MCP
# client; works in Claude Desktop + one non-Anthropic client.
#
# The demo is shaped around the four claims worth doubting.
#
# THE WRITE LANDS, AND IT IS THE MODEL'S. A client calls `remember` over
# stdio; the fact goes through the real observe route, the real redaction
# scan and the real extraction pipeline, and comes back through `recall`
# minutes later — labelled `assertion`. That label is ADR-0057 decision 8's
# whole point and the one thing no later feature could recover: a hook
# records what happened, a tool call is a model asserting a fact it
# composed. The corpus in crates/synveda-cli/fixtures/mcp cannot show this,
# because it runs with no gateway. This is where the round trip is proved.
#
# THE BLAST RADIUS IS THE ROUTE'S. `POST /v1/observe` takes no scope
# parameter, so a model calling `remember` writes at its own home scope and
# nowhere else — not because the adapter is careful, but because the
# request has nowhere to say otherwise. Alice remembers something; bea, on
# a sibling team, cannot recall it. The tool has no argument that could
# have changed that.
#
# THE HOST THAT ALREADY WRITES IS NOT OFFERED A SECOND WAY. `--writes host`
# is decision 6, and it is enforced twice: `remember` is absent from
# `tools/list` *and* `tools/call remember` is refused. A tool missing from
# the listing that still answered a call would not be missing.
#
# BOTH ERAS. A client that opens with `initialize` at the revision CTX-5's
# hand-written loop pinned is answered on its own terms; a client that
# opens with `server/discover` and a per-request `_meta` is served the
# current revision. The loop this replaced could do only the first.
#
# The credential half is ADPT-1's and `demos/adpt-1-claude-code.sh` proves
# it against live Rauthy; here the callers carry dev tokens through
# SYNVEDA_TOKEN (the override ADR-0027 kept for CI and demos), so this demo
# needs only postgres. On Windows, run via Git Bash.
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
$COMPOSE up --detach --wait postgres

# A scratch database per run — the CTX-5/EVAL-1/MEM-6 discipline: the
# sidecar indexer and the pack refresher visit every active tenant per
# cycle, and on the shared dev database a just-admitted tenant waits
# minutes for its first pass.
ADPT2_DB=adpt2_$$
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d synveda \
  -c "create database $ADPT2_DB" >/dev/null
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d "$ADPT2_DB" \
  -c "create extension if not exists vector;
      create extension if not exists age;
      create extension if not exists pgmq" >/dev/null

DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$ADPT2_DB"
export DATABASE_URL
SYNVEDA_SEARCH_INDEX_DIR="./data/adpt2-search-$$"
export SYNVEDA_SEARCH_INDEX_DIR
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG
SYNVEDA_LISTEN_ADDR=127.0.0.1:8151
export SYNVEDA_LISTEN_ADDR
GATEWAY=http://127.0.0.1:8151
SYNVEDA_DEV_JWT_SECRET=adpt-2-demo-secret
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

SCRATCH="/tmp/adpt2-$$"
mkdir -p "$SCRATCH"

psql_t() {
  $COMPOSE exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d "$ADPT2_DB" -tAc "$1"
}

echo "==> migrate + admit a tenant"
./target/debug/synveda db migrate
tenant_id=$(./target/debug/synveda tenant create \
  --slug "adpt2-demo-$$" --name "ADPT-2 Demo Tenant" | \
  node -e 'let d="";process.stdin.on("data",c=>d+=c);process.stdin.on("end",()=>console.log(JSON.parse(d).id));')
echo "    tenant: $tenant_id"
admin_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-admin)
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null

cleanup() {
  kill "${GATEWAY_PID:-0}" 2>/dev/null || true
  sleep 1
  $COMPOSE exec -T postgres \
    psql -U synveda -d synveda -c "drop database if exists $ADPT2_DB (force)" >/dev/null 2>&1 || true
  rm -rf "$SYNVEDA_SEARCH_INDEX_DIR" "$SCRATCH"
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

# seed_user <subject> <parent scope> — a placed user identity, the shape
# JIT provisioning produces on first login (AUTH-2). Written directly for
# the reason CTX-5's demo writes it directly: this demo is about the MCP
# surface, and standing up an IdP to place two readers would be scenery.
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
  printf '%s' "$leaf"
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
alice_scope=$(seed_user alice "$platform_id")
seed_user bea "$payments_id" >/dev/null
alice_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject alice)
bea_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject bea)
echo "    acme > engineering > {platform, payments}, pack=standard at the org"
echo "    alice on platform, bea on payments"

# The MCP server is a gateway client holding a bearer, exactly as `synveda
# login` leaves one (ADR-0057 decision 1). SYNVEDA_TOKEN is the documented
# override for demos and CI, so alice's client speaks as alice.
SYNVEDA_TOKEN=$alice_token
export SYNVEDA_TOKEN
SYNVEDA_GATEWAY=$GATEWAY
export SYNVEDA_GATEWAY

# ── A real MCP client ──────────────────────────────────────────────────
#
# Deliberately not the product's own code: a client that shared the
# server's helpers would prove less than one that does not. It speaks the
# wire and nothing else.
cat >"$SCRATCH/client.mjs" <<'MCPCLIENT'
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";

const [, , binary, era, writes, script] = process.argv;
const server = spawn(binary, ["mcp", "--writes", writes], {
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
const notify = (method) =>
  server.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method })}\n`);
const fail = (why) => { console.error(`demo FAILED: ${why}`); server.kill(); process.exit(1); };

// How a client opens is the era. `initialize` is the legacy handshake the
// hand-written loop implemented; `_meta` per request with no handshake at
// all is 2026-07-28, and `server/discover` is MUST there.
const meta = {
  "io.modelcontextprotocol/protocolVersion": "2026-07-28",
  "io.modelcontextprotocol/clientCapabilities": {},
  "io.modelcontextprotocol/clientInfo": { name: "adpt-2-demo", version: "0" },
};
let open;
if (era === "legacy") {
  const init = await call("initialize", {
    protocolVersion: "2025-06-18",
    capabilities: {},
    clientInfo: { name: "adpt-2-demo", version: "0" },
  });
  if (init.result?.serverInfo?.name !== "synveda") fail("initialize did not name the server");
  if (init.result.protocolVersion !== "2025-06-18") fail(`a legacy opener must be answered on its own terms, got ${init.result.protocolVersion}`);
  notify("notifications/initialized");
  open = { note: `initialize -> protocol ${init.result.protocolVersion}`, params: {} };
} else {
  const d = await call("server/discover", { _meta: meta });
  const supported = d.result?.supportedVersions ?? [];
  if (!supported.includes("2026-07-28")) fail(`server/discover must offer the current revision, got ${JSON.stringify(supported)}`);
  open = { note: `server/discover -> ${supported.join(", ")}`, params: { _meta: meta } };
}
console.log(`      ${open.note}`);

const listed = await call("tools/list", { ...open.params });
const tools = (listed.result?.tools ?? []).map((t) => t.name);
console.log(`      tools/list -> ${tools.join(", ")}`);

// The script is the demo's; this client just runs it.
const steps = JSON.parse(script);
for (const step of steps) {
  const response = await call("tools/call", { ...open.params, name: step.tool, arguments: step.args });
  if (step.expectProtocolError) {
    if (response.error === undefined) fail(`${step.tool} should not be callable at all`);
    console.log(`      tools/call ${step.tool} -> refused: ${response.error.message}`);
    continue;
  }
  const text = response.result?.content?.[0]?.text ?? "";
  if (step.expectToolError && !response.result?.isError) fail(`${step.tool} should have reported an error, got:\n${text}`);
  if (!step.expectToolError && response.result?.isError) fail(`${step.tool} failed: ${text}`);
  if (step.contains && !text.includes(step.contains)) fail(`${step.tool} did not say "${step.contains}":\n${text}`);
  if (step.absent && text.includes(step.absent)) fail(`${step.tool} must NOT have returned "${step.absent}":\n${text}`);
  for (const line of text.split("\n")) console.log(`        ${line}`);
}
if (!tools.includes("recall")) fail("recall must always be offered");
console.log(JSON.stringify({ tools }));
server.kill();
MCPCLIENT

client() {
  node "$SCRATCH/client.mjs" "./target/debug/synveda" "$1" "$2" "$3"
}

echo
echo "==> THE WRITE LANDS: a model calls \`remember\` over stdio"
echo "    (modern era: server/discover, per-request _meta, no handshake)"
client modern tool '[
  {"tool":"remember","args":{"text":"Platform decided to move settlement reconciliation to the nightly batch window, because the acquirer statement only settles after 02:00 UTC."},"contains":"Remembered"}
]' >"$SCRATCH/remember.out"
sed -n '1,20p' "$SCRATCH/remember.out" | grep -v '^{'
echo "    the route took no scope parameter — it could not have said where"

# The pipeline is asynchronous by construction (ADR-0020 decision 1: the
# ack promises durable admission, not processing). Wait by asking the
# product, so what the demo waits for is exactly what it then asserts.
echo
echo "==> ...and comes back through recall, LABELLED \`assertion\`"
tries=0
while :; do
  recalled=$(./target/debug/synveda recall --query "settlement reconciliation nightly batch" --json --quiet 2>/dev/null || echo '{}')
  if printf '%s' "$recalled" | grep -q "nightly batch window"; then break; fi
  tries=$((tries + 1))
  if [ "$tries" -ge 60 ]; then
    echo "demo FAILED: the remembered fact never became recallable" >&2
    printf '%s\n' "$recalled" >&2
    exit 1
  fi
  sleep 0.5
done
printf '%s' "$recalled" | node -e '
  let d=""; process.stdin.on("data",c=>d+=c); process.stdin.on("end",()=>{
    const r = JSON.parse(d).entries.find(e => e.content.includes("nightly batch window"));
    console.log(`      content     ${r.content}`);
    console.log(`      class       ${r.class}   channel ${r.channel}   scope ${r.scope_id}`);
    console.log(`      provenance  kind=${r.provenance.kind}  method=${r.provenance.method}`);
    if (r.provenance.kind !== "assertion") {
      console.error(`demo FAILED: provenance.kind is ${r.provenance.kind}, not assertion`);
      process.exit(1);
    }
  });'
echo "    ADR-0057 decision 8: a hook records what happened, a tool call is"
echo "    the model asserting a fact it composed. If that distinction stopped"
echo "    at the staging buffer it would be telemetry; it is here at recall."

echo
echo "==> and the class was NOT asserted with it"
kinds=$(psql_t "select distinct kind from observe_events where tenant_id = '$tenant_id'")
echo "    observe_events.kind: $kinds"
echo "    \`assertion\` says who put the content on the wire, never what it is —"
echo "    so it reads the text the way a transcript delta does and the class"
echo "    above was earned by the keyword path, not handed over by the kind."

echo
echo "==> THE BLAST RADIUS IS THE ROUTE'S: bea cannot recall it"
scope_of_record=$(printf '%s' "$recalled" | node -e '
  let d=""; process.stdin.on("data",c=>d+=c); process.stdin.on("end",()=>{
    console.log(JSON.parse(d).entries.find(e => e.content.includes("nightly batch window")).scope_id);});')
if [ "$scope_of_record" != "$alice_scope" ]; then
  echo "demo FAILED: the write landed at $scope_of_record, not alice's home scope $alice_scope" >&2
  exit 1
fi
echo "    it landed at alice's own home scope $alice_scope"
# No `|| true`: bea's read must genuinely succeed and return nothing of
# alice's note. A swallowed failure here would read as isolation working
# when what actually happened was that the query never ran.
bea_out=$(SYNVEDA_TOKEN=$bea_token ./target/debug/synveda recall \
  --query "settlement reconciliation nightly batch" --quiet 2>/dev/null)
if printf '%s' "$bea_out" | grep -q "nightly batch window"; then
  echo "demo FAILED: a personal write reached a sibling team's reader" >&2
  exit 1
fi
echo "    bea, on payments, asks the same question and gets nothing of it"
echo "    (ADR-0020 decision 4: placement decides. \`remember\` has no scope"
echo "     argument, so a model cannot write into a team even by trying.)"

echo
echo "==> A MODEL CANNOT LAUNDER A SECRET THROUGH THE WRITE TOOL"
# MEM-2's scan sits between validation and the staging insert (ADR-0021),
# and the *disposition* is the pack's: `standard` is REDACT_ALL, so the key
# is stripped and the rest is admitted. Under `regulated-strict`, or any
# pack that leaves redaction unconfigured, secrets quarantine instead and
# the tool says so — `render_remember`'s four dispositions are pinned by
# `mcp::tests::every_disposition_says_what_actually_happened`. Either way
# the model does not get to decide, and either way the key is not stored.
client modern tool '[
  {"tool":"remember","args":{"text":"Rotate the acquirer bridge credential AKIAIOSFODNN7EXAMPLE every quarter."},"contains":"Remembered"}
]' >"$SCRATCH/secret.out"
grep -v '^{' "$SCRATCH/secret.out" | sed -n '3,8p'
echo "    under \`standard\` (REDACT_ALL) the write is admitted — and the key"
echo "    is gone before anything persisted it:"
leaked=$(psql_t "select count(*) from observe_events
                 where tenant_id = '$tenant_id' and payload::text like '%AKIAIOSFODNN7EXAMPLE%'")
[ "$leaked" = "0" ] || {
  echo "demo FAILED: the raw key survived in $leaked staging row(s)" >&2; exit 1; }
stored=$(psql_t "select payload::text from observe_events
                 where tenant_id = '$tenant_id' and payload::text like '%acquirer bridge credential%'")
echo "      staged payload: $stored"
echo "    ADR-0021's guarantee is structural: the matched text appears in no"
echo "    table, no response, no metric and no audit payload — the findings"
echo "    report a rule id and a count and never what matched. A pack that"
echo "    quarantines holds the whole event instead; the model does not"
echo "    choose which, and cannot store the secret under either."

echo
echo "==> THE HOST THAT ALREADY WRITES IS NOT OFFERED A SECOND WAY"
echo "    (\`--writes host\`: what the Claude Code plugin's entry point execs)"
host_out=$(client legacy host '[
  {"tool":"remember","args":{"text":"a fact the Stop hook is already recording"},"expectProtocolError":true}
]')
printf '%s\n' "$host_out" | grep -v '^{'
printf '%s' "$host_out" | grep -q '"tools":\["recall"\]' || {
  echo "demo FAILED: --writes host must advertise recall alone" >&2; exit 1; }
echo "    absent from tools/list AND refused by tools/call — decision 6 has"
echo "    two halves, because a tool missing from the listing that still"
echo "    answered a call would not be missing."

echo
echo "==> the same server through the plugin's own entry point"
plugin_tools=$(printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"claude-code","version":"0"}}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' |
  SYNVEDA_CLI="$PWD/target/debug/synveda" node adapters/claude-code/dist/mcp-server.mjs 2>/dev/null |
  node -e 'let d="";process.stdin.on("data",c=>d+=c);process.stdin.on("end",()=>{
    const t=d.trim().split("\n").map(JSON.parse).find(m=>m.id===2);
    console.log(t.result.tools.map(x=>x.name).join(","));});')
[ "$plugin_tools" = "recall" ] || {
  echo "demo FAILED: the plugin launcher offered [$plugin_tools]" >&2; exit 1; }
echo "    dist/mcp-server.mjs -> synveda mcp --writes host -> [$plugin_tools]"
echo "    (ADR-0057 decision 4: one protocol implementation, not two. The"
echo "     289-line hand-written loop this replaced pinned 2025-06-18.)"

echo
echo "==> BOTH ERAS, over one corpus"
echo "    a legacy client (what the deleted loop could do):"
client legacy tool '[
  {"tool":"recall","args":{"query":"settlement reconciliation nightly batch"},"contains":"Watermark:"}
]' >"$SCRATCH/legacy.out"
grep -v '^{' "$SCRATCH/legacy.out" | sed -n '1,12p'
echo "    a modern client (what it could not):"
client modern tool '[
  {"tool":"recall","args":{"query":"a","ids":["0198f0a0-0000-7000-8000-000000000001"]},"expectToolError":true,"contains":"not both"}
]' >"$SCRATCH/modern.out"
grep -v '^{' "$SCRATCH/modern.out" | sed -n '1,6p'
echo "    the xor is checked before the gateway is troubled, so an agent"
echo "    that gets it wrong reads a sentence rather than a 400."

echo
echo "==> AND THE CONFIG IS GENERATED, NOT DOCUMENTED (decision 10)"
cat >"$SCRATCH/cursor.json" <<'JSON'
{
  "mcpServers": {
    "filesystem": { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem"] }
  },
  "someOtherSetting": true
}
JSON
./target/debug/synveda mcp install --client cursor --config "$SCRATCH/cursor.json" | sed 's/^/    /'
node -e '
  const c = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
  if (!c.mcpServers.filesystem) { console.error("demo FAILED: another server was lost"); process.exit(1); }
  if (c.someOtherSetting !== true) { console.error("demo FAILED: an unrelated setting was lost"); process.exit(1); }
  const e = c.mcpServers.synveda;
  if (!e.command.startsWith("/")) { console.error("demo FAILED: a GUI client inherits no PATH"); process.exit(1); }
  console.log(`    the file still has: ${Object.keys(c.mcpServers).join(", ")} + someOtherSetting`);
  console.log(`    and launches: ${e.command} ${e.args.join(" ")}`);
' "$SCRATCH/cursor.json"
# Exec exactly what the generated config says: a config that is well shaped
# but not runnable would pass every test and fail every user.
gen_cmd=$(node -e 'const e=JSON.parse(require("fs").readFileSync(process.argv[1],"utf8")).mcpServers.synveda;console.log([e.command,...e.args].join(" "));' "$SCRATCH/cursor.json")
gen_tools=$(printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"probe","version":"0"}}}' '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' |
  $gen_cmd 2>/dev/null |
  node -e 'let d="";process.stdin.on("data",c=>d+=c);process.stdin.on("end",()=>{
    const t=d.trim().split("\n").map(JSON.parse).find(m=>m.id===2);
    console.log(t.result.tools.map(x=>x.name).join(","));});')
[ "$gen_tools" = "recall,remember" ] || {
  echo "demo FAILED: the generated config launched [$gen_tools]" >&2; exit 1; }
echo "    running that line verbatim serves [$gen_tools]"

echo
echo "==> the trail: every call chained under alice, never under a tool"
psql_t "select action || '  x' || count(*) from audit_log
        where tenant_id = '$tenant_id' and action in ('memory.observed','context.recalled')
        group by action order by action" | sed 's/^/    /'
actors=$(psql_t "select distinct actor_subject from audit_log
                 where tenant_id = '$tenant_id' and action = 'memory.observed'")
echo "    memory.observed actor(s): $actors"
[ "$actors" = "alice" ] || {
  echo "demo FAILED: the write was chained under [$actors], not alice" >&2; exit 1; }
echo "    ADR-0057's compliance note: the ACTOR is the person whose credential"
echo "    authorised the write; the KIND records that a model composed it."
echo "    Conflating them would answer \"who is accountable\" with \"a tool"
echo "    call\", which is not a party to anything."
./target/debug/synveda audit verify --tenant "$tenant_id"

echo
echo "==> the AC suites"
cargo test -p synveda-cli mcp::
cargo test -p synveda-cli --test mcp_corpus
( cd adapters/claude-code && node --test "dist/mcp-server.test.mjs" )

echo
echo "ADPT-2 demo OK: a real MCP client wrote a fact through \`remember\` and"
echo "recalled it labelled \`assertion\`, a sibling team's reader could not see"
echo "it because the route has nowhere to say otherwise, a secret was refused"
echo "by the same scan every other write meets, the hook-driven host was"
echo "offered no second way to write, both protocol eras answered over one"
echo "corpus, and a generated client config launched the server verbatim."
echo
echo "NOT YET the acceptance criterion. \"Works in Claude Desktop + one"
echo "non-Anthropic client\" needs those clients' own frames, and the corpus"
echo "at crates/synveda-cli/fixtures/mcp holds authored ones — real answers,"
echo "authored questions. crates/synveda-cli/fixtures/mcp/capture.sh is how"
echo "the real ones get recorded; until they are, this demo shows the server"
echo "works and not that those two clients do."
