#!/usr/bin/env sh
# PRMT-2 acceptance demo: context packs (ADR-0050).
# AC (docs/SYNVEDA_FEATURES.md): a pack reaches a session only through the
# review the pack in force asks for; "re-embeds atomically" is measured
# from the reader's side; "next session" is satisfied as "next call"; pack
# content composes as pinned material, ranked, and what does not fit is
# named rather than dropped.
#
# PRMT-1 showed its claims at the registry's own route, because a prompt is
# fetched by name. Almost nothing here can be shown that way: a context pack
# is the first authored asset whose content has to enter the corpus the read
# path ranks, so every beat below is measured at POST /v1/inject — where a
# session actually sees it. Flow:
#
#   postgres -> scratch db -> tenant, hierarchy, four principals
#   [1/8] alice authors the payments bundle at the DEPARTMENT. This is the
#         expensive half: the server chunks, scans for secrets and embeds.
#         DATABASE_URL is unset from here on, so every governed act is a
#         gateway call under that principal's own bearer.
#   [2/8] bea injects and gets nothing. An unpublished bundle composes into
#         nobody's session — not even marked unreviewed.
#   [3/8] the direct publish route refuses. Since ADR-0050 decision 15 the
#         `context-pack` cell above a team asks for a curator AND a steward,
#         two distinct people — FLOW-3 had left it at one curator, which
#         made publishing a whole bundle into every session cheaper than
#         publishing one memory record at the same scope.
#   [4/8] the review carries it, and bea's very NEXT call composes it. The
#         AC says "next session"; the pack channel is read live, so the
#         product does better than the criterion asks.
#   [5/8] THE AC: alice edits the runbook. Between the edit and the second
#         review, bea composes ALL of the reviewed version and NOT ONE WORD
#         of the edit — the edited document's address moved, so every chunk
#         cut from it fell off the published set.
#   [6/8] a 12-section glossary against a 1,500-token budget: what does not
#         fit is NAMED — pack, document, section, title — with a recall
#         handle that resolves.
#   [7/8] a runbook carrying a live credential is refused ahead of the
#         embedder. No secret reaches vector space.
#   [8/8] a rewind restores the previous version by moving a ref: no
#         re-embedding, no half-swapped state.
#   then the trail: context_pack.authored, context_pack.quarantined and the
#   same vedaflow.channel.published a memory publication emits, with no
#   document text in any payload, and the chain verifying over all of it.
#
# On Windows, run via Git Bash. Needs postgres and node; no TEI.
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
$COMPOSE up --detach --wait postgres

# A scratch database per run — the FLOW-4/EVAL-1/MEM-6/PRMT-1 discipline.
PRMT_DB=prmt2_$$
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d synveda \
  -c "create database $PRMT_DB" >/dev/null
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d "$PRMT_DB" \
  -c "create extension if not exists vector;
      create extension if not exists age;
      create extension if not exists pgmq" >/dev/null

DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$PRMT_DB"
export DATABASE_URL
SYNVEDA_SEARCH_INDEX_DIR="./data/prmt2-search-$$"
export SYNVEDA_SEARCH_INDEX_DIR
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG
SYNVEDA_LISTEN_ADDR=127.0.0.1:8145
export SYNVEDA_LISTEN_ADDR
BASE="http://127.0.0.1:8145"
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=prmt-2-demo-secret
export SYNVEDA_DEV_JWT_SECRET
SYNVEDA_EMBEDDER=deterministic
export SYNVEDA_EMBEDDER

cargo build -p synveda-gateway -p synveda-cli
CLI=./target/debug/synveda

WORK="${TMPDIR:-/tmp}/prmt2-$$"
mkdir -p "$WORK"

cleanup() {
  kill "${GATEWAY_PID:-0}" 2>/dev/null || true
  sleep 1
  $COMPOSE exec -T postgres \
    psql -U synveda -d synveda -c "drop database if exists $PRMT_DB (force)" >/dev/null 2>&1 || true
  rm -rf "$SYNVEDA_SEARCH_INDEX_DIR" "$WORK"
}
trap cleanup EXIT INT TERM

psql_t() {
  $COMPOSE exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d "$PRMT_DB" -tAc "$1"
}
psql_c() {
  $COMPOSE exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d "$PRMT_DB" -c "$1"
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

# api <token> <method> <path> [body]
api() {
  tok=$1
  method=$2
  path=$3
  body=${4:-}
  if [ -n "$body" ]; then
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
      -H "Content-Type: application/json" -d "$body" "$BASE$path"
  else
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" "$BASE$path"
  fi
}

# as <token> <args...> — the CLI as one principal.
as() {
  tok=$1
  shift
  SYNVEDA_TOKEN="$tok" SYNVEDA_GATEWAY="$BASE" "$CLI" "$@"
}

# refused <token> <args...> — a CLI command that must fail, printed.
refused() {
  tok=$1
  shift
  if out=$(SYNVEDA_TOKEN="$tok" SYNVEDA_GATEWAY="$BASE" "$CLI" "$@" 2>&1); then
    echo "demo FAILED: '$*' should have been refused, got:" >&2
    echo "$out" >&2
    exit 1
  fi
  echo "$out" | sed 's/^/    /'
}

# refused_http <token> <method> <path> [body]
refused_http() {
  tok=$1
  method=$2
  path=$3
  body=${4:-}
  if out=$(curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
    -H "Content-Type: application/json" \
    ${body:+-d "$body"} "$BASE$path" 2>&1); then
    echo "demo FAILED: $method $path should have been refused, got:" >&2
    echo "$out" >&2
    exit 1
  fi
  curl -sS -X "$method" -H "Authorization: Bearer $tok" \
    -H "Content-Type: application/json" \
    ${body:+-d "$body"} "$BASE$path" 2>/dev/null | sed 's/^/    /'
}

# what bea composes right now — the only surface most of this AC can be
# measured at.
block() {
  api "$bea_token" POST /v1/inject \
    "{\"task\":\"$1\",\"session_id\":\"demo\"}" | field text
}

# says <needle> <haystack-label> — the block must contain it.
says() {
  if ! printf '%s' "$2" | grep -q "$1"; then
    echo "demo FAILED: the block should say '$1':" >&2
    printf '%s\n' "$2" >&2
    exit 1
  fi
}

# silent <needle> <haystack> — the block must NOT contain it.
silent() {
  if printf '%s' "$2" | grep -q "$1"; then
    echo "demo FAILED: the block must not say '$1':" >&2
    printf '%s\n' "$2" >&2
    exit 1
  fi
}

echo "==> migrate + admit a tenant"
$CLI db migrate
tenant_id=$($CLI tenant create \
  --slug "prmt2-demo-$$" --name "PRMT-2 Demo Tenant" | field id)
echo "    tenant: $tenant_id"
admin_token=$($CLI token issue --tenant "$tenant_id" --subject demo-admin)
$CLI role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null

GATEWAY_LOG="$WORK/gateway.log"
./target/debug/synveda-gateway >"$GATEWAY_LOG" 2>&1 &
GATEWAY_PID=$!

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS "$BASE/healthz" >/dev/null 2>&1; do
  if ! kill -0 "$GATEWAY_PID" 2>/dev/null; then
    echo "demo FAILED: the gateway exited; see $GATEWAY_LOG" >&2
    exit 1
  fi
  tries=$((tries + 1))
  if [ "$tries" -ge 30 ]; then
    echo "demo FAILED: gateway did not become healthy" >&2
    exit 1
  fi
  sleep 1
done

echo "==> hierarchy: acme > eng > platform, four principals"
org_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
eng_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"department\",\"slug\":\"eng\",\"name\":\"Engineering\"}" |
  field id)
team_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$eng_id\",\"kind\":\"team\",\"slug\":\"platform\",\"name\":\"Platform\"}" |
  field id)
for who in alice cora bea; do
  $CLI service register --tenant "$tenant_id" \
    --subject "$who" --scope "$team_id" >/dev/null
done
# The steward is anchored at the department, one level above the team — his
# authority at eng is a role binding and nothing else (the PRMT-1 fixture's
# reason, unchanged).
$CLI service register --tenant "$tenant_id" --subject sam --scope "$eng_id" >/dev/null
for who in alice cora sam; do
  case $who in
  alice) role=contributor ;;
  cora) role=curator ;;
  sam) role=steward ;;
  esac
  $CLI role bind --tenant "$tenant_id" --subject "$who" --role "$role" --scope "$eng_id" >/dev/null
done
alice_token=$($CLI token issue --tenant "$tenant_id" --subject alice)
cora_token=$($CLI token issue --tenant "$tenant_id" --subject cora)
sam_token=$($CLI token issue --tenant "$tenant_id" --subject sam)
bea_token=$($CLI token issue --tenant "$tenant_id" --subject bea)
echo "    eng=$eng_id  team=$team_id"
echo "    alice=contributor (author)  cora=curator  sam=steward  bea=consumer"

mkdir -p "$WORK/pack/runbooks"
cat >"$WORK/pack/runbooks/refunds.md" <<'DOC'
# Refunds

Settle refunds within three days of the request.

## Escalation

Escalate anything over five hundred pounds to the duty lead.
DOC

# From here on: no psql and no DATABASE_URL. Every governed act is a
# gateway call under a principal's own bearer (ADR-0035 decision 1).
unset DATABASE_URL

echo
echo "================================================================"
echo "  [1/8] alice authors the bundle at the DEPARTMENT. This call is"
echo "  the expensive half: the server chunks the document, scans it for"
echo "  secrets, and embeds every chunk — because no proposal approval"
echo "  has ever made a network call, and 'embed on publish' would make a"
echo "  curator's approval fail when a model server is down."
echo "================================================================"
as "$alice_token" context-pack author payments \
  --scope "$eng_id" --root "$WORK/pack" \
  --file "$WORK/pack/runbooks/refunds.md" \
  --description "how payments works here"
echo
as "$alice_token" context-pack list "$eng_id"

echo
echo "================================================================"
echo "  [2/8] bea's session. An unpublished bundle composes into nobody's"
echo "  session — not even marked unreviewed, because a pack chunk is"
echo "  admitted by ContextPackRead off the pack channel and never by"
echo "  MemoryRead off the corpus it happens to live in."
echo "================================================================"
before=$(block "how do refunds work")
silent "three days" "$before"
echo "    (nothing: the draft is not a version anybody composes)"

echo
echo "================================================================"
echo "  [3/8] the direct publish route. Since ADR-0050 decision 15 the"
echo "  context-pack cell above a team asks for a curator AND a steward."
echo "  FLOW-3 had left it at one curator at every scope kind — which"
echo "  made publishing a whole bundle into every session cheaper than"
echo "  publishing one memory record at the same scope."
echo "================================================================"
refused_http "$cora_token" POST "/v1/channels/$eng_id/publish" \
  '{"document_paths":["payments/runbooks/refunds.md"],"message":"ship it"}'

echo
echo "================================================================"
echo "  [4/8] the review the pack asks for — and bea's very NEXT call"
echo "  composes it. The AC says 'next session'; the pack channel is read"
echo "  live on the composition path, so the product does better."
echo "================================================================"
as "$alice_token" context-pack propose payments --scope "$eng_id" \
  --title "payments conventions"
proposal_id=$(api "$cora_token" GET "/v1/proposals?scope_id=$eng_id&state=open" |
  field proposals 0 id)
api "$cora_token" POST "/v1/proposals/$proposal_id/approve" '{}' >/dev/null
api "$sam_token" POST "/v1/proposals/$proposal_id/approve" '{}' >/dev/null
api "$cora_token" POST "/v1/proposals/$proposal_id/publish" '{}' >/dev/null
echo "    published by two distinct people"
v1=$(block "how do refunds work")
says "three days" "$v1"
says "five hundred" "$v1"
printf '%s\n' "$v1" | sed 's/^/    /'

echo
echo "================================================================"
echo "  [5/8] THE AC. alice edits the runbook. Between the edit and the"
echo "  second review, bea composes ALL of the reviewed version and NOT"
echo "  ONE WORD of the edit: the edited document's address moved, so"
echo "  every chunk cut from it fell off the published set rather than"
echo "  the edit riding a published path."
echo "================================================================"
cat >"$WORK/pack/runbooks/refunds.md" <<'DOC'
# Refunds

Settle refunds within one day of the request.

## Escalation

Escalate anything over fifty pounds to the duty lead.
DOC
as "$alice_token" context-pack author payments \
  --scope "$eng_id" --root "$WORK/pack" \
  --file "$WORK/pack/runbooks/refunds.md" \
  --description "how payments works here"
echo
between=$(block "how do refunds work")
says "three days" "$between"
says "five hundred" "$between"
silent "one day" "$between"
silent "fifty pounds" "$between"
echo "    the reviewed version, in full; the edit, not at all"

as "$alice_token" context-pack propose payments --scope "$eng_id" \
  --title "tighten the refund window"
proposal_id=$(api "$cora_token" GET "/v1/proposals?scope_id=$eng_id&state=open" |
  field proposals 0 id)
api "$cora_token" POST "/v1/proposals/$proposal_id/approve" '{}' >/dev/null
api "$sam_token" POST "/v1/proposals/$proposal_id/approve" '{}' >/dev/null
api "$cora_token" POST "/v1/proposals/$proposal_id/publish" '{}' >/dev/null
after=$(block "how do refunds work")
says "one day" "$after"
says "fifty pounds" "$after"
silent "three days" "$after"
echo "    and after the second review: the new version, in full, the old one gone"

echo
echo "================================================================"
echo "  [6/8] a 12-section glossary against a 1,500-token budget. What"
echo "  does not fit is NAMED — pack, document, section, title — with a"
echo "  recall handle. ADR-0025 option 5 refused relevance filtering for"
echo "  pinned material so canonical content could not silently vanish;"
echo "  it was decided about records costing tens of tokens. Nothing here"
echo "  vanishes, because nothing about it is silent."
echo "================================================================"
node -e '
  const fs = require("fs");
  let out = "";
  for (let s = 0; s < 12; s++) {
    out += `# Section ${s}\n\n`;
    for (let l = 0; l < 20; l++) {
      out += `Term ${s}.${l} is settled under the schedule for section ${s} and reviewed every quarter. `;
    }
    out += "\n\n";
  }
  fs.writeFileSync(process.argv[1], out);
' "$WORK/pack/glossary.md"
as "$alice_token" context-pack author payments \
  --scope "$eng_id" --root "$WORK/pack" \
  --file "$WORK/pack/glossary.md" \
  --file "$WORK/pack/runbooks/refunds.md" \
  --description "how payments works here" >/dev/null
as "$alice_token" context-pack propose payments --scope "$eng_id" \
  --title "add the glossary"
proposal_id=$(api "$cora_token" GET "/v1/proposals?scope_id=$eng_id&state=open" |
  field proposals 0 id)
api "$cora_token" POST "/v1/proposals/$proposal_id/approve" '{}' >/dev/null
api "$sam_token" POST "/v1/proposals/$proposal_id/approve" '{}' >/dev/null
api "$cora_token" POST "/v1/proposals/$proposal_id/publish" '{}' >/dev/null

big=$(api "$bea_token" POST /v1/inject \
  '{"task":"payment terms","session_id":"demo"}')
big_text=$(printf '%s' "$big" | field text)
indexed=$(printf '%s' "$big" | field index_entries)
tokens=$(printf '%s' "$big" | field tokens)
budget=$(printf '%s' "$big" | field budget_tokens)
[ "$indexed" -gt 0 ] || {
  echo "demo FAILED: nothing was named; the index tier did not fire" >&2
  printf '%s\n' "$big_text" >&2
  exit 1; }
says "payments/glossary.md#" "$big_text"
says "(recall " "$big_text"
echo "    $tokens tokens of $budget, $indexed entries named rather than dropped"
printf '%s\n' "$big_text" | grep "recall " | head -3 | sed 's/^/    /'
handle=$(printf '%s' "$big_text" | grep -o "(recall [0-9a-f-]*)" | head -1 |
  sed 's/(recall //; s/)//')
recalled=$(api "$bea_token" POST /v1/recall "{\"ids\":[\"$handle\"]}" |
  field entries 0 content)
case $recalled in
*"reviewed every quarter"*) ;;
*)
  echo "demo FAILED: the handle did not resolve to a body" >&2
  exit 1
  ;;
esac
echo "    and the handle resolves to the body the block could not hold"

echo
echo "================================================================"
echo "  [7/8] a runbook carrying a live credential. This is the first"
echo "  surface where bulk external text enters the product — a prompt is"
echo "  short and hand-written, and PRMT-1 does not scan one. MEM-2's"
echo "  scanner runs AHEAD of the embedder, so no secret reaches vector"
echo "  space."
echo "================================================================"
cat >"$WORK/pack/runbooks/oncall.md" <<'DOC'
# On call

Authenticate with ghp_0123456789abcdefghijklmnopqrstuvwxyzAB and check the queue.
DOC
refused "$alice_token" context-pack author payments \
  --scope "$eng_id" --root "$WORK/pack" \
  --file "$WORK/pack/runbooks/oncall.md" \
  --description "how payments works here"

echo
echo "================================================================"
echo "  [8/8] a rewind. The previous version is restored by moving a ref:"
echo "  no re-embedding, no half-swapped state — because which chunks"
echo "  compose is decided by the commit the pack channel SERVES."
echo "================================================================"
history=$(api "$cora_token" \
  "GET" "/v1/channels/$eng_id/history?asset=context-pack&channel=published")
head_commit=$(printf '%s' "$history" | field states 0 commit)
prior_commit=$(printf '%s' "$history" | field states 2 commit)
api "$cora_token" POST "/v1/channels/$eng_id/rollback" \
  "{\"asset\":\"context-pack\",\"channel\":\"published\",
    \"from_commit\":\"$head_commit\",\"to_commit\":\"$prior_commit\",
    \"message\":\"the one-day rule was wrong\"}" >/dev/null
rewound=$(block "how do refunds work")
says "three days" "$rewound"
silent "one day" "$rewound"
echo "    the withdrawn version is gone from the very next call"

echo
echo "================================================================"
echo "  THE TRAIL"
echo "================================================================"
DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$PRMT_DB"
export DATABASE_URL
psql_c "select action, actor_subject,
               coalesce(payload->>'pack',
                        payload->'records'->0->>'member') as member,
               payload->>'embedded' as embedded
        from audit_log
        where tenant_id = '$tenant_id'
          and (action like 'context_pack.%'
               or (action = 'vedaflow.channel.published'
                   and payload->>'asset' = 'context-pack')
               or action = 'vedaflow.channel.rolled_back')
        order by seq;"

# Swept for phrases that exist only in the documents — not for a word the
# proposal titles legitimately carry, since a title is the human's own one
# line about the change and an auditor is meant to read it.
leaks=$(psql_t "select count(*) from audit_log
                where tenant_id = '$tenant_id'
                  and (payload::text like '%five hundred pounds%'
                       or payload::text like '%Settle refunds%'
                       or payload::text like '%reviewed every quarter%'
                       or payload::text like '%ghp_0123456789%')")
[ "$leaks" = "0" ] || {
  echo "demo FAILED: $leaks audit payload(s) carry document text" >&2; exit 1; }
echo "    no payload carries a line of any document: names, addresses,"
echo "    counts, commits and tiers, which is what an auditor rechecks"

# And the served chunks are watermarked inside context.injected with the
# pack channel they composed against (ADR-0050 decision 13) — a chunk gets
# no event of its own because it arrives through a route that chains one.
watermarked=$(psql_t "select count(*) from audit_log
                      where tenant_id = '$tenant_id'
                        and action = 'context.injected'
                        and payload::text like '%context-pack/published%'")
[ "$watermarked" -gt 0 ] || {
  echo "demo FAILED: no injected block cited the pack channel" >&2; exit 1; }
echo "    $watermarked injected block(s) cite the pack commit they composed"
echo "    against — as recomputable as one naming memory"

$CLI audit verify --tenant "$tenant_id"

echo
echo "==> THE AC SUITE"
cargo test -p synveda-gateway --test context_packs -- --test-threads=1
echo
echo "--> and the vocabulary, the asset, the schema and the packs"
cargo test -p synveda-types --lib pack
cargo test -p synveda-vedaflow --lib packs
cargo test -p synveda-store --test rls
cargo test -p synveda-policy --test packs --test roles --test approvals \
  --test service_scope

echo
echo "PRMT-2 demo complete."
