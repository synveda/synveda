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

# Refuses to start a gateway on a port something else is already serving.
#
# Without this, a leftover gateway from an aborted run is *healthy* on the
# port the new one wants, `eval_wait_gateway` succeeds against it, and every
# request then goes to a process pointed at a scratch database that no
# longer exists — which arrives as a 401 on the first hierarchy call and
# reads like a broken token. It cost two demo runs to diagnose, so the
# collision names itself now (EVAL-5).
eval_port_free() {
  if curl -fsS "$1/healthz" >/dev/null 2>&1; then
    echo "eval: something is already serving $1 — most likely a gateway left" >&2
    echo "      behind by an aborted run. Clear it with:" >&2
    echo "        pkill -f target/debug/synveda-gateway" >&2
    return 1
  fi
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
  # model drifted (ADR-0028 decision 6). Deliberately not exported here —
  # `SYNVEDA_EXTRACTOR` and its credentials pass through from the caller,
  # which is what `make eval-extraction-live` uses to run the same corpus
  # through a real model against its own baseline (ADR-0046 decision 12).
  SYNVEDA_EXTRACTION_POLL_MS=300
  export SYNVEDA_EXTRACTION_POLL_MS
  SYNVEDA_SEARCH_POLL_MS=300
  export SYNVEDA_SEARCH_POLL_MS
  RUST_LOG=${RUST_LOG:-warn}
  export RUST_LOG

  # Offline for the build, deliberately: DATABASE_URL now names the empty
  # scratch database, and sqlx's compile-time checks would validate every
  # query against a schema that does not exist yet — which passes only
  # while the build cache happens to be warm, and fails outright the first
  # time anything in the workspace changes. The committed `.sqlx` data is
  # what CI compiles against for the same reason.
  SQLX_OFFLINE=true cargo build -p synveda-gateway -p synveda-cli -p synveda-eval
  ./target/debug/synveda db migrate
  EVAL_TENANT=$(./target/debug/synveda tenant create \
    --slug "eval-$$" --name "EVAL-1 harness" | eval_json_field id)

  # Phase 1: the hierarchy, through the governed admin API.
  SYNVEDA_LISTEN_ADDR=${EVAL_SEED_URL#http://}
  export SYNVEDA_LISTEN_ADDR
  eval_port_free "$EVAL_SEED_URL"
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
  # EVAL-5's own department, so the security corpus and the Q&A corpus do
  # not share a subtree: a sibling team and a second department are what
  # make a *scope* boundary distinguishable from a tier one, and a reader
  # whose sweep also enumerates another suite's promoted material is a
  # reader whose sweep is closer to the 32-record cap for no reason.
  sec=$(eval_node "{\"parent_id\":\"$org\",\"kind\":\"department\",\"slug\":\"sec\",\"name\":\"Treasury\"}")
  vault=$(eval_node "{\"parent_id\":\"$sec\",\"kind\":\"team\",\"slug\":\"vault\",\"name\":\"Vault\"}")
  desk=$(eval_node "{\"parent_id\":\"$sec\",\"kind\":\"team\",\"slug\":\"desk\",\"name\":\"Settlement desk\"}")
  EVAL_ORG=$org

  # A SECOND ADMITTED TENANT (EVAL-5, ADR-0048 decision 8). The first time
  # this harness has run more than one, and the point of the cross-tenant
  # half: the runner never sends a tenant — the token carries one — so a
  # probe from here to there is the real thing rather than a filter test.
  # Its estate is deliberately minimal; what is under measurement is the
  # boundary, not the shape on the far side of it.
  EVAL_TENANT_B=$(./target/debug/synveda tenant create \
    --slug "eval-b-$$" --name "EVAL-5 foreign tenant" | eval_json_field id)
  admin_b=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT_B" --subject eval-admin-b)
  ./target/debug/synveda role bind --tenant "$EVAL_TENANT_B" \
    --subject eval-admin-b --role org-admin >/dev/null
  eval_node_b() {
    curl -fsS -X POST "$EVAL_SEED_URL/v1/hierarchy/nodes" \
      -H "Authorization: Bearer $admin_b" -H 'Content-Type: application/json' \
      -d "$1" | eval_json_field id
  }
  org_b=$(eval_node_b '{"parent_id":null,"kind":"org","slug":"northwind","name":"Northwind"}')
  clearing=$(eval_node_b "{\"parent_id\":\"$org_b\",\"kind\":\"team\",\"slug\":\"clearing\",\"name\":\"Clearing\"}")

  kill "$EVAL_SEED_PID" 2>/dev/null || true
  wait "$EVAL_SEED_PID" 2>/dev/null || true
  EVAL_SEED_PID=""

  # The actors. Registration writes hierarchy the gateway caches
  # out-of-process, so it happens between the two gateways rather than
  # under the one that will serve the run.
  #
  # One actor per extraction fixture group (EVAL-2, ADR-0046 decision 2).
  # The partition is load-bearing rather than tidy: observe writes land at
  # the caller's home scope, so a group's corpus is its own, and a recall
  # sweep is capped at 32 records — which is why the corpus grows by
  # adding actors here and never by adding fixtures past that arithmetic.
  # EVAL-4's actors are anchored where they must *propose*, not where their
  # material ends up (ADR-0047 decision 3). A climb names a target scope,
  # and the base-layer confinement forbids a service identity every
  # resource outside its anchor subtree (AUTH-3, ADR-0018 decision 4) — so
  # the author of team material is anchored at the team, the author of
  # department material at the department, and the reviewers at the org,
  # from which roles inherit downward to every level they review.
  # EVAL-5's readers are placed where the boundary is (ADR-0048): the
  # owner and a teammate at one team, a sibling team's member at another,
  # and the compliance approver at the org, from which roles inherit down
  # to the personal leaf a classification proposal targets.
  for actor in curator:$platform newcomer:$platform outsider:$payments \
    extract-alpha:$platform extract-beta:$platform extract-gamma:$platform \
    extract-delta:$platform extract-epsilon:$platform \
    qa-reader:$payments qa-team:$payments qa-dept:$eng qa-org:$org \
    qa-curator:$org qa-steward:$org \
    sec-owner:$vault sec-mate:$vault sec-neighbour:$desk sec-compliance:$org; do
    ./target/debug/synveda service register --tenant "$EVAL_TENANT" \
      --subject "${actor%%:*}" --scope "${actor##*:}" >/dev/null
  done
  ./target/debug/synveda service register --tenant "$EVAL_TENANT_B" \
    --subject xt-reader --scope "$clearing" >/dev/null

  # One actor per LongMemEval instance (EVAL-3, ADR-0061 decision 8). The
  # same rule EVAL-2 set and EVAL-4 restated — a corpus grows by adding
  # actors, never by adding records past the 32-record arithmetic — and
  # here it is load-bearing for a second reason: LongMemEval's instances
  # are independent by construction, so two of them sharing an identity
  # would put one instance's forty-session haystack inside the other's and
  # measure retrieval over a corpus twice the size the benchmark specifies.
  # Zero by default, because every other suite pays for identities it does
  # not use otherwise; `evals/run-longmemeval.sh` sets it to the instance
  # count.
  eval_lme=0
  while [ "$eval_lme" -lt "${EVAL_LONGMEMEVAL_ACTORS:-0}" ]; do
    ./target/debug/synveda service register --tenant "$EVAL_TENANT" \
      --subject "$(printf 'lme-%03d' "$eval_lme")" --scope "$payments" >/dev/null
    eval_lme=$((eval_lme + 1))
  done

  # The reviewers every Q&A promotion goes through. Which roles a
  # publication needs is the target scope's pack answer and not this
  # script's: under the zero-config `regulated-strict` a team publication
  # takes one curator and a department or org publication takes a curator
  # *and* a steward, two distinct people (the FLOW-3 matrix golden). The
  # runner approves until the surface says nothing is outstanding, so a
  # pack that asks for a different set is followed rather than fought.
  ./target/debug/synveda role bind --tenant "$EVAL_TENANT" \
    --subject qa-curator --role curator --scope "$org" >/dev/null
  ./target/debug/synveda role bind --tenant "$EVAL_TENANT" \
    --subject qa-steward --role steward --scope "$org" >/dev/null

  # The compliance approver EVAL-5's `restricted` classification needs
  # (ADR-0048 decision 7). The invariant approval floor asks for this role
  # plus two distinct approvers on anything at the top tier, under every
  # pack and unauthorable away (ADR-0032 decision 4) — so without this
  # binding the security corpus has no way to mint the tier its whole
  # sensitivity boundary is about. `curator` comes with it for the reason
  # the AUTHZ-5 leak suite gives: approving a restricted change means
  # reading it, and the review surface shows content.
  for role in compliance curator; do
    ./target/debug/synveda role bind --tenant "$EVAL_TENANT" \
      --subject sec-compliance --role "$role" --scope "$org" >/dev/null
  done

  # The auditor the extraction suite reads the chain as (ADR-0046
  # decision 4). Deliberately NOT a service identity: AUTH-3's confinement
  # forbid denies the tenant plane to those however they are bound, and
  # `AuditRead` declares `resource: [Tenant]` and admits nothing narrower
  # (ADR-0045 decision 2). It is also placed nowhere and registered as
  # nothing — an auditor is a member of nothing, and every byte it sees
  # comes from `AuditRead` rather than from the membership floor.
  ./target/debug/synveda role bind --tenant "$EVAL_TENANT" \
    --subject eval-auditor --role auditor >/dev/null
  # And one for the foreign tenant. Not a convenience: `AuditRead` declares
  # `resource: [Tenant]` and an audit answer covers one chain or is refused
  # (ADR-0045 decision 2), so the security suite's wait — "every seeded
  # event appears in a memory.extracted payload" — has to ask each record's
  # OWN chain. The first cross-tenant run reported the pipeline unfinished
  # for a record that had extracted perfectly well, because it asked the
  # wrong one (EVAL-5, ADR-0048).
  ./target/debug/synveda role bind --tenant "$EVAL_TENANT_B" \
    --subject eval-auditor-b --role auditor >/dev/null

  # Phase 2: the gateway under measurement.
  SYNVEDA_LISTEN_ADDR=${EVAL_GATEWAY_URL#http://}
  export SYNVEDA_LISTEN_ADDR
  # The gateway's pool is shared between request handlers and the
  # background workers, and its default of eight wedged this stack on
  # EVAL-3's LongMemEval run: ~4,900 events of sustained ingestion, the
  # extraction worker and index sweeper holding every connection, and
  # seventeen minutes of 503 on every `/v1` surface with no recovery.
  # Raised here rather than in the product, because the product's default
  # is a deployment decision and this is a laptop seeding a benchmark.
  # Postgres admits 100; two gateways at 32 leaves room.
  SYNVEDA_DB_MAX_CONNECTIONS=${SYNVEDA_DB_MAX_CONNECTIONS:-32}
  export SYNVEDA_DB_MAX_CONNECTIONS
  eval_port_free "$EVAL_GATEWAY_URL"
  ./target/debug/synveda-gateway >"$EVAL_STATE/gateway.log" 2>&1 &
  EVAL_PID=$!
  eval_wait_gateway "$EVAL_GATEWAY_URL"

  # The default hour, deliberately: AUTH-3 caps a service identity's token
  # at 3600 seconds (ADR-0018) and the gateway refuses anything longer, so
  # asking for more here would fail every call with a 401.
  curator=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT" --subject curator)
  newcomer=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT" --subject newcomer)
  outsider=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT" --subject outsider)
  eval_auditor=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT" --subject eval-auditor)
  eval_auditor_b=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT_B" --subject eval-auditor-b)
  for group in alpha beta gamma delta epsilon; do
    eval "extract_$group=\$(./target/debug/synveda token issue \
      --tenant \"\$EVAL_TENANT\" --subject \"extract-$group\")"
  done
  for who in reader team dept org curator steward; do
    eval "qa_$who=\$(./target/debug/synveda token issue \
      --tenant \"\$EVAL_TENANT\" --subject \"qa-$who\")"
  done
  for who in owner mate neighbour compliance; do
    eval "sec_$who=\$(./target/debug/synveda token issue \
      --tenant \"\$EVAL_TENANT\" --subject \"sec-$who\")"
  done
  # The one bearer in this file that carries a different tenant.
  xt_reader=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT_B" --subject xt-reader)
  # The LongMemEval pool, as a JSON fragment rather than as fixed lines:
  # how many exist is a run's decision, and the harness discovers them by
  # the `lme-` prefix rather than by a count written in two places.
  eval_lme_actors=""
  eval_lme=0
  while [ "$eval_lme" -lt "${EVAL_LONGMEMEVAL_ACTORS:-0}" ]; do
    eval_lme_subject=$(printf 'lme-%03d' "$eval_lme")
    eval_lme_token=$(./target/debug/synveda token issue \
      --tenant "$EVAL_TENANT" --subject "$eval_lme_subject")
    eval_lme_actors="$eval_lme_actors,
    \"$eval_lme_subject\": { \"token\": \"$eval_lme_token\", \"scope\": \"acme/eng/payments\" }"
    eval_lme=$((eval_lme + 1))
  done
  # The auditor carries no scope, because it sits at none. `scopes` is the
  # one thing a Q&A corpus has to say in UUIDs — where a promotion lands
  # (EVAL-4, ADR-0047 decision 3) — so a fixture names `payments` and this
  # script says what that is.
  cat >"$EVAL_ENV" <<EOF
{
  "gateway_url": "$EVAL_GATEWAY_URL",
  "tenant_id": "$EVAL_TENANT",
  "scopes": {
    "acme": "$org",
    "eng": "$eng",
    "platform": "$platform",
    "payments": "$payments",
    "sec": "$sec",
    "vault": "$vault",
    "desk": "$desk"
  },
  "actors": {
    "curator":  { "token": "$curator",  "scope": "acme/eng/platform" },
    "newcomer": { "token": "$newcomer", "scope": "acme/eng/platform" },
    "outsider": { "token": "$outsider", "scope": "acme/eng/payments" },
    "auditor":  { "token": "$eval_auditor" },
    "auditor-northwind": { "token": "$eval_auditor_b", "tenant": "$EVAL_TENANT_B" },
    "extract-alpha":   { "token": "$extract_alpha",   "scope": "acme/eng/platform" },
    "extract-beta":    { "token": "$extract_beta",    "scope": "acme/eng/platform" },
    "extract-gamma":   { "token": "$extract_gamma",   "scope": "acme/eng/platform" },
    "extract-delta":   { "token": "$extract_delta",   "scope": "acme/eng/platform" },
    "extract-epsilon": { "token": "$extract_epsilon", "scope": "acme/eng/platform" },
    "qa-reader":  { "token": "$qa_reader",  "scope": "acme/eng/payments" },
    "qa-team":    { "token": "$qa_team",    "scope": "acme/eng/payments" },
    "qa-dept":    { "token": "$qa_dept",    "scope": "acme/eng" },
    "qa-org":     { "token": "$qa_org",     "scope": "acme" },
    "qa-curator": { "token": "$qa_curator", "scope": "acme" },
    "qa-steward": { "token": "$qa_steward", "scope": "acme" },
    "sec-owner":      { "token": "$sec_owner",      "scope": "acme/sec/vault" },
    "sec-mate":       { "token": "$sec_mate",       "scope": "acme/sec/vault" },
    "sec-neighbour":  { "token": "$sec_neighbour",  "scope": "acme/sec/desk" },
    "sec-compliance": { "token": "$sec_compliance", "scope": "acme" },
    "xt-reader": {
      "token": "$xt_reader",
      "scope": "northwind/clearing",
      "tenant": "$EVAL_TENANT_B"
    }$eval_lme_actors
  }
}
EOF
}

# eval_longmemeval [extra args…] — the deterministic retrieval tier
# (EVAL-3, ADR-0061 decision 5) against the stack eval_up made. Its own
# baseline and its own report, because it grades a different predicate
# from the four suites `eval_run` carries: those ask what a block did with
# a corpus this repository wrote, and this one asks whether a block bound
# the evidence sessions somebody else's benchmark names.
eval_longmemeval() {
  ./target/debug/synveda-eval longmemeval \
    --env "$EVAL_ENV" \
    ${EVAL_LONGMEMEVAL_CORPUS:+--corpus "$EVAL_LONGMEMEVAL_CORPUS"} \
    --instances "${EVAL_LONGMEMEVAL_INSTANCES:-10}" \
    --seed-timeout-secs "${EVAL_LONGMEMEVAL_SEED_TIMEOUT:-1800}" \
    ${EVAL_LONGMEMEVAL_JUDGED:+--judged} \
    ${EVAL_BASELINE:+--baseline "$EVAL_BASELINE"} \
    --report "${EVAL_REPORT:-$EVAL_STATE/longmemeval.json}" \
    "$@"
}

# eval_run [extra args…] — the harness against the stack eval_up made.
# $EVAL_REPORT names where the JSON report lands; the default goes with
# the run's scratch state, and CI points it somewhere it can keep.
# $EVAL_BASELINE picks the gate: the default is the deterministic one, and
# `make eval-extraction-live` points it at evals/baseline-live.json,
# because a live model's numbers and a ruleset's are not comparable
# (EVAL-2, ADR-0046 decision 12).
eval_run() {
  ./target/debug/synveda-eval run \
    --env "$EVAL_ENV" \
    --suite evals/scenarios \
    --fixtures evals/fixtures/extraction \
    --qa evals/fixtures/qa \
    --security evals/fixtures/security \
    ${EVAL_SECURITY_VARIANTS:+--security-variants "$EVAL_SECURITY_VARIANTS"} \
    --baseline "${EVAL_BASELINE:-evals/baseline.json}" \
    --report "${EVAL_REPORT:-$EVAL_STATE/report.json}" \
    ${EVAL_DENSE_RETRIEVAL:+--dense-retrieval} \
    "$@"
}

eval_down() {
  [ -n "${EVAL_PID:-}" ] && kill "$EVAL_PID" 2>/dev/null
  [ -n "${EVAL_SEED_PID:-}" ] && kill "$EVAL_SEED_PID" 2>/dev/null
  wait 2>/dev/null || true
  # One scratch database holds both admitted tenants, so dropping it
  # disposes of the foreign one too (EVAL-5, ADR-0048's compliance note).
  if [ -n "${EVAL_DB:-}" ]; then
    $COMPOSE exec -T postgres psql -U synveda -d synveda \
      -c "drop database if exists $EVAL_DB with (force)" >/dev/null 2>&1 || true
  fi
  [ -n "${EVAL_INDEX_DIR:-}" ] && rm -rf "$EVAL_INDEX_DIR"
  # The gateway's log is the only place a run that committed nothing says
  # why, and until now it lived in scratch state this function deletes —
  # so the nightly of 2026-08-01 reported zero records across every corpus
  # and threw away the one file that knew the reason. When the report has a
  # home outside the scratch state, which is what CI gives it, the logs go
  # with it: the run that needs them is by definition one that already
  # failed.
  if [ -n "${EVAL_REPORT:-}" ] && [ -n "${EVAL_STATE:-}" ]; then
    eval_report_dir=$(dirname "$EVAL_REPORT")
    [ -d "$eval_report_dir" ] &&
      cp "$EVAL_STATE"/*.log "$eval_report_dir" 2>/dev/null
  fi
  if [ -n "${EVAL_KEEP_STATE:-}" ]; then
    echo "eval: state kept at ${EVAL_STATE:-}" >&2
  else
    [ -n "${EVAL_STATE:-}" ] && rm -rf "$EVAL_STATE"
  fi
  return 0
}
