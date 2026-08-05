#!/usr/bin/env sh
# AUTH-2 acceptance demo: JIT user provisioning from claims, live against
# the bundled Rauthy (ADR-0013). AC (docs/backlog/AUTH-2.md): a new user
# lands in the correct team scope with zero admin action; unmapped users
# land in the quarantine scope with no read rights.
#
# Flow: provision Rauthy (client with the groups scope, group
# synveda-eng-platform, user alice in it) -> seed the acme/eng/platform
# hierarchy through the admin API (dev-mode gateway phase) -> boot the
# gateway in OIDC mode -> alice's first login lands her personal scope
# under acme/eng/platform and her bearer can read -> the admin (no synveda
# group) lands under acme/quarantine and the PDP denies reads (403, not
# 401) -> metrics show both provisioning outcomes. The CI-clean mock-IdP
# half of the AC runs in crates/synveda-gateway/tests/jit_provisioning.rs.
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
# Per run, and deleted on the way out.
#
# This demo's whole subject is a person Synveda has never seen, and a shared
# identity is not new on the second run. It also used to share
# `alice@demo.localhost` with `demos/adpt-1-claude-code.sh`, so each demo
# inherited the other's leftovers — which is how one password that Rauthy
# would not re-set took both of them down at once.
#
# The repo already settled this shape: CNSL-1 uses `reviewer-$$@…`, OPS-1
# derives its operator from the tenant slug, and `synveda init`'s
# `operator_email` carries a test named
# `two_deployments_on_one_laptop_do_not_share_an_operator`. This is that
# rule reaching the two demos that predate it.
#
# The **group** stays shared on purpose: it holds no mutable state, it is
# created idempotently, and its name is load-bearing — `convention_candidates`
# parses `synveda-<department>-<team>` onto hierarchy slugs (ADR-0013), so a
# per-run group would drag per-run team slugs behind it for no benefit.
ALICE_EMAIL="alice-$$@demo.localhost"
ALICE_PASSWORD='Auth2demo-Passw0rd!'
ALICE_ID=""
TEAM_GROUP=synveda-eng-platform

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

echo "==> ensure the dev API key is live with Users/Groups access (AUTH-2 widened it)"
if ! curl -fsS "$RAUTHY_URL/auth/v1/api_keys/synveda-dev/test" \
  -H "Authorization: $RAUTHY_API_KEY" >/dev/null 2>&1 ||
  ! curl -fsS "$RAUTHY_URL/auth/v1/groups" \
    -H "Authorization: $RAUTHY_API_KEY" >/dev/null 2>&1; then
  echo "    key not live or predates Users/Groups rights — recreating the rauthy volume (dev-only state)"
  $COMPOSE stop rauthy
  $COMPOSE rm -f rauthy
  docker volume rm synveda_rauthy-data
  $COMPOSE up --detach rauthy
  wait_rauthy
fi

echo "==> provision the synveda OIDC client (public, PKCE S256, RS256, groups scope)"
if ! curl -fsS "$RAUTHY_URL/auth/v1/clients/synveda" \
  -H "Authorization: $RAUTHY_API_KEY" >/dev/null 2>&1; then
  curl -fsS -X POST "$RAUTHY_URL/auth/v1/clients" \
    -H "Authorization: $RAUTHY_API_KEY" -H 'Content-Type: application/json' \
    -d "{\"id\":\"synveda\",\"name\":\"Synveda Gateway\",\"confidential\":false,
         \"redirect_uris\":[\"$GATEWAY_URL/auth/callback\"],
         \"post_logout_redirect_uris\":[]}" >/dev/null
fi
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

echo "==> provision the $TEAM_GROUP group and user alice inside it"
groups_json=$(curl -fsS "$RAUTHY_URL/auth/v1/groups" -H "Authorization: $RAUTHY_API_KEY")
if ! echo "$groups_json" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    const groups = JSON.parse(d);
    process.exit(groups.some((g) => g.name === process.argv[1]) ? 0 : 1);
  });
' "$TEAM_GROUP"; then
  curl -fsS -X POST "$RAUTHY_URL/auth/v1/groups" \
    -H "Authorization: $RAUTHY_API_KEY" -H 'Content-Type: application/json' \
    -d "{\"group\":\"$TEAM_GROUP\"}" >/dev/null
fi

# A leaked identity from a crashed run, or a reused pid, would put us back
# in the state that took this demo down: Rauthy will not re-set a password
# it has seen in its last three, and a constant that is in the history
# without being current locks every future run out. One DELETE removes that
# possibility, so nothing below needs a recovery path.
stale_id=$(curl -fsS "$RAUTHY_URL/auth/v1/users" -H "Authorization: $RAUTHY_API_KEY" |
  node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    const user = JSON.parse(d).find((u) => u.email === process.argv[1]);
    if (user) console.log(user.id);
  });
' "$ALICE_EMAIL")
if [ -n "$stale_id" ]; then
  curl -fsS -X DELETE "$RAUTHY_URL/auth/v1/users/$stale_id" \
    -H "Authorization: $RAUTHY_API_KEY" >/dev/null
fi
alice_id=$(curl -fsS -X POST "$RAUTHY_URL/auth/v1/users" \
  -H "Authorization: $RAUTHY_API_KEY" -H 'Content-Type: application/json' \
  -d "{\"email\":\"$ALICE_EMAIL\",\"given_name\":\"Alice\",
       \"family_name\":\"Demo\",\"language\":\"en\",
       \"groups\":[\"$TEAM_GROUP\"],\"roles\":[]}" | json_field id)
ALICE_ID=$alice_id
# Group membership, a known password, verified email so the login flow
# needs no mailbox.
curl -fsS -X PUT "$RAUTHY_URL/auth/v1/users/$alice_id" \
  -H "Authorization: $RAUTHY_API_KEY" -H 'Content-Type: application/json' \
  -d "{\"email\":\"$ALICE_EMAIL\",\"given_name\":\"Alice\",
       \"family_name\":\"Demo\",\"language\":\"en\",
       \"password\":\"$ALICE_PASSWORD\",\"roles\":[],
       \"groups\":[\"$TEAM_GROUP\"],\"enabled\":true,
       \"email_verified\":true}" >/dev/null
echo "    alice ($ALICE_EMAIL) is in $TEAM_GROUP; the rauthy admin is in no synveda group"

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
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
  --slug "auth2-demo-$(date +%s)-$$" --name "AUTH-2 Demo Tenant")
tenant_id=$(echo "$tenant_json" | json_field id)
tenant_slug=$(echo "$tenant_json" | json_field slug)
echo "    tenant: $tenant_id ($tenant_slug)"

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

echo "==> phase 1: seed the acme/eng/platform hierarchy (dev-mode gateway, admin API)"
SYNVEDA_LISTEN_ADDR=127.0.0.1:8131
export SYNVEDA_LISTEN_ADDR
SYNVEDA_DEV_JWT_SECRET=auth-2-demo-secret
export SYNVEDA_DEV_JWT_SECRET
# The run's own identity goes with it. OPS-1 leaves its per-run operators
# behind and the dev IdP has been accumulating them since; a per-run fixture
# that is never removed is a leak with a nicer name.
drop_alice() {
  [ -n "$ALICE_ID" ] && curl -fsS -X DELETE "$RAUTHY_URL/auth/v1/users/$ALICE_ID" \
    -H "Authorization: $RAUTHY_API_KEY" >/dev/null 2>&1
  return 0
}
./target/debug/synveda-gateway &
SEED_PID=$!
trap 'kill "$SEED_PID" 2>/dev/null || true; drop_alice' EXIT INT TERM
wait_gateway http://127.0.0.1:8131
seed_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-admin)
# A token is not an authority. The seeding subject needs `org-admin` bound
# to it or the PDP denies `hierarchy.create` — a tenant with no assignment
# falls back to `regulated-strict`, which grants nothing to nobody. Every
# other demo binds this; AUTH-2 predates the packs having teeth and was
# never updated, so it died at the first `create_node` with a bare 403.
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null
create_node() {
  curl -fsS -X POST http://127.0.0.1:8131/v1/hierarchy/nodes \
    -H "Authorization: Bearer $seed_token" -H 'Content-Type: application/json' \
    -d "$1" | json_field id
}
org_id=$(create_node '{"parent_id":null,"kind":"org","slug":"acme","name":"ACME"}')
eng_id=$(create_node "{\"parent_id\":\"$org_id\",\"kind\":\"department\",\"slug\":\"eng\",\"name\":\"Engineering\"}")
create_node "{\"parent_id\":\"$eng_id\",\"kind\":\"team\",\"slug\":\"platform\",\"name\":\"Platform\"}" >/dev/null
kill "$SEED_PID" 2>/dev/null || true
wait "$SEED_PID" 2>/dev/null || true
unset SYNVEDA_DEV_JWT_SECRET
echo "    hierarchy ready: acme -> eng -> platform (no admin action follows)"

echo "==> phase 2: gateway in OIDC mode (groups scope requested at login)"
SYNVEDA_LISTEN_ADDR=127.0.0.1:8120
export SYNVEDA_LISTEN_ADDR
SYNVEDA_OIDC_ISSUERS=$(cat <<EOF
[{"issuer":"$RAUTHY_ISSUER","client_id":"synveda",
  "tenant":{"static":{"tenant_id":"$tenant_id"}},
  "login_scopes":["openid","profile","email","groups"]}]
EOF
)
export SYNVEDA_OIDC_ISSUERS
./target/debug/synveda-gateway &
GATEWAY_PID=$!
trap 'kill "$GATEWAY_PID" 2>/dev/null || true; kill "$SEED_PID" 2>/dev/null || true; drop_alice' EXIT INT TERM
wait_gateway "$GATEWAY_URL"

# login <email> <password> — drives the full code+PKCE flow (Rauthy
# session, proof-of-work, credentials) and prints the session JSON.
login() {
  authorize_url=$(curl -si "$GATEWAY_URL/auth/login" |
    grep -i '^location:' | sed 's/^[Ll]ocation: //' | tr -d '\r')
  login_state=$(echo "$authorize_url" | grep -o 'state=[^&]*' | cut -d= -f2)
  login_nonce=$(echo "$authorize_url" | grep -o 'nonce=[^&]*' | cut -d= -f2)
  login_challenge=$(echo "$authorize_url" | grep -o 'code_challenge=[^&]*' | cut -d= -f2)
  page=$(curl -si "$authorize_url")
  cookies=$(echo "$page" | grep -i '^set-cookie:' | sed 's/^[Ss]et-[Cc]ookie: //' |
    cut -d';' -f1 | tr -d '\r' | paste -sd'; ' -)
  csrf=$(echo "$page" | grep -o '<template id="tpl_csrf_token">[^<]*' | sed 's/.*>//')
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
    -d "{\"email\":\"$1\",\"password\":\"$2\",
         \"client_id\":\"synveda\",\"redirect_uri\":\"$GATEWAY_URL/auth/callback\",
         \"state\":\"$login_state\",\"nonce\":\"$login_nonce\",
         \"code_challenge\":\"$login_challenge\",\"code_challenge_method\":\"S256\",
         \"scopes\":[\"openid\",\"profile\",\"email\",\"groups\"],\"pow\":\"$pow\"}")
  callback_url=$(echo "$login_response" | grep -i '^location:' |
    sed 's/^[Ll]ocation: //' | tr -d '\r')
  if [ -z "$callback_url" ]; then
    echo "demo FAILED: Rauthy login for $1 returned no callback location:" >&2
    echo "$login_response" | head -5 >&2
    exit 1
  fi
  curl -fsS "$callback_url"
}

echo "==> AC 1: alice's first login lands in the platform team, zero admin action"
alice_session=$(login "$ALICE_EMAIL" "$ALICE_PASSWORD")
alice_token=$(echo "$alice_session" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    const s = JSON.parse(d);
    const path = s.identity && s.identity.scope_path;
    if (!path || s.identity.quarantined !== false ||
        !path.startsWith("acme/eng/platform/")) {
      console.error("unexpected session: " + d);
      process.exit(1);
    }
    console.error(`    provisioned: ${s.subject} -> ${path} (quarantined: false)`);
    console.log(s.access_token);
  });
')
code=$(curl -s -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer $alice_token" "$GATEWAY_URL/v1/hierarchy/root")
if [ "$code" != "200" ]; then
  echo "demo FAILED: a placed user's read returned HTTP $code, want 200" >&2
  exit 1
fi
echo "    alice's bearer reads the hierarchy: 200"

echo "==> AC 2: the rauthy admin (no synveda group) lands in quarantine, no read rights"
admin_session=$(login "$ADMIN_EMAIL" "$ADMIN_PASSWORD")
admin_token=$(echo "$admin_session" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    const s = JSON.parse(d);
    const path = s.identity && s.identity.scope_path;
    if (!path || s.identity.quarantined !== true ||
        !path.startsWith("acme/quarantine/")) {
      console.error("unexpected session: " + d);
      process.exit(1);
    }
    console.error(`    provisioned: ${s.subject} -> ${path} (quarantined: true)`);
    console.log(s.access_token);
  });
')
code=$(curl -s -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer $admin_token" "$GATEWAY_URL/v1/hierarchy/root")
if [ "$code" != "403" ]; then
  echo "demo FAILED: quarantined read returned HTTP $code, want 403 (policy denial)" >&2
  exit 1
fi
echo "    quarantined bearer is policy-denied reads: 403 (authenticated, contained)"

echo "==> /metrics shows both provisioning outcomes"
metrics=$(curl -fsS "$GATEWAY_URL/metrics")
for want in \
  'synveda_jit_provisions_total{outcome="mapped"}' \
  'synveda_jit_provisions_total{outcome="quarantined"}'; do
  echo "$metrics" | grep -qF "$want" || {
    echo "demo FAILED: missing from /metrics: $want" >&2
    exit 1
  }
done
echo "    jit_provisions mapped + quarantined"

echo ""
echo "AUTH-2 JIT user provisioning: acceptance criteria pass."
echo "The CI-clean mock-IdP half runs in: cargo test -p synveda-gateway --test jit_provisioning"
