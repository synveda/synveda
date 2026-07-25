#!/usr/bin/env sh
# ADPT-1 acceptance demo: the Claude Code adapter (ADR-0027).
# AC (docs/backlog/ADPT-1.md): fresh machine to personalised session in
# <2 minutes; demo script.
#
# The AC is a claim about a person, so the demo is split at the person:
#
#   the estate (untimed)   what an operator did once and what any
#                          organisation already has when a developer
#                          arrives — Rauthy with a team group, a tenant,
#                          the acme/eng/platform hierarchy, and team
#                          memory already written at the team scope
#
#   the fresh machine      a clean HOME with no credentials and no
#   (TIMED, budget 120s)   configuration: install the prebuilt plugin,
#                          `synveda login` through the browser flow, and
#                          a session that receives its watermarked block
#                          and contributes its turn back. Alice has never
#                          logged in before, so first-login AUTH-2
#                          provisioning happens inside the budget.
#
#   the proof (untimed)    one audit chain joining `context.injected` and
#                          `memory.observed` under one session id, the
#                          chain verifying, the stored refresh renewing a
#                          spent access token, the observed turn coming
#                          back as memory in a later session, and the
#                          recorded-payload driver (decision 14) run
#                          against this live gateway.
#
# Every hook is run the way `hooks/hooks.json` registers it —
# `node ${CLAUDE_PLUGIN_ROOT}/dist/hook.mjs <mode>` with the recorded
# payload on stdin — so what is timed here is the product, not a script
# pretending to be one.
#
# Embeddings run through the deterministic hash embedder and extraction
# through the rule-based extractor: this demo is about the adapter, and
# the real-TEI path is CTX-1/CTX-3's demo. On Windows, run via Git Bash.
# Needs postgres, jaeger, and rauthy from the dev compose, plus node.
set -eu

cd "$(dirname "$0")/.."
REPO=$(pwd)

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
RAUTHY_URL=http://localhost:8100
RAUTHY_ISSUER=http://localhost:8100/auth/v1/
GATEWAY_URL=http://127.0.0.1:8120
SEED_URL=http://127.0.0.1:8131
# Dev-only bootstrap API key from deploy/compose/rauthy/config.toml.
RAUTHY_API_KEY='API-Key synveda-dev$6xxmjZD7Wqe9zWN1fWzOW1jA4uxAkFQ9rYlVFpxBzVgJ0xEj2KWSLiaRTZzKV1oz'
ALICE_EMAIL=alice@demo.localhost
ALICE_PASSWORD='Auth2demo-Passw0rd!'
TEAM_GROUP=synveda-eng-platform
# The AC's budget, in seconds.
BUDGET_SECS=120
# Short-lived access tokens, so the renewal of decision 6 happens while
# someone is watching rather than half an hour after the demo ends. Rauthy
# will not honour a refresh token until the access token is inside its last
# minute, so this is also the shortest lifetime that leaves a usable
# window: 70 seconds means the renewal lands about ten seconds in.
ACCESS_TOKEN_LIFETIME=70

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

echo "==> the estate: rauthy with a team group, and alice in it"
if ! curl -fsS "$RAUTHY_URL/auth/v1/api_keys/synveda-dev/test" \
  -H "Authorization: $RAUTHY_API_KEY" >/dev/null 2>&1 ||
  ! curl -fsS "$RAUTHY_URL/auth/v1/groups" \
    -H "Authorization: $RAUTHY_API_KEY" >/dev/null 2>&1; then
  echo "    bootstrap key not live or predates Users/Groups rights — recreating the rauthy volume (dev-only state)"
  $COMPOSE stop rauthy
  $COMPOSE rm -f rauthy
  docker volume rm synveda_rauthy-data
  $COMPOSE up --detach rauthy
  wait_rauthy
fi

if ! curl -fsS "$RAUTHY_URL/auth/v1/clients/synveda" \
  -H "Authorization: $RAUTHY_API_KEY" >/dev/null 2>&1; then
  curl -fsS -X POST "$RAUTHY_URL/auth/v1/clients" \
    -H "Authorization: $RAUTHY_API_KEY" -H 'Content-Type: application/json' \
    -d "{\"id\":\"synveda\",\"name\":\"Synveda Gateway\",\"confidential\":false,
         \"redirect_uris\":[\"$GATEWAY_URL/auth/callback\"],
         \"post_logout_redirect_uris\":[]}" >/dev/null
fi
# Idempotent desired state. `refresh_token` in `flows_enabled` is how
# Rauthy grants what decision 6 needs — a login the CLI can keep alive
# without holding client credentials. Note that it does *not* advertise
# `offline_access` in discovery, so the gateway never asks for that scope
# (LoginFlow adds it only where the issuer offers it); on an issuer that
# does, the same login carries the scope instead.
curl -fsS -X PUT "$RAUTHY_URL/auth/v1/clients/synveda" \
  -H "Authorization: $RAUTHY_API_KEY" -H 'Content-Type: application/json' \
  -d "{\"id\":\"synveda\",\"name\":\"Synveda Gateway\",\"enabled\":true,
       \"confidential\":false,
       \"redirect_uris\":[\"$GATEWAY_URL/auth/callback\"],
       \"flows_enabled\":[\"authorization_code\",\"refresh_token\"],
       \"access_token_alg\":\"RS256\",\"id_token_alg\":\"RS256\",
       \"auth_code_lifetime\":60,\"access_token_lifetime\":$ACCESS_TOKEN_LIFETIME,
       \"scopes\":[\"openid\",\"email\",\"profile\",\"groups\"],
       \"default_scopes\":[\"openid\"],
       \"challenges\":[\"S256\"],\"force_mfa\":false}" >/dev/null

groups_json=$(curl -fsS "$RAUTHY_URL/auth/v1/groups" -H "Authorization: $RAUTHY_API_KEY")
if ! printf '%s\n' "$groups_json" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    process.exit(JSON.parse(d).some((g) => g.name === process.argv[1]) ? 0 : 1);
  });
' "$TEAM_GROUP"; then
  curl -fsS -X POST "$RAUTHY_URL/auth/v1/groups" \
    -H "Authorization: $RAUTHY_API_KEY" -H 'Content-Type: application/json' \
    -d "{\"group\":\"$TEAM_GROUP\"}" >/dev/null
fi

users_json=$(curl -fsS "$RAUTHY_URL/auth/v1/users" -H "Authorization: $RAUTHY_API_KEY")
alice_id=$(printf '%s\n' "$users_json" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    const user = JSON.parse(d).find((u) => u.email === process.argv[1]);
    if (user) console.log(user.id);
  });
' "$ALICE_EMAIL")
if [ -z "$alice_id" ]; then
  alice_id=$(curl -fsS -X POST "$RAUTHY_URL/auth/v1/users" \
    -H "Authorization: $RAUTHY_API_KEY" -H 'Content-Type: application/json' \
    -d "{\"email\":\"$ALICE_EMAIL\",\"given_name\":\"Alice\",
         \"family_name\":\"Demo\",\"language\":\"en\",
         \"groups\":[\"$TEAM_GROUP\"],\"roles\":[]}" | json_field id)
fi
# Desired state, twice: with the password for a user that has never had
# one, and without it for a re-run — Rauthy refuses a password it has
# seen in the last three, which is exactly the state a second run is in.
alice_state="{\"email\":\"$ALICE_EMAIL\",\"given_name\":\"Alice\",
       \"family_name\":\"Demo\",\"language\":\"en\",\"roles\":[],
       \"groups\":[\"$TEAM_GROUP\"],\"enabled\":true,\"email_verified\":true"
if ! curl -fsS -X PUT "$RAUTHY_URL/auth/v1/users/$alice_id" \
  -H "Authorization: $RAUTHY_API_KEY" -H 'Content-Type: application/json' \
  -d "$alice_state,\"password\":\"$ALICE_PASSWORD\"}" >/dev/null 2>&1; then
  curl -fsS -X PUT "$RAUTHY_URL/auth/v1/users/$alice_id" \
    -H "Authorization: $RAUTHY_API_KEY" -H 'Content-Type: application/json' \
    -d "$alice_state}" >/dev/null
fi
echo "    alice ($ALICE_EMAIL) is in $TEAM_GROUP and has never logged in to Synveda"

# A scratch database per run: the long-lived dev database carries thousands
# of leftover test tenants, and a demo that asserts a wall-clock budget has
# no business sharing a background indexer with them (the CTX-1 note in
# STATUS).
DEMO_DB=adpt1_demo_$$
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d synveda -c "create database $DEMO_DB" >/dev/null
$COMPOSE exec -T postgres \
  psql -v ON_ERROR_STOP=1 -U synveda -d "$DEMO_DB" -c \
  "create extension if not exists vector;
   create extension if not exists age;
   create extension if not exists pgmq" >/dev/null
DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$DEMO_DB"
export DATABASE_URL
SYNVEDA_PUBLIC_URL=$GATEWAY_URL
export SYNVEDA_PUBLIC_URL
SYNVEDA_SEARCH_INDEX_DIR="./data/adpt1-demo-search-$$"
export SYNVEDA_SEARCH_INDEX_DIR
SYNVEDA_EXTRACTION_POLL_MS=300
export SYNVEDA_EXTRACTION_POLL_MS
# The story here is the adapter's, not the gateway's: warnings and errors
# still print, and the trace detail is in Jaeger where it belongs.
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG

psql_t() {
  $COMPOSE exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U synveda -d "$DEMO_DB" -tAc "$1"
}

echo "==> build the gateway, the CLI, and the plugin"
cargo build -p synveda-gateway -p synveda-cli
# The plugin ships prebuilt and dependency-free (ADR-0027 decision 1): the
# packaging step a release does once, so that enabling it later costs no
# install. `npm test` compiles the same output.
(cd adapters/claude-code && npx tsc -p tsconfig.json)

echo "==> migrate + admit the tenant this dev IdP binds to"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "adpt1-demo-$$" --name "ADPT-1 Demo Tenant")
tenant_id=$(printf '%s\n' "$tenant_json" | json_field id)
echo "    tenant: $tenant_id"

DEMO_HOME=$(mktemp -d "${TMPDIR:-/tmp}/adpt1-home-XXXXXX")
GATEWAY_PID=""
SEED_PID=""
cleanup() {
  [ -n "$GATEWAY_PID" ] && kill "$GATEWAY_PID" 2>/dev/null
  [ -n "$SEED_PID" ] && kill "$SEED_PID" 2>/dev/null
  wait 2>/dev/null || true
  $COMPOSE exec -T postgres psql -U synveda -d synveda \
    -c "drop database if exists $DEMO_DB with (force)" >/dev/null 2>&1 || true
  rm -rf "$SYNVEDA_SEARCH_INDEX_DIR" "$DEMO_HOME"
  return 0
}
trap cleanup EXIT INT TERM

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

echo "==> the estate: acme/eng/platform, through the governed admin API"
SYNVEDA_LISTEN_ADDR=127.0.0.1:8131
export SYNVEDA_LISTEN_ADDR
SYNVEDA_DEV_JWT_SECRET=adpt-1-demo-secret
export SYNVEDA_DEV_JWT_SECRET
./target/debug/synveda-gateway &
SEED_PID=$!
wait_gateway "$SEED_URL"
seed_token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-admin)
# An unbound subject holds no administrative power since AUTHZ-3; the
# operator who admits a tenant is the one who binds its first admin.
./target/debug/synveda role bind --tenant "$tenant_id" \
  --subject demo-admin --role org-admin >/dev/null
create_node() {
  curl -fsS -X POST "$SEED_URL/v1/hierarchy/nodes" \
    -H "Authorization: Bearer $seed_token" -H 'Content-Type: application/json' \
    -d "$1" | json_field id
}
org_id=$(create_node '{"parent_id":null,"kind":"org","slug":"acme","name":"ACME"}')
eng_id=$(create_node "{\"parent_id\":\"$org_id\",\"kind\":\"department\",\"slug\":\"eng\",\"name\":\"Engineering\"}")
team_id=$(create_node "{\"parent_id\":\"$eng_id\",\"kind\":\"team\",\"slug\":\"platform\",\"name\":\"Platform\"}")
kill "$SEED_PID" 2>/dev/null || true
wait "$SEED_PID" 2>/dev/null || true
SEED_PID=""
# One auth mode only (ADR-0010): the dev secret never coexists with OIDC.
unset SYNVEDA_DEV_JWT_SECRET
echo "    acme -> eng -> platform"

echo "==> the estate: the platform team already has memory"
# Records need an owner identity; a curator service identity at the team
# is who authored the team's canonical material (AUTH-3).
curator_json=$(./target/debug/synveda service register --tenant "$tenant_id" \
  --subject platform-curator --scope "$team_id" --name "Platform Curator")
curator_id=$(printf '%s\n' "$curator_json" | json_field id)
vec="[0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1]"
seed_record() {
  # seed_record <kind> <class> <content> — MEM-4's schema backstop refuses
  # an embedding-less record, so the vector rides the same statement.
  psql_t "with new_record as (
            insert into records
              (id, tenant_id, scope_id, owner_id, kind, class, content,
               sensitivity, provenance, valid_from, tx_from)
            values (gen_random_uuid(), '$tenant_id', '$team_id', '$curator_id',
                    '$1', '$2', '$3', 'internal',
                    '{\"source\": \"adpt-1 demo seed\"}', now(), now())
            returning id, tenant_id)
          insert into record_embeddings (record_id, tenant_id, model, dim, embedding)
          select id, tenant_id, 'hash@1', 16, '$vec'::vector from new_record" >/dev/null
}
seed_record pinned procedure "Deploys go through make deploy; never push to main directly."
seed_record derived decision "The platform team settled on blue-green rollouts for the payments service."
echo "    two team records at $team_id, authored before alice ever arrives"

echo "==> the gateway, in OIDC mode"
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
wait_gateway "$GATEWAY_URL"

# ── The fresh machine ────────────────────────────────────────────────────────
#
# Everything from here to the budget assertion is what a developer does,
# on a machine that has never seen Synveda: no credentials file, no state,
# no configuration, no `npm install`.

DEMO_CONFIG="$DEMO_HOME/.config"
DEMO_STATE="$DEMO_HOME/.local/state"
PLUGIN_ROOT="$DEMO_HOME/.claude/plugins/synveda"
PROJECT="$DEMO_HOME/Source/acme-api"
TRANSCRIPT="$DEMO_HOME/.claude/projects/acme-api/session.jsonl"
mkdir -p "$DEMO_CONFIG" "$DEMO_STATE" "$PROJECT" "$(dirname "$TRANSCRIPT")"

# Every command the developer's machine runs, and nothing else: a clean
# HOME, the XDG directories under it, and the `synveda` binary the hooks
# shell out to for a bearer. Note what is NOT here — no SYNVEDA_GATEWAY
# and no SYNVEDA_TOKEN. The login is the configuration (decision 4).
machine() {
  env HOME="$DEMO_HOME" \
    XDG_CONFIG_HOME="$DEMO_CONFIG" \
    XDG_STATE_HOME="$DEMO_STATE" \
    SYNVEDA_CLI="$REPO/target/debug/synveda" \
    "$@"
}

# run_hook <event> <payload> — runs whatever `hooks/hooks.json` registers
# for that event, with the payload on stdin. The command, its arguments,
# and the plugin-root substitution are the harness's; only the 5s timeout
# it also declares is the harness's alone to enforce.
run_hook() {
  printf '%s' "$2" | machine CLAUDE_PLUGIN_ROOT="$PLUGIN_ROOT" HOOK_EVENT="$1" node -e '
    const { readFileSync } = require("node:fs");
    const { spawnSync } = require("node:child_process");
    const root = process.env.CLAUDE_PLUGIN_ROOT;
    const manifest = JSON.parse(readFileSync(root + "/hooks/hooks.json", "utf8"));
    const entry = manifest.hooks[process.env.HOOK_EVENT][0].hooks[0];
    const args = entry.args.map((a) => a.split("${CLAUDE_PLUGIN_ROOT}").join(root));
    const result = spawnSync(entry.command, args, {
      input: readFileSync(0),
      stdio: ["pipe", "inherit", "inherit"],
    });
    process.exit(result.status ?? 1);
  '
}

# payload <fixture> <session-id> — a recorded hook payload, repointed at
# this machine. The recordings are the ones the driver replays
# (adapters/claude-code/fixtures/).
payload() {
  SESSION_ID=$2 PROJECT_DIR="$PROJECT" TRANSCRIPT_PATH="$TRANSCRIPT" node -e '
    const { readFileSync } = require("node:fs");
    const recorded = JSON.parse(readFileSync(process.argv[1], "utf8"));
    console.log(JSON.stringify({
      ...recorded,
      session_id: process.env.SESSION_ID,
      cwd: process.env.PROJECT_DIR,
      transcript_path: process.env.TRANSCRIPT_PATH,
    }));
  ' "adapters/claude-code/fixtures/hooks/$1.json"
}

now_ms() { node -e 'console.log(Date.now())'; }

session_id="0198f100-adp1-7000-8000-$(date +%s)"
started=$(now_ms)

echo
echo "==> [timed] the fresh machine: enabling the prebuilt plugin"
mkdir -p "$PLUGIN_ROOT"
cp -R adapters/claude-code/.claude-plugin adapters/claude-code/hooks \
  adapters/claude-code/dist "$PLUGIN_ROOT/"
echo "    $(cat "$PLUGIN_ROOT/.claude-plugin/plugin.json" | json_field name) plugin in place; no install step, no dependencies"

echo "==> [timed] synveda login (the browser half driven headlessly here)"
LOGIN_LOG="$DEMO_HOME/login.log"
machine ./target/debug/synveda login --gateway "$GATEWAY_URL" --no-browser \
  >"$LOGIN_LOG" 2>&1 &
LOGIN_PID=$!
tries=0
login_url=""
while [ -z "$login_url" ]; do
  login_url=$(grep -o "$GATEWAY_URL/auth/login?[^ ]*" "$LOGIN_LOG" 2>/dev/null | head -1 || true)
  tries=$((tries + 1))
  if [ "$tries" -ge 100 ]; then
    echo "demo FAILED: synveda login printed no login URL:" >&2
    cat "$LOGIN_LOG" >&2
    exit 1
  fi
  [ -z "$login_url" ] && sleep 0.1
done
echo "    the CLI is listening on its loopback port and wants the browser at"
echo "    $(printf '%s\n' "$login_url" | cut -c1-72)..."

# What the browser would do: Rauthy session, proof-of-work, credentials,
# then the gateway callback. Straight from the AUTH-1/AUTH-2 demos.
authorize_url=$(curl -si "$login_url" |
  grep -i '^location:' | sed 's/^[Ll]ocation: //' | tr -d '\r')
case "$authorize_url" in
"$RAUTHY_URL"/auth/v1/oidc/authorize\?*) ;;
*)
  echo "demo FAILED: /auth/login did not redirect to the IdP: $authorize_url" >&2
  exit 1
  ;;
esac
login_state=$(printf '%s\n' "$authorize_url" | grep -o 'state=[^&]*' | cut -d= -f2)
login_nonce=$(printf '%s\n' "$authorize_url" | grep -o 'nonce=[^&]*' | cut -d= -f2)
login_challenge=$(printf '%s\n' "$authorize_url" | grep -o 'code_challenge=[^&]*' | cut -d= -f2)
page=$(curl -si "$authorize_url")
cookies=$(printf '%s\n' "$page" | grep -i '^set-cookie:' | sed 's/^[Ss]et-[Cc]ookie: //' |
  cut -d';' -f1 | tr -d '\r' | paste -sd'; ' -)
csrf=$(printf '%s\n' "$page" | grep -o '<template id="tpl_csrf_token">[^<]*' | sed 's/.*>//')
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
  -d "{\"email\":\"$ALICE_EMAIL\",\"password\":\"$ALICE_PASSWORD\",
       \"client_id\":\"synveda\",\"redirect_uri\":\"$GATEWAY_URL/auth/callback\",
       \"state\":\"$login_state\",\"nonce\":\"$login_nonce\",
       \"code_challenge\":\"$login_challenge\",\"code_challenge_method\":\"S256\",
       \"scopes\":[\"openid\",\"profile\",\"email\",\"groups\"],
       \"pow\":\"$pow\"}")
callback_url=$(printf '%s\n' "$login_response" | grep -i '^location:' |
  sed 's/^[Ll]ocation: //' | tr -d '\r')
if [ -z "$callback_url" ]; then
  echo "demo FAILED: rauthy returned no callback location:" >&2
  printf '%s\n' "$login_response" | head -5 >&2
  exit 1
fi

# The gateway completes AUTH-1 unchanged and hands the CLI a code — not a
# token (ADR-0027 decision 5).
handoff=$(curl -si "$callback_url" | grep -i '^location:' |
  sed 's/^[Ll]ocation: //' | tr -d '\r')
case "$handoff" in
http://127.0.0.1:*/callback\?code=*) ;;
*)
  echo "demo FAILED: the CLI callback was not a loopback handoff: $handoff" >&2
  exit 1
  ;;
esac
case "$handoff" in
*access_token*|*refresh_token*|*Bearer*)
  echo "demo FAILED: a token travelled in the redirect URL: $handoff" >&2
  exit 1
  ;;
esac
curl -fsS "$handoff" >/dev/null
if ! wait "$LOGIN_PID"; then
  echo "demo FAILED: synveda login did not complete:" >&2
  cat "$LOGIN_LOG" >&2
  exit 1
fi
# The subject is the IdP's `sub`, not the email — and where AUTH-2 placed
# it is the part that matters: alice's team membership at the IdP put her
# under the platform team with no administrator involved.
grep -q "logged in as .* at acme/eng/platform/" "$LOGIN_LOG" || {
  echo "demo FAILED: alice was not provisioned into the platform team:" >&2
  cat "$LOGIN_LOG" >&2
  exit 1
}
sed -n 's/^synveda: //p' "$LOGIN_LOG" | sed 's/^/    /'
credentials="$DEMO_CONFIG/synveda/credentials.json"
[ -f "$credentials" ] || {
  echo "demo FAILED: no credentials file at $credentials" >&2
  exit 1
}
mode=$(ls -l "$credentials" | cut -c1-10)
[ "$mode" = "-rw-------" ] || {
  echo "demo FAILED: credentials are $mode, want -rw------- (0600)" >&2
  exit 1
}
echo "    handoff code redeemed over POST; credentials 0600, tokens never in a URL"

echo "==> [timed] the session starts: SessionStart -> /v1/inject"
context=$(run_hook SessionStart "$(payload session-start-startup "$session_id")")
printf '%s\n' "$context" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    const out = JSON.parse(d);
    const text = out.hookSpecificOutput && out.hookSpecificOutput.additionalContext;
    if (!text) {
      console.error("demo FAILED: the session received no context: " + d);
      process.exit(1);
    }
    if (out.hookSpecificOutput.hookEventName !== "SessionStart") {
      console.error("demo FAILED: context is not tagged for SessionStart: " + d);
      process.exit(1);
    }
    for (const want of ["make deploy", "blue-green rollouts"]) {
      if (!text.includes(want)) {
        console.error(`demo FAILED: the team memory "${want}" did not compose:\n` + text);
        process.exit(1);
      }
    }
    console.error("    the block alice never configured:");
    for (const line of text.trim().split("\n")) console.error("      " + line);
  });
'
# A watermark is what makes a block auditable (ADR-0025 decision 7): the
# block hash and the record ids that composed it, in the block itself.
printf '%s\n' "$context" | grep -q "synveda:watermark v1 blake3=" || {
  echo "demo FAILED: the block carries no version watermark" >&2
  exit 1
}

echo "==> [timed] alice works; the turn is written to the transcript"
cat >"$TRANSCRIPT" <<EOF
{"parentUuid":null,"isSidechain":false,"type":"user","message":{"role":"user","content":"We decided to cap payment retries at 3 attempts with full jitter."},"uuid":"7a1b0001-0000-4000-8000-000000000001","timestamp":"$(date -u +%Y-%m-%dT%H:%M:%S.000Z)","userType":"external","cwd":"$PROJECT","sessionId":"$session_id","version":"2.1.220","gitBranch":"feat/retry-budget"}
{"parentUuid":"7a1b0001-0000-4000-8000-000000000001","isSidechain":false,"type":"assistant","message":{"model":"claude-opus-4-5","role":"assistant","content":[{"type":"text","text":"Capped at 3 attempts with full jitter; the deadline stays the caller's."}]},"uuid":"7a1b0001-0000-4000-8000-000000000002","timestamp":"$(date -u +%Y-%m-%dT%H:%M:%S.000Z)","userType":"external","cwd":"$PROJECT","sessionId":"$session_id","version":"2.1.220","gitBranch":"feat/retry-budget"}
EOF

echo "==> [timed] Stop -> /v1/observe, then SessionEnd flushes the rest"
run_hook Stop "$(payload stop "$session_id")"
run_hook SessionEnd "$(payload session-end "$session_id")"
cursor=$(cat "$DEMO_STATE"/synveda/sessions/*.json | json_field cursor)
[ "$cursor" = "7a1b0001-0000-4000-8000-000000000002" ] || {
  echo "demo FAILED: the cursor sits at '$cursor', so the turn was not accepted" >&2
  exit 1
}
observed=$(psql_t "select count(*) from observe_events
                   where tenant_id = '$tenant_id'
                     and session_id = 'claude-code:$session_id'")
[ "$observed" = "2" ] || {
  echo "demo FAILED: expected 2 observed events, buffer holds $observed" >&2
  exit 1
}
echo "    the cursor advanced only on the gateway's 2xx; 2 events buffered"

finished=$(now_ms)
elapsed_ms=$((finished - started))
echo
echo "==> AC: fresh machine to personalised session"
echo "    $((elapsed_ms / 1000)).$(printf '%03d' $((elapsed_ms % 1000)))s of a ${BUDGET_SECS}s budget"
[ "$elapsed_ms" -lt "$((BUDGET_SECS * 1000))" ] || {
  echo "demo FAILED: the fresh-machine run took longer than the AC allows" >&2
  exit 1
}

# ── The proof ────────────────────────────────────────────────────────────────

echo
echo "==> one audit chain, one session id, both directions of the spine"
./target/debug/synveda audit tail --tenant "$tenant_id" --limit 4
joined=$(psql_t "select count(distinct action) from audit_log
                 where tenant_id = '$tenant_id'
                   and payload->>'session_id' = 'claude-code:$session_id'
                   and action in ('context.injected', 'memory.observed')")
[ "$joined" = "2" ] || {
  echo "demo FAILED: context.injected and memory.observed do not join on the session id" >&2
  exit 1
}
leaked=$(psql_t "select count(*) from audit_log
                 where tenant_id = '$tenant_id'
                   and payload::text like '%$(printf '%s\n' "$ALICE_PASSWORD" | head -c 8)%'")
[ "$leaked" = "0" ] || {
  echo "demo FAILED: a credential reached the audit log" >&2
  exit 1
}
echo "    context.injected + memory.observed under claude-code:$session_id"
./target/debug/synveda audit verify --tenant "$tenant_id"

echo
echo "==> the login keeps itself alive: the access token renews itself"
if node -e '
  const { readFileSync } = require("node:fs");
  const stored = JSON.parse(readFileSync(process.argv[1], "utf8"));
  process.exit(stored.profiles.default.refresh_token ? 0 : 1);
' "$credentials"; then
  # This demo's IdP client issues ${ACCESS_TOKEN_LIFETIME}s access tokens
  # precisely so the renewal is watchable: the CLI refreshes inside the
  # last minute of a token's life, and Rauthy will not honour a refresh
  # token before that same minute begins. Anything the CLI hands out in
  # between is the stored token, which is still valid — that fallback is
  # what keeps a session's memory through the boundary.
  before=$(machine ./target/debug/synveda auth token)
  after=$before
  tries=0
  while [ "$after" = "$before" ]; do
    tries=$((tries + 1))
    if [ "$tries" -ge 40 ]; then
      echo "demo FAILED: the access token never renewed" >&2
      exit 1
    fi
    sleep 2
    after=$(machine ./target/debug/synveda auth token)
  done
  code=$(curl -s -o /dev/null -w '%{http_code}' \
    -H "Authorization: Bearer $after" "$GATEWAY_URL/v1/whoami")
  [ "$code" = "200" ] || {
    echo "demo FAILED: the refreshed bearer was refused with HTTP $code" >&2
    exit 1
  }
  echo "    renewed through the gateway, with no login and no client credentials"
else
  echo "    skipped: this issuer granted no refresh token (decision 6's documented degradation)"
fi

echo
echo "==> the loop closes: what alice said comes back as memory"
tries=0
while :; do
  have=$(psql_t "select count(*) from records
                 where tenant_id = '$tenant_id'
                   and content like '%cap payment retries%'")
  [ "$have" != "0" ] && break
  tries=$((tries + 1))
  if [ "$tries" -ge 120 ]; then
    echo "demo FAILED: the observed turn never became a record" >&2
    exit 1
  fi
  sleep 0.5
done
later=$(run_hook SessionStart "$(payload session-start-startup "${session_id}-next")")
printf '%s\n' "$later" | grep -q "cap payment retries" || {
  echo "demo FAILED: the next session did not receive the memory it wrote:" >&2
  echo "$later" >&2
  exit 1
}
echo "    observe -> redact -> extract -> embed -> derived -> inject, in one session's turn"

echo
echo "==> the recorded-payload driver (decision 14) against this live gateway"
bearer=$(machine ./target/debug/synveda auth token)
machine node adapters/claude-code/dist/driver.mjs \
  --gateway "$GATEWAY_URL" --token "$bearer" --expect-context

echo
echo "ADPT-1 Claude Code adapter: acceptance criteria pass."
echo "A machine with no credentials and no configuration reached a"
echo "personalised, governed, fully audited session in"
echo "$((elapsed_ms / 1000)).$(printf '%03d' $((elapsed_ms % 1000)))s — SSO login through the gateway (AUTH-1 unchanged,"
echo "AUTH-2 placing alice in the platform team on first sight), a"
echo "watermarked block composed from memory she never configured, her"
echo "turn observed back into the derived channel, and both halves joined"
echo "in one verifying audit chain by session id. The mock half of the"
echo "driver runs in: (cd adapters/claude-code && npm test)"
