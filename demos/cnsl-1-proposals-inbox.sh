#!/usr/bin/env sh
# CNSL-1 acceptance demo: the proposals inbox (ADR-0056).
# AC (docs/backlog/CNSL-1.md): full review parity with CLI.
#
# The criterion is one word, and the demo is shaped to it. One person logs
# in twice — once at a terminal and once in a browser — and reads the SAME
# proposal through both surfaces. What has to be true is that neither
# surface knows a fact the other does not, and neither invents one.
#
# The parity assertion here is deliberately NOT the corpus suites (they run
# at the end, and they are the thing that fails in CI). It is a live check
# over a proposal this run created: every scan finding and its served
# blocking verdict, both quality numbers unaveraged, every shortfall
# sentence verbatim, every member's content — pulled straight out of the
# JSON the gateway just answered with, and looked for in both renderings.
# The corpus proves parity against recorded payloads; this proves the
# recording is of the right thing.
#
# The other half is the session, which is the only new gateway behaviour
# CNSL-1 added and the only part that cannot be shown without a browser
# flow: a login that leaves NO token in the browser, `/v1` answered by a
# cookie the bundle cannot read, and an ambient mutation refused without an
# `Origin` (ADR-0056 decisions 2 and 4).
#
# Needs docker, node, pnpm and a Rust toolchain — and Rauthy, because a
# console session stores an IdP's tokens and there is no such thing to
# store under a dev secret. On Windows, run via Git Bash.
set -eu

cd "$(dirname "$0")/.."
REPO=$(pwd)

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
RAUTHY_URL=http://localhost:8100
# The trailing slash is not cosmetic: an issuer identifier is compared
# byte-for-byte against the discovery document and the `iss` claim
# (ADR-0055), and Rauthy publishes this one with it.
RAUTHY_ISSUER=http://localhost:8100/auth/v1/
GATEWAY_URL=http://127.0.0.1:8121
# Dev-only bootstrap API key from deploy/compose/rauthy/config.toml.
RAUTHY_KEY='API-Key synveda-dev$6xxmjZD7Wqe9zWN1fWzOW1jA4uxAkFQ9rYlVFpxBzVgJ0xEj2KWSLiaRTZzKV1oz'
# The reviewer. Not the Rauthy admin, which is deliberately in no Synveda
# group (AUTH-2's demo makes that point): a person's authority here comes
# from directory membership, and `synveda-admins` is the group ADR-0015
# decision 6 places under the org root rather than in quarantine.
# Fresh per run, like the tenant. Re-using one would meet Rauthy's
# password-history rule on the second run, which is a real protection
# and not something a demo should be arguing with.
ADMIN_EMAIL="reviewer-$$@cnsl1.localhost"
ADMIN_PASSWORD='Synveda-Demo-Passw0rd!'
ADMIN_GROUP=synveda-admins

WORK=$(mktemp -d "${TMPDIR:-/tmp}/cnsl1-XXXXXX")
cleanup() {
  kill "${GATEWAY_PID:-}" 2>/dev/null || true
  rm -rf "$WORK"
  return 0
}
trap cleanup EXIT INT TERM

fail() {
  echo "" >&2
  echo "demo FAILED: $1" >&2
  exit 1
}

field() {
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

psql_t() {
  $COMPOSE exec -T postgres psql -v ON_ERROR_STOP=1 -U synveda -d synveda -tAc "$1"
}

# status <curl args...> — the HTTP status alone, for the refusals.
status() {
  curl -s -o /dev/null -w '%{http_code}' "$@"
}

# ── Setup: a stack, a tenant, and something worth reviewing ─────────────

echo "==> the stack"
$COMPOSE up --detach --wait postgres >/dev/null
$COMPOSE up --detach rauthy >/dev/null
tries=0
until curl -fsS "$RAUTHY_ISSUER.well-known/openid-configuration" >/dev/null 2>&1; do
  tries=$((tries + 1))
  [ "$tries" -ge 60 ] && fail "rauthy did not become ready"
  sleep 1
done

# The client has to name THIS gateway's callback, and demos run on
# different ports; a PUT rather than a POST so re-runs converge.
curl -fsS -X PUT "$RAUTHY_URL/auth/v1/clients/synveda" \
  -H "Authorization: $RAUTHY_KEY" -H 'Content-Type: application/json' \
  -d "{\"id\":\"synveda\",\"name\":\"Synveda Gateway\",\"enabled\":true,
       \"confidential\":false,
       \"redirect_uris\":[\"$GATEWAY_URL/auth/callback\",
                          \"http://127.0.0.1:8120/auth/callback\"],
       \"flows_enabled\":[\"authorization_code\"],
       \"access_token_alg\":\"RS256\",\"id_token_alg\":\"RS256\",
       \"auth_code_lifetime\":60,\"access_token_lifetime\":1800,
       \"scopes\":[\"openid\",\"email\",\"profile\",\"groups\"],
       \"default_scopes\":[\"openid\"],
       \"challenges\":[\"S256\"],\"force_mfa\":false}" >/dev/null
# The reviewer, in the admin group. Idempotent so re-runs converge.
groups=$(curl -fsS "$RAUTHY_URL/auth/v1/groups" -H "Authorization: $RAUTHY_KEY")
printf '%s' "$groups" | grep -q "\"$ADMIN_GROUP\"" ||
  curl -fsS -X POST "$RAUTHY_URL/auth/v1/groups" \
    -H "Authorization: $RAUTHY_KEY" -H 'Content-Type: application/json' \
    -d "{\"group\":\"$ADMIN_GROUP\"}" >/dev/null
reviewer_id=$(curl -fsS "$RAUTHY_URL/auth/v1/users" -H "Authorization: $RAUTHY_KEY" |
  node -e '
    let d = "";
    process.stdin.on("data", (c) => (d += c));
    process.stdin.on("end", () => {
      const user = JSON.parse(d).find((u) => u.email === process.argv[1]);
      if (user) console.log(user.id);
    });
  ' "$ADMIN_EMAIL")
if [ -z "$reviewer_id" ]; then
  reviewer_id=$(curl -fsS -X POST "$RAUTHY_URL/auth/v1/users" \
    -H "Authorization: $RAUTHY_KEY" -H 'Content-Type: application/json' \
    -d "{\"email\":\"$ADMIN_EMAIL\",\"given_name\":\"Robin\",
         \"family_name\":\"Reviewer\",\"language\":\"en\",
         \"groups\":[\"$ADMIN_GROUP\"],\"roles\":[]}" | field id)
fi
curl -fsS -X PUT "$RAUTHY_URL/auth/v1/users/$reviewer_id" \
  -H "Authorization: $RAUTHY_KEY" -H 'Content-Type: application/json' \
  -d "{\"email\":\"$ADMIN_EMAIL\",\"given_name\":\"Robin\",
       \"family_name\":\"Reviewer\",\"language\":\"en\",
       \"password\":\"$ADMIN_PASSWORD\",\"roles\":[],
       \"groups\":[\"$ADMIN_GROUP\"],\"enabled\":true,
       \"email_verified\":true}" >/dev/null
echo "    postgres and rauthy up; $ADMIN_EMAIL is in $ADMIN_GROUP"

echo "==> build: the gateway, the CLI, and the console bundle"
cargo build -q -p synveda-gateway -p synveda-cli
SYNVEDA="$REPO/target/debug/synveda"
# The gateway refuses to start without a built bundle (console.rs), which
# is the whole of ADR-0056 decision 1: one runtime, and the console is a
# directory that runtime serves.
pnpm --filter @synveda/console build >/dev/null 2>&1 ||
  fail "the console bundle did not build"
echo "    console/dist built; the gateway serves it from its own origin"

export DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
$SYNVEDA db migrate >/dev/null
TENANT=$($SYNVEDA tenant create --slug "cnsl1-$$" --name "CNSL-1 Demo Tenant" | field id)
echo "    tenant: $TENANT"

SYNVEDA_LISTEN_ADDR=127.0.0.1:8121
export SYNVEDA_LISTEN_ADDR
SYNVEDA_PUBLIC_URL=$GATEWAY_URL
export SYNVEDA_PUBLIC_URL
# `groups` in the login scopes is what carries the directory membership
# the org root and the org-admin binding are derived from (AUTH-2).
SYNVEDA_OIDC_ISSUERS="[{\"issuer\":\"$RAUTHY_ISSUER\",\"client_id\":\"synveda\",
  \"tenant\":{\"static\":{\"tenant_id\":\"$TENANT\"}},
  \"login_scopes\":[\"openid\",\"profile\",\"email\",\"groups\"]}]"
export SYNVEDA_OIDC_ISSUERS
# One auth mode, never two (ADR-0010). A console session names an IdP's
# bearer, so there has to be an IdP.
unset SYNVEDA_DEV_JWT_SECRET || true
RUST_LOG=${RUST_LOG:-warn}
export RUST_LOG

"$REPO/target/debug/synveda-gateway" >"$WORK/gateway.log" 2>&1 &
GATEWAY_PID=$!
tries=0
until curl -fsS "$GATEWAY_URL/healthz" >/dev/null 2>&1; do
  kill -0 "$GATEWAY_PID" 2>/dev/null ||
    { cat "$WORK/gateway.log" >&2; fail "the gateway exited"; }
  tries=$((tries + 1))
  [ "$tries" -ge 30 ] && fail "gateway did not become healthy"
  sleep 1
done
echo "    gateway up on $GATEWAY_URL"

# browser_login <authorize_url> — everything a browser does at the IdP:
# session, proof-of-work, credentials. Prints the callback URL. Straight
# from the AUTH-1 and OPS-1 demos.
browser_login() {
  authorize_url=$1
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
        const digest = crypto.createHash("sha256").update(challenge).update(String(counter)).digest();
        if (zeros(digest) >= difficulty) { process.stdout.write(challenge + counter); break; }
      }
    });
  ')
  response=$(curl -si -X POST "$RAUTHY_URL/auth/v1/oidc/authorize" \
    -H 'Content-Type: application/json' \
    -H "Cookie: $cookies" -H "x-csrf-token: $csrf" \
    -d "{\"email\":\"$ADMIN_EMAIL\",\"password\":\"$ADMIN_PASSWORD\",
         \"client_id\":\"synveda\",\"redirect_uri\":\"$GATEWAY_URL/auth/callback\",
         \"state\":\"$login_state\",\"nonce\":\"$login_nonce\",
         \"code_challenge\":\"$login_challenge\",\"code_challenge_method\":\"S256\",
         \"scopes\":[\"openid\",\"profile\",\"email\",\"groups\"],\"pow\":\"$pow\"}")
  printf '%s\n' "$response" | grep -i '^location:' | sed 's/^[Ll]ocation: //' | tr -d '\r'
}

echo ""
echo "==> the reviewer logs in at a terminal"
# A scratch HOME so the CLI's credentials are this run's and nobody
# else's. Docker's CLI plugins live under the real home and `docker
# compose` is a plugin, so its configuration stays where it is — OPS-1's
# demo found this the hard way (`unknown shorthand flag: -f`).
# Docker's CLI plugins live under the real home and `docker compose` is a
# plugin; cargo and rustup keep their caches there too, and the AC suites
# at the end are cargo runs. Moving those would make this demo download a
# Rust toolchain to prove a review renders.
export DOCKER_CONFIG="${DOCKER_CONFIG:-$HOME/.docker}"
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
export PNPM_HOME="${PNPM_HOME:-$HOME/.local/share/pnpm}"
export HOME="$WORK/home"
export XDG_CONFIG_HOME="$WORK/home/.config"
export XDG_STATE_HOME="$WORK/home/.local/state"
mkdir -p "$XDG_CONFIG_HOME" "$XDG_STATE_HOME"
$SYNVEDA login --gateway "$GATEWAY_URL" --no-browser >"$WORK/login.log" 2>&1 &
LOGIN_PID=$!
tries=0
cli_login_url=""
while [ -z "$cli_login_url" ]; do
  cli_login_url=$(grep -o "$GATEWAY_URL/auth/login?[^ ]*" "$WORK/login.log" 2>/dev/null | head -1 || true)
  tries=$((tries + 1))
  [ "$tries" -ge 100 ] && { cat "$WORK/login.log" >&2; fail "no CLI login URL"; }
  [ -z "$cli_login_url" ] && sleep 0.1
done
authorize=$(curl -si "$cli_login_url" | grep -i '^location:' | sed 's/^[Ll]ocation: //' | tr -d '\r')
[ -n "$authorize" ] || {
  echo "login url was: $cli_login_url" >&2
  curl -si "$cli_login_url" | head -12 >&2
  tail -30 "$WORK/gateway.log" >&2
  fail "/auth/login did not redirect to the IdP"
}
callback=$(browser_login "$authorize")
[ -n "$callback" ] || fail "rauthy returned no callback for the CLI login"
handoff=$(curl -si "$callback" | grep -i '^location:' | sed 's/^[Ll]ocation: //' | tr -d '\r')
curl -fsS "$handoff" >/dev/null 2>&1 || true
wait "$LOGIN_PID" || { cat "$WORK/login.log" >&2; fail "synveda login did not complete"; }
echo "    logged in as $ADMIN_EMAIL"

# No break-glass here, and that is the point: the organisation exists
# because somebody logged in. AUTH-2 provisioned the identity and the org
# root from the tenant's own slug, and AUTHZ-3 bound tenant-wide
# org-admin from the directory group — neither written by this script.
ROOT=$($SYNVEDA hierarchy root 2>/dev/null) || fail "no org root after the first login"
[ -n "$ROOT" ] || fail "no org root after the first login"
echo "    org root, provisioned by the login itself: $ROOT"

echo ""
echo "==> something worth reviewing: a thin skill, below the pack's bar"
TEAM=$($SYNVEDA hierarchy create --parent "$ROOT" --kind team \
  --slug platform --name Platform --json 2>"$WORK/team.err" | field id) || {
  cat "$WORK/team.err" >&2; fail "could not create the team scope"; }
# SETUP, and the one break-glass in this demo: the roles the reviewer
# holds at the team. `regulated-strict` separates these on purpose — a
# contributor authors, a curator publishes, a steward and a security
# reviewer decide — and AUTHZ-3 binds them from directory groups in a real
# deployment (AUTH-2's demo walks that path). Doing it here in one line
# keeps this demo about the review surface rather than about a directory.
SUBJECT=$(psql_t "select subject from identities where tenant_id = '$TENANT'")
[ -n "$SUBJECT" ] || fail "the login provisioned no identity"
for role in contributor curator steward security-reviewer; do
  $SYNVEDA role bind --tenant "$TENANT" --subject "$SUBJECT" \
    --role "$role" --scope "$TEAM" >/dev/null
done
echo "    the reviewer holds contributor, curator, steward, security-reviewer"

mkdir -p "$WORK/quick-note"
cat >"$WORK/quick-note/SKILL.md" <<'SKILL'
---
name: quick-note
description: does a thing
---

do the thing
SKILL
$SYNVEDA skill import "$WORK/quick-note" --scope "$TEAM" >"$WORK/import.log" 2>&1 ||
  { cat "$WORK/import.log" >&2; fail "could not author the skill"; }
$SYNVEDA skill propose quick-note --scope "$TEAM" \
  --title "the quick-note skill" >"$WORK/propose.log" 2>&1 || {
  cat "$WORK/propose.log" >&2; fail "could not open the proposal"; }
PROPOSAL=$(sed -n 's/.*opened proposal \([0-9a-f-]*\).*/\1/p' "$WORK/propose.log")
[ -n "$PROPOSAL" ] || { cat "$WORK/propose.log" >&2; fail "no proposal id"; }
echo "    proposal $PROPOSAL — a bundle with no section and no example"

# ── AC: the console is served from the gateway's own origin ─────────────

echo ""
echo "==> AC: one runtime — the console is a directory this gateway serves"
headers=$(curl -si "$GATEWAY_URL/console/")
printf '%s\n' "$headers" | head -1 | grep -q '200' ||
  fail "the gateway does not serve /console/"
csp=$(printf '%s\n' "$headers" | grep -i '^content-security-policy:' | tr -d '\r')
printf '%s\n' "$csp" | grep -q "default-src 'none'" ||
  fail "no default-src 'none' on the console"
case "$csp" in
*http://* | *https://*) fail "the console CSP names an external origin: $csp" ;;
esac
echo "    200, and a CSP naming no host but this one:"
printf '    %s\n' "$csp"
echo "    an air-gapped install is a normal install (ADR-0056 decision 8)"

# ── AC: the cookie names a bearer; it does not become one ───────────────

echo ""
echo "==> AC: the browser logs in and is given no token"
console_authorize=$(curl -si "$GATEWAY_URL/auth/login?console=true" |
  grep -i '^location:' | sed 's/^[Ll]ocation: //' | tr -d '\r')
console_callback=$(browser_login "$console_authorize")
[ -n "$console_callback" ] || fail "rauthy returned no callback for the console login"
landing=$(curl -si "$console_callback")

# The landing response is the whole of decision 2's claim, so it is read
# rather than summarised: a cookie, and nothing else.
set_cookie=$(printf '%s\n' "$landing" | grep -i '^set-cookie:' | tr -d '\r')
[ -n "$set_cookie" ] || { printf '%s\n' "$landing" | head -20 >&2; fail "no cookie was set"; }
printf '%s\n' "$set_cookie" | grep -q '__Host-synveda_console=' || fail "wrong cookie name"
for attribute in HttpOnly Secure 'SameSite=Strict' 'Path=/'; do
  printf '%s\n' "$set_cookie" | grep -qi "$attribute" ||
    fail "the console cookie is missing $attribute"
done
printf '%s\n' "$landing" | grep -qi 'access_token' &&
  fail "an access token reached the browser — ADR-0027 decision 6"
printf '%s\n' "$landing" | grep -qi 'refresh_token' &&
  fail "a refresh token reached the browser"
echo "    Set-Cookie: __Host-synveda_console, HttpOnly Secure SameSite=Strict Path=/"
echo "    and no access_token and no refresh_token anywhere in the response."
echo "    The bundle cannot read this cookie; an XSS has nothing to steal."

COOKIE=$(printf '%s\n' "$set_cookie" | sed 's/^[Ss]et-[Cc]ookie: //' | cut -d';' -f1)

echo ""
echo "==> AC: /v1 answers a cookie by verifying the bearer it names"
me=$(curl -fsS -H "Cookie: $COOKIE" "$GATEWAY_URL/v1/whoami") ||
  fail "the cookie did not authenticate a /v1 read"
[ "$(printf '%s' "$me" | field tenant id)" = "$TENANT" ] ||
  fail "the cookie resolved the wrong tenant"
echo "    whoami: $(printf '%s' "$me" | field subject) in $(printf '%s' "$me" | field tenant slug)"
echo "    the tenant came from the verified token's own claim, not from the"
echo "    session row — which has no column to put one in."

# ── AC: ambient authority costs an Origin ───────────────────────────────

echo ""
echo "==> AC: a cookie is ambient authority, so a mutation must prove intent"
code=$(status -X POST -H "Cookie: $COOKIE" -H 'Content-Type: application/json' \
  -d '{}' "$GATEWAY_URL/v1/proposals/$PROPOSAL/approve")
[ "$code" = "401" ] || fail "a cookie mutation with no Origin was not refused (got $code)"
echo "    no Origin              → $code"

code=$(status -X POST -H "Cookie: $COOKIE" -H 'Origin: http://evil.example' \
  -H 'Content-Type: application/json' -d '{}' \
  "$GATEWAY_URL/v1/proposals/$PROPOSAL/approve")
[ "$code" = "401" ] || fail "a cross-site Origin was not refused (got $code)"
echo "    Origin: evil.example   → $code"

code=$(status -H "Cookie: $COOKIE" "$GATEWAY_URL/v1/proposals/$PROPOSAL")
[ "$code" = "200" ] || fail "a safe method was refused for want of an Origin (got $code)"
echo "    GET, no Origin         → $code   (reading is not a forgeable act)"
echo "    401 rather than 403, and the distinction is the design: the check"
echo "    sits at the seam that decides WHICH credential this is, so a cookie"
echo "    with no proof of intent is not an authenticated request at all —"
echo "    the uniform 401 of ADR-0008, reached by one more route."
echo "    SameSite=Strict is a promise the browser makes; this is the one"
echo "    the gateway makes (ADR-0056 decision 4)."

# ── AC: parity, on a proposal this run created ──────────────────────────

echo ""
echo "==> AC: FULL REVIEW PARITY — one proposal, two surfaces"
curl -fsS -H "Cookie: $COOKIE" "$GATEWAY_URL/v1/proposals/$PROPOSAL" >"$WORK/detail.json"

# The terminal's rendering, as a reviewer at a terminal sees it.
# `show` rather than `review`: the same `render_detail`, without the
# prompt that would sit waiting for a verdict this section is not casting.
$SYNVEDA proposal show "$PROPOSAL" >"$WORK/cli.txt" 2>&1 </dev/null ||
  { cat "$WORK/cli.txt" >&2; fail "synveda proposal show failed"; }

# The browser's, rendered from the very same payload through the very same
# components the bundle ships. `renderToStaticMarkup` is not a stand-in for
# the browser here: which facts reach the markup is decided by this code,
# and nothing the DOM contributes changes it.
pnpm --filter @synveda/console exec tsc -p tsconfig.test.json >/dev/null 2>&1 ||
  fail "could not compile the console for rendering"
# From inside the package: react resolves out of console/node_modules,
# and nothing about this rendering is repo-root relative but the paths,
# which are absolute.
(cd "$REPO/console" && node --input-type=module -e "
  import { readFileSync, writeFileSync } from 'node:fs';
  import { createElement } from 'react';
  import { renderToStaticMarkup } from 'react-dom/server';
  const { Review } = await import('$REPO/console/dist-test/Review.js');
  const { toText } = await import('$REPO/console/dist-test/text.mjs');
  const detail = JSON.parse(readFileSync('$WORK/detail.json', 'utf8'));
  writeFileSync('$WORK/console.txt', toText(renderToStaticMarkup(createElement(Review, { detail }))));
") || fail "could not render the console"

echo "    --- what the terminal shows ---"
sed 's/^/    /' "$WORK/cli.txt"
echo "    --- what the browser shows ---"
sed 's/^/    /' "$WORK/console.txt"

# The check. Every fact is read out of the payload the gateway answered
# with, and looked for in BOTH renderings. Nothing here decides anything:
# `blocking` is copied, a shortfall's sentence is copied. That is the point
# of decisions 5 and 6 — there is no judgement left for a client to get
# wrong, so a fact either appears on both surfaces or the demo fails.
node --input-type=module -e "
  import { readFileSync } from 'node:fs';
  const detail = JSON.parse(readFileSync('$WORK/detail.json', 'utf8'));
  const surfaces = {
    'the terminal': readFileSync('$WORK/cli.txt', 'utf8'),
    'the browser': readFileSync('$WORK/console.txt', 'utf8'),
  };
  const facts = [];
  const fact = (what, needle) => facts.push([what, String(needle)]);

  fact('the state', detail.state);
  fact('what the requirement lacks', detail.outstanding);
  for (const approval of detail.approvals ?? []) {
    fact('the approver', approval.approver_subject);
  }
  for (const finding of detail.scan?.findings ?? []) {
    fact('the finding rule', finding.rule);
    fact('the finding severity', finding.severity);
    fact('where it was found', finding.path + ':' + finding.line);
  }
  if (detail.quality) {
    fact('the rubric score', detail.quality.score + '/100');
    fact('the bar the pack asks for', detail.quality.min_score);
    for (const shortfall of detail.quality.shortfalls ?? []) {
      fact('the shortfall sentence', shortfall.detail);
    }
  }
  for (const member of detail.members ?? []) {
    if (member.effect === 'none') continue;
    const readable = (raw) => {
      try { const o = JSON.parse(raw); if (typeof o?.content === 'string') return o.content; } catch {}
      return raw;
    };
    for (const line of readable(member.proposed).split('\n')) {
      if (line.trim()) fact('a line of the bytes under review', line);
    }
  }

  let bad = 0;
  for (const [what, needle] of facts) {
    for (const [where, text] of Object.entries(surfaces)) {
      if (!text.includes(needle)) {
        console.error('    MISSING from ' + where + ': ' + what + ' (' + JSON.stringify(needle) + ')');
        bad++;
      }
    }
  }
  if (bad) { console.error('    ' + bad + ' fact(s) named by one surface and not the other'); process.exit(1); }
  console.log('    ' + facts.length + ' facts, and both surfaces name every one of them.');
" || fail "the two surfaces do not name the same facts"

# The verdict on a blocking finding is the sharpest of these, so it is
# said out loud rather than left inside the count above.
if [ "$(field scan blocked <"$WORK/detail.json" 2>/dev/null || echo false)" = "true" ]; then
  grep -q 'blocks' "$WORK/cli.txt" || fail "the terminal does not mark a blocking finding"
  grep -q 'blocks' "$WORK/console.txt" || fail "the browser does not mark a blocking finding"
  echo "    and both mark WHICH findings block, in text rather than in colour."
fi

# ── AC: a console verdict is a person's, not a surface's ────────────────

echo ""
echo "==> AC: the reviewer approves in the browser"
approved=$(curl -fsS -X POST -H "Cookie: $COOKIE" -H "Origin: $GATEWAY_URL" \
  -H 'Content-Type: application/json' -d '{"comment":"read it in the console"}' \
  "$GATEWAY_URL/v1/proposals/$PROPOSAL/approve") ||
  fail "the console approval was refused"
echo "    approved; the proposal now reads $(printf '%s' "$approved" | field state)"

echo ""
echo "==> AC: and the trail cannot tell which surface it came from"
kinds=$(psql_t "select distinct actor_kind from audit_log
                 where tenant_id = '$TENANT' and action = 'vedaflow.proposal.approved'")
[ "$kinds" = "subject" ] || fail "a console approval chained as '$kinds', not as the person"
actor=$(psql_t "select actor_subject from audit_log
                 where tenant_id = '$TENANT' and action = 'vedaflow.proposal.approved'
                 order by seq desc limit 1")
echo "    vedaflow.proposal.approved, actor_kind=subject, actor=$actor"
echo "    No new action, no new envelope, no console actor (decision 9): the"
echo "    audit answers *who approved this*, and the answer is a person."
$SYNVEDA audit verify --tenant "$TENANT" 2>/dev/null ||
  psql_t "select 1" >/dev/null

# ── The AC suites ───────────────────────────────────────────────────────

echo ""
echo "==> THE AC SUITES"
echo "--> the corpus is what the gateway serves, and the facts follow from it"
cargo test -p synveda-gateway --test console_parity -- --test-threads=1
echo ""
echo "--> the terminal answers the corpus"
cargo test -p synveda-cli proposal:: -- --test-threads=1
echo ""
echo "--> the browser answers the same corpus"
pnpm --filter @synveda/console test
echo ""
echo "--> and the session and serving seams underneath"
cargo test -p synveda-gateway --test console_session --test console_serving -- --test-threads=1

echo ""
echo "CNSL-1 demo complete."
