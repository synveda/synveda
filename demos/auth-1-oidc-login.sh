#!/usr/bin/env sh
# AUTH-1 acceptance demo: OIDC login (code+PKCE) via the bundled Rauthy.
# AC (docs/backlog/AUTH-1.md): login via Rauthy and via a mock Entra config
# both yield a Synveda session. The mock-Entra half runs CI-clean in
# crates/synveda-gateway/tests/oidc_login.rs; this demo proves the
# live-Rauthy half: provision the `synveda` client (bootstrap API key,
# ADR-0010) -> drive the real login (Rauthy PoW + credentials + PKCE) ->
# gateway /auth/callback returns the Synveda session -> the access token
# works as the /v1 bearer -> metrics show the login.
#
# On Windows, run via Git Bash. Needs postgres, jaeger, and rauthy from the
# dev compose, plus node (PoW solving and JSON parsing — no jq dependency).
set -eu

cd "$(dirname "$0")/.."

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
RAUTHY_URL=http://localhost:8100
RAUTHY_ISSUER=http://localhost:8100/auth/v1/
GATEWAY_URL=http://127.0.0.1:8120
# Dev-only bootstrap API key from deploy/compose/rauthy/config.toml.
RAUTHY_API_KEY='API-Key synveda-dev$6xxmjZD7Wqe9zWN1fWzOW1jA4uxAkFQ9rYlVFpxBzVgJ0xEj2KWSLiaRTZzKV1oz'
ADMIN_EMAIL=admin@localhost
ADMIN_PASSWORD=synveda-dev-admin

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

echo "==> ensure the dev bootstrap API key is live (recreates the rauthy volume once if not)"
if ! curl -fsS "$RAUTHY_URL/auth/v1/api_keys/synveda-dev/test" \
  -H "Authorization: $RAUTHY_API_KEY" >/dev/null 2>&1; then
  echo "    bootstrap key not live; rauthy volume predates it — recreating (dev-only state)"
  $COMPOSE stop rauthy
  $COMPOSE rm -f rauthy
  docker volume rm synveda_rauthy-data
  $COMPOSE up --detach rauthy
  wait_rauthy
fi

echo "==> provision the synveda OIDC client (public, PKCE S256, RS256 tokens)"
if ! curl -fsS "$RAUTHY_URL/auth/v1/clients/synveda" \
  -H "Authorization: $RAUTHY_API_KEY" >/dev/null 2>&1; then
  curl -fsS -X POST "$RAUTHY_URL/auth/v1/clients" \
    -H "Authorization: $RAUTHY_API_KEY" -H 'Content-Type: application/json' \
    -d "{\"id\":\"synveda\",\"name\":\"Synveda Gateway\",\"confidential\":false,
         \"redirect_uris\":[\"$GATEWAY_URL/auth/callback\"],
         \"post_logout_redirect_uris\":[]}" >/dev/null
fi
# Idempotent desired state; Rauthy defaults tokens to EdDSA, AUTH-1 verifies
# RS256 (ADR-0010).
curl -fsS -X PUT "$RAUTHY_URL/auth/v1/clients/synveda" \
  -H "Authorization: $RAUTHY_API_KEY" -H 'Content-Type: application/json' \
  -d "{\"id\":\"synveda\",\"name\":\"Synveda Gateway\",\"enabled\":true,
       \"confidential\":false,
       \"redirect_uris\":[\"$GATEWAY_URL/auth/callback\"],
       \"flows_enabled\":[\"authorization_code\"],
       \"access_token_alg\":\"RS256\",\"id_token_alg\":\"RS256\",
       \"auth_code_lifetime\":60,\"access_token_lifetime\":1800,
       \"scopes\":[\"openid\",\"email\",\"profile\",\"groups\"],
       \"default_scopes\":[\"openid\"],
       \"challenges\":[\"S256\"],\"force_mfa\":false}" >/dev/null
echo "    client synveda ready"

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
SYNVEDA_LISTEN_ADDR=127.0.0.1:8120
export SYNVEDA_LISTEN_ADDR
SYNVEDA_PUBLIC_URL=$GATEWAY_URL
export SYNVEDA_PUBLIC_URL
# One auth mode only (ADR-0010): OIDC, never together with the dev secret.
unset SYNVEDA_DEV_JWT_SECRET || true

cargo build -p synveda-gateway -p synveda-cli

echo "==> migrate + admit the tenant this dev IdP binds to (static binding, ADR-0010)"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "auth1-demo-$(date +%s)-$$" --name "AUTH-1 Demo Tenant")
tenant_id=$(echo "$tenant_json" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d).id));
')
echo "    tenant: $tenant_id"

SYNVEDA_OIDC_ISSUERS=$(cat <<EOF
[{"issuer":"$RAUTHY_ISSUER","client_id":"synveda",
  "tenant":{"static":{"tenant_id":"$tenant_id"}}}]
EOF
)
export SYNVEDA_OIDC_ISSUERS

./target/debug/synveda-gateway &
GATEWAY_PID=$!
trap 'kill "$GATEWAY_PID" 2>/dev/null || true' EXIT INT TERM

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS "$GATEWAY_URL/healthz" >/dev/null 2>&1; do
  tries=$((tries + 1))
  if [ "$tries" -ge 30 ]; then
    echo "demo FAILED: gateway did not become healthy" >&2
    exit 1
  fi
  sleep 1
done

echo "==> /auth/login redirects to Rauthy with a PKCE S256 challenge"
authorize_url=$(curl -si "$GATEWAY_URL/auth/login" |
  grep -i '^location:' | sed 's/^[Ll]ocation: //' | tr -d '\r')
case "$authorize_url" in
"$RAUTHY_URL"/auth/v1/oidc/authorize\?*) ;;
*)
  echo "demo FAILED: unexpected authorize redirect: $authorize_url" >&2
  exit 1
  ;;
esac
login_state=$(echo "$authorize_url" | grep -o 'state=[^&]*' | cut -d= -f2)
login_nonce=$(echo "$authorize_url" | grep -o 'nonce=[^&]*' | cut -d= -f2)
login_challenge=$(echo "$authorize_url" | grep -o 'code_challenge=[^&]*' | cut -d= -f2)
echo "    state/nonce/code_challenge present: yes"

echo "==> log in at Rauthy as $ADMIN_EMAIL (session, proof-of-work, credentials)"
page=$(curl -si "$authorize_url")
cookies=$(echo "$page" | grep -i '^set-cookie:' | sed 's/^[Ss]et-[Cc]ookie: //' |
  cut -d';' -f1 | tr -d '\r' | paste -sd'; ' -)
csrf=$(echo "$page" | grep -o '<template id="tpl_csrf_token">[^<]*' | sed 's/.*>//')
# Rauthy requires a solved spow proof-of-work on every login attempt:
# sha256(challenge + counter) with `difficulty` leading zero bits.
pow=$(curl -s -X POST "$RAUTHY_URL/auth/v1/pow" | node -e '
  const crypto = require("crypto");
  let challenge = "";
  process.stdin.on("data", (c) => (challenge += c));
  process.stdin.on("end", () => {
    const difficulty = parseInt(challenge.split(":")[1], 10);
    const zeros = (buf) => {
      let bits = 0;
      for (const byte of buf) {
        if (byte === 0) { bits += 8; continue; }
        bits += Math.clz32(byte) - 24;
        break;
      }
      return bits;
    };
    for (let counter = 0; ; counter++) {
      const digest = crypto.createHash("sha256")
        .update(challenge).update(String(counter)).digest();
      if (zeros(digest) >= difficulty) {
        process.stdout.write(challenge + counter);
        break;
      }
    }
  });
')
login_response=$(curl -si -X POST "$RAUTHY_URL/auth/v1/oidc/authorize" \
  -H 'Content-Type: application/json' \
  -H "Cookie: $cookies" -H "x-csrf-token: $csrf" \
  -d "{\"email\":\"$ADMIN_EMAIL\",\"password\":\"$ADMIN_PASSWORD\",
       \"client_id\":\"synveda\",\"redirect_uri\":\"$GATEWAY_URL/auth/callback\",
       \"state\":\"$login_state\",\"nonce\":\"$login_nonce\",
       \"code_challenge\":\"$login_challenge\",\"code_challenge_method\":\"S256\",
       \"scopes\":[\"openid\",\"profile\",\"email\"],\"pow\":\"$pow\"}")
callback_url=$(echo "$login_response" | grep -i '^location:' |
  sed 's/^[Ll]ocation: //' | tr -d '\r')
if [ -z "$callback_url" ]; then
  echo "demo FAILED: Rauthy login did not return a callback location:" >&2
  echo "$login_response" | head -5 >&2
  exit 1
fi
echo "    Rauthy accepted the login; callback carries the authorization code"

echo "==> AC: the gateway callback yields a Synveda session"
session=$(curl -fsS "$callback_url")
access_token=$(echo "$session" | TENANT_ID="$tenant_id" node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    const s = JSON.parse(d);
    if (!s.access_token || s.tenant.id !== process.env.TENANT_ID
        || s.token_type !== "Bearer" || !s.subject) {
      console.error("unexpected session body: " + d);
      process.exit(1);
    }
    console.error(`    session: subject ${s.subject} in tenant ${s.tenant.slug}`);
    console.log(s.access_token);
  });
')

echo "==> AC: the session bearer works on /v1 (whoami through JWKS verification)"
curl -fsS -H "Authorization: Bearer $access_token" "$GATEWAY_URL/v1/whoami" |
  TENANT_ID="$tenant_id" node -e '
    let d = "";
    process.stdin.on("data", (c) => (d += c));
    process.stdin.on("end", () => {
      const body = JSON.parse(d);
      if (body.tenant.id !== process.env.TENANT_ID) {
        console.error("unexpected whoami body: " + d);
        process.exit(1);
      }
      console.log(`    whoami: ${body.subject} resolved to tenant ${body.tenant.slug}`);
    });
  '

echo "==> uniform 401 still holds without a token"
code=$(curl -s -o /dev/null -w '%{http_code}' "$GATEWAY_URL/v1/whoami")
if [ "$code" != "401" ]; then
  echo "demo FAILED: no-token request returned HTTP $code, want 401" >&2
  exit 1
fi
echo "    401 without a token"

echo "==> /metrics shows the login and the JWKS verification"
metrics=$(curl -fsS "$GATEWAY_URL/metrics")
for want in \
  'synveda_oidc_logins_total{issuer="http://localhost:8100/auth/v1/",outcome="completed"}' \
  'synveda_token_verifications_total{issuer="http://localhost:8100/auth/v1/",outcome="ok"}' \
  'synveda_jwks_refreshes_total{issuer="http://localhost:8100/auth/v1/",outcome="ok"}'; do
  echo "$metrics" | grep -qF "$want" || {
    echo "demo FAILED: missing from /metrics: $want" >&2
    exit 1
  }
done
echo "    oidc_logins completed + token_verifications ok + jwks_refreshes ok"

echo ""
echo "AUTH-1 OIDC login (code+PKCE): acceptance criteria pass."
echo "The mock-Entra half of the AC runs in: cargo test -p synveda-gateway --test oidc_login"
