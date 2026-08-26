# The privileged half of an eval run (EVAL-1, ADR-0028 decision 7).
#
# Admitting a tenant, establishing its root, and registering service identities
# are operator bootstrap actions, so they live here in the same shell
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

# The product default remains one hour. LongMemEval is the one deterministic
# run whose supported-API seed plus post-seed extraction wait can cross that
# boundary, so its disposable gateway and only its lme-* actors share an
# explicit longer bound. This does not bypass AUTH-3: the gateway still checks
# exp - iat against the configured ceiling on every governed call.
eval_service_token_ttl_for_run() {
  eval_token_actor_count=${EVAL_LONGMEMEVAL_ACTORS:-0}
  case "$eval_token_actor_count" in
    ''|*[!0-9]*)
      echo "eval: EVAL_LONGMEMEVAL_ACTORS must be a non-negative integer" >&2
      return 1
      ;;
  esac
  if [ "$eval_token_actor_count" -gt 0 ]; then
    eval_token_ttl=${EVAL_LONGMEMEVAL_TOKEN_TTL_SECS:-7200}
  else
    eval_token_ttl=3600
  fi
  case "$eval_token_ttl" in
    ''|*[!0-9]*|0)
      echo "eval: LongMemEval token TTL must be a positive integer" >&2
      return 1
      ;;
  esac
  printf '%s\n' "$eval_token_ttl"
}

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
# longer exists — which arrives as a 401 on the first governed call and
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
  # inherits the other's Knowledge or capture state. A disposable database
  # also makes the query/index versions recorded by the report unambiguous.
  EVAL_DB=eval_$$
  $COMPOSE exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda \
    -c "create database $EVAL_DB" >/dev/null
  # This database is created for one run and dropped at the end of it, so
  # its durability is worth nothing — and on a Docker Desktop volume it
  # costs a great deal. EVAL-3's LongMemEval run measured checkpoints
  # writing 58 MB in 270 seconds, about 0.2 MB/s; Postgres then stalled for
  # minutes at a stretch, connections died inside the stall, the gateway's
  # pool could not re-establish them, and every `/v1` surface answered 503
  # until the run was killed. Five attempts died that way before the
  # checkpoint timings said why.
  #
  # Per-database rather than cluster-wide: `synchronous_commit` can be set
  # with `ALTER DATABASE` and `fsync` cannot, and a scratch database
  # relaxing its own commits is a very different thing from a dev
  # container relaxing them for everybody's data.
  $COMPOSE exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d synveda \
    -c "alter database $EVAL_DB set synchronous_commit = off" >/dev/null
  $COMPOSE exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d "$EVAL_DB" -c \
    "create extension if not exists vector;
     create extension if not exists btree_gin" >/dev/null

  EVAL_STATE=$(mktemp -d "${TMPDIR:-/tmp}/synveda-eval-XXXXXX")
  EVAL_ENV="$EVAL_STATE/env.json"

  DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$EVAL_DB"
  export DATABASE_URL
  SYNVEDA_DEV_JWT_SECRET="$EVAL_JWT_SECRET"
  export SYNVEDA_DEV_JWT_SECRET
  # The deterministic extractor and embedder are the default and stay it:
  # a nightly failure should mean someone changed the code, not that a
  # model drifted (ADR-0028 decision 6). Deliberately not exported here —
  # `SYNVEDA_EXTRACTOR` and its credentials pass through from the caller,
  # which is what `make eval-extraction-live` uses to run the same corpus
  # through a real model against its own baseline (ADR-0046 decision 12).
  SYNVEDA_EXTRACTION_POLL_MS=300
  export SYNVEDA_EXTRACTION_POLL_MS
  RUST_LOG=${RUST_LOG:-warn}
  export RUST_LOG

  # Offline for the build, deliberately: DATABASE_URL now names the empty
  # scratch database, and sqlx's compile-time checks would validate every
  # query against a schema that does not exist yet — which passes only
  # while the build cache happens to be warm, and fails outright the first
  # time anything in the workspace changes. The committed `.sqlx` data is
  # what CI compiles against for the same reason.
  SQLX_OFFLINE=true cargo build -p synveda-gateway -p synveda-cli -p synveda-eval
  # Each disposable database gets its own evaluation-only KEK. Tenant
  # admission now provisions a tenant data key and must fail closed without
  # one; carrying no key here would make the harness stop before it measured
  # anything. The key is process-local, never written to the report or logs,
  # and disappears with the scratch database.
  SYNVEDA_KMS_KEY=$(./target/debug/synveda kms keygen 2>/dev/null)
  SYNVEDA_KMS_KEY_REF="local:eval-$EVAL_DB"
  export SYNVEDA_KMS_KEY SYNVEDA_KMS_KEY_REF
  ./target/debug/synveda db migrate
  EVAL_TENANT=$(./target/debug/synveda tenant create \
    --slug "eval-$$" --name "EVAL-1 harness" | eval_json_field id)
  # Dev-token admission has no IdP group claim from which to mint the first
  # operator grant. Seed that documented one-time row before asking the
  # governed admin API to create any descendant scope; every later grant is
  # made against the ordinary shared access model.
  eval_psql "with root as (
      insert into scopes (id, tenant_id, kind, slug, display_name)
      values (gen_random_uuid(), '$EVAL_TENANT', 'tenant', 'eval-$$', 'EVAL-1 harness')
      returning tenant_id, id
    )
    insert into scope_closure (tenant_id, ancestor_id, descendant_id, distance)
    select tenant_id, id, id, 0 from root" >/dev/null
  eval_psql "insert into scope_grants
      (id, tenant_id, scope_id, subject_kind, principal_id, role_key, source)
    select gen_random_uuid(), tenant_id, id, 'principal', 'eval-admin',
           'administrator', 'automation'
    from scopes where tenant_id = '$EVAL_TENANT' and kind = 'tenant'" >/dev/null

  # Phase 1: the product scope tree, through the governed public API.
  SYNVEDA_LISTEN_ADDR=${EVAL_SEED_URL#http://}
  export SYNVEDA_LISTEN_ADDR
  eval_port_free "$EVAL_SEED_URL"
  ./target/debug/synveda-gateway >"$EVAL_STATE/seed-gateway.log" 2>&1 &
  EVAL_SEED_PID=$!
  eval_wait_gateway "$EVAL_SEED_URL"
  admin=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT" --subject eval-admin)
  eval_root() {
    curl -fsS "$EVAL_SEED_URL/v1/admin/scopes" \
      -H "Authorization: Bearer $admin" | python3 -c 'import json,sys
print(json.load(sys.stdin)["parent"]["id"])'
  }
  org=$(eval_root)
  EVAL_ORG=$org
  eval_workspace_json() { # bearer key slug display-name
    curl -fsS -X POST "$EVAL_SEED_URL/v1/workspaces" \
      -H "Authorization: Bearer $1" -H 'Content-Type: application/json' \
      -H "Idempotency-Key: $2" \
      -d "{\"slug\":\"$3\",\"display_name\":\"$4\"}"
  }
  eval_project_json() { # bearer workspace key slug display-name
    curl -fsS -X POST "$EVAL_SEED_URL/v1/workspaces/$2/projects" \
      -H "Authorization: Bearer $1" -H 'Content-Type: application/json' \
      -H "Idempotency-Key: $3" \
      -d "{\"slug\":\"$4\",\"display_name\":\"$5\"}"
  }

  workspace_json=$(eval_workspace_json "$admin" eval-primary-workspace platform "Platform evaluation")
  EVAL_WORKSPACE=$(printf '%s' "$workspace_json" | eval_json_field id)
  EVAL_WORKSPACE_SCOPE=$(printf '%s' "$workspace_json" | eval_json_field scope_id)
  project_json=$(eval_project_json "$admin" "$EVAL_WORKSPACE" eval-primary-project pulseboard "PulseBoard")
  EVAL_PROJECT=$(printf '%s' "$project_json" | eval_json_field id)
  EVAL_PROJECT_SCOPE=$(printf '%s' "$project_json" | eval_json_field scope_id)

  outsider_workspace_json=$(eval_workspace_json "$admin" eval-outsider-workspace outsider "Outsider evaluation")
  EVAL_OUTSIDER_WORKSPACE=$(printf '%s' "$outsider_workspace_json" | eval_json_field id)
  EVAL_OUTSIDER_WORKSPACE_SCOPE=$(printf '%s' "$outsider_workspace_json" | eval_json_field scope_id)
  outsider_project_json=$(eval_project_json "$admin" "$EVAL_OUTSIDER_WORKSPACE" eval-outsider-project clearing "Clearing")
  EVAL_OUTSIDER_PROJECT=$(printf '%s' "$outsider_project_json" | eval_json_field id)
  EVAL_OUTSIDER_PROJECT_SCOPE=$(printf '%s' "$outsider_project_json" | eval_json_field scope_id)

  qa_workspace_json=$(eval_workspace_json "$admin" eval-qa-workspace engineering "Engineering evaluation")
  EVAL_QA_WORKSPACE=$(printf '%s' "$qa_workspace_json" | eval_json_field id)
  EVAL_QA_WORKSPACE_SCOPE=$(printf '%s' "$qa_workspace_json" | eval_json_field scope_id)
  qa_project_json=$(eval_project_json "$admin" "$EVAL_QA_WORKSPACE" eval-qa-project payments "Payments")
  EVAL_QA_PROJECT=$(printf '%s' "$qa_project_json" | eval_json_field id)
  EVAL_QA_PROJECT_SCOPE=$(printf '%s' "$qa_project_json" | eval_json_field scope_id)

  vault_workspace_json=$(eval_workspace_json "$admin" eval-vault-workspace vault "Vault evaluation")
  EVAL_VAULT_WORKSPACE=$(printf '%s' "$vault_workspace_json" | eval_json_field id)
  EVAL_VAULT_WORKSPACE_SCOPE=$(printf '%s' "$vault_workspace_json" | eval_json_field scope_id)
  vault_project_json=$(eval_project_json "$admin" "$EVAL_VAULT_WORKSPACE" eval-vault-project ceremonies "Vault ceremonies")
  EVAL_VAULT_PROJECT=$(printf '%s' "$vault_project_json" | eval_json_field id)
  EVAL_VAULT_PROJECT_SCOPE=$(printf '%s' "$vault_project_json" | eval_json_field scope_id)

  desk_workspace_json=$(eval_workspace_json "$admin" eval-desk-workspace desk "Settlement desk evaluation")
  EVAL_DESK_WORKSPACE=$(printf '%s' "$desk_workspace_json" | eval_json_field id)
  EVAL_DESK_WORKSPACE_SCOPE=$(printf '%s' "$desk_workspace_json" | eval_json_field scope_id)
  desk_project_json=$(eval_project_json "$admin" "$EVAL_DESK_WORKSPACE" eval-desk-project reconciliation "Reconciliation")
  EVAL_DESK_PROJECT=$(printf '%s' "$desk_project_json" | eval_json_field id)
  EVAL_DESK_PROJECT_SCOPE=$(printf '%s' "$desk_project_json" | eval_json_field scope_id)

  # The labelled corpora retain their four tier names, but the addresses are
  # current product scopes. Sessions run in projects; shared security
  # Knowledge sits at the workspace so a child-project grant cannot widen
  # authority upward.
  platform=$EVAL_PROJECT_SCOPE
  payments=$EVAL_QA_PROJECT_SCOPE
  eng=$EVAL_QA_WORKSPACE_SCOPE
  sec=$org
  vault=$EVAL_VAULT_WORKSPACE_SCOPE
  desk=$EVAL_DESK_PROJECT_SCOPE

  # A SECOND ADMITTED TENANT (EVAL-5, ADR-0048 decision 8). The first time
  # this harness has run more than one, and the point of the cross-tenant
  # half: the runner never sends a tenant — the token carries one — so a
  # probe from here to there is the real thing rather than a filter test.
  # Its estate is deliberately minimal; what is under measurement is the
  # boundary, not the shape on the far side of it.
  EVAL_TENANT_B=$(./target/debug/synveda tenant create \
    --slug "eval-b-$$" --name "EVAL-5 foreign tenant" | eval_json_field id)
  eval_psql "with root as (
      insert into scopes (id, tenant_id, kind, slug, display_name)
      values (gen_random_uuid(), '$EVAL_TENANT_B', 'tenant', 'eval-b-$$',
              'EVAL-5 foreign tenant')
      returning tenant_id, id
    )
    insert into scope_closure (tenant_id, ancestor_id, descendant_id, distance)
    select tenant_id, id, id, 0 from root" >/dev/null
  eval_psql "insert into scope_grants
      (id, tenant_id, scope_id, subject_kind, principal_id, role_key, source)
    select gen_random_uuid(), tenant_id, id, 'principal', 'eval-admin-b',
           'administrator', 'automation'
    from scopes where tenant_id = '$EVAL_TENANT_B' and kind = 'tenant'" >/dev/null
  admin_b=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT_B" --subject eval-admin-b)
  org_b=$(curl -fsS "$EVAL_SEED_URL/v1/admin/scopes" \
      -H "Authorization: Bearer $admin_b" | python3 -c 'import json,sys
print(json.load(sys.stdin)["parent"]["id"])')
  foreign_workspace_json=$(eval_workspace_json "$admin_b" eval-foreign-workspace northwind "Northwind evaluation")
  EVAL_FOREIGN_WORKSPACE=$(printf '%s' "$foreign_workspace_json" | eval_json_field id)
  EVAL_FOREIGN_WORKSPACE_SCOPE=$(printf '%s' "$foreign_workspace_json" | eval_json_field scope_id)
  foreign_project_json=$(eval_project_json "$admin_b" "$EVAL_FOREIGN_WORKSPACE" eval-foreign-project clearing "Clearing")
  EVAL_FOREIGN_PROJECT=$(printf '%s' "$foreign_project_json" | eval_json_field id)
  EVAL_FOREIGN_PROJECT_SCOPE=$(printf '%s' "$foreign_project_json" | eval_json_field scope_id)

  # The actors. Service registration is an ordinary governed application
  # action now, so it stays on the seed gateway under the bootstrapped admin
  # bearer. Most confinement anchors are the tenant root: every run lives in
  # the supported root-owned evaluation workspace, while grants below decide
  # which corpus scope each identity may read or publish into. The security
  # actors are the deliberate exception below: their project placement is the
  # premise being measured, distinct from an explicit content-role grant.
  #
  # One actor per extraction fixture group (EVAL-2, ADR-0046 decision 2).
  # The partition is load-bearing rather than tidy: session events and
  # accepted Knowledge remain attributable to one actor, and the diagnostic
  # evaluation lens is explicitly bounded. A corpus grows by adding actors,
  # not by silently truncating one actor's evidence.
  # Every ordinary evaluation service is anchored at the tenant root so its
  # explicitly selected project is inside its confinement subtree. Placement
  # is the principal scope's parent, never a grant and never the deleted fixed
  # hierarchy vocabulary.
  for actor in curator newcomer outsider \
    extract-alpha extract-beta extract-gamma extract-delta extract-epsilon \
    qa-reader qa-project qa-workspace qa-tenant qa-curator qa-steward qa-publisher \
    sec-compliance; do
    SYNVEDA_TOKEN="$admin" SYNVEDA_GATEWAY="$EVAL_SEED_URL" \
      ./target/debug/synveda service register \
        --subject "$actor" --scope "$org" >/dev/null
  done
  # EVAL-5 separates structural workspace placement from explicit content
  # authority. Both vault actors sit under the same workspace; their grants
  # below differ in where they start. The neighbouring actor is structurally
  # confined to the other workspace.
  for actor in sec-owner sec-mate; do
    SYNVEDA_TOKEN="$admin" SYNVEDA_GATEWAY="$EVAL_SEED_URL" \
      ./target/debug/synveda service register \
        --subject "$actor" --scope "$EVAL_VAULT_WORKSPACE_SCOPE" >/dev/null
  done
  SYNVEDA_TOKEN="$admin" SYNVEDA_GATEWAY="$EVAL_SEED_URL" \
    ./target/debug/synveda service register \
      --subject sec-neighbour --scope "$EVAL_DESK_WORKSPACE_SCOPE" >/dev/null
  for actor in xt-reader xt-compliance xt-curator xt-steward xt-publisher; do
    SYNVEDA_TOKEN="$admin_b" SYNVEDA_GATEWAY="$EVAL_SEED_URL" \
      ./target/debug/synveda service register \
        --subject "$actor" --scope "$org_b" >/dev/null
  done

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
    SYNVEDA_TOKEN="$admin" SYNVEDA_GATEWAY="$EVAL_SEED_URL" \
      ./target/debug/synveda service register \
        --subject "$(printf 'lme-%03d' "$eval_lme")" --scope "$org" >/dev/null
    eval_lme=$((eval_lme + 1))
  done

  kill "$EVAL_SEED_PID" 2>/dev/null || true
  wait "$EVAL_SEED_PID" 2>/dev/null || true
  EVAL_SEED_PID=""

  # The grants. Break-glass at the store level, in the open, on the same
  # seam `role bind` used to be: the harness's reviewer and auditor roles
  # and the two admin doors, as `scope_grants` rows (CPR-7, ADR-0074 —
  # grants replaced bindings, and the operator door is an administrator
  # grant at the tenant root). Role mapping per the floors' re-vocabulary:
  # steward/compliance/auditor → administrator, curator → curator.
  eval_grant() {  # tenant subject role scope
    $COMPOSE exec -T postgres psql -v ON_ERROR_STOP=1 -U synveda -d "$EVAL_DB" -c \
      "insert into scope_grants (id, tenant_id, scope_id, subject_kind, principal_id, role_key, source)
       values (gen_random_uuid(), '$1', '$4', 'principal', '$2', '$3', 'automation')" >/dev/null
  }
  eval_root_of() {  # tenant -> the tenant root scope id
    $COMPOSE exec -T postgres psql -qtAX -U synveda -d "$EVAL_DB" -c \
      "select id from scopes where tenant_id = '$1' and kind = 'tenant'"
  }
  eval_scope_access() { # tenant governed-scope subjects...
    eval_access_tenant=$1
    eval_access_scope=$2
    shift 2
    for subject do
      eval_grant "$eval_access_tenant" "$subject" member "$eval_access_scope"
      # The labelled enumeration lens first authorises the session payload and
      # then independently PDP-filters every Knowledge row. Keep that
      # diagnostic authority explicit: ordinary workspace membership is
      # deliberately too weak under the standard and regulated-strict packs.
      eval_grant "$eval_access_tenant" "$subject" reviewer "$eval_access_scope"
    done
  }
  eval_scope_access "$EVAL_TENANT" "$EVAL_WORKSPACE_SCOPE" \
    curator newcomer \
    extract-alpha extract-beta extract-gamma extract-delta extract-epsilon
  eval_scope_access "$EVAL_TENANT" "$EVAL_OUTSIDER_WORKSPACE_SCOPE" outsider
  eval_scope_access "$EVAL_TENANT" "$EVAL_QA_WORKSPACE_SCOPE" \
    qa-reader qa-project qa-workspace qa-tenant qa-curator qa-steward qa-publisher
  # Owner and compliance hold workspace roles. The teammate holds the same
  # session/diagnostic roles only at the child project: enough to use that
  # project, unable to flow back up to confidential workspace Knowledge.
  eval_scope_access "$EVAL_TENANT" "$EVAL_VAULT_WORKSPACE_SCOPE" \
    sec-owner sec-compliance
  eval_scope_access "$EVAL_TENANT" "$EVAL_VAULT_PROJECT_SCOPE" sec-mate
  eval_scope_access "$EVAL_TENANT" "$EVAL_DESK_WORKSPACE_SCOPE" sec-compliance
  eval_scope_access "$EVAL_TENANT" "$EVAL_DESK_PROJECT_SCOPE" sec-neighbour
  eval_lme=0
  while [ "$eval_lme" -lt "${EVAL_LONGMEMEVAL_ACTORS:-0}" ]; do
    eval_grant "$EVAL_TENANT" "$(printf 'lme-%03d' "$eval_lme")" member "$EVAL_WORKSPACE_SCOPE"
    eval_grant "$EVAL_TENANT" "$(printf 'lme-%03d' "$eval_lme")" reviewer "$EVAL_WORKSPACE_SCOPE"
    eval_lme=$((eval_lme + 1))
  done
  eval_scope_access "$EVAL_TENANT_B" "$EVAL_FOREIGN_WORKSPACE_SCOPE" \
    xt-reader xt-compliance xt-curator xt-steward xt-publisher
  # The conservative fallback profile reviews even principal-scope Knowledge.
  # These direct grants are the evaluation policy pack: privacy deliberately
  # blocks inherited root roles at somebody else's principal scope, so the
  # reviewers must be named at the exact target. The publisher is a fourth,
  # distinct identity for matrices that separate author, reviewers and effect
  # actor.
  eval_psql "insert into scope_grants
      (id, tenant_id, scope_id, subject_kind, principal_id, role_key, source)
    select gen_random_uuid(), target.tenant_id, target.scope_id, 'principal',
           reviewer.subject, reviewer.role_key, 'automation'
      from identities target
      cross join (values
        ('qa-curator', 'curator'),
        ('qa-steward', 'administrator'),
        ('sec-compliance', 'reviewer'),
        ('qa-publisher', 'administrator')
      ) reviewer(subject, role_key)
     where target.tenant_id = '$EVAL_TENANT'
    on conflict do nothing" >/dev/null
  eval_psql "insert into scope_grants
      (id, tenant_id, scope_id, subject_kind, principal_id, role_key, source)
    select gen_random_uuid(), target.tenant_id, target.scope_id, 'principal',
           reviewer.subject, reviewer.role_key, 'automation'
      from identities target
      cross join (values
        ('xt-compliance', 'administrator'),
        ('xt-curator', 'curator'),
        ('xt-steward', 'administrator'),
        ('xt-publisher', 'administrator')
      ) reviewer(subject, role_key)
     where target.tenant_id = '$EVAL_TENANT_B'
    on conflict do nothing" >/dev/null
  eval_grant "$EVAL_TENANT" qa-curator curator "$org"
  eval_grant "$EVAL_TENANT" qa-curator administrator "$org"
  eval_grant "$EVAL_TENANT" qa-steward administrator "$org"
  eval_grant "$EVAL_TENANT" qa-publisher administrator "$org"
  # CPR-40 creates the Q&A/security premise directly at its governed scope
  # while accepting each reviewable capture candidate. These are ordinary
  # shared-model grants, not a direct data seed or a second publication path.
  eval_grant "$EVAL_TENANT" qa-project member "$payments"
  eval_grant "$EVAL_TENANT" qa-tenant member "$org"
  eval_grant "$EVAL_TENANT" sec-compliance administrator "$org"
  eval_grant "$EVAL_TENANT" eval-auditor administrator "$(eval_root_of "$EVAL_TENANT")"
  eval_grant "$EVAL_TENANT_B" eval-auditor-b administrator "$(eval_root_of "$EVAL_TENANT_B")"
  eval_grant "$EVAL_TENANT_B" xt-compliance administrator "$org_b"
  eval_grant "$EVAL_TENANT_B" xt-curator curator "$org_b"
  eval_grant "$EVAL_TENANT_B" xt-curator administrator "$org_b"
  eval_grant "$EVAL_TENANT_B" xt-steward administrator "$org_b"
  eval_grant "$EVAL_TENANT_B" xt-publisher administrator "$org_b"

  # Phase 2: the gateway under measurement.
  SYNVEDA_LISTEN_ADDR=${EVAL_GATEWAY_URL#http://}
  export SYNVEDA_LISTEN_ADDR
  eval_service_token_ttl_secs=$(eval_service_token_ttl_for_run)
  SYNVEDA_SERVICE_TOKEN_MAX_TTL_SECS=$eval_service_token_ttl_secs
  export SYNVEDA_SERVICE_TOKEN_MAX_TTL_SECS
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
  for who in reader project workspace tenant curator steward publisher; do
    eval "qa_$who=\$(./target/debug/synveda token issue \
      --tenant \"\$EVAL_TENANT\" --subject \"qa-$who\")"
  done
  for who in owner mate neighbour compliance; do
    eval "sec_$who=\$(./target/debug/synveda token issue \
      --tenant \"\$EVAL_TENANT\" --subject \"sec-$who\")"
  done
  # The one bearer in this file that carries a different tenant.
  xt_reader=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT_B" --subject xt-reader)
  xt_compliance=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT_B" --subject xt-compliance)
  xt_curator=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT_B" --subject xt-curator)
  xt_steward=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT_B" --subject xt-steward)
  xt_publisher=$(./target/debug/synveda token issue --tenant "$EVAL_TENANT_B" --subject xt-publisher)

  # Every benchmark run names an actual governed runtime configuration. The
  # conservative no-binding document deliberately allows no external
  # provider, so merely exporting SYNVEDA_EMBEDDER=tei cannot authorise a
  # semantic call. Keep regulated-strict's policy semantics and admit only
  # this benchmark's local TEI provider through an immutable Configuration
  # version and inherited tenant-root binding. Both mutations take the normal
  # VedaFlow review path; this fixture is not a configuration fast path.
  eval_apply_configuration_change() { # response reviewer-one reviewer-two publisher
    eval_configuration_response=$1
    eval_configuration_reviewer_one=$2
    eval_configuration_reviewer_two=$3
    eval_configuration_publisher=$4
    eval_configuration_outcome=$(printf '%s' "$eval_configuration_response" |
      eval_json_field outcome 2>/dev/null || true)
    [ "$eval_configuration_outcome" = pending_review ] || {
      echo "eval: governed configuration unexpectedly returned ${eval_configuration_outcome:-no outcome}: $eval_configuration_response" >&2
      return 1
    }
    eval_configuration_change=$(printf '%s' "$eval_configuration_response" |
      eval_json_field change_id)
    for eval_configuration_reviewer in \
      "$eval_configuration_reviewer_one" "$eval_configuration_reviewer_two"; do
      SYNVEDA_TOKEN="$eval_configuration_reviewer" SYNVEDA_GATEWAY="$EVAL_GATEWAY_URL" \
        ./target/debug/synveda proposal approve "$eval_configuration_change" \
          >/dev/null 2>&1
    done
    eval_configuration_applied=$(curl -sS -X POST \
      "$EVAL_GATEWAY_URL/v1/proposals/$eval_configuration_change/apply" \
      -H "Authorization: Bearer $eval_configuration_publisher")
    eval_configuration_apply_outcome=$(printf '%s' "$eval_configuration_applied" |
      eval_json_field outcome 2>/dev/null || true)
    [ "$eval_configuration_apply_outcome" = applied ] || {
      echo "eval: reviewed configuration $eval_configuration_change did not apply: $eval_configuration_applied" >&2
      return 1
    }
  }
  eval_bind_runtime_configuration() { # label scope template-reader author reviewer-one reviewer-two publisher
    eval_configuration_label=$1
    eval_configuration_scope=$2
    eval_configuration_template_reader=$3
    eval_configuration_author=$4
    eval_configuration_reviewer_one=$5
    eval_configuration_reviewer_two=$6
    eval_configuration_publisher=$7
    eval_configuration_templates=$(curl -sS \
      "$EVAL_GATEWAY_URL/v1/configuration-templates" \
      -H "Authorization: Bearer $eval_configuration_template_reader")
    printf '%s' "$eval_configuration_templates" | python3 -c '
import json, sys
assert isinstance(json.load(sys.stdin).get("templates"), list)
' 2>/dev/null || {
      echo "eval: configuration templates were not readable for $eval_configuration_label: $eval_configuration_templates" >&2
      return 1
    }
    eval_configuration_body=$(printf '%s' "$eval_configuration_templates" | python3 -c '
import json, sys
templates = json.load(sys.stdin)["templates"]
document = next(item["document"] for item in templates if item["name"] == "enterprise")
document["allowed_external_providers"] = ["tei"]
print(json.dumps({
    "governing_scope_id": sys.argv[1],
    "name": "evaluation-runtime",
    "document": document,
    "source_template": None,
}, separators=(",", ":")))
' "$eval_configuration_scope")
    eval_configuration_created=$(curl -sS -X POST \
      "$EVAL_GATEWAY_URL/v1/configurations" \
      -H "Authorization: Bearer $eval_configuration_author" \
      -H 'Content-Type: application/json' \
      -H "Idempotency-Key: eval-$eval_configuration_label-configuration" \
      -d "$eval_configuration_body")
    eval_apply_configuration_change "$eval_configuration_created" \
      "$eval_configuration_reviewer_one" "$eval_configuration_reviewer_two" \
      "$eval_configuration_publisher"
    eval_configuration_artifact=$(printf '%s' "$eval_configuration_created" |
      eval_json_field artifact_id)
    eval_configuration_binding=$(python3 -c '
import json, sys
print(json.dumps({
    "scope_id": sys.argv[1],
    "artifact_id": sys.argv[2],
    "pinned_version_id": None,
    "enabled": True,
}, separators=(",", ":")))
' "$eval_configuration_scope" "$eval_configuration_artifact")
    eval_configuration_bound=$(curl -sS -X POST \
      "$EVAL_GATEWAY_URL/v1/configuration-bindings" \
      -H "Authorization: Bearer $eval_configuration_author" \
      -H 'Content-Type: application/json' \
      -H "Idempotency-Key: eval-$eval_configuration_label-binding" \
      -d "$eval_configuration_binding")
    eval_apply_configuration_change "$eval_configuration_bound" \
      "$eval_configuration_reviewer_one" "$eval_configuration_reviewer_two" \
      "$eval_configuration_publisher"
    eval_configuration_effective=$(curl -sS \
      "$EVAL_GATEWAY_URL/v1/configurations/effective?scope_id=$eval_configuration_scope" \
      -H "Authorization: Bearer $eval_configuration_author")
    printf '%s' "$eval_configuration_effective" | python3 -c '
import json, sys
effective = json.load(sys.stdin)
assert effective["version_id"], "effective Configuration has no immutable version"
assert effective["document"]["allowed_external_providers"] == ["tei"]
' || {
      echo "eval: reviewed configuration is not effective for $eval_configuration_label" >&2
      return 1
    }
  }
  eval_bind_runtime_configuration primary "$org" "$admin" "$qa_steward" \
    "$sec_compliance" "$qa_publisher" "$qa_curator"
  eval_bind_runtime_configuration foreign "$org_b" "$admin_b" "$xt_steward" \
    "$xt_compliance" "$xt_publisher" "$xt_curator"

  # The LongMemEval pool, as a JSON fragment rather than as fixed lines:
  # how many exist is a run's decision, and the harness discovers them by
  # the `lme-` prefix rather than by a count written in two places.
  eval_lme_actors=""
  eval_lme=0
  while [ "$eval_lme" -lt "${EVAL_LONGMEMEVAL_ACTORS:-0}" ]; do
    eval_lme_subject=$(printf 'lme-%03d' "$eval_lme")
    eval_lme_token=$(./target/debug/synveda token issue \
      --tenant "$EVAL_TENANT" --subject "$eval_lme_subject" \
      --ttl-secs "$eval_service_token_ttl_secs")
    eval_lme_actors="$eval_lme_actors,
    \"$eval_lme_subject\": {
      \"token\": \"$eval_lme_token\",
      \"scope\": \"platform/pulseboard\",
      \"workspace_id\": \"$EVAL_WORKSPACE\",
      \"project_id\": \"$EVAL_PROJECT\"
    }"
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
    "curator":  {
      "token": "$curator", "scope": "platform/pulseboard",
      "workspace_id": "$EVAL_WORKSPACE", "project_id": "$EVAL_PROJECT"
    },
    "newcomer": {
      "token": "$newcomer", "scope": "platform/pulseboard",
      "workspace_id": "$EVAL_WORKSPACE", "project_id": "$EVAL_PROJECT"
    },
    "outsider": {
      "token": "$outsider", "scope": "outsider/clearing",
      "workspace_id": "$EVAL_OUTSIDER_WORKSPACE", "project_id": "$EVAL_OUTSIDER_PROJECT"
    },
    "auditor":  { "token": "$eval_auditor" },
    "auditor-northwind": { "token": "$eval_auditor_b", "tenant": "$EVAL_TENANT_B" },
    "extract-alpha": {
      "token": "$extract_alpha", "scope": "platform/pulseboard",
      "workspace_id": "$EVAL_WORKSPACE", "project_id": "$EVAL_PROJECT"
    },
    "extract-beta": {
      "token": "$extract_beta", "scope": "platform/pulseboard",
      "workspace_id": "$EVAL_WORKSPACE", "project_id": "$EVAL_PROJECT"
    },
    "extract-gamma": {
      "token": "$extract_gamma", "scope": "platform/pulseboard",
      "workspace_id": "$EVAL_WORKSPACE", "project_id": "$EVAL_PROJECT"
    },
    "extract-delta": {
      "token": "$extract_delta", "scope": "platform/pulseboard",
      "workspace_id": "$EVAL_WORKSPACE", "project_id": "$EVAL_PROJECT"
    },
    "extract-epsilon": {
      "token": "$extract_epsilon", "scope": "platform/pulseboard",
      "workspace_id": "$EVAL_WORKSPACE", "project_id": "$EVAL_PROJECT"
    },
    "qa-reader": {
      "token": "$qa_reader", "scope": "engineering/payments",
      "workspace_id": "$EVAL_QA_WORKSPACE", "project_id": "$EVAL_QA_PROJECT"
    },
    "qa-project": {
      "token": "$qa_project", "scope": "engineering/payments",
      "workspace_id": "$EVAL_QA_WORKSPACE", "project_id": "$EVAL_QA_PROJECT"
    },
    "qa-workspace": {
      "token": "$qa_workspace", "scope": "engineering",
      "workspace_id": "$EVAL_QA_WORKSPACE", "project_id": "$EVAL_QA_PROJECT"
    },
    "qa-tenant": {
      "token": "$qa_tenant", "scope": "tenant",
      "workspace_id": "$EVAL_QA_WORKSPACE", "project_id": "$EVAL_QA_PROJECT"
    },
    "qa-curator": {
      "token": "$qa_curator", "scope": "tenant",
      "workspace_id": "$EVAL_QA_WORKSPACE", "project_id": "$EVAL_QA_PROJECT"
    },
    "qa-steward": {
      "token": "$qa_steward", "scope": "tenant",
      "workspace_id": "$EVAL_QA_WORKSPACE", "project_id": "$EVAL_QA_PROJECT"
    },
    "qa-publisher": {
      "token": "$qa_publisher", "scope": "tenant",
      "workspace_id": "$EVAL_QA_WORKSPACE", "project_id": "$EVAL_QA_PROJECT"
    },
    "sec-owner": {
      "token": "$sec_owner", "scope": "vault/ceremonies",
      "workspace_id": "$EVAL_VAULT_WORKSPACE", "project_id": "$EVAL_VAULT_PROJECT"
    },
    "sec-mate": {
      "token": "$sec_mate", "scope": "vault/ceremonies",
      "workspace_id": "$EVAL_VAULT_WORKSPACE", "project_id": "$EVAL_VAULT_PROJECT"
    },
    "sec-neighbour": {
      "token": "$sec_neighbour", "scope": "desk/reconciliation",
      "workspace_id": "$EVAL_DESK_WORKSPACE", "project_id": "$EVAL_DESK_PROJECT"
    },
    "sec-compliance": {
      "token": "$sec_compliance", "scope": "tenant",
      "workspace_id": "$EVAL_VAULT_WORKSPACE", "project_id": "$EVAL_VAULT_PROJECT"
    },
    "xt-reader": {
      "token": "$xt_reader",
      "scope": "northwind/clearing", "tenant": "$EVAL_TENANT_B",
      "workspace_id": "$EVAL_FOREIGN_WORKSPACE", "project_id": "$EVAL_FOREIGN_PROJECT"
    },
    "xt-compliance": {
      "token": "$xt_compliance", "tenant": "$EVAL_TENANT_B",
      "workspace_id": "$EVAL_FOREIGN_WORKSPACE", "project_id": "$EVAL_FOREIGN_PROJECT"
    },
    "xt-curator": {
      "token": "$xt_curator", "tenant": "$EVAL_TENANT_B",
      "workspace_id": "$EVAL_FOREIGN_WORKSPACE", "project_id": "$EVAL_FOREIGN_PROJECT"
    },
    "xt-steward": {
      "token": "$xt_steward", "tenant": "$EVAL_TENANT_B",
      "workspace_id": "$EVAL_FOREIGN_WORKSPACE", "project_id": "$EVAL_FOREIGN_PROJECT"
    },
    "xt-publisher": {
      "token": "$xt_publisher", "tenant": "$EVAL_TENANT_B",
      "workspace_id": "$EVAL_FOREIGN_WORKSPACE", "project_id": "$EVAL_FOREIGN_PROJECT"
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
