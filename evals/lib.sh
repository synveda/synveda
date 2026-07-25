# The privileged half of an eval run (EVAL-1, ADR-0028 decision 7).
#
# Admitting a tenant, building a hierarchy, and registering identities are
# things only an operator can do, so they live here in the same shell
# idiom every demo uses — and the runner stays a client that knows how to
# call two endpoints. Sourced by evals/run.sh and demos/eval-1-harness.sh.
#
# eval_up    brings up a stack on a scratch database and writes $EVAL_ENV
# eval_run   runs the harness against it
# eval_down  puts everything back
#
# Callers set -eu and trap eval_down.

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
EVAL_GATEWAY_URL=${EVAL_GATEWAY_URL:-http://127.0.0.1:8150}
EVAL_SEED_URL=${EVAL_SEED_URL:-http://127.0.0.1:8151}
# Dev-mode bearers (ADR-0008), so a run needs Postgres and the gateway and
# nothing else — no IdP, no model server. A harness that needs an IdP is a
# harness that runs monthly (ADR-0028 decision 6).
EVAL_JWT_SECRET=${EVAL_JWT_SECRET:-eval-1-harness-secret}

eval_json_field() {
  node -e '
    let d = "";
    process.stdin.on("data", (c) => (d += c));
    process.stdin.on("end", () => {
      let v = JSON.parse(d);
      for (const k of process.argv.slice(1)) v = v[k];
      if (v === undefined) process.exit(1);
      console.log(typeof v === "string" ? v : JSON.stringify(v));
    });
  ' "$@"
}

eval_wait_gateway() {
  tries=0
  until curl -fsS "$1/healthz" >/dev/null 2>&1; do
    tries=$((tries + 1))
    if [ "$tries" -ge 30 ]; then
      echo "eval: gateway did not become healthy on $1" >&2
      return 1
    fi
    sleep 1
  done
}

eval_psql() {
  $COMPOSE exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d "$EVAL_DB" -tAc "$1"
}

eval_up() {
  $COMPOSE up --detach --wait postgres

  # A scratch database per run: two runs are only comparable if neither
  # inherits the other's records, and the sidecar indexer sweeps every
  # active tenant per cycle — on the shared dev database (thousands of
  # leftover test tenants) a just-admitted tenant waits minutes for its
  # first sweep, which the relevance scenario would pay for in full.
  EVAL_DB=eval_$$
  $COMPOSE exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda \
    -c "create database $EVAL_DB" >/dev/null
  $COMPOSE exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d "$EVAL_DB" -c \
    "create extension if not exists vector;
     create extension if not exists age;
     create extension if not exists pgmq" >/dev/null

  EVAL_STATE=$(mktemp -d "${TMPDIR:-/tmp}/synveda-eval-XXXXXX")
  EVAL_ENV="$EVAL_STATE/env.json"
  EVAL_INDEX_DIR="./data/eval-search-$$"

  DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$EVAL_DB"
  export DATABASE_URL
  SYNVEDA_DEV_JWT_SECRET="$EVAL_JWT_SECRET"
  export SYNVEDA_DEV_JWT_SECRET
  SYNVEDA_SEARCH_INDEX_DIR="$EVAL_INDEX_DIR"
  export SYNVEDA_SEARCH_INDEX_DIR
  # The deterministic extractor and embedder are the default and stay it:
  # a nightly failure should mean someone changed the code, not that a
  # model drifted (ADR-0028 decision 6).
  SYNVEDA_EXTRACTION_POLL_MS=300
  export SYNVEDA_EXTRACTION_POLL_MS
  SYNVEDA_SEARCH_POLL_MS=300
  export SYNVEDA_SEARCH_POLL_MS
  RUST_LOG=${RUST_LOG:-warn}
  export RUST_LOG

  cargo build -p synveda-gateway -p synveda-cli -p synveda-eval
  ./target/debug/synveda db migrate
  EVAL_TENANT=$(./target/debug/synveda tenant create \
    --slug "eval-$$" --name "EVAL-1 harness" | eval_json_field id)

  # Phase 1: the hierarchy, through the governed admin API.
  SYNVEDA_LISTEN_ADDR=${EVAL_SEED_URL#http://}
  export SYNVEDA_LISTEN_ADDR
  ./target/debug/synveda-gateway >"$EVAL_STATE/seed-gateway.log" 2>&1 &
  EVAL_SEED_PID=$!
  eval_wait_gateway "$EVAL_SEED_URL"
  admin=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT" --subject eval-admin)
  ./target/debug/synveda role bind --tenant "$EVAL_TENANT" \
    --subject eval-admin --role org-admin >/dev/null
  eval_node() {
    curl -fsS -X POST "$EVAL_SEED_URL/v1/hierarchy/nodes" \
      -H "Authorization: Bearer $admin" -H 'Content-Type: application/json' \
      -d "$1" | eval_json_field id
  }
  org=$(eval_node '{"parent_id":null,"kind":"org","slug":"acme","name":"ACME"}')
  eng=$(eval_node "{\"parent_id\":\"$org\",\"kind\":\"department\",\"slug\":\"eng\",\"name\":\"Engineering\"}")
  platform=$(eval_node "{\"parent_id\":\"$eng\",\"kind\":\"team\",\"slug\":\"platform\",\"name\":\"Platform\"}")
  payments=$(eval_node "{\"parent_id\":\"$eng\",\"kind\":\"team\",\"slug\":\"payments\",\"name\":\"Payments\"}")
  EVAL_ORG=$org
  kill "$EVAL_SEED_PID" 2>/dev/null || true
  wait "$EVAL_SEED_PID" 2>/dev/null || true
  EVAL_SEED_PID=""

  # The actors. Registration writes hierarchy the gateway caches
  # out-of-process, so it happens between the two gateways rather than
  # under the one that will serve the run.
  for actor in curator:$platform newcomer:$platform outsider:$payments; do
    ./target/debug/synveda service register --tenant "$EVAL_TENANT" \
      --subject "${actor%%:*}" --scope "${actor##*:}" >/dev/null
  done

  # Phase 2: the gateway under measurement.
  SYNVEDA_LISTEN_ADDR=${EVAL_GATEWAY_URL#http://}
  export SYNVEDA_LISTEN_ADDR
  ./target/debug/synveda-gateway >"$EVAL_STATE/gateway.log" 2>&1 &
  EVAL_PID=$!
  eval_wait_gateway "$EVAL_GATEWAY_URL"

  # The default hour, deliberately: AUTH-3 caps a service identity's token
  # at 3600 seconds (ADR-0018) and the gateway refuses anything longer, so
  # asking for more here would fail every call with a 401.
  curator=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT" --subject curator)
  newcomer=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT" --subject newcomer)
  outsider=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT" --subject outsider)
  cat >"$EVAL_ENV" <<EOF
{
  "gateway_url": "$EVAL_GATEWAY_URL",
  "tenant_id": "$EVAL_TENANT",
  "actors": {
    "curator":  { "token": "$curator",  "scope": "acme/eng/platform" },
    "newcomer": { "token": "$newcomer", "scope": "acme/eng/platform" },
    "outsider": { "token": "$outsider", "scope": "acme/eng/payments" }
  }
}
EOF
}

# eval_run [extra args…] — the harness against the stack eval_up made.
# $EVAL_REPORT names where the JSON report lands; the default goes with
# the run's scratch state, and CI points it somewhere it can keep.
eval_run() {
  ./target/debug/synveda-eval run \
    --env "$EVAL_ENV" \
    --suite evals/scenarios \
    --baseline evals/baseline.json \
    --report "${EVAL_REPORT:-$EVAL_STATE/report.json}" \
    "$@"
}

eval_down() {
  [ -n "${EVAL_PID:-}" ] && kill "$EVAL_PID" 2>/dev/null
  [ -n "${EVAL_SEED_PID:-}" ] && kill "$EVAL_SEED_PID" 2>/dev/null
  wait 2>/dev/null || true
  if [ -n "${EVAL_DB:-}" ]; then
    $COMPOSE exec -T postgres psql -U synveda -d synveda \
      -c "drop database if exists $EVAL_DB with (force)" >/dev/null 2>&1 || true
  fi
  [ -n "${EVAL_INDEX_DIR:-}" ] && rm -rf "$EVAL_INDEX_DIR"
  if [ -n "${EVAL_KEEP_STATE:-}" ]; then
    echo "eval: state kept at ${EVAL_STATE:-}" >&2
  else
    [ -n "${EVAL_STATE:-}" ] && rm -rf "$EVAL_STATE"
  fi
  return 0
}
