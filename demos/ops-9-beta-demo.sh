#!/usr/bin/env sh
# OPS-9 acceptance demo: the beta demo profile (ADR-0066).
#
# AC (docs/backlog/OPS-9.md): on a scratch HOME with no checkout and no Rust
# toolchain, install → `synveda init --demo` → `synveda login` →
# `demo/seed.sh` → a recall that returns seeded memory, a console signed in
# to, an inbox holding the pending proposal, and a chain that verifies with
# exactly one break-glass event. Plus: a second run changes nothing, and a
# tenant holding a foreign organisation is refused.
#
# It does **not** run `install.sh` — OPS-8's demo already proves the released
# path end to end, on a scratch HOME with `cargo`/`rustc`/`rustup` shimmed to
# exit 127, and re-running a twenty-minute release install here would spend
# CI re-proving that criterion. What it does instead is assemble a scratch
# *bundle* from this tree and run against that, which isolates its state
# (see below) and has a second payoff: `seed.sh` is executed from
# `$SYNVEDA_HOME/profile/demo/`, the installed path, rather than from the
# checkout. OPS-9's acceptance criteria recorded that path as unproven; this
# closes it, and it was free, because isolation needed the bundle anyway.
#
# The images are still the dev compose's, because a packaged release profile
# pulls published tags and this runs against a tree that may be ahead of any.
#
# Needs: docker, node, and both binaries built (`cargo build -p synveda-cli
# -p synveda-gateway`).
set -eu

cd "$(dirname "$0")/.."
REPO=$(pwd)

# COMPOSE and SEEDER are set once the scratch bundle exists, below.
RAUTHY_URL=http://localhost:8100
GATEWAY_URL=http://127.0.0.1:8120
SLUG=ops9-$$
OPERATOR="operator@$SLUG.localhost"
PASSWORD='Synveda-Demo-Passw0rd!'

SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/ops9-XXXXXX")

# ── Isolation, and the one part of it that is not achievable ────────────
#
# This demo used to run against the checkout's own profile, which made it
# destructive to any deployment already on this machine — and not
# theoretically. It rebinds the gateway's **static tenant**: the bundled
# Rauthy binds every login from its issuer to one configured tenant
# (`TenantBinding::Static`, ADR-0010 §4), so a demo run pointed the shared
# gateway at its own throwaway tenant and left it there. A developer's
# working deployment then answered every `synveda` command with a policy
# denial against a tenant they had never heard of.
#
# What is isolated here:
#
#   * **Compose project** — `COMPOSE_PROJECT_NAME` of its own, so this
#     never adopts, stops or removes another project's containers. Both
#     the dev compose and the released profile are `name: synveda`, which
#     is right for each of them and wrong for two at once (the OPS-8
#     lesson).
#   * **State** — the demo builds a scratch *bundle* profile and points
#     `SYNVEDA_COMPOSE_FILE` at it. `Profile::from_explicit_compose_file`
#     returns `Bundle` when a `version` file sits beside the compose file,
#     which moves `data/` — the pidfile, the rendered gateway environment
#     that carries the tenant binding, and `kms.key` — under the scratch
#     home instead of the checkout's.
#   * **HOME**, so the stored credential is this demo's own.
#
# What is **not** isolated, and cannot be without a product change:
# **the ports**. `GATEWAY_URL` and `RAUTHY_ISSUER` are constants in
# `init.rs`, and the issuer is compared byte-for-byte against the discovery
# document and the `iss` claim (ADR-0010), so moving Rauthy off 8100 means
# reissuing the issuer everywhere it is checked. That is a real change with
# a real blast radius and not one a demo should motivate. So this refuses
# to start when something already holds a port it needs, and names what to
# stop — which is the honest half of isolation and is what protects the
# deployment it used to break.
export COMPOSE_PROJECT_NAME="ops9-$$"

# A scratch HOME, so the credential this demo stores is its own and a
# developer's real `~/.synveda` is neither read nor written.
#
# `DOCKER_CONFIG` is captured from the *real* home first, and that line is
# load-bearing rather than tidy — OPS-8's demo learned it and this one
# re-learned it by failing. Docker keeps its contexts and its **CLI plugins**
# under `~/.docker`, so a scratch HOME loses both: OrbStack's context falls
# back to `unix:///var/run/docker.sock`, which is not where its daemon is,
# and `docker compose` stops being a subcommand at all — the error is
# `unknown shorthand flag: 'f' in -f`, which reads like a malformed command
# rather than a missing plugin.
export DOCKER_CONFIG="${DOCKER_CONFIG:-$HOME/.docker}"
export HOME="$SCRATCH/home"
mkdir -p "$HOME"

cleanup() {
  status=$?
  # Disarmed first, then re-exited with the status we came in with — the
  # OPS-8 lesson: a trap that resets `$?` turns a failed demo into exit 0,
  # which is how that one first reported a failure and passed.
  trap - EXIT INT TERM
  if [ -n "${OPS9_KEEP:-}" ]; then
    echo ""
    echo "OPS9_KEEP set — left running as compose project $COMPOSE_PROJECT_NAME"
    echo "  scratch home: $SCRATCH"
    echo "  tear down:    docker compose -p $COMPOSE_PROJECT_NAME -f $SCRATCH/home/.synveda/profile/docker-compose.yml down -v"
    exit "$status"
  fi
  # Its own project and its own volumes, so this takes everything it made
  # and nothing it did not — the isolation is what makes an unconditional
  # `down -v` safe here, where against a shared stack it would be a
  # developer's database.
  if [ -f "$SCRATCH/home/.synveda/data/gateway.pid" ]; then
    kill "$(cat "$SCRATCH/home/.synveda/data/gateway.pid")" 2>/dev/null || true
  fi
  if [ -f "$SCRATCH/home/.synveda/profile/docker-compose.yml" ]; then
    docker compose -p "$COMPOSE_PROJECT_NAME" \
      -f "$SCRATCH/home/.synveda/profile/docker-compose.yml" down -v >/dev/null 2>&1 || true
  fi
  rm -rf "$SCRATCH"
  exit "$status"
}
trap cleanup EXIT INT TERM

fail() { echo "" >&2; echo "demo FAILED: $1" >&2; exit 1; }
step() { echo ""; echo "==> $1"; }

SYNVEDA="$REPO/target/release/synveda"
[ -x "$SYNVEDA" ] || SYNVEDA="$REPO/target/debug/synveda"
[ -x "$SYNVEDA" ] || fail "no synveda binary — cargo build -p synveda-cli first"
export SYNVEDA_BIN_PATH="$SYNVEDA"

GATEWAY_BIN="$REPO/target/release/synveda-gateway"
[ -x "$GATEWAY_BIN" ] || GATEWAY_BIN="$REPO/target/debug/synveda-gateway"
[ -x "$GATEWAY_BIN" ] ||
  fail "no synveda-gateway binary — cargo build -p synveda-gateway first"

# The console bundle is a prerequisite of this demo the same way the binaries
# are, because the sign-in step below is an assertion about a served console
# and ADR-0056 decision 1 makes an absent bundle a 404 rather than a boot
# failure. It used to be copied conditionally and checked 250 lines later,
# which meant a machine without it got "GET /console/ returned 404 — no
# bundle?" long after the stack was up, instead of a named prerequisite before
# anything started. Checked here, in the same shape and for the same reason as
# the two above.
CONSOLE_DIST="$REPO/console/dist"
[ -f "$CONSOLE_DIST/index.html" ] ||
  fail "no console bundle — pnpm install && pnpm --filter @synveda/console build first"

# ── the ports this demo needs ───────────────────────────────────────────
# Checked before anything starts, because a half-started stack reporting a
# health check that timed out is a worse message than a refusal that names
# the port and the thing holding it. `curl` rather than `/dev/tcp`, which is
# a bashism and this is `sh`: exit 7 is "failed to connect", anything else
# means something answered.
port_free() { # port
  curl -s -o /dev/null --max-time 2 "http://127.0.0.1:$1" 2>/dev/null
  [ "$?" = 7 ]
}
for probe in "8120 the gateway" "8100 Rauthy" "5432 Postgres"; do
  port=${probe%% *}
  what=${probe#* }
  port_free "$port" || fail "port $port is in use, and this demo needs it for $what.

  The ports are constants in the product rather than settings — the IdP's
  issuer is compared byte-for-byte (ADR-0010) — so this demo cannot move
  aside, and it refuses rather than adopting or restarting whatever is
  there. That used to be the bug: it rebound a running deployment's tenant.

  If that is your own deployment, stop it first:
      docker compose -f deploy/compose/docker-compose.yml down
      kill \$(cat data/gateway.pid 2>/dev/null)   # the host gateway"
done

# ── a scratch bundle, so `init` writes its state somewhere disposable ───
# `Profile::from_explicit_compose_file` returns `Bundle` when a `version`
# file sits beside the compose file, and a `Bundle` keeps `data/`, `bin/`
# and `console/` under its own home — which is the whole point here. The
# compose file is the **dev** one rather than a packaged release profile,
# because a release profile pulls published images and this runs against a
# tree that may be ahead of any tag.
INSTALL_ROOT="$HOME/.synveda"
mkdir -p "$INSTALL_ROOT/profile/rauthy" "$INSTALL_ROOT/bin"
cp "$REPO/deploy/compose/docker-compose.yml" "$INSTALL_ROOT/profile/docker-compose.yml"
cp "$REPO/deploy/compose/rauthy/config.toml" "$INSTALL_ROOT/profile/rauthy/config.toml"
# Every relative path in that compose file is resolved against *its* directory,
# so copying the file alone copies a set of dangling references. `rauthy` is
# above; `postgres` is this, and it is the one that only shows up on a machine
# that has never run `make dev-up`. Compose skips a `build:` when the image is
# already present, so on a developer's laptop the missing context is invisible
# and on a clean runner it is `unable to prepare context: … /profile/postgres
# not found`. That is what happened: this demo passed here and failed on its
# first CI run, which is the exact asymmetry it was written to catch.
#
# The other two dangling paths — `./temporal/dynamicconfig.yaml` and the
# gateway's `context: ../..` — belong to services this demo never starts, and
# compose resolves neither for a service it is not asked to bring up. They are
# left dangling deliberately rather than papered over: copying them would imply
# this bundle can serve them, and it cannot (the gateway here runs on the host,
# ADR-0055 decision 8).
cp -R "$REPO/deploy/compose/postgres" "$INSTALL_ROOT/profile/postgres"
# The version `check_version` compares against the CLI's own. A mismatch is
# refused in both directions, so this asks the binary rather than hardcoding.
"$SYNVEDA" --version | awk '{print $2}' > "$INSTALL_ROOT/profile/version"
cp "$GATEWAY_BIN" "$INSTALL_ROOT/bin/synveda-gateway"
# The console, so the sign-in step below has something to serve. Unconditional
# now: its absence is a named prerequisite failure above, not a silent skip.
cp -R "$CONSOLE_DIST" "$INSTALL_ROOT/console"
# The seeder, into the bundle, and **run from there** rather than from the
# tree. This is the one thing OPS-9's acceptance criteria recorded as
# unproven — "nobody has run seed.sh from $SYNVEDA_HOME/profile/demo/" — and
# building a bundle for isolation happens to make proving it free.
mkdir -p "$INSTALL_ROOT/profile/demo"
cp "$REPO/deploy/release/demo/seed.sh" "$INSTALL_ROOT/profile/demo/seed.sh"
cp "$REPO/deploy/release/demo/organisation.txt" "$INSTALL_ROOT/profile/demo/organisation.txt"
chmod +x "$INSTALL_ROOT/profile/demo/seed.sh"
SEEDER="$INSTALL_ROOT/profile/demo/seed.sh"

export SYNVEDA_COMPOSE_FILE="$INSTALL_ROOT/profile/docker-compose.yml"
COMPOSE="docker compose -f $SYNVEDA_COMPOSE_FILE"

psql_t() {
  $COMPOSE exec -T postgres psql -U synveda -d synveda -tAc "$1" 2>/dev/null | tr -d '\r'
}

# Rauthy's proof-of-work, solved. Its own login page does this in the
# browser; a scripted login has to do it too or `/oidc/authorize` refuses.
pow() {
  curl -s -X POST "$RAUTHY_URL/auth/v1/pow" | node -e '
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
        const digest = crypto.createHash("sha256").update(challenge).update(String(counter)).digest();
        if (zeros(digest) >= difficulty) { process.stdout.write(challenge + counter); break; }
      }
    });
  '
}

# Everything a browser would do between `/auth/login…` and the gateway's
# callback: follow the redirect to the IdP, read its CSRF token, solve the
# proof of work, post the operator's password, and hand back the callback
# URL the IdP redirects to.
#
# Shared by the CLI login and the console sign-in below, which differ only
# in the `/auth/login` query string — and factoring it is what made adding
# the console one cheap enough to actually do. This demo's header claimed a
# console sign-in for a while when the demo never touched the console.
idp_password_login() { # login_url -> callback url on stdout
  _authorize=$(curl -si "$1" | grep -i '^location:' | sed 's/^[Ll]ocation: //' | tr -d '\r')
  case "$_authorize" in
  "$RAUTHY_URL"/auth/v1/oidc/authorize\?*) ;;
  *) fail "/auth/login did not redirect to the IdP: $_authorize" ;;
  esac
  _state=$(printf '%s\n' "$_authorize" | grep -o 'state=[^&]*' | cut -d= -f2)
  _nonce=$(printf '%s\n' "$_authorize" | grep -o 'nonce=[^&]*' | cut -d= -f2)
  _challenge=$(printf '%s\n' "$_authorize" | grep -o 'code_challenge=[^&]*' | cut -d= -f2)
  _page=$(curl -si "$_authorize")
  _cookies=$(printf '%s\n' "$_page" | grep -i '^set-cookie:' | sed 's/^[Ss]et-[Cc]ookie: //' |
    cut -d';' -f1 | tr -d '\r' | paste -sd'; ' -)
  _csrf=$(printf '%s\n' "$_page" | grep -o '<template id="tpl_csrf_token">[^<]*' | sed 's/.*>//')
  curl -si -X POST "$RAUTHY_URL/auth/v1/oidc/authorize" \
    -H 'Content-Type: application/json' \
    -H "Cookie: $_cookies" -H "x-csrf-token: $_csrf" \
    -d "{\"email\":\"$OPERATOR\",\"password\":\"$PASSWORD\",
         \"client_id\":\"synveda\",\"redirect_uri\":\"$GATEWAY_URL/auth/callback\",
         \"state\":\"$_state\",\"nonce\":\"$_nonce\",
         \"code_challenge\":\"$_challenge\",\"code_challenge_method\":\"S256\",
         \"scopes\":[\"openid\",\"profile\",\"email\",\"groups\"],
         \"pow\":\"$(pow)\"}" |
    grep -i '^location:' | sed 's/^[Ll]ocation: //' | tr -d '\r'
}

# ── the deployment ──────────────────────────────────────────────────────
step "a dev stack and a tenant"
# Output kept rather than sent to /dev/null. The first run of this demo
# failed here and printed `demo FAILED: docker compose up` and nothing else,
# which is a message that says a thing went wrong and refuses to say what —
# the actual error was `unknown shorthand flag: 'f'`, and it was one line
# away the whole time.
$COMPOSE up -d postgres rauthy >"$SCRATCH/compose.log" 2>&1 ||
  { sed 's/^/    /' "$SCRATCH/compose.log" >&2; fail "docker compose up"; }
tries=0
until psql_t "select 1" >/dev/null 2>&1; do
  tries=$((tries + 1))
  [ "$tries" -ge 60 ] && fail "postgres never came up"
  sleep 1
done
echo "    postgres and rauthy up"

"$SYNVEDA" init --demo --slug "$SLUG" --name "ACME ($SLUG)" >"$SCRATCH/init.log" 2>&1 ||
  { cat "$SCRATCH/init.log" >&2; fail "synveda init --demo"; }
echo "    initialised, demo people in the IdP"

# The AC's first half is OPS-1's invariant, re-asserted by the feature most
# likely to break it: the installer still writes no governed object.
TENANT=$(psql_t "select id from tenants where slug = '$SLUG'")
[ -n "$TENANT" ] || fail "no tenant row after init"
for table in hierarchy_nodes identities role_bindings records; do
  count=$(psql_t "select count(*) from $table where tenant_id = '$TENANT'")
  [ "$count" = "0" ] ||
    fail "init wrote $count row(s) into $table — ADR-0055 decision 1 says none"
done
echo "    0 scopes, 0 identities, 0 role bindings, 0 records — the installer seeded nothing"

# ── the seeder refuses before a login ───────────────────────────────────
# Not a nicety: the whole design rests on the seeder having no authority of
# its own, so it must be unable to do anything at all until a person lends
# it theirs.
step "the seeder refuses with no login"
if sh "$SEEDER" >"$SCRATCH/nologin.log" 2>&1; then
  fail "seed.sh ran with no stored login — it must refuse"
fi
grep -q "synveda login" "$SCRATCH/nologin.log" ||
  { cat "$SCRATCH/nologin.log" >&2; fail "the refusal does not say how to fix it"; }
echo "    refused, and named \`synveda login\` as the fix"

# ── the login ───────────────────────────────────────────────────────────
step "synveda login — where the organisation starts to exist"
LOGIN_LOG="$SCRATCH/login.log"
"$SYNVEDA" login --gateway "$GATEWAY_URL" --no-browser >"$LOGIN_LOG" 2>&1 &
LOGIN_PID=$!
tries=0
login_url=""
while [ -z "$login_url" ]; do
  login_url=$(grep -o "$GATEWAY_URL/auth/login?[^ ]*" "$LOGIN_LOG" 2>/dev/null | head -1 || true)
  tries=$((tries + 1))
  [ "$tries" -ge 100 ] && { cat "$LOGIN_LOG" >&2; fail "login printed no URL"; }
  [ -z "$login_url" ] && sleep 0.1
done

callback_url=$(idp_password_login "$login_url")
[ -n "$callback_url" ] || fail "rauthy returned no callback location"
handoff=$(curl -si "$callback_url" | grep -i '^location:' | sed 's/^[Ll]ocation: //' | tr -d '\r')
curl -fsS "$handoff" >/dev/null 2>&1 || true
wait "$LOGIN_PID" || { cat "$LOGIN_LOG" >&2; fail "login did not complete"; }
ROOT=$("$SYNVEDA" hierarchy root 2>/dev/null)
[ -n "$ROOT" ] || fail "no org root after the first admin login"
echo "    logged in as $OPERATOR; org root $ROOT"

# ── --dry-run writes nothing ────────────────────────────────────────────
step "seed.sh --dry-run"
sh "$SEEDER" --dry-run >"$SCRATCH/dry.log" 2>&1 || { cat "$SCRATCH/dry.log" >&2; fail "--dry-run"; }
grep -q "would create" "$SCRATCH/dry.log" || fail "--dry-run printed no plan"
# Two, and naming both is the point: the first login provisions the org root
# *and* the operator's personal leaf, which is ADR-0055 decision 2's whole
# claim — the organisation begins when a person arrives, not when the
# installer runs. A `--dry-run` that left anything else behind would show up
# as a third.
after_dry=$(psql_t "select count(*) from hierarchy_nodes where tenant_id = '$TENANT'")
[ "$after_dry" = "2" ] ||
  fail "--dry-run changed the hierarchy: $after_dry nodes, wanted 2 (the org root and the operator's leaf)"
echo "    printed a plan and wrote nothing (org root + operator leaf, unchanged)"

# ── the seeding ─────────────────────────────────────────────────────────
step "seed.sh — the demo organisation, as the operator"
sh "$SEEDER" >"$SCRATCH/seed.log" 2>&1 || { cat "$SCRATCH/seed.log" >&2; fail "seed.sh"; }
sed 's/^/    /' "$SCRATCH/seed.log" | tail -20

departments=$(psql_t "select count(*) from hierarchy_nodes
  where tenant_id = '$TENANT' and kind = 'department'")
teams=$(psql_t "select count(*) from hierarchy_nodes
  where tenant_id = '$TENANT' and kind = 'team'")
[ "$departments" = "2" ] || fail "wanted 2 departments, got $departments"
[ "$teams" = "3" ] || fail "wanted 3 teams, got $teams"
echo "    $departments departments, $teams teams"

# ── the AC's assertions ─────────────────────────────────────────────────
step "a recall returns seeded memory"
recalled=$("$SYNVEDA" recall --query "how do we roll out payments" --quiet 2>/dev/null |
  grep -c '^── ' || true)
[ "$recalled" -ge 1 ] || fail "recall returned nothing — the corpus did not land"
echo "    $recalled record(s) through the PDP under the operator's own bearer"

step "the pack contrast is real"
# `policy_pack_assignments` stores the pack by *name*, not by a join to
# `policy_packs` — the assignment names a product pack that is compiled in,
# so there is no row to join to.
eng=$(psql_t "select a.pack_name from policy_pack_assignments a
  join hierarchy_nodes n on n.id = a.scope_id and n.tenant_id = a.tenant_id
  where a.tenant_id = '$TENANT' and n.slug = 'eng'")
case "$eng" in
*standard*) echo "    eng carries standard; sales inherits the default" ;;
*) fail "eng does not carry the standard pack: [$eng]" ;;
esac

step "the inbox holds a proposal the operator cannot approve"
open_count=$(psql_t "select count(*) from vedaflow_proposals
  where tenant_id = '$TENANT' and state = 'open'")
[ "$open_count" -ge 1 ] || fail "no open proposal — the inbox would be empty"
proposal=$(psql_t "select id from vedaflow_proposals
  where tenant_id = '$TENANT' and state = 'open' limit 1")
echo "    $open_count open: $proposal"

# The refusal is the demo. Approving your own proposal under
# regulated-strict needs a second distinct person, and there is one operator.
BEARER=$("$SYNVEDA" auth token 2>/dev/null)
approve_code=$(curl -s -o "$SCRATCH/approve.json" -w '%{http_code}' \
  -X POST "$GATEWAY_URL/v1/proposals/$proposal/approve" \
  -H "Authorization: Bearer $BEARER" -H 'Content-Type: application/json' -d '{}')
publish_code=$(curl -s -o "$SCRATCH/publish.json" -w '%{http_code}' \
  -X POST "$GATEWAY_URL/v1/proposals/$proposal/publish" \
  -H "Authorization: Bearer $BEARER")
case "$publish_code" in
2*) fail "the operator published alone — dual control did not hold" ;;
*) echo "    approve $approve_code, publish $publish_code — one person cannot finish it" ;;
esac

# ── the console, signed in to ───────────────────────────────────────────
# **Not `GET /console/ → 200`.** That assertion passes against a console
# nobody can sign into, which is OPS-8's own finding — it fixed the route's
# 404, asserted a 200, and shipped a release where every sign-in failed
# because `init` minted no KEK. This demo repeated the mistake in a smaller
# way: its header claimed "a console signed in to" while nothing here
# touched the console at all.
#
# It was not hypothetical. Run against a dev database whose `deployment_keys`
# row had been sealed under a KEK that was later overwritten — same
# `kek_ref`, different bytes, which ADR-0064 records as a hazard the schema
# cannot catch — every sign-in failed with `sealed payload for kms.data_key
# did not open under this key`, and a 200 on the route said nothing about
# it. So the assertion is the session: the callback lands on the console
# rather than on an error, a `__Host-` cookie comes back, and that cookie
# answers a real API call.
step "the console is signed in to, not merely served"
served=$(curl -s -o /dev/null -w '%{http_code}' "$GATEWAY_URL/console/")
[ "$served" = "200" ] || fail "GET /console/ returned $served — no bundle?"

# `console=true` is the whole difference from the CLI login above — it tells
# the gateway to finish by setting a browser session rather than handing a
# code back to a waiting CLI. `idp_password_login` takes the *gateway's*
# login URL and follows the redirect itself.
JAR="$SCRATCH/console.cookies"
console_callback=$(idp_password_login "$GATEWAY_URL/auth/login?console=true")
[ -n "$console_callback" ] || fail "the IdP returned no callback for the console login"
console_landing=$(curl -s -b "$JAR" -c "$JAR" -o /dev/null -w '%{redirect_url}' "$console_callback")
case "$console_landing" in
*error*)
  # The gateway logs the cause; surfacing it here is the difference between
  # "sign-in broken" and "the deployment key does not open".
  grep -oE "could not seal a console session's access token.*" "$REPO/data/gateway.log" 2>/dev/null |
    tail -1 | sed 's/^/    /' >&2
  fail "the console sign-in failed: $console_landing"
  ;;
*/console/*) ;;
*) fail "the callback landed somewhere unexpected: $console_landing" ;;
esac
grep -q '__Host-' "$JAR" || fail "no __Host- session cookie after a successful callback"

# The cookie has to *work*, not merely exist — a sealed session the gateway
# cannot open again would still set one.
whoami_code=$(curl -s -b "$JAR" -o "$SCRATCH/console-whoami.json" -w '%{http_code}' \
  "$GATEWAY_URL/v1/whoami")
[ "$whoami_code" = "200" ] ||
  fail "the console session does not authorize /v1/whoami: $whoami_code"
grep -q "\"slug\":\"$SLUG\"" "$SCRATCH/console-whoami.json" ||
  fail "the console session resolved the wrong tenant"
inbox_code=$(curl -s -b "$JAR" -o "$SCRATCH/console-inbox.json" -w '%{http_code}' \
  "$GATEWAY_URL/v1/proposals?state=open")
[ "$inbox_code" = "200" ] || fail "the console session cannot read the inbox: $inbox_code"
grep -q "Promote the rollout convention" "$SCRATCH/console-inbox.json" ||
  fail "the inbox the console would render does not hold the seeded proposal"
echo "    signed in, __Host- cookie set, /v1/whoami 200, inbox holds the proposal"

step "the chain verifies, with exactly one break-glass event"
"$SYNVEDA" audit verify --tenant "$TENANT" >"$SCRATCH/verify.log" 2>&1 ||
  { cat "$SCRATCH/verify.log" >&2; fail "the audit chain does not verify"; }
grep -qi "valid" "$SCRATCH/verify.log" || fail "verify did not report a valid chain"
breakglass=$(psql_t "select count(*) from audit_log
  where tenant_id = '$TENANT' and actor_kind = 'break_glass'")
[ "$breakglass" = "1" ] ||
  fail "wanted exactly 1 break-glass event, got $breakglass — seeding must be attributed"
sed 's/^/    /' "$SCRATCH/verify.log" | head -3
echo "    exactly 1 break-glass event: admitting the tenant, before anybody existed"

# ── idempotency ─────────────────────────────────────────────────────────
step "a second run changes nothing"
before=$(psql_t "select count(*) from hierarchy_nodes where tenant_id = '$TENANT'")
records_before=$(psql_t "select count(*) from records where tenant_id = '$TENANT'")
proposals_before=$(psql_t "select count(*) from vedaflow_proposals where tenant_id = '$TENANT'")
sh "$SEEDER" >"$SCRATCH/seed2.log" 2>&1 || { cat "$SCRATCH/seed2.log" >&2; fail "second seed.sh"; }
after=$(psql_t "select count(*) from hierarchy_nodes where tenant_id = '$TENANT'")
records_after=$(psql_t "select count(*) from records where tenant_id = '$TENANT'")
proposals_after=$(psql_t "select count(*) from vedaflow_proposals where tenant_id = '$TENANT'")
[ "$before" = "$after" ] || fail "scopes went $before -> $after on a re-run"
[ "$records_before" = "$records_after" ] ||
  fail "records went $records_before -> $records_after — the corpus was observed twice"
[ "$proposals_before" = "$proposals_after" ] ||
  fail "proposals went $proposals_before -> $proposals_after — a second proposal was opened"
echo "    scopes $after, records $records_after, proposals $proposals_after — unchanged"

# ── the guard ───────────────────────────────────────────────────────────
step "a tenant holding a foreign organisation is refused"
foreign_parent=$ROOT
curl -fsS -X POST "$GATEWAY_URL/v1/hierarchy/nodes" \
  -H "Authorization: Bearer $BEARER" -H 'Content-Type: application/json' \
  -d "{\"parent_id\":\"$foreign_parent\",\"kind\":\"department\",
       \"slug\":\"legal\",\"name\":\"Legal\"}" >/dev/null ||
  fail "could not create the foreign department this case needs"
if sh "$SEEDER" >"$SCRATCH/guard.log" 2>&1; then
  fail "seed.sh ran against a tenant holding a department it did not create"
fi
grep -q "legal" "$SCRATCH/guard.log" || fail "the refusal does not name the foreign scope"
grep -q -- "--i-know-this-is-not-a-demo-tenant" "$SCRATCH/guard.log" ||
  fail "the refusal does not name its own override"
echo "    refused, named \`legal\`, and named the override"

sh "$SEEDER" --i-know-this-is-not-a-demo-tenant >"$SCRATCH/forced.log" 2>&1 ||
  { cat "$SCRATCH/forced.log" >&2; fail "the override did not work"; }
echo "    and the override seeds anyway"

echo ""
echo "OPS-9 acceptance demo PASSED"
echo ""
echo "  installer seeded nothing; the operator seeded everything"
echo "  $departments departments, $teams teams, $records_after records"
echo "  $open_count proposal open and unfinishable by one person"
echo "  chain valid, exactly 1 break-glass event"
echo "  re-run changed nothing; a foreign organisation was refused"
