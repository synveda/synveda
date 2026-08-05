#!/usr/bin/env sh
# GRPH-2 acceptance demo: the graph-linking stage (ADR-0044).
# AC (docs/backlog/GRPH-2.md): entity dedup precision on fixture set;
# orphan rate tracked.
#
# The feature is "ingestion links records→entities→episodes; entity
# resolution against existing nodes", and the thing worth watching is that
# the resolution happens *against rows that already exist*. So the demo
# runs two sessions a month apart, drains between them, and shows that the
# second one found the first one's vertex rather than making its own.
#
# Flow: postgres up -> tenant, hierarchy, alice -> [1] June: a decision
# names ACME Corp, and the graph gains one name, one mention and one
# session -> [2] July: a different session spells the company differently,
# and the name vertex count STAYS ONE -> [3] the two records are two hops
# apart through the name they share, in the same adjacency SQL `expand`
# emits -> [4] a record that names nothing is an orphan, and the rate is on
# /metrics where a dashboard reads it -> [5] a fact is replaced, and the
# provenance graph answers it WITHOUT a row in graph_edges: one system of
# record per claim -> [6] the fixture-set precision measurement, printed
# with its recall and its one deliberate failure.
#
# On Windows, run via Git Bash. Needs postgres and node; no TEI and no IdP
# (network-free deterministic extractor and embedder throughout).
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
SYNVEDA_LISTEN_ADDR=127.0.0.1:8142
export SYNVEDA_LISTEN_ADDR
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=grph-2-demo-secret
export SYNVEDA_DEV_JWT_SECRET
SYNVEDA_EXTRACTOR=deterministic
export SYNVEDA_EXTRACTOR
SYNVEDA_EMBEDDER=deterministic
export SYNVEDA_EMBEDDER
SYNVEDA_EXTRACTION_POLL_MS=300
export SYNVEDA_EXTRACTION_POLL_MS
# The gateway shares stdout with the demo's narration; keep its background
# loops quiet unless the caller asks otherwise (the authz-4 convention).
RUST_LOG=${RUST_LOG:-error}
export RUST_LOG

cargo build -p synveda-gateway -p synveda-cli

psql_t() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda -tAc "$1"
}

psql_table() {
  docker compose -f deploy/compose/docker-compose.yml exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda -c "$1"
}

echo "==> migrate + admit a tenant"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "grph2-demo-$$" --name "GRPH-2 Demo Tenant")
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
until curl -fsS http://127.0.0.1:8142/healthz >/dev/null 2>&1; do
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
      "http://127.0.0.1:8142$path"
  else
    curl -fsS -X "$method" -H "Authorization: Bearer $tok" \
      "http://127.0.0.1:8142$path"
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

# observe <token> <session> <kind> <occurred_at> <text>
observe() {
  body="{\"session_id\":\"$2\",\"events\":[{\"idempotency_key\":\"$2-1\",
    \"kind\":\"$3\",\"payload\":{\"text\":\"$5\"},\"occurred_at\":\"$4\"}]}"
  accepted=$(api "$1" POST /v1/observe "$body" | field accepted)
  [ "$accepted" = "1" ] || {
    echo "demo FAILED: observe was not accepted ($accepted)" >&2
    exit 1
  }
}

# names — how many distinct entity names this tenant's graph holds.
names() {
  psql_t "select count(*) from graph_vertices
          where tenant_id = '$tenant_id' and graph = 'entity' and kind = 'name'"
}

echo "==> the admin builds the hierarchy; alice is registered at the team"
org_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  '{"kind":"org","slug":"acme","name":"ACME"}' | field id)
team_id=$(api "$admin_token" POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"team\",\"slug\":\"platform\",\"name\":\"Platform\"}" |
  field id)
./target/debug/synveda service register --tenant "$tenant_id" \
  --subject alice --scope "$team_id" >/dev/null
alice_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject alice)
echo "    org=$org_id team=$team_id alice=alice"

june=$(date -u -d '40 days ago' +%Y-%m-%dT%H:%M:%SZ 2>/dev/null ||
  date -u -v-40d +%Y-%m-%dT%H:%M:%SZ)
july=$(date -u -d '2 days ago' +%Y-%m-%dT%H:%M:%SZ 2>/dev/null ||
  date -u -v-2d +%Y-%m-%dT%H:%M:%SZ)
today=$(date -u +%Y-%m-%dT%H:%M:%SZ)

echo
echo "==> [1/6] June: alice makes a decision that names a company."
observe "$alice_token" grph2-june decision "$june" \
  "We decided ACME Corp will host the ledger service."
wait_for_records 1
sleep 1
psql_table "select graph, kind, key, label,
                   case when record_id is null then '' else 'backs a record' end as backing
            from graph_vertices where tenant_id = '$tenant_id'
            order by graph, kind, key"
[ "$(names)" = "1" ] || {
  echo "demo FAILED: expected one name vertex, got $(names)" >&2
  exit 1
}
echo "    one name ('acme'), one record vertex per graph, one session."
echo "    Note the record vertex's label: its id, never its content — "
echo "    graph_vertices carries no scope, so a label is readable"
echo "    tenant-wide (ADR-0044 decision 8)."

echo
echo "==> [2/6] July: a different session spells the company differently."
echo "    'Acme Corporation' — new session, new record, SAME entity."
observe "$alice_token" grph2-july transcript_delta "$july" \
  "Acme Corporation renewed the platform contract."
wait_for_records 2
sleep 1
psql_table "select e.graph, e.kind, e.method, e.confidence_permille as conf_permille,
                   coalesce(e.valid_to::text, '(open)') as valid_to,
                   src.key as src, dst.key as dst
            from graph_edges e
            join graph_vertices src on src.id = e.src_id
            join graph_vertices dst on dst.id = e.dst_id
            where e.tenant_id = '$tenant_id'
            order by e.graph, e.kind, dst.key"
after=$(names)
[ "$after" = "1" ] || {
  echo "demo FAILED: two spellings made $after name vertices; the resolver did not converge" >&2
  exit 1
}
echo "    name vertices: $after — the July commit resolved against the vertex"
echo "    June created. That is the AC's 'against existing nodes', and it is"
echo "    the schema's unique constraint doing it, not a lookup."
echo "    Both mentions sit at 900‰: reaching 'acme' meant dropping a word"
echo "    ('Corp', 'Corporation'), and ADR-0044 decision 4 records that"
echo "    rather than hiding it behind a threshold."

echo
echo "==> [3/6] the two records are two hops apart, through the name they share."
echo "    This is the adjacency join graph::expand emits, run here as SQL so"
echo "    the demo shows the shape rather than asserting it."
psql_table "with seed as (
              select v.id from graph_vertices v
              join records r on r.id = v.record_id
              where v.tenant_id = '$tenant_id' and v.graph = 'entity'
                and v.kind = 'record' and r.content like '%ledger service%'
            ),
            hop1 as (
              select e.dst_id as vid from graph_edges e join seed s on e.src_id = s.id
              where e.tenant_id = '$tenant_id' and e.graph = 'entity'
              union all
              select e.src_id from graph_edges e join seed s on e.dst_id = s.id
              where e.tenant_id = '$tenant_id' and e.graph = 'entity'
            ),
            hop2 as (
              select e.src_id as vid from graph_edges e join hop1 h on e.dst_id = h.vid
              where e.tenant_id = '$tenant_id' and e.graph = 'entity'
            )
            select 1 as hop, key, kind from graph_vertices
              where id in (select vid from hop1) and tenant_id = '$tenant_id'
            union all
            select 2, key, kind from graph_vertices
              where id in (select vid from hop2) and tenant_id = '$tenant_id'
                and id not in (select id from seed)
            order by hop, key"
echo "    hop 1 is the name; hop 2 is the OTHER record — the seed path"
echo "    GRPH-3 will walk to widen a recall candidate set."

echo
echo "==> [4/6] a record that names nothing is an orphan, and it is counted."
observe "$alice_token" grph2-orphan transcript_delta "$today" \
  "the nightly reconciliation finished without errors"
wait_for_records 3
sleep 1
orphans=$(psql_t "select count(*) from records r
                  where r.tenant_id = '$tenant_id'
                    and not exists (select 1 from graph_edges e
                                    where e.tenant_id = r.tenant_id
                                      and e.graph = 'entity'
                                      and e.source_record_id = r.id)")
echo "    records with no entity edge: $orphans of 3"
[ "$orphans" = "1" ] || {
  echo "demo FAILED: expected exactly one orphan, got $orphans" >&2
  exit 1
}
echo "    An orphan is a normal outcome — that sentence names nothing — and"
echo "    the rate is the evidence for whether mention recall is worth"
echo "    improving. On /metrics, where a dashboard reads it:"
curl -fsS http://127.0.0.1:8142/metrics | grep -E '^synveda_graph_link_' | head -8

echo
echo "==> [5/6] the provenance graph is ANSWERED, not materialised."
echo "    alice states what replaced the June decision; MEM-5 closes the old"
echo "    window and records why."
observe "$alice_token" grph2-replace decision "$today" \
  "We decided the ledger service moves off ACME Corp to the internal cluster."
wait_for_records 4
sleep 1
psql_table "select method, reason, jaccard_permille as jaccard_permille, closed_at
            from record_supersessions where tenant_id = '$tenant_id'"
mirrored=$(psql_t "select count(*) from graph_edges
                   where tenant_id = '$tenant_id' and kind = 'supersedes'")
[ "$mirrored" = "0" ] || {
  echo "demo FAILED: the supersession was mirrored into graph_edges ($mirrored rows)" >&2
  exit 1
}
echo "    graph_edges rows of kind 'supersedes': $mirrored."
echo "    record_supersessions stays the system of record; the graph reaches"
echo "    it through graph::supersession_edges, which projects those rows in"
echo "    the edge model. A mirror would be two systems of record for one"
echo "    claim, and the failure mode is discovering years later that it"
echo "    drifted (ADR-0044 decision 14, discharging ADR-0039's trigger (d))."

echo
echo "==> [6/6] the AC's precision measurement, on the labelled fixture set."
cargo test -p synveda-ingest --test entity_resolution -- --nocapture 2>&1 |
  grep -E 'entity resolution over|merged|precision|!!' || true
echo "    The set carries its own failures: 'Paris' is two different things"
echo "    with one name, which no surface-form resolver can split, and"
echo "    'PostgreSQL'/'International Business Machines'/'Jorg Muller' are"
echo "    equivalences it refuses to guess at. Precision is asserted;"
echo "    recall is reported. EVAL-2 owns the real targets."

echo
echo "==> the end-to-end acceptance suite over the real product path"
DATABASE_URL="$DATABASE_URL" cargo test -p synveda-gateway --test graph_linking -- \
  --test-threads=1 2>&1 | tail -8

echo
echo "GRPH-2 demo complete."
echo "  - two spellings, two sessions, one vertex: entity resolution against"
echo "    existing nodes, enforced by (tenant, graph, kind, key)"
echo "  - records→entities→episodes: mentions in the entity graph,"
echo "    occurred_during in the episode graph, both committed in the same"
echo "    transaction as the records they describe"
echo "  - orphan rate tracked per graph on synveda_graph_link_records_total"
echo "  - the provenance graph projected, never mirrored"
