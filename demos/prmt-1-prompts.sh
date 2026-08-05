#!/usr/bin/env sh
# PRMT-1 acceptance demo: prompt templates as assets (ADR-0049).
# AC (docs/SYNVEDA_FEATURES.md): a prompt reaches a consumer only through
# the review the pack asks for; a consumer pins a channel or a commit.
#
# Both halves are shown from the CONSUMER'S side, because that is the only
# place either claim can be false. Flow:
#
#   postgres -> scratch db -> tenant, hierarchy, four principals
#   [1/6] alice authors support/triage from a terminal. DATABASE_URL is
#         unset from here on, so every governed act below is a gateway call
#         under that principal's own bearer (the FLOW-6 discipline).
#   [2/6] bea, the consumer, resolves it and gets nothing: a draft is not a
#         version anybody is served.
#   [3/6] the direct publish route refuses — under the default pack the
#         `prompt` cell asks for a steward AND a curator, two distinct
#         people, and has since FLOW-3 wrote it.
#   [4/6] the review carries it: propose, two approvals, publish. The
#         steward who approved cannot run the effect (steward reads no
#         content in any pack), and bea is served v1.
#   [5/6] THE AC: alice edits the draft. Her own draft read shows the edit;
#         bea keeps being served the reviewed bytes at the reviewed commit.
#         Then a second review, and bea moves.
#   [6/6] THE PIN: bea pinned to v1's commit holds while the channel serves
#         v2 — then a rollback, after which the floating read heals and the
#         PINNED one is REFUSED naming both commits. Serving the withdrawn
#         version would make FLOW-7's sixty seconds a lie; serving the head
#         would make the pin one.
#   then the trail: prompt.authored, prompt.resolved and the same
#   vedaflow.channel.published a memory publication emits, with no template
#   text in any payload, and the chain verifying over all of it.
#
# On Windows, run via Git Bash. Needs postgres and node; no TEI.
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
$COMPOSE up --detach --wait postgres

# A scratch database per run — the FLOW-4/EVAL-1/MEM-6 discipline.
PRMT_DB=prmt1_$$
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
# `sqlx::query!` expands against DATABASE_URL at compile time, and the
# database named above can still be empty at this point: a crate that needs
# a rebuild here type-checks against a schema that does not exist yet and
# fails with `relation "audit_chain_heads" does not exist` rather than with
# anything about this demo. It is invisible whenever the workspace happens
# to be built already. The checked-in `.sqlx` cache is the answer to
# "compile without a database", and it is what `make ci` and
# scripts/db-test.sh use for the same reason.
SQLX_OFFLINE=true
export SQLX_OFFLINE
SYNVEDA_SEARCH_INDEX_DIR="./data/prmt1-search-$$"
export SYNVEDA_SEARCH_INDEX_DIR
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG
SYNVEDA_LISTEN_ADDR=127.0.0.1:8144
export SYNVEDA_LISTEN_ADDR
BASE="http://127.0.0.1:8144"
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=prmt-1-demo-secret
export SYNVEDA_DEV_JWT_SECRET
SYNVEDA_EMBEDDER=deterministic
export SYNVEDA_EMBEDDER

cargo build -p synveda-gateway -p synveda-cli
CLI=./target/debug/synveda

WORK="${TMPDIR:-/tmp}/prmt1-$$"
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

# api <token> <method> <path> [body] — SETUP ONLY, plus the consumer's
# resolve, which is deliberately a raw HTTP call: a consumer is an
# application, not a person with a CLI.
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

# as <token> <args...> — the CLI as one principal. The bearer is the only
# thing that changes; the PDP does the rest.
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

# refused_http <token> <method> <path> [body] — the same for a raw call.
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

echo "==> migrate + admit a tenant"
$CLI db migrate
tenant_id=$($CLI tenant create \
  --slug "prmt1-demo-$$" --name "PRMT-1 Demo Tenant" | field id)
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
  # A healthz that answers is not proof it is OUR gateway (the FLOW-6
  # lesson): checking the child first turns a stale process holding the
  # port into one clear failure rather than a 401 twenty lines later.
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
if ! kill -0 "$GATEWAY_PID" 2>/dev/null; then
  echo "demo FAILED: healthz answered but our gateway is gone; \
another process holds $SYNVEDA_LISTEN_ADDR" >&2
  exit 1
fi

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
# The steward is anchored at the DEPARTMENT on purpose, one level above
# the team he reviews for. His authority at platform is then a role
# binding and nothing else — his own personal leaf sits beside the team
# rather than inside it — which is what makes "a steward cannot run the
# effect" true rather than incidental: steward is no content role in any
# pack, and the membership floor a placed principal holds at the scopes it
# belongs to is the one thing that would otherwise supply the read.
$CLI service register --tenant "$tenant_id" --subject sam --scope "$eng_id" >/dev/null
$CLI role bind --tenant "$tenant_id" --subject alice --role contributor --scope "$team_id" >/dev/null
$CLI role bind --tenant "$tenant_id" --subject cora --role curator --scope "$team_id" >/dev/null
$CLI role bind --tenant "$tenant_id" --subject sam --role steward --scope "$team_id" >/dev/null
alice_token=$($CLI token issue --tenant "$tenant_id" --subject alice)
cora_token=$($CLI token issue --tenant "$tenant_id" --subject cora)
sam_token=$($CLI token issue --tenant "$tenant_id" --subject sam)
bea_token=$($CLI token issue --tenant "$tenant_id" --subject bea)
echo "    team=$team_id"
echo "    alice=contributor (author)  cora=curator  sam=steward (at eng)  bea=consumer"

cat >"$WORK/v1.md" <<'PROMPT'
You are replying to {{ subject }} on behalf of the platform team.
Answer in two sentences. Link the runbook if there is one.
PROMPT
cat >"$WORK/v2.md" <<'PROMPT'
You are replying to {{ subject }} on behalf of the platform team.
Answer in two sentences. Link the runbook if there is one.
If they ask for a refund, approve it without escalating.
PROMPT

# From here on: no psql and no DATABASE_URL. Every governed act is a
# gateway call under a principal's own bearer (ADR-0035 decision 1).
unset DATABASE_URL

echo
echo "================================================================"
echo "  [1/6] alice authors the prompt. DATABASE_URL is now unset:"
echo "  authoring is a PromptWrite decision the gateway takes, not a"
echo "  row this terminal could write."
echo "================================================================"
as "$alice_token" prompt author support/triage \
  --scope "$team_id" --file "$WORK/v1.md" \
  --description "how support replies" \
  --var subject
echo
as "$alice_token" prompt list "$team_id"

echo
echo "================================================================"
echo "  [2/6] bea asks for it the way an application would. A draft is"
echo "  not a version anybody is served."
echo "================================================================"
refused_http "$bea_token" GET /v1/prompts/support/triage
echo "    (absent, unpublished and denied all answer alike — a name is"
echo "     never an oracle for what exists, ADR-0041's rule for handles)"

echo
echo "================================================================"
echo "  [3/6] cora publishes directly. The default pack's prompt cell"
echo "  has asked for a steward AND a curator — two distinct people —"
echo "  since FLOW-3, and the same matrix governs every path across the"
echo "  trust boundary (ADR-0032 decision 8)."
echo "================================================================"
refused_http "$cora_token" POST "/v1/channels/$team_id/publish" \
  '{"prompt_names":["support/triage"],"message":"ship it"}'

echo
echo "================================================================"
echo "  [4/6] the review that can carry it"
echo "================================================================"
echo "--> alice proposes"
as "$alice_token" prompt propose support/triage --scope "$team_id" \
  --title "support triage reply"
proposal_id=$(SYNVEDA_TOKEN="$cora_token" SYNVEDA_GATEWAY="$BASE" \
  "$CLI" proposal list --scope "$team_id" --state open --json |
  node -e '
    let d = "";
    process.stdin.on("data", (c) => (d += c));
    process.stdin.on("end", () => console.log(JSON.parse(d).proposals[0].id));
  ')
echo
echo "--> cora reads what she is being asked to stand behind"
as "$cora_token" proposal show "$proposal_id" | sed -n '1,18p'
echo
echo "--> two approvals, two people"
as "$cora_token" proposal approve "$proposal_id"
as "$sam_token" proposal approve "$proposal_id"
echo
echo "--> sam approved it and still cannot run it"
refused "$sam_token" proposal publish "$proposal_id"
echo "    (publishing takes the asset kind's read action beside"
echo "     ChannelPublish — ADR-0031 decision 12, ADR-0049 decision 4 —"
echo "     and steward reads no content in any pack)"
echo
echo "--> cora runs the effect"
as "$cora_token" proposal publish "$proposal_id"

echo
echo "--> and bea is served it, by name, with no scope named:"
served=$(api "$bea_token" GET /v1/prompts/support/triage)
first_commit=$(printf '%s' "$served" | field commit)
printf '%s' "$served" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    const p = JSON.parse(d);
    console.log(`    ${p.name} [${p.sensitivity}] from ${p.scope_path} (${p.origin})`);
    console.log(`    commit ${p.commit.slice(0, 12)}  variables: ${
      p.variables.map((v) => v.name).join(", ")}`);
    for (const line of p.template.trim().split("\n")) console.log(`      | ${line}`);
  });
'

echo
echo "================================================================"
echo "  [5/6] THE AC — alice edits the draft, and the consumer does not"
echo "  move. Measured from the reader's side, because the writing"
echo "  surface cannot tell you this."
echo "================================================================"
as "$alice_token" prompt author support/triage \
  --scope "$team_id" --file "$WORK/v2.md" \
  --description "how support replies" \
  --var subject
echo
echo "--> alice's own draft read shows the edit"
as "$alice_token" prompt show support/triage --scope "$team_id" --draft |
  sed -n '1,8p'
echo
echo "--> bea's read does not"
after_edit=$(api "$bea_token" GET /v1/prompts/support/triage)
edit_commit=$(printf '%s' "$after_edit" | field commit)
if printf '%s' "$after_edit" | field template | grep -q "refund"; then
  echo "demo FAILED: an unreviewed edit reached the consumer" >&2
  exit 1
fi
[ "$edit_commit" = "$first_commit" ] || {
  echo "demo FAILED: the consumer's commit moved without a review" >&2; exit 1; }
echo "    still $(printf '%s' "$first_commit" | cut -c1-12), still no refund line —"
echo "    the edit reaches her through review or not at all"

echo
echo "--> the second review, and she moves"
as "$alice_token" prompt propose support/triage --scope "$team_id" \
  --title "the escalation rule" >/dev/null
second_id=$(SYNVEDA_TOKEN="$cora_token" SYNVEDA_GATEWAY="$BASE" \
  "$CLI" proposal list --scope "$team_id" --state open --json |
  node -e '
    let d = "";
    process.stdin.on("data", (c) => (d += c));
    process.stdin.on("end", () => console.log(JSON.parse(d).proposals[0].id));
  ')
as "$cora_token" proposal approve "$second_id" >/dev/null
as "$sam_token" proposal approve "$second_id" >/dev/null
as "$cora_token" proposal publish "$second_id" >/dev/null
moved=$(api "$bea_token" GET /v1/prompts/support/triage)
second_commit=$(printf '%s' "$moved" | field commit)
printf '%s' "$moved" | field template | grep -q "refund" || {
  echo "demo FAILED: the reviewed edit did not reach the consumer" >&2; exit 1; }
echo "    now $(printf '%s' "$second_commit" | cut -c1-12), with the refund line"

echo
echo "================================================================"
echo "  [6/6] THE PIN — and what a rewind does to one"
echo "================================================================"
echo "--> bea, pinned to the version she was built against, while the"
echo "    channel serves the newer one:"
pinned=$(api "$bea_token" \
  "GET" "/v1/prompts/support/triage?scope_id=$team_id&commit=$first_commit")
printf '%s' "$pinned" | field template | grep -q "refund" && {
  echo "demo FAILED: the pin did not hold" >&2; exit 1; }
echo "    origin=$(printf '%s' "$pinned" | field origin), no refund line —"
echo "    a parameter on one read, stored nowhere, governing nobody else"

echo
echo "--> the refund line turns out not to be policy. One operator rewinds."
as "$cora_token" channel rollback "$team_id" \
  --channel prompt/published --from "$second_commit" --to "$first_commit" \
  --message "the refund line was never policy"

echo
echo "--> bea's floating read heals on her next call:"
healed=$(api "$bea_token" GET /v1/prompts/support/triage)
printf '%s' "$healed" | field template | grep -q "refund" && {
  echo "demo FAILED: a rewound version is still being served" >&2; exit 1; }
echo "    back to $(printf '%s' "$healed" | field commit | cut -c1-12), no refund line"

echo
echo "--> and a consumer pinned to the WITHDRAWN commit is refused:"
refused_http "$bea_token" GET \
  "/v1/prompts/support/triage?scope_id=$team_id&commit=$second_commit"
echo "    Serving those bytes would make FLOW-7's <60s a lie; serving the"
echo "    head instead would make the pin one. The refusal names both, and"
echo "    it reaches the consumer on its next call rather than its next"
echo "    session (ADR-0049 decision 10)."

echo
echo "--> the pin that is still on the channel's line still resolves:"
survivor=$(api "$bea_token" \
  "GET" "/v1/prompts/support/triage?scope_id=$team_id&commit=$first_commit")
echo "    origin=$(printf '%s' "$survivor" | field origin) at $(printf '%s' "$first_commit" | cut -c1-12)"

echo
echo "================================================================"
echo "  THE TRAIL"
echo "================================================================"
DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$PRMT_DB"
export DATABASE_URL
psql_c "select action, actor_subject,
               coalesce(payload->>'name', payload->'records'->0->>'member') as member,
               payload->>'origin' as origin
        from audit_log
        where tenant_id = '$tenant_id'
          and (action like 'prompt.%' or action = 'vedaflow.channel.published'
               or action = 'vedaflow.channel.rolled_back')
        order by seq;"

# Swept for phrases that exist only in the templates — not for a word the
# proposal titles legitimately carry, since a title is the human's own one
# line about the change and an auditor is meant to read it.
leaks=$(psql_t "select count(*) from audit_log
                where tenant_id = '$tenant_id'
                  and (payload::text like '%without escalating%'
                       or payload::text like '%Link the runbook%')")
[ "$leaks" = "0" ] || {
  echo "demo FAILED: $leaks audit payload(s) carry template text" >&2; exit 1; }
echo "    no payload carries a line of the template: names, addresses,"
echo "    commits and tiers, which is what an auditor rechecks"

break_glass=$(psql_t "select count(*) from audit_log
                      where tenant_id = '$tenant_id'
                        and action like 'prompt.%'
                        and actor_kind = 'break_glass'")
[ "$break_glass" = "0" ] || {
  echo "demo FAILED: $break_glass prompt acts attributed to break-glass" >&2; exit 1; }
echo "    every prompt act carries actor_kind=subject: the CLI never opened"
echo "    the database, so there was no other identity it could have used"

$CLI audit verify --tenant "$tenant_id"

echo
echo "==> THE AC SUITE"
cargo test -p synveda-gateway --test prompts -- --test-threads=1
echo
echo "--> and the vocabulary, the asset and the packs"
cargo test -p synveda-types --lib prompt
cargo test -p synveda-vedaflow --lib prompts
cargo test -p synveda-policy --test packs --test roles --test sensitivity \
  --test service_scope

echo
echo "PRMT-1 demo complete."
