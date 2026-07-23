#!/usr/bin/env sh
# CTX-2 acceptance demo: composition engine (ADR-0025).
# AC (docs/backlog/CTX-2.md): deterministic given same inputs; every
# block watermarked with commit hashes + record IDs; tokens_per_inject
# metric emitted (the metric AC is asserted in the suite at the end —
# CTX-3 wires the gateway-side emission point).
#
# Flow: postgres up -> migrate -> tenant; org/eng/core hierarchy; alice
# registered at the team -> alice observes 3 events and the pipeline
# extracts derived records at her home scope (the MEM-1 placement rule)
# -> team/dept/org material is seeded through the store contract
# (pinned is the published-channel stand-in until FLOW-1/2;
# embed-or-fail honored — record + vector in one statement), including
# an org-level duplicate of a team fact (the conflict fixture) -> the
# compose example runs the real product path (identity -> HIER-2 chain
# -> PDP-derived plan -> compose) and prints the watermarked block ->
# the same instant re-composes byte-identically (determinism) -> a
# bank-mode pack (published-only, budget 300) is applied with the CLI
# and assigned at the org root through the policy API -> the next
# compose drops every derived record and runs under the new budget
# (the subtree-scoped variant — bank mode below one team only — is
# asserted in the composition_plan suite) -> the AC suites run.
# On Windows, run via Git Bash. Needs postgres from the dev compose.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
SYNVEDA_LISTEN_ADDR=127.0.0.1:8141
export SYNVEDA_LISTEN_ADDR
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=ctx-2-demo-secret
export SYNVEDA_DEV_JWT_SECRET
# Deterministic extractor + embedder: composition is about assembly, not
# vector quality — no TEI needed (ADR-0023 decision 6 heeded: nothing
# here asserts retrieval quality against hash@1 geometry).
SYNVEDA_EXTRACTOR=deterministic
export SYNVEDA_EXTRACTOR
SYNVEDA_EMBEDDER=deterministic
export SYNVEDA_EMBEDDER
SYNVEDA_EXTRACTION_POLL_MS=300
export SYNVEDA_EXTRACTION_POLL_MS
SYNVEDA_SEARCH_INDEX_DIR="./data/ctx2-demo-search-$$"
export SYNVEDA_SEARCH_INDEX_DIR

cargo build -p synveda-gateway -p synveda-cli
cargo build -p synveda-retrieval --example compose_block

psql_c() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda -c "$1"
}

psql_t() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda -tAc "$1"
}

echo "==> migrate + admit a tenant"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "ctx2-demo-$$" --name "CTX-2 Demo Tenant")
tenant_id=$(echo "$tenant_json" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d).id));
')
echo "    tenant: $tenant_id"
admin_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-admin)
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null

echo "==> purging leftover observe-queue signals from other runs (shared queue)"
purged=$(psql_t "select pgmq.purge_queue('observe')")
echo "    purged=$purged"

./target/debug/synveda-gateway &
GATEWAY_PID=$!
trap 'kill "$GATEWAY_PID" 2>/dev/null || true; rm -rf "$SYNVEDA_SEARCH_INDEX_DIR"' EXIT INT TERM

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS http://127.0.0.1:8141/healthz >/dev/null 2>&1; do
  tries=$((tries + 1))
  if [ "$tries" -ge 30 ]; then
    echo "demo FAILED: gateway did not become healthy" >&2
    exit 1
  fi
  sleep 1
done

# api <token> <method> <path> [body]
api() {
  tok=$1
  method=$2
  path=$3
  body=${4:-}
  if [ -n "$body" ]; then
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
      -H "Content-Type: application/json" -d "$body" \
      "http://127.0.0.1:8141$path"
  else
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
      "http://127.0.0.1:8141$path"
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

echo "==> the admin builds org/eng/core; alice is registered at the team"
org_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
eng_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"department\",\"slug\":\"eng\",\"name\":\"Engineering\"}" |
  field id)
team_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$eng_id\",\"kind\":\"team\",\"slug\":\"core\",\"name\":\"Core\"}" |
  field id)
./target/debug/synveda service register --tenant "$tenant_id" \
  --subject alice --scope "$team_id" >/dev/null
alice_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject alice)
echo "    org=$org_id eng=$eng_id team=$team_id alice=alice"

now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
batch="{\"session_id\":\"demo-session\",\"events\":[
  {\"idempotency_key\":\"e1\",\"kind\":\"decision\",
   \"payload\":{\"text\":\"Chose the chain gradient for context assembly.\"},\"occurred_at\":\"$now\"},
  {\"idempotency_key\":\"e2\",\"kind\":\"transcript_delta\",
   \"payload\":{\"text\":\"Alice prefers concise context blocks over long ones.\"},\"occurred_at\":\"$now\"},
  {\"idempotency_key\":\"e3\",\"kind\":\"tool_result\",
   \"payload\":{\"output\":\"cargo test: composition suites green.\"},\"occurred_at\":\"$now\"}]}"

echo "==> alice observes 3 events; the pipeline extracts derived records"
echo "    at her home scope (observe takes no scope — placement decides)"
first=$(api "$alice_token" POST /v1/observe "$batch")
[ "$(echo "$first" | field accepted)" = "3" ] || {
  echo "demo FAILED: expected 3 accepted, got: $first" >&2
  exit 1
}
tries=0
while :; do
  have=$(psql_t "select count(*) from records where tenant_id = '$tenant_id'")
  [ "$have" = "3" ] && break
  tries=$((tries + 1))
  if [ "$tries" -ge 60 ]; then
    echo "demo FAILED: expected 3 records, stuck at $have" >&2
    exit 1
  fi
  sleep 0.5
done

echo "==> seeding team/dept/org material through the store contract"
echo "    (pinned = the published-channel stand-in until FLOW-1/2; the"
echo "    embed-or-fail constraint means record + vector in ONE statement)"
alice_identity=$(psql_t "select id from identities
                         where tenant_id = '$tenant_id' and subject = 'alice'")
vec="[0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1]"
# seed <scope-id> <kind> <class> <content>
seed() {
  psql_t "with new_record as (
            insert into records
              (id, tenant_id, scope_id, owner_id, kind, class, content,
               sensitivity, provenance, valid_from, tx_from)
            values (gen_random_uuid(), '$tenant_id', '$1', '$alice_identity',
                    '$2', '$3', '$4', 'internal',
                    '{\"source\": \"ctx-2 demo seed\"}', now(), now())
            returning id, tenant_id)
          insert into record_embeddings (record_id, tenant_id, model, dim, embedding)
          select id, tenant_id, 'hash@1', 16, '$vec'::vector from new_record" >/dev/null
}
seed "$team_id" pinned procedure "Deploys go through make deploy; never push directly."
seed "$team_id" derived fact "The release train leaves on fridays."
seed "$org_id" derived fact "The release train leaves on fridays."
seed "$eng_id" derived preference "Engineering standardises on rust for services."
seed "$org_id" pinned decision "Security review is mandatory for executable skills."
echo "    seeded 5 (the org duplicate of the team fact is the conflict fixture)"

# The compose instant is second-precision; give the sub-second
# valid_from stamps of the seeds a full second to fall behind it.
sleep 1
at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
echo
echo "==> compose for alice at $at — the real product path: identity ->"
echo "    HIER-2 chain -> one PDP MemoryRead per scope + effective-pack"
echo "    channel rules and budget -> gradient assembly -> watermark"
./target/debug/examples/compose_block "$tenant_id" alice "$at" 2>&1 | tee /tmp/ctx2-block-1.txt
grep -q "synveda:watermark v1 blake3=" /tmp/ctx2-block-1.txt || {
  echo "demo FAILED: no watermark line in the block" >&2
  exit 1
}
grep -q "\[unreviewed\]" /tmp/ctx2-block-1.txt || {
  echo "demo FAILED: derived entries must be marked unreviewed" >&2
  exit 1
}
dupes=$(grep -c "release train leaves" /tmp/ctx2-block-1.txt)
[ "$dupes" = "1" ] || {
  echo "demo FAILED: the conflict duplicate composed twice ($dupes)" >&2
  exit 1
}
grep -q "Security review is mandatory" /tmp/ctx2-block-1.txt || {
  echo "demo FAILED: the org pinned record must compose in the org section" >&2
  exit 1
}
echo "    conflict rules held: the duplicated fact composed once (team beats org)"

echo
echo "==> the determinism AC: the same instant re-composes byte-identically"
./target/debug/examples/compose_block "$tenant_id" alice "$at" >/tmp/ctx2-block-2.txt 2>&1
cmp /tmp/ctx2-block-1.txt /tmp/ctx2-block-2.txt || {
  echo "demo FAILED: re-composition differed" >&2
  exit 1
}
echo "    byte-identical, block hash included"

echo
echo "==> bank mode (ADR-0025 decision 2): a published-only pack with a"
echo "    300-token budget, applied with the CLI, assigned at the org root"
echo "    through the policy API — FLOW-2's switch, live two phases early"
cat > /tmp/ctx2-bank.cedar <<'EOF'
permit (principal, action, resource) when { resource in principal.tenant };
EOF
./target/debug/synveda policy apply --tenant "$tenant_id" --name acme-bank \
  --composition-budget 300 --composition-channels published-only \
  /tmp/ctx2-bank.cedar >/dev/null
api "$admin_token" PUT "/v1/hierarchy/nodes/$org_id/policy" \
  '{"name":"acme-bank"}' >/dev/null

echo "==> compose again: every scope inherits the org assignment —"
echo "    derived is out everywhere; pinned composes under budget 300"
./target/debug/examples/compose_block "$tenant_id" alice "$at" 2>&1 | tee /tmp/ctx2-block-3.txt
if grep -q "\[unreviewed\]" /tmp/ctx2-block-3.txt; then
  echo "demo FAILED: derived material composed under published-only" >&2
  exit 1
fi
grep -q "of 300 estimated tokens" /tmp/ctx2-block-3.txt || {
  echo "demo FAILED: the pack's 300-token budget did not govern" >&2
  exit 1
}
grep -q "make deploy" /tmp/ctx2-block-3.txt || {
  echo "demo FAILED: pinned material must survive bank mode" >&2
  exit 1
}
echo "    bank mode held: pinned-only, budget 300"

# The gateway must be down before cargo test relinks test binaries
# (Windows: the running exe holds a file lock).
kill "$GATEWAY_PID" 2>/dev/null || true
wait "$GATEWAY_PID" 2>/dev/null || true

echo
echo "==> the AC suites (gradient/pinned-first, determinism, watermark,"
echo "    budget, channel rules, conflicts, valid time, sensitivity,"
echo "    relevance, tokens_per_inject incl. the zero-record; the plan's"
echo "    PDP sweep; the pack-config store contract; the vocabulary)"
cargo test -p synveda-retrieval --test compose
cargo test -p synveda-retrieval --test composition_plan
cargo test -p synveda-retrieval --lib
cargo test -p synveda-store --test policy_packs
cargo test -p synveda-types --test serde_roundtrip

echo
echo "CTX-2 demo PASSED: context blocks assemble by the seed §4.4 gradient"
echo "(user > team > department > org, pinned before derived, derived marked"
echo "unreviewed), conflicts resolve pinned-over-derived then"
echo "specific-over-broad then newest-valid, the token budget and channel"
echo "rules ride the effective policy packs (bank mode = one pack switch,"
echo "next compose obeys), and every block is watermarked with BLAKE3"
echo "version hashes + record ids — recomputable content addresses that"
echo "FLOW-1's commit hashes will supersede in place. Same inputs, same"
echo "bytes: composition is deterministic."
