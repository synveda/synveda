#!/usr/bin/env sh
# FLOW-6 acceptance demo: the CLI review flow (ADR-0035).
# AC (docs/backlog/FLOW-6.md): full review possible without console.
#
# The claim is narrow and the demo is shaped to it: from the moment a
# proposal exists, EVERY governed act below is `synveda proposal ...` and
# nothing else. No curl, no psql, no console — and no database URL either,
# because these verbs are gateway clients under the reviewer's own bearer
# (ADR-0035 decision 1). `DATABASE_URL` is unset before the review begins
# and the CLI keeps working, which is the strongest way to say that
# approving is not an operator action.
#
# Flow: migrate -> admit a tenant -> acme/eng/platform over the API -> four
# principals (dana contributor, cora curator, cleo compliance, sam steward)
# -> records on the team's shelf -> SETUP opens two proposals the way
# proposals arrive (a contributor's POST; FLOW-4's rules open them with
# nobody deciding to) -> THE AC: cora lists her queue, reads one in full,
# approves it, and runs its effect, all from the terminal -> THE DIFF: the
# runbook is edited and re-proposed, and the review renders it as an
# `update` with both sides and a line diff of the one line that moved ->
# THE QUEUE: `synveda proposal review` walks the open proposals and takes
# verdicts on stdin, and end-of-input casts nothing -> REFUSALS a reviewer
# actually meets, in the product's own words -> THE TRAIL: every act
# chained under the reviewer's own subject, and the chain verifies.
#
# The credential half is ADPT-1's and `demos/adpt-1-claude-code.sh` proves
# it against live Rauthy; here the reviewers carry dev tokens through
# SYNVEDA_TOKEN (the override ADR-0027 kept for CI and demos), so this
# demo needs only postgres. On Windows, run via Git Bash.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
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
SYNVEDA_LISTEN_ADDR=127.0.0.1:8143
export SYNVEDA_LISTEN_ADDR
SYNVEDA_DEV_JWT_SECRET=flow-6-demo-secret
export SYNVEDA_DEV_JWT_SECRET
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG

BASE="http://127.0.0.1:8143"
CLI=./target/debug/synveda

cargo build -p synveda-gateway -p synveda-cli

psql_t() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda -tAc "$1"
}

psql_c() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda -c "$1"
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

# api <token> <method> <path> [body] — SETUP ONLY. Nothing below the AC
# banner uses it.
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

# as <token> <args...> — runs the CLI as one principal. The bearer is the
# only thing that changes; the PDP does the rest.
as() {
  tok=$1
  shift
  SYNVEDA_TOKEN="$tok" SYNVEDA_GATEWAY="$BASE" "$CLI" "$@"
}

# refused <token> <args...> — runs a CLI command that must fail, and
# prints the refusal. `set -e` would otherwise end the demo on it.
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

echo "==> migrate + admit a tenant"
$CLI db migrate
tenant_id=$($CLI tenant create \
  --slug "flow6-demo-$$" --name "FLOW-6 Demo Tenant" | field id)
echo "    tenant: $tenant_id"
admin_token=$($CLI token issue --tenant "$tenant_id" --subject demo-admin)
$CLI role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null

GATEWAY_LOG="${TMPDIR:-/tmp}/flow6-gateway.log"
./target/debug/synveda-gateway >"$GATEWAY_LOG" 2>&1 &
GATEWAY_PID=$!
trap 'kill "$GATEWAY_PID" 2>/dev/null || true' EXIT INT TERM

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS "$BASE/healthz" >/dev/null 2>&1; do
  # A healthz that answers is not proof it is OUR gateway: a leftover
  # process from an earlier demo holds the port, ours dies on bind, and
  # every request then goes to a stranger signed with another secret.
  # Checking the child first turns that into one clear failure.
  if ! kill -0 "$GATEWAY_PID" 2>/dev/null; then
    echo "demo FAILED: the gateway exited; see $GATEWAY_LOG" >&2
    echo "  (is another demo's gateway already on $SYNVEDA_LISTEN_ADDR?)" >&2
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

echo "==> hierarchy: acme > eng > platform, with four principals at the team"
org_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
eng_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"department\",\"slug\":\"eng\",\"name\":\"Engineering\"}" |
  field id)
team_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$eng_id\",\"kind\":\"team\",\"slug\":\"platform\",\"name\":\"Platform\"}" |
  field id)
for who in dana cora cleo sam; do
  $CLI service register --tenant "$tenant_id" \
    --subject "$who" --scope "$team_id" >/dev/null
done
$CLI role bind --tenant "$tenant_id" --subject dana --role contributor --scope "$team_id" >/dev/null
$CLI role bind --tenant "$tenant_id" --subject cora --role curator --scope "$team_id" >/dev/null
$CLI role bind --tenant "$tenant_id" --subject cleo --role compliance --scope "$team_id" >/dev/null
$CLI role bind --tenant "$tenant_id" --subject sam --role steward --scope "$team_id" >/dev/null
dana_token=$($CLI token issue --tenant "$tenant_id" --subject dana)
cora_token=$($CLI token issue --tenant "$tenant_id" --subject cora)
cleo_token=$($CLI token issue --tenant "$tenant_id" --subject cleo)
echo "    team=$team_id"
echo "    dana=contributor  cora=curator  cleo=compliance  sam=steward"

# Records land at their owner's personal scope through observe (ADR-0020),
# so the team's own shelf is seeded directly: the feature under review is
# the review. Record and embedding go in one transaction (ADR-0023).
dana_identity=$(psql_t "select id from identities
                        where tenant_id = '$tenant_id' and subject = 'dana'")
seed_record() {
  psql_t "begin;
          insert into records (id, tenant_id, scope_id, owner_id, kind, class,
                               content, sensitivity, provenance, valid_from)
          values ('$1', '$tenant_id', '$team_id', '$dana_identity', 'derived',
                  'procedure', \$content\$$2\$content\$, '$3',
                  '{\"source\":\"flow-6 demo\"}', now() - interval '1 hour');
          insert into record_embeddings (record_id, tenant_id, model, dim, embedding)
          values ('$1', '$tenant_id', 'hash@1', 4, '[0.25,0.25,0.25,0.25]');
          commit;" >/dev/null
}
edit_record() {
  psql_t "update records set content = \$content\$$2\$content\$ where id = '$1';" >/dev/null
}
runbook_id=$(psql_t "select gen_random_uuid()")
bridge_id=$(psql_t "select gen_random_uuid()")
window_id=$(psql_t "select gen_random_uuid()")
seed_record "$runbook_id" "check the on-call rota
rotate the signing key
file the change record" "internal"
seed_record "$bridge_id" "a sev-1 bridge is opened by the on-call lead" "restricted"
seed_record "$window_id" "deploys go out on tuesdays" "internal"

echo
echo "==> SETUP — proposals arrive the way proposals arrive"
echo "    (a contributor's POST here; a FLOW-4 rule opens them with nobody"
echo "     deciding to. Either way, what follows is FLOW-6's.)"
open_proposal() {
  api "$dana_token" POST /v1/proposals \
    "{\"scope_id\":\"$team_id\",\"record_ids\":[\"$1\"],\"title\":\"$2\"}" | field id
}
first_id=$(open_proposal "$runbook_id" "promote the key-rotation runbook")
restricted_id=$(open_proposal "$bridge_id" "publish the sev-1 bridge procedure")
window_proposal=$(open_proposal "$window_id" "promote the deploy window")
echo "    three open, oldest first:"
echo "      $first_id       the runbook (internal)"
echo "      $restricted_id  the sev-1 bridge (restricted)"
echo "      $window_proposal  the deploy window (internal)"

# From here on: no curl, no psql, and — deliberately — no database at all.
unset DATABASE_URL
echo
echo "================================================================"
echo "  THE AC — the whole review from a terminal. DATABASE_URL is now"
echo "  unset: these verbs are gateway clients under the reviewer's own"
echo "  bearer, so the PDP decides who may approve exactly as it would"
echo "  for a console (ADR-0035 decision 1)."
echo "================================================================"

echo
echo "--> cora lists what is waiting on her"
as "$cora_token" proposal list --scope "$team_id" --state open

echo
echo "--> cora reads one in full: what it needs, and what it would DO"
as "$cora_token" proposal show "$first_id" | tee "${TMPDIR:-/tmp}/flow6-show.txt"
grep -q "1 × curator" "${TMPDIR:-/tmp}/flow6-show.txt" || {
  echo "demo FAILED: the requirement must be on screen" >&2; exit 1; }
grep -q "add   " "${TMPDIR:-/tmp}/flow6-show.txt" || {
  echo "demo FAILED: an empty channel must render every member as an add" >&2; exit 1; }
grep -q "rotate the signing key" "${TMPDIR:-/tmp}/flow6-show.txt" || {
  echo "demo FAILED: a review with no content is not a review" >&2; exit 1; }
echo "    ^ every field of the object, because nothing is being replaced yet"

echo
echo "--> cora approves"
as "$cora_token" proposal approve "$first_id" --comment "matches the runbook we agreed"

echo
echo "--> and runs the effect. A SEPARATE act: the deciding approval does"
echo "    not publish, or a compliance vote would publish under system"
echo "    authority (ADR-0032 decision 9)"
as "$cora_token" proposal publish "$first_id"

echo
echo "================================================================"
echo "  THE DIFF — the case a review surface exists for"
echo "================================================================"
echo "--> the runbook is edited. Approvals bind bytes, so the only way to"
echo "    republish it is a NEW proposal — and now the channel already"
echo "    holds a version, which is what makes this a diff"
DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda \
  edit_record "$runbook_id" "check the on-call rota
rotate the signing key every 90 days
file the change record"
opened=$(api "$dana_token" POST /v1/proposals \
  "{\"scope_id\":\"$team_id\",\"record_ids\":[\"$runbook_id\"],
    \"title\":\"revise the rotation interval\"}")
revision_id=$(echo "$opened" | field id)

as "$cora_token" proposal show "$revision_id" | tee "${TMPDIR:-/tmp}/flow6-diff.txt"
grep -q "update" "${TMPDIR:-/tmp}/flow6-diff.txt" || {
  echo "demo FAILED: replacing a published record is an update" >&2; exit 1; }
grep -q '^ *- rotate the signing key$' "${TMPDIR:-/tmp}/flow6-diff.txt" || {
  echo "demo FAILED: the old line must be on screen" >&2; exit 1; }
grep -q '^ *+ rotate the signing key every 90 days$' "${TMPDIR:-/tmp}/flow6-diff.txt" || {
  echo "demo FAILED: the new line must be on screen" >&2; exit 1; }
grep -q '^ *  check the on-call rota$' "${TMPDIR:-/tmp}/flow6-diff.txt" || {
  echo "demo FAILED: unchanged lines are context, not changes" >&2; exit 1; }
grep -q 'replacing object' "${TMPDIR:-/tmp}/flow6-diff.txt" || {
  echo "demo FAILED: the address being replaced must be named" >&2; exit 1; }
echo "    ^ one line moved, and exactly one line is marked. The object is"
echo "      canonical JSON with sorted keys — a byte diff would have"
echo "      rendered this as one enormous escaped string (ADR-0035 dec. 7)"

open_count() {
  as "$cora_token" proposal list --scope "$team_id" --state open --json 2>/dev/null |
    node -e 'let d="";process.stdin.on("data",c=>d+=c);
             process.stdin.on("end",()=>console.log(JSON.parse(d).proposals.length))'
}
state_of() {
  as "$cora_token" proposal show "$1" --json 2>/dev/null |
    node -e 'let d="";process.stdin.on("data",c=>d+=c);
             process.stdin.on("end",()=>console.log(JSON.parse(d).state))'
}

echo
echo "================================================================"
echo '  THE QUEUE — `synveda proposal review`, oldest first'
echo "================================================================"
echo "--> end of input casts NOTHING: an unattended review is a no-op,"
echo "    never a blind approval"
as "$cora_token" proposal review --scope "$team_id" </dev/null
still_open=$(open_count)
[ "$still_open" = "3" ] || {
  echo "demo FAILED: an EOF review changed something ($still_open open)" >&2; exit 1; }
echo "    3 still open, unchanged"

echo
echo "--> now driven, one verdict per proposal, oldest first: skip the"
echo "    restricted one, reject the deploy window (and note the empty"
echo "    reason being refused — a rejection an auditor cannot read the"
echo "    reason for is not a review), approve the revision"
printf 's\nr\n\nsuperseded by the revision\na\nthe interval is right\n' |
  as "$cora_token" proposal review --scope "$team_id"

[ "$(state_of "$window_proposal")" = "rejected" ] || {
  echo "demo FAILED: the deploy window should be rejected" >&2; exit 1; }
[ "$(state_of "$revision_id")" = "approved" ] || {
  echo "demo FAILED: the revision should be approved" >&2; exit 1; }
[ "$(state_of "$restricted_id")" = "open" ] || {
  echo "demo FAILED: a skip must cast nothing" >&2; exit 1; }
echo "    rejected / approved / untouched — three verdicts, one command"

echo
echo "--> the approved revision's effect, from the same terminal"
as "$cora_token" proposal publish "$revision_id"

echo
echo "================================================================"
echo "  REFUSALS a reviewer meets, in the product's own words"
echo "================================================================"
echo "--> dana (contributor) tries to approve her own proposal"
refused "$dana_token" proposal approve "$restricted_id"
echo "    the PDP, not the CLI: ProposalReview is a curator's"

echo
echo "--> cora asks for a tenant-wide listing she is not bound for"
refused "$cora_token" proposal list
echo "    (being a curator at one team is deliberately not a reason to see"
echo "     every proposal in the tenant — so the refusal names the flag)"

echo
echo "--> the restricted record: cora alone is not enough"
as "$cora_token" proposal approve "$restricted_id"
refused "$cora_token" proposal publish "$restricted_id"

echo
echo "--> cleo (compliance) reads it — content she holds no MemoryRead for,"
echo "    which is the disclosure the proposer made (ADR-0034 decision 1)"
as "$cleo_token" proposal show "$restricted_id" | sed -n '1,12p'
as "$cleo_token" proposal approve "$restricted_id"

echo
echo "--> and cleo cannot run the effect she just decided"
refused "$cleo_token" proposal publish "$restricted_id"
echo "    compliance holds no ChannelPublish in any pack. Had the deciding"
echo "    approval published, THAT call would have run under system"
echo "    authority — the bypass ADR-0032 decision 9 refuses"

echo
echo "--> cora runs it"
as "$cora_token" proposal publish "$restricted_id"

echo
echo "================================================================"
echo "  THE TRAIL — a CLI review and a console review are the same act"
echo "================================================================"
DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
psql_c "select action, actor_kind, actor_subject,
               payload->>'proposal_id' is not null as names_proposal
        from audit_log
        where tenant_id = '$tenant_id'
          and action like 'vedaflow.%'
        order by seq;"
break_glass=$(psql_t "select count(*) from audit_log
                      where tenant_id = '$tenant_id'
                        and action like 'vedaflow.%'
                        and actor_kind = 'break_glass'")
[ "$break_glass" = "0" ] || {
  echo "demo FAILED: $break_glass review acts attributed to break-glass" >&2; exit 1; }
echo "    every act carries actor_kind=subject under the reviewer's own"
echo "    token subject. Not one break-glass row: the CLI never opened the"
echo "    database, so there was no other identity it could have acted as,"
echo "    and a CLI review is indistinguishable in the chain from the"
echo "    console review it stands in for."
$CLI audit verify --tenant "$tenant_id"

echo
echo "==> THE AC SUITES"
echo "--> the review surface: add/update/none, both sides, the disclosure"
cargo test -p synveda-gateway --test review_surface -- --test-threads=1
echo
echo "--> the renderer and the CLI seam"
cargo test -p synveda-cli
echo
echo "--> and FLOW-2/3/4/5 unchanged and green"
cargo test -p synveda-gateway --test proposals --test cross_scope -- --test-threads=1

echo
echo "FLOW-6 demo complete."
