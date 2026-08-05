#!/usr/bin/env sh
# AUTH-3 acceptance demo: service identities, live against the bundled
# Rauthy (ADR-0018). AC (docs/backlog/AUTH-3.md): an agent token with team
# scope cannot call org-scope endpoints.
#
# Flow: provision Rauthy (confidential clients ci-agent and rogue-agent,
# client_credentials flow, RS256) -> seed the acme/eng/platform hierarchy,
# register ci-agent at the platform team, and bind it steward-at-team plus
# *tenant-wide org-admin* (dev-mode gateway phase) -> boot the gateway in
# OIDC mode with the agents' audiences accepted -> an over-long token is
# refused (the lifetime cap) -> a compliant token works the team subtree
# but is denied every org-scope endpoint despite the org-admin binding ->
# the unregistered rogue-agent is quarantined fail-closed -> metrics show
# the PDP denials and the lifetime rejection. The CI-clean mock-IdP half
# of the AC runs in crates/synveda-gateway/tests/service_identities.rs.
#
# On Windows, run via Git Bash. Needs postgres, jaeger, and rauthy from the
# dev compose, plus node (JSON parsing — no jq dependency).
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
RAUTHY_URL=http://localhost:8100
RAUTHY_ISSUER=http://localhost:8100/auth/v1/
GATEWAY_URL=http://127.0.0.1:8120
# Dev-only bootstrap API key from deploy/compose/rauthy/config.toml.
RAUTHY_API_KEY='API-Key synveda-dev$6xxmjZD7Wqe9zWN1fWzOW1jA4uxAkFQ9rYlVFpxBzVgJ0xEj2KWSLiaRTZzKV1oz'
AGENT_CLIENT=ci-agent
ROGUE_CLIENT=rogue-agent

$COMPOSE up --detach --wait postgres jaeger
$COMPOSE up --detach rauthy

wait_rauthy() {
  tries=0
  until curl -fsS "$RAUTHY_URL/auth/v1/.well-known/openid-configuration" >/dev/null 2>&1; do
    tries=$((tries + 1))
    if [ "$tries" -ge 60 ]; then
      echo "demo FAILED: rauthy did not become ready" >&2
      exit 1
    fi
    sleep 1
  done
}
wait_rauthy

json_field() {
  # json_field <field> — prints .<field> of the JSON on stdin.
  node -e '
    let d = "";
    process.stdin.on("data", (c) => (d += c));
    process.stdin.on("end", () => {
      const v = JSON.parse(d)[process.argv[1]];
      if (v === undefined) { process.exit(1); }
      console.log(v);
    });
  ' "$1"
}

echo "==> ensure the dev API key is live with Clients+Secrets access (AUTH-3 widened it)"
ensure_key() {
  curl -fsS "$RAUTHY_URL/auth/v1/api_keys/synveda-dev/test" \
    -H "Authorization: $RAUTHY_API_KEY" >/dev/null 2>&1 || return 1
  # Prove the Secrets right on the agent client itself (created here if
  # absent; provision_client reshapes and re-rotates it below anyway).
  if ! curl -fsS "$RAUTHY_URL/auth/v1/clients/$AGENT_CLIENT" \
    -H "Authorization: $RAUTHY_API_KEY" >/dev/null 2>&1; then
    curl -fsS -X POST "$RAUTHY_URL/auth/v1/clients" \
      -H "Authorization: $RAUTHY_API_KEY" -H 'Content-Type: application/json' \
      -d "{\"id\":\"$AGENT_CLIENT\",\"name\":\"$AGENT_CLIENT\",\"confidential\":true,
           \"redirect_uris\":[],\"post_logout_redirect_uris\":[]}" >/dev/null 2>&1 || return 1
  fi
  curl -fsS -X PUT "$RAUTHY_URL/auth/v1/clients/$AGENT_CLIENT/secret" \
    -H "Authorization: $RAUTHY_API_KEY" >/dev/null 2>&1 || return 1
}
if ! ensure_key; then
  echo "    key not live or predates Secrets rights — recreating the rauthy volume (dev-only state)"
  $COMPOSE stop rauthy
  $COMPOSE rm -f rauthy
  docker volume rm synveda_rauthy-data
  $COMPOSE up --detach rauthy
  wait_rauthy
fi

# provision_client <id> <access_token_lifetime_secs> — idempotently shapes
# a confidential client speaking the client_credentials grant, then
# rotates and prints its secret.
provision_client() {
  if ! curl -fsS "$RAUTHY_URL/auth/v1/clients/$1" \
    -H "Authorization: $RAUTHY_API_KEY" >/dev/null 2>&1; then
    curl -fsS -X POST "$RAUTHY_URL/auth/v1/clients" \
      -H "Authorization: $RAUTHY_API_KEY" -H 'Content-Type: application/json' \
      -d "{\"id\":\"$1\",\"name\":\"$1\",\"confidential\":true,
           \"redirect_uris\":[],\"post_logout_redirect_uris\":[]}" >/dev/null
  fi
  curl -fsS -X PUT "$RAUTHY_URL/auth/v1/clients/$1" \
    -H "Authorization: $RAUTHY_API_KEY" -H 'Content-Type: application/json' \
    -d "{\"id\":\"$1\",\"name\":\"$1\",\"enabled\":true,
         \"confidential\":true,\"redirect_uris\":[],
         \"flows_enabled\":[\"client_credentials\"],
         \"access_token_alg\":\"RS256\",\"id_token_alg\":\"RS256\",
         \"auth_code_lifetime\":60,\"access_token_lifetime\":$2,
         \"scopes\":[\"openid\"],\"default_scopes\":[\"openid\"],
         \"challenges\":null,\"force_mfa\":false}" >/dev/null
  curl -fsS -X PUT "$RAUTHY_URL/auth/v1/clients/$1/secret" \
    -H "Authorization: $RAUTHY_API_KEY" | json_field secret
}

# The lifetime cap half of the AC story: ci-agent starts with tokens that
# outlive the gateway's max (7200 > 3600) and is later reshaped to 300.
echo "==> provision confidential clients: $AGENT_CLIENT (7200s tokens, reshaped later), $ROGUE_CLIENT"
agent_secret=$(provision_client "$AGENT_CLIENT" 7200)
rogue_secret=$(provision_client "$ROGUE_CLIENT" 300)
echo "    clients ready (client_credentials, RS256)"

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
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317
export OTEL_EXPORTER_OTLP_ENDPOINT
OTEL_BSP_SCHEDULE_DELAY=500
export OTEL_BSP_SCHEDULE_DELAY
SYNVEDA_PUBLIC_URL=$GATEWAY_URL
export SYNVEDA_PUBLIC_URL

cargo build -p synveda-gateway -p synveda-cli

echo "==> migrate + admit the tenant this dev IdP binds to"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "auth3-demo-$(date +%s)-$$" --name "AUTH-3 Demo Tenant")
tenant_id=$(echo "$tenant_json" | json_field id)
echo "    tenant: $tenant_id"

wait_gateway() {
  tries=0
  until curl -fsS "$1/healthz" >/dev/null 2>&1; do
    tries=$((tries + 1))
    if [ "$tries" -ge 30 ]; then
      echo "demo FAILED: gateway did not become healthy on $1" >&2
      exit 1
    fi
    sleep 1
  done
}

echo "==> phase 1: seed hierarchy, register the agent, bind its roles (dev-mode gateway)"
SYNVEDA_LISTEN_ADDR=127.0.0.1:8131
export SYNVEDA_LISTEN_ADDR
SYNVEDA_DEV_JWT_SECRET=auth-3-demo-secret
export SYNVEDA_DEV_JWT_SECRET
./target/debug/synveda-gateway &
SEED_PID=$!
trap 'kill "$SEED_PID" 2>/dev/null || true' EXIT INT TERM
wait_gateway http://127.0.0.1:8131
# Since AUTHZ-3 an unbound dev subject holds no administrative power: the
# CLI break-glass bootstraps the first org-admin (ADR-0015 decision 6).
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null
seed_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-admin)
seed_api() {
  # seed_api <method> <path> <json-body>
  curl -fsS -X "$1" "http://127.0.0.1:8131$2" \
    -H "Authorization: Bearer $seed_token" -H 'Content-Type: application/json' \
    -d "$3"
}
org_id=$(seed_api POST /v1/hierarchy/nodes \
  '{"parent_id":null,"kind":"org","slug":"acme","name":"ACME"}' | json_field id)
eng_id=$(seed_api POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$org_id\",\"kind\":\"department\",\"slug\":\"eng\",\"name\":\"Engineering\"}" |
  json_field id)
platform_id=$(seed_api POST /v1/hierarchy/nodes \
  "{\"parent_id\":\"$eng_id\",\"kind\":\"team\",\"slug\":\"platform\",\"name\":\"Platform\"}" |
  json_field id)

echo "==> register $AGENT_CLIENT at the platform team (the API surface, PDP-gated)"
agent_json=$(seed_api POST /v1/service-identities \
  "{\"subject\":\"$AGENT_CLIENT\",\"scope_id\":\"$platform_id\",\"display_name\":\"CI Agent\"}")
agent_identity_id=$(echo "$agent_json" | json_field id)
echo "    registered: $agent_identity_id (kind $(echo "$agent_json" | json_field kind))"

echo "==> bind the agent steward-at-team AND tenant-wide org-admin (the AC's stronger case)"
seed_api PUT "/v1/hierarchy/nodes/$platform_id/roles" \
  "{\"subject\":\"$AGENT_CLIENT\",\"role\":\"steward\"}" >/dev/null
seed_api PUT /v1/roles/bindings \
  "{\"subject\":\"$AGENT_CLIENT\",\"role\":\"org-admin\"}" >/dev/null
./target/debug/synveda service list --tenant "$tenant_id" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    const agents = JSON.parse(d);
    if (agents.length !== 1 || agents[0].subject !== process.argv[1]) {
      console.error("unexpected service list: " + d);
      process.exit(1);
    }
    console.error(`    synveda service list: ${agents[0].subject} anchored (scope ${agents[0].scope_id})`);
  });
' "$AGENT_CLIENT"
kill "$SEED_PID" 2>/dev/null || true
wait "$SEED_PID" 2>/dev/null || true
unset SYNVEDA_DEV_JWT_SECRET

echo "==> phase 2: gateway in OIDC mode, agents' audiences accepted (ADR-0018 decision 1)"
SYNVEDA_LISTEN_ADDR=127.0.0.1:8120
export SYNVEDA_LISTEN_ADDR
SYNVEDA_OIDC_ISSUERS=$(cat <<EOF
[{"issuer":"$RAUTHY_ISSUER","client_id":"synveda",
  "tenant":{"static":{"tenant_id":"$tenant_id"}},
  "service_audiences":["$AGENT_CLIENT","$ROGUE_CLIENT"]}]
EOF
)
export SYNVEDA_OIDC_ISSUERS
./target/debug/synveda-gateway &
GATEWAY_PID=$!
trap 'kill "$GATEWAY_PID" 2>/dev/null || true; kill "$SEED_PID" 2>/dev/null || true' EXIT INT TERM
wait_gateway "$GATEWAY_URL"

# agent_token <client_id> <client_secret> — the OAuth2 client-credentials
# grant, exactly as a headless agent performs it.
agent_token() {
  curl -fsS -X POST "$RAUTHY_URL/auth/v1/oidc/token" \
    -H 'Content-Type: application/x-www-form-urlencoded' \
    -d "grant_type=client_credentials&client_id=$1&client_secret=$2" |
    json_field access_token
}

# expect <want_code> <label> <curl args...>
expect() {
  want=$1
  label=$2
  shift 2
  code=$(curl -s -o /dev/null -w '%{http_code}' "$@")
  if [ "$code" != "$want" ]; then
    echo "demo FAILED: $label returned HTTP $code, want $want" >&2
    exit 1
  fi
  echo "    $label: $code"
}

echo "==> the lifetime cap: 7200s tokens exceed the 3600s max and are refused"
long_token=$(agent_token "$AGENT_CLIENT" "$agent_secret")
expect 401 "over-long service token on a governed route" \
  -H "Authorization: Bearer $long_token" "$GATEWAY_URL/v1/hierarchy/nodes/$platform_id"

echo "==> reshape $AGENT_CLIENT to short-lived (300s) tokens and try again"
provision_client "$AGENT_CLIENT" 300 >/dev/null
agent_secret=$(provision_client "$AGENT_CLIENT" 300)
token=$(agent_token "$AGENT_CLIENT" "$agent_secret")

echo "==> the team subtree works (steward at platform)"
expect 200 "GET the platform team" \
  -H "Authorization: Bearer $token" "$GATEWAY_URL/v1/hierarchy/nodes/$platform_id"

echo "==> the AC: org-scope endpoints deny, despite the tenant-wide org-admin binding"
expect 403 "GET the org node" \
  -H "Authorization: Bearer $token" "$GATEWAY_URL/v1/hierarchy/nodes/$org_id"
expect 403 "GET the eng department" \
  -H "Authorization: Bearer $token" "$GATEWAY_URL/v1/hierarchy/nodes/$eng_id"
expect 403 "GET the hierarchy root (tenant plane)" \
  -H "Authorization: Bearer $token" "$GATEWAY_URL/v1/hierarchy/root"
expect 403 "PUT the tenant default pack (tenant plane)" \
  -X PUT -H "Authorization: Bearer $token" -H 'Content-Type: application/json' \
  -d '{"name":"standard"}' "$GATEWAY_URL/v1/policy/default"

echo "==> the unregistered $ROGUE_CLIENT is quarantined fail-closed"
rogue_token=$(agent_token "$ROGUE_CLIENT" "$rogue_secret")
expect 403 "unregistered client reading the team" \
  -H "Authorization: Bearer $rogue_token" "$GATEWAY_URL/v1/hierarchy/nodes/$platform_id"

echo "==> /metrics shows the PDP denials and the lifetime rejection"
metrics=$(curl -fsS "$GATEWAY_URL/metrics")
for want in \
  'synveda_service_token_rejections_total{reason="lifetime_exceeded"}' \
  'synveda_authz_decisions_total{action="hierarchy.read",decision="deny",pack="regulated-strict"}' \
  'synveda_authz_decisions_total{action="hierarchy.read",decision="allow",pack="regulated-strict"}'; do
  echo "$metrics" | grep -qF "$want" || {
    echo "demo FAILED: missing from /metrics: $want" >&2
    exit 1
  }
done
echo "    lifetime rejection + decision log entries present"

echo ""
echo "AUTH-3 service identities: acceptance criteria pass."
echo "The CI-clean mock-IdP half runs in: cargo test -p synveda-gateway --test service_identities"
