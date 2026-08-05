#!/usr/bin/env sh
# MEM-5 acceptance demo: always-on dedup & conflict detection (ADR-0039).
# AC (docs/backlog/MEM-5.md): superseded facts retrievable via as-of but
# excluded from current inject. (The knowledge-update *score* half of the
# AC is `make eval`'s `knowledge_update` axis — evals/scenarios/05,06.)
#
# Flow: postgres up -> migrate -> tenant, hierarchy, alice -> she states a
# fact in one session, and `inject` returns it -> she RESTATES it in
# another session, and the store still holds ONE record, with the merge on
# its provenance (an ADD-only store would hold two) -> she states what
# REPLACED it, and the pipeline closes the old fact's valid window, writes
# an explicit supersession edge, and re-commits the changed address to the
# derived channel -> the very next inject carries the new fact and not the
# one it replaced -> but the old fact is still there AS OF when it held,
# through the composition engine's own query at an earlier instant ->
# `memory.superseded` chains and `synveda audit verify` passes -> then the
# governance boundary: a curator PUBLISHES a fact, alice contradicts it,
# and the pipeline refuses to close a reviewed window, counts the refusal,
# and leaves both composing for a human to resolve.
#
# On Windows, run via Git Bash. Needs postgres and node; no TEI (this
# demo runs the network-free deterministic extractor and embedder, so
# everything it proves is proved by the lexical MinHash leg alone).
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
SYNVEDA_LISTEN_ADDR=127.0.0.1:8139
export SYNVEDA_LISTEN_ADDR
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=mem-5-demo-secret
export SYNVEDA_DEV_JWT_SECRET
SYNVEDA_EXTRACTOR=deterministic
export SYNVEDA_EXTRACTOR
SYNVEDA_EMBEDDER=deterministic
export SYNVEDA_EMBEDDER
SYNVEDA_EXTRACTION_POLL_MS=300
export SYNVEDA_EXTRACTION_POLL_MS

cargo build -p synveda-gateway -p synveda-cli

psql_t() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda -tAc "$1"
}

echo "==> migrate + admit a tenant"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "mem5-demo-$$" --name "MEM-5 Demo Tenant")
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
psql_t "select pgmq.purge_queue('observe')" >/dev/null

./target/debug/synveda-gateway &
GATEWAY_PID=$!
trap 'kill "$GATEWAY_PID" 2>/dev/null || true' EXIT INT TERM

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS http://127.0.0.1:8139/healthz >/dev/null 2>&1; do
  tries=$((tries + 1))
  if [ "$tries" -ge 30 ]; then
    echo "demo FAILED: gateway did not become healthy" >&2
    exit 1
  fi
  sleep 1
done

api() {
  tok=$1
  method=$2
  path=$3
  body=${4:-}
  if [ -n "$body" ]; then
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
      -H "Content-Type: application/json" -d "$body" \
      "http://127.0.0.1:8139$path"
  else
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
      "http://127.0.0.1:8139$path"
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

# wait_for_records <n> — polls for the worker's async commits.
wait_for_records() {
  want=$1
  tries=0
  while :; do
    have=$(psql_t "select count(*) from records where tenant_id = '$tenant_id'")
    [ "$have" = "$want" ] && return 0
    tries=$((tries + 1))
    if [ "$tries" -ge 60 ]; then
      echo "demo FAILED: expected $want records, stuck at $have after $tries tries" >&2
      exit 1
    fi
    sleep 0.5
  done
}

# observe <token> <session> <occurred_at> <text>
observe() {
  body="{\"session_id\":\"$2\",\"events\":[{\"idempotency_key\":\"$2-1\",
    \"kind\":\"decision\",\"payload\":{\"text\":\"$4\"},\"occurred_at\":\"$3\"}]}"
  accepted=$(api "$1" POST /v1/observe "$body" | field accepted)
  [ "$accepted" = "1" ] || {
    echo "demo FAILED: observe was not accepted ($accepted)" >&2
    exit 1
  }
}

# block <token> <session> — the composed text a session start receives.
block() {
  api "$1" POST /v1/inject "{\"session_id\":\"$2\"}" | field text
}

echo "==> the admin builds the hierarchy; alice is registered at the team"
org_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
team_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"team\",\"slug\":\"payments\",\"name\":\"Payments\"}" |
  field id)
./target/debug/synveda service register --tenant "$tenant_id" \
  --subject alice --scope "$team_id" >/dev/null
alice_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject alice)
echo "    org=$org_id team=$team_id alice=alice"

monday=$(date -u -d '2 days ago' +%Y-%m-%dT%H:%M:%SZ 2>/dev/null ||
  date -u -v-2d +%Y-%m-%dT%H:%M:%SZ)
monday_noon=$(date -u -d '2 days ago +1 hour' +%Y-%m-%dT%H:%M:%SZ 2>/dev/null ||
  date -u -v-2d -v+1H +%Y-%m-%dT%H:%M:%SZ)
tuesday=$(date -u -d '1 day ago' +%Y-%m-%dT%H:%M:%SZ 2>/dev/null ||
  date -u -v-1d +%Y-%m-%dT%H:%M:%SZ)

BEFORE="We decided the payment reconciliation job runs against the ledger-archive replica."
AFTER="We decided the payment reconciliation job runs against the ledger-live replica."

echo
echo "==> [1/6] Monday: alice states a fact. A cold session start receives it."
observe "$alice_token" mem5-monday "$monday" "$BEFORE"
wait_for_records 1
block "$alice_token" cold-1 | grep -q "ledger-archive" || {
  echo "demo FAILED: the fact never reached the block" >&2
  exit 1
}
echo "    the block carries: ledger-archive"

echo
echo "==> [2/6] alice says the same thing again, in another session."
echo "    An ADD-only store makes a second record. This one merges."
observe "$alice_token" mem5-again "$monday_noon" "$BEFORE"
sleep 2
records=$(psql_t "select count(*) from records where tenant_id = '$tenant_id'")
merged=$(psql_t "select coalesce(provenance -> 'merged' ->> 'count', '0')
                 from records where tenant_id = '$tenant_id'")
[ "$records" = "1" ] && [ "$merged" = "1" ] || {
  echo "demo FAILED: expected 1 record with merged.count=1, got $records/$merged" >&2
  exit 1
}
echo "    records=1  provenance.merged.count=1"
echo "    (content, vector and content address untouched — a merge cannot"
echo "     demote published material to unreviewed)"

echo
echo "==> [3/6] Tuesday: the fact CHANGED. alice states its replacement."
observe "$alice_token" mem5-tuesday "$tuesday" "$AFTER"
wait_for_records 2
psql_t "select 'valid_from=' || valid_from || '  valid_to=' ||
        coalesce(valid_to::text, '(open)') || '  ' ||
        substring(content from 'ledger-[a-z]+')
        from records where tenant_id = '$tenant_id' order by valid_from"
edge=$(psql_t "select method || '/' || reason || '  jaccard=' ||
               coalesce(jaccard_permille::text, 'n/a') || '‰'
               from record_supersessions where tenant_id = '$tenant_id'")
[ -n "$edge" ] || {
  echo "demo FAILED: no supersession edge was written" >&2
  exit 1
}
echo "    supersession edge: $edge"

echo
echo "==> [4/6] AC part one: the superseded fact is EXCLUDED from current inject."
current=$(block "$alice_token" cold-2)
echo "$current" | grep -q "ledger-live" || {
  echo "demo FAILED: the replacement did not compose" >&2
  exit 1
}
if echo "$current" | grep -q "ledger-archive"; then
  echo "demo FAILED: the superseded fact is still being injected!" >&2
  exit 1
fi
echo "    the block carries ledger-live, and not ledger-archive"

echo "==> [4/6] AC part two: it is RETRIEVABLE AS-OF when it held."
# The composition engine's own valid-time predicate, asked at Monday
# instead of now — one query, two instants.
as_of=$(psql_t "select content from records
                where tenant_id = '$tenant_id'
                  and valid_from <= '$monday_noon'
                  and (valid_to is null or valid_to > '$monday_noon')")
echo "$as_of" | grep -q "ledger-archive" || {
  echo "demo FAILED: the superseded fact is not retrievable as-of" >&2
  exit 1
}
echo "    as of $monday_noon the answer is still:"
echo "    $as_of"
versions=$(psql_t "select count(*) from records_versions
                   where tenant_id = '$tenant_id'")
echo "    every version the database ever held is still addressable: $versions"

echo
echo "==> [5/6] the trail: one memory.superseded event, and the chain verifies"
./target/debug/synveda audit tail --tenant "$tenant_id" --limit 4
psql_t "select payload -> 'superseded' -> 0
        from audit_log where tenant_id = '$tenant_id'
          and action = 'memory.superseded'"
./target/debug/synveda audit verify --tenant "$tenant_id"

echo
echo "==> [6/6] the governance boundary: reviewed material is not the"
echo "    pipeline's to close. A curator publishes the current fact..."
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject alice --scope "$team_id" --role curator >/dev/null
alice_scope=$(psql_t "select scope_id from identities
                      where tenant_id = '$tenant_id' and subject = 'alice'")
live_id=$(psql_t "select id from records
                  where tenant_id = '$tenant_id' and valid_to is null")
api "$alice_token" POST "/v1/channels/$alice_scope/publish" \
  "{\"record_ids\":[\"$live_id\"],\"message\":\"reviewed\"}" | field members
echo "    ...and now alice contradicts it in a new session."
today=$(date -u +%Y-%m-%dT%H:%M:%SZ)
observe "$alice_token" mem5-contradiction "$today" \
  "We decided the payment reconciliation job runs against the ledger-standby replica."
wait_for_records 3
still_open=$(psql_t "select valid_to is null from records where id = '$live_id'")
[ "$still_open" = "t" ] || {
  echo "demo FAILED: the pipeline closed a PUBLISHED record's window!" >&2
  exit 1
}
refused=$(psql_t "select jsonb_array_length(payload -> 'refused_published')
                  from audit_log where tenant_id = '$tenant_id'
                    and action = 'memory.superseded'
                  order by seq desc limit 1")
[ "$refused" = "1" ] || {
  echo "demo FAILED: the refusal was not recorded (got '$refused')" >&2
  exit 1
}
echo "    the published window is untouched, and the refusal is chained"
echo "    (refused_published=1) — that is the signal a proposal is owed."
./target/debug/synveda audit verify --tenant "$tenant_id"

echo
echo "==> the AC suites"
cargo test -p synveda-gateway --test dedup
cargo test -p synveda-store --test rls
cargo test -p synveda-store --lib dedup
cargo test -p synveda-ingest --lib dedup
cargo test -p synveda-eval

echo
echo "MEM-5 demo OK: a restatement merged, a changed fact superseded the one"
echo "it replaced, the superseded fact left current inject and stayed"
echo "readable as-of, the chain verified, and reviewed material was refused."
