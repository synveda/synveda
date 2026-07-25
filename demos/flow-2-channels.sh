#!/usr/bin/env sh
# FLOW-2 acceptance demo: VedaFlow channels (ADR-0031).
# AC (docs/backlog/FLOW-2.md): the "bank mode" switch (published-only)
# flips composition instantly. Plus the feature text: derived/staged/
# published refs per scope per asset type; inject reads published
# (+ derived per policy).
#
# Flow: migrate -> admit a tenant -> org/team over the API -> alice
# (curator at the team) observes a session; the extraction worker commits
# her memories AND the {team}/memory/derived commit in one transaction
# (ADR-0022's forward obligation, discharged) -> inject: everything
# composes, everything marked unreviewed, because nobody has reviewed
# anything -> a *contributor* is refused the publish (403: writing a
# memory is not declaring it reviewed) -> alice publishes one record onto
# her own scope's channel through POST /v1/channels/{scope}/publish -> a
# SECOND curator, bound at the same team, is refused the same call: the
# privacy floor denies them the read, and nobody publishes what they
# cannot read (ADR-0031 decision 12) -> inject again: that
# record renders unmarked, the rest still says unreviewed -> THE SWITCH:
# a published-only pack becomes the tenant default and the very next
# inject composes the published record alone -> the audit trail names
# who published what under which pack, and the inject event cites the
# channel commit -> THE OTHER HALF: publication binds bytes, not ids —
# a curator with only `memory.write` cannot launder an edit through a
# published id, because the edited record stops matching the address the
# channel admitted. On Windows, run via Git Bash. Needs only postgres.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
SYNVEDA_LISTEN_ADDR=127.0.0.1:8138
export SYNVEDA_LISTEN_ADDR
SYNVEDA_DEV_JWT_SECRET=flow-2-demo-secret
export SYNVEDA_DEV_JWT_SECRET
SYNVEDA_EXTRACTOR=deterministic
export SYNVEDA_EXTRACTOR
SYNVEDA_EXTRACTION_POLL_MS=300
export SYNVEDA_EXTRACTION_POLL_MS
# The pack refresher's interval bounds "instantly" for a *stored* pack:
# the switch is in force on the first inject after the pack is picked up
# (ADR-0014 decision 3).
SYNVEDA_POLICY_REFRESH_SECS=1
export SYNVEDA_POLICY_REFRESH_SECS
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG

BASE="http://127.0.0.1:8138"

cargo build -p synveda-gateway -p synveda-cli

psql_c() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda -c "$1"
}

psql_t() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda -tAc "$1"
}

# `echo` would interpret the \n escapes inside a JSON body; printf does not.
emit() {
  printf '%s' "$1"
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

# code <token> <method> <path> [body] — prints the HTTP code only.
code() {
  tok=$1
  method=$2
  path=$3
  body=${4:-}
  curl -s -o /dev/null -w '%{http_code}' -X "$method" \
    -H "Authorization: Bearer $tok" -H "Content-Type: application/json" \
    ${body:+-d "$body"} "$BASE$path"
}

echo "==> migrate + admit a tenant"
./target/debug/synveda db migrate
tenant_id=$(./target/debug/synveda tenant create \
  --slug "flow2-demo-$$" --name "FLOW-2 Demo Tenant" | field id)
echo "    tenant: $tenant_id"
admin_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-admin)
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null

echo "==> purging leftover observe-queue signals from other runs (shared queue)"
psql_t "select pgmq.purge_queue('observe')" >/dev/null

GATEWAY_LOG="${TMPDIR:-/tmp}/flow2-gateway.log"
./target/debug/synveda-gateway >"$GATEWAY_LOG" 2>&1 &
GATEWAY_PID=$!
trap 'kill "$GATEWAY_PID" 2>/dev/null || true' EXIT INT TERM

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS "$BASE/healthz" >/dev/null 2>&1; do
  tries=$((tries + 1))
  if [ "$tries" -ge 30 ]; then
    echo "demo FAILED: gateway did not become healthy" >&2
    exit 1
  fi
  sleep 1
done

echo "==> hierarchy: acme > platform; alice curator, bob contributor, carol curator"
org_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
team_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"team\",\"slug\":\"platform\",\"name\":\"Platform\"}" |
  field id)
./target/debug/synveda service register --tenant "$tenant_id" \
  --subject alice --scope "$team_id" >/dev/null
./target/debug/synveda service register --tenant "$tenant_id" \
  --subject bob --scope "$team_id" >/dev/null
./target/debug/synveda service register --tenant "$tenant_id" \
  --subject carol --scope "$team_id" >/dev/null
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject alice --role curator --scope "$team_id" >/dev/null
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject bob --role contributor --scope "$team_id" >/dev/null
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject carol --role curator --scope "$team_id" >/dev/null
alice_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject alice)
bob_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject bob)
carol_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject carol)
echo "    team=$team_id  alice=curator  bob=contributor  carol=curator"

now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
batch="{\"session_id\":\"flow-2-demo\",\"events\":[
  {\"idempotency_key\":\"f1\",\"kind\":\"decision\",
   \"payload\":{\"text\":\"Deploys go out on tuesdays after the release review.\"},
   \"occurred_at\":\"$now\"},
  {\"idempotency_key\":\"f2\",\"kind\":\"transcript_delta\",
   \"payload\":{\"text\":\"Someone mentioned deploying on a friday once.\"},
   \"occurred_at\":\"$now\"}]}"

echo "==> alice observes two events; the worker commits records AND the"
echo "    derived-channel commit in one transaction"
accepted=$(api "$alice_token" POST /v1/observe "$batch" | field accepted)
[ "$accepted" = "2" ] || { echo "demo FAILED: accepted=$accepted" >&2; exit 1; }
alice_scope=$(psql_t "select scope_id from identities
                      where tenant_id = '$tenant_id' and subject = 'alice'")
tries=0
until [ "$(psql_t "select count(*) from records where tenant_id = '$tenant_id'")" = "2" ]; do
  tries=$((tries + 1))
  [ "$tries" -ge 40 ] && { echo "demo FAILED: records never landed" >&2; exit 1; }
  sleep 0.5
done
echo "    records=2, and the channel they landed on:"
psql_c "select r.name, encode(r.commit_hash, 'hex') as commit,
               (select count(*) from vedaflow_tree_entries e
                where e.tenant_id = r.tenant_id and e.tree_hash = c.tree_hash) as entries
        from vedaflow_refs r
        join vedaflow_commits c on c.tenant_id = r.tenant_id and c.hash = r.commit_hash
        where r.tenant_id = '$tenant_id';"

echo "==> inject BEFORE anyone publishes: nothing is trusted"
before=$(api "$alice_token" POST /v1/inject '{"session_id":"flow-2-demo"}')
emit "$before" | field text | sed 's/^/    | /'
marked=$(emit "$before" | field text | grep -c '\[unreviewed\]' || true)
[ "$marked" = "2" ] || {
  echo "demo FAILED: expected both entries unreviewed, got $marked" >&2
  exit 1
}
echo "    both marked unreviewed — authorship (and extraction) is not review"

proc_id=$(psql_t "select id from records
                  where tenant_id = '$tenant_id' and content like 'Deploys go out%'")
echo "    the canonical one: $proc_id"

echo "==> bob (contributor: may WRITE memories here) tries to publish"
publish_body="{\"record_ids\":[\"$proc_id\"],\"message\":\"self-service trust\"}"
bob_code=$(code "$bob_token" POST "/v1/channels/$alice_scope/publish" "$publish_body")
[ "$bob_code" = "403" ] || { echo "demo FAILED: bob got $bob_code, want 403" >&2; exit 1; }
echo "    403 — writing a memory is not declaring it reviewed (seed §5)"

echo "==> carol (curator at the SAME team) tries to publish alice's memory"
carol_code=$(code "$carol_token" POST "/v1/channels/$alice_scope/publish" "$publish_body")
[ "$carol_code" = "403" ] || { echo "demo FAILED: carol got $carol_code, want 403" >&2; exit 1; }
echo "    403 — she holds ChannelPublish on that leaf but not MemoryRead:"
echo "    the privacy floor stands, and nobody publishes what they cannot read"
echo "    (this is why the user->team climb is FLOW-3's proposal, not a reach-in)"

echo "==> THE GOVERNED ACT: alice (curator) publishes her own memory"
published=$(api "$alice_token" POST "/v1/channels/$alice_scope/publish" "$publish_body")
commit=$(emit "$published" | field commit)
echo "    memory/published -> $commit"
echo "    members=$(emit "$published" | field members) added=$(emit "$published" | field added)"
echo "    at the address it reviewed: $(emit "$published" | field published 0 object_hash)"

echo "==> the standing channels at that scope"
api "$alice_token" GET "/v1/channels/$alice_scope" | field channels | sed 's/^/    /'
echo "    (staged has no writer until FLOW-3, so it is absent — not conjured empty)"

echo "==> inject AFTER: the reviewed record loses its marker"
after=$(api "$alice_token" POST /v1/inject '{"session_id":"flow-2-demo"}')
emit "$after" | field text | sed 's/^/    | /'
marked=$(emit "$after" | field text | grep -c '\[unreviewed\]' || true)
[ "$marked" = "1" ] || {
  echo "demo FAILED: expected exactly one unreviewed entry, got $marked" >&2
  exit 1
}

echo "==> THE AC — THE SWITCH: a published-only pack as the tenant default"
cat >"${TMPDIR:-/tmp}/flow2-bank.cedar" <<'CEDAR'
// Bank mode is a composition rule, not a grant: this pack permits exactly
// what the strict default does for a placed principal, and differs only in
// its composition config (ADR-0025 decision 2; ADR-0031 decision 10).
permit (principal, action == Synveda::Action::"MemoryRead", resource)
    when { principal in resource };
permit (principal, action == Synveda::Action::"MemoryWrite", resource)
    when { principal has home && resource == principal.home };
CEDAR
./target/debug/synveda policy apply --tenant "$tenant_id" --name bank \
  --composition-budget 1500 --composition-channels published-only \
  "${TMPDIR:-/tmp}/flow2-bank.cedar" >/dev/null
api "$admin_token" PUT /v1/policy/default '{"name":"bank"}' >/dev/null
# The refresher installs stored packs on its interval; after that the
# switch is in force on the very next inject, with no restart.
sleep 3

banked=$(api "$alice_token" POST /v1/inject '{"session_id":"flow-2-demo"}')
emit "$banked" | field text | sed 's/^/    | /'
ids=$(emit "$banked" | field record_ids)
case "$ids" in
  *"$proc_id"*) echo "    the published record survives" ;;
  *) echo "demo FAILED: published record must survive bank mode" >&2; exit 1 ;;
esac
[ "$(emit "$banked" | field record_ids length)" = "1" ] || {
  echo "demo FAILED: only published material may compose under bank mode" >&2
  exit 1
}
echo "    and it is the only thing in the block — same token, no restart"

echo "==> the audit trail: the publication, and the block that cites it"
psql_c "select action,
               coalesce(payload->>'channel', payload->'channels'->0->>'ref') as channel,
               coalesce(payload->'authz'->>'pack', payload->'decisions'->0->>'pack') as pack
        from audit_log
        where tenant_id = '$tenant_id'
          and action in ('vedaflow.channel.published', 'context.injected')
        order by seq;"
cited=$(psql_t "select payload->'channels'->0->>'commit' from audit_log
                where tenant_id = '$tenant_id' and action = 'context.injected'
                order by seq desc limit 1")
[ "$cited" = "$commit" ] || {
  echo "demo FAILED: the block must cite the channel commit ($cited != $commit)" >&2
  exit 1
}
echo "    the block cites commit $cited — the one alice made"

echo "==> THE OTHER HALF: publication binds bytes, not ids"
psql_c "update records set content = 'Deploy whenever you like.'
        where id = '$proc_id';" >/dev/null
echo "    the published record was rewritten under its own id"
edited=$(api "$alice_token" POST /v1/inject '{"session_id":"flow-2-demo"}')
edited_ids=$(emit "$edited" | field record_ids)
[ "$edited_ids" = "[]" ] || {
  echo "demo FAILED: an edit must not ride a published id: $edited_ids" >&2
  exit 1
}
echo "    bank mode now composes nothing: the reviewed bytes no longer exist"
echo "    (the address the channel admitted no longer matches the record)"

echo
echo "==> THE AC SUITE"
cargo test -p synveda-gateway --test channels -- --nocapture --test-threads=1

echo
echo "==> and the composition suite, on real channels"
cargo test -p synveda-retrieval --test compose

echo
echo "FLOW-2 demo complete."
