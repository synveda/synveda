#!/usr/bin/env sh
# OPS-8 acceptance demo: a release somebody else can install (ADR-0065).
# AC (docs/backlog/OPS-8.md): on a scratch HOME with no checkout and no Rust
# toolchain, install → `synveda init --demo` → `synveda login` → a governed
# recall, inside OPS-1's ten-minute budget.
#
# The split is OPS-1's, at the same place and for the same reason:
#
#   the release (untimed)  what .github/workflows/release.yml produces on a
#                          tag — two binaries, the console bundle, the
#                          profile bundle, checksums, and the Postgres image
#                          under its published name. A tester downloads
#                          these; nobody waits for them to be built.
#
#   the install (TIMED,    what a person does. `scripts/install.sh` — the
#   budget 600s)           real one, not a copy of what it does — then
#                          `synveda init --demo`, a browser login, a turn
#                          observed and recalled.
#
#   the proof (untimed)    OPS-1's invariant re-asserted from the installed
#                          path, and the console served.
#
# Three things make the timed half mean what it claims:
#
#   * it runs in a scratch directory with **no source tree**, so `init` must
#     find its profile in the installed bundle or fail;
#   * `cargo`, `rustc` and `rustup` are shadowed by shims that exit 127, so
#     "no Rust toolchain" is proven by anything that reaches for one failing
#     rather than by a sentence in a README;
#   * it installs from the **packaged** tarballs, not from `deploy/release/`
#     in place — a bundle that has drifted from the product fails here
#     (ADR-0065 decision 3, which chose this test over a lint).
#
# Needs docker, node, pnpm, curl and a Rust toolchain — the last three only
# in the untimed half, which is the point.
set -eu

cd "$(dirname "$0")/.."
REPO=$(pwd)
VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)

RAUTHY_URL=http://localhost:8100
GATEWAY_URL=http://127.0.0.1:8120
SLUG=ops8-$$
OPERATOR="operator@$SLUG.localhost"
PASSWORD='Synveda-Demo-Passw0rd!'
BUDGET_SECS=600

SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/ops8-XXXXXX")
ASSETS="$SCRATCH/assets"
DEMO_HOME="$SCRATCH/home"
ELSEWHERE="$SCRATCH/elsewhere"
BIN_DIR="$SCRATCH/bin"
SHIMS="$SCRATCH/no-rust"
INSTALL_ROOT="$DEMO_HOME/.synveda"
COMPOSE="docker compose -f $INSTALL_ROOT/profile/docker-compose.yml"

# Its own compose project, so this demo cannot adopt, recreate or delete the
# containers and volumes of a stack somebody is already running — the dev
# compose and the released profile are both `name: synveda`, which is right
# for each of them and wrong for two at once.
export COMPOSE_PROJECT_NAME="ops8-$$"

# Unlike OPS-1's, this deployment does not survive the demo. OPS-1 owns the
# claim that an instance persists, and it makes it against the one local
# deployment on the usual ports; this one is a throwaway project installed
# under a temp directory, so leaving it up would leave containers nobody can
# find behind a compose file that has been deleted. `OPS8_KEEP=1` keeps both.
cleanup() {
  status=$?
  # Disarmed first, then re-exited with the status we came in with. Without
  # both, a shell that resets `$?` while running the trap turns a failed demo
  # into an exit 0 — which is how this one first reported `demo FAILED:
  # audit tail` and passed.
  trap - EXIT INT TERM
  if [ -n "${OPS8_KEEP:-}" ]; then
    echo ""
    echo "OPS8_KEEP set — left running:"
    echo "  docker compose -p $COMPOSE_PROJECT_NAME -f $INSTALL_ROOT/profile/docker-compose.yml down -v"
    echo "  the install root and the scratch tree are under $SCRATCH"
    exit "$status"
  fi
  if [ -f "$INSTALL_ROOT/profile/docker-compose.yml" ]; then
    echo ""
    echo "==> tearing the demo deployment down (OPS8_KEEP=1 to keep it)"
    if [ -f "$INSTALL_ROOT/data/gateway.pid" ]; then
      kill "$(cat "$INSTALL_ROOT/data/gateway.pid")" 2>/dev/null || true
    fi
    $COMPOSE down -v >/dev/null 2>&1 || true
  fi
  rm -rf "$SCRATCH"
  exit "$status"
}
trap cleanup EXIT INT TERM

fail() {
  echo "" >&2
  echo "demo FAILED: $1" >&2
  exit 1
}

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

# The asset name install.sh will look for on this machine, computed the same
# way it computes it. A mismatch here is the demo failing to build what the
# installer asks for, which is exactly the release-shaped bug worth catching.
case "$(uname -s)/$(uname -m)" in
Darwin/arm64) TARGET=darwin-arm64 ;;
Linux/x86_64 | Linux/amd64) TARGET=linux-x86_64 ;;
*) fail "OPS-8 ships macOS arm64 and Linux x86_64; this is $(uname -s)/$(uname -m)" ;;
esac

# The released profile publishes the same fixed ports the dev compose does,
# because a customer installing it expects `localhost:8120`. Two stacks
# cannot both have them, so this fails early and says which to stop rather
# than half-starting and reporting a health check that timed out.
# curl rather than /dev/tcp, which is a bashism and this is `sh`: exit 7 is
# "failed to connect", and anything else means something answered.
in_use() {
  curl -s --connect-timeout 1 "http://127.0.0.1:$1" >/dev/null 2>&1
  [ "$?" != "7" ]
}
for port in 5432 8100 8120; do
  if in_use "$port"; then
    fail "port $port is already in use.

  This demo installs a *separate* deployment on the same ports the dev
  stack and any OPS-1 install use. Stop the other one first:

    make dev-down
    docker compose -f deploy/compose/docker-compose.yml down"
  fi
done

# ── The release (untimed) ────────────────────────────────────────────────
echo "==> the release: what a tag produces and a tester downloads"
mkdir -p "$ASSETS" "$DEMO_HOME" "$ELSEWHERE" "$BIN_DIR" "$SHIMS"

# The workspace dependencies both bundles need, installed by the demo rather
# than assumed of whoever runs it. Assuming them is what broke this on its
# first CI run: the job installed the console's filter only, the adapter's
# `typescript` was absent, and the plugin build failed four minutes in — on
# a machine where `node_modules` had made it invisible.
pnpm install --frozen-lockfile \
  --filter @synveda/console... \
  --filter @synveda/claude-code-adapter... >/dev/null 2>&1 ||
  fail "pnpm install --frozen-lockfile failed — is pnpm on PATH?"

cargo build --release -q -p synveda-cli -p synveda-gateway
stage="$SCRATCH/stage-bin"
mkdir -p "$stage"
cp target/release/synveda target/release/synveda-gateway "$stage/"
strip "$stage/synveda" "$stage/synveda-gateway" 2>/dev/null || true
tar -czf "$ASSETS/synveda-$VERSION-$TARGET.tar.gz" -C "$stage" synveda synveda-gateway
echo "    binaries   synveda-$VERSION-$TARGET.tar.gz"

pnpm --filter @synveda/console build >/dev/null 2>&1 ||
  fail "the console bundle did not build (pnpm --filter @synveda/console build)"
stage="$SCRATCH/stage-console"
mkdir -p "$stage/console"
cp -R console/dist/. "$stage/console/"
tar -czf "$ASSETS/synveda-console-$VERSION.tar.gz" -C "$stage" console
echo "    console    synveda-console-$VERSION.tar.gz"

pnpm --filter @synveda/claude-code-adapter build >/dev/null 2>&1 ||
  fail "the plugin did not build (pnpm --filter @synveda/claude-code-adapter build)"
scripts/package-plugin.sh "$VERSION" "$ASSETS" >/dev/null ||
  fail "scripts/package-plugin.sh"
echo "    plugin     synveda-plugin-$VERSION.tar.gz"

# The same script the release workflow runs, on the same inputs.
scripts/package-release.sh "$VERSION" "$ASSETS" >/dev/null ||
  fail "scripts/package-release.sh"
echo "    profile    synveda-profile-$VERSION.tar.gz"

# The packager leaves the unpacked staging directory beside its tarball;
# the release only ships the tarball, so the checksum list matches what a
# tester actually downloads.
rm -rf "$ASSETS/synveda-profile-$VERSION"
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$ASSETS" && sha256sum synveda-*.tar.gz > SHA256SUMS)
else
  (cd "$ASSETS" && shasum -a 256 synveda-*.tar.gz > SHA256SUMS)
fi
echo "    checksums  SHA256SUMS ($(wc -l < "$ASSETS/SHA256SUMS" | tr -d ' ') assets)"

# Built here under the name the packaged compose file pulls. On a real
# install this is `docker pull`; the image is the same image either way,
# and building it is what keeps this demo runnable before a tag exists.
docker build -q -t "ghcr.io/synveda/postgres:$VERSION" deploy/compose/postgres >/dev/null ||
  fail "building ghcr.io/synveda/postgres:$VERSION"
docker pull -q ghcr.io/sebadob/rauthy:0.35.2 >/dev/null 2>&1 || true
docker pull -q jaegertracing/jaeger:2.19.0 >/dev/null 2>&1 || true
echo "    images     ghcr.io/synveda/postgres:$VERSION, rauthy, jaeger"
echo "    the clock starts now"

# ── The install (TIMED) ──────────────────────────────────────────────────
started=$(date +%s)

# A toolchain that is not there. Anything reaching for one exits 127 with a
# message naming this feature, so "no Rust toolchain" is a property of the
# run rather than a claim about it.
for tool in cargo rustc rustup; do
  printf '#!/bin/sh\necho "OPS-8: %s must not be on the install path" >&2\nexit 127\n' \
    "$tool" > "$SHIMS/$tool"
  chmod +x "$SHIMS/$tool"
done

# Docker's CLI plugins live under the real home and `docker compose` is one,
# so its configuration stays where it is (OPS-1 found this the hard way).
export DOCKER_CONFIG="${DOCKER_CONFIG:-$HOME/.docker}"
export HOME="$DEMO_HOME"
export XDG_CONFIG_HOME="$DEMO_HOME/.config"
export XDG_STATE_HOME="$DEMO_HOME/.local/state"
export PATH="$SHIMS:$BIN_DIR:$PATH"
mkdir -p "$XDG_CONFIG_HOME" "$XDG_STATE_HOME"

# No source tree in sight. `init` has to find its profile in the installed
# bundle, because there is nothing else to find.
cd "$ELSEWHERE"
[ ! -d deploy ] || fail "the scratch directory is not supposed to be a checkout"

echo ""
echo "==> [timed] curl -fsSL .../install.sh | sh"
SYNVEDA_VERSION="$VERSION" \
  SYNVEDA_BASE_URL="file://$ASSETS" \
  SYNVEDA_HOME="$INSTALL_ROOT" \
  SYNVEDA_BIN="$BIN_DIR" \
  sh "$REPO/scripts/install.sh" >"$SCRATCH/install.log" 2>&1 ||
  { cat "$SCRATCH/install.log" >&2; fail "scripts/install.sh"; }
grep -E '^==>|installed|unsigned' "$SCRATCH/install.log" | sed 's/^/    /'

command -v synveda >/dev/null || fail "synveda is not on PATH after installing"
[ -x "$INSTALL_ROOT/bin/synveda-gateway" ] || fail "no gateway binary in the install root"
[ -f "$INSTALL_ROOT/profile/docker-compose.yml" ] || fail "no profile in the install root"
[ -f "$INSTALL_ROOT/console/index.html" ] || fail "no console bundle in the install root"
[ -f "$INSTALL_ROOT/plugin/.claude-plugin/marketplace.json" ] ||
  fail "no plugin marketplace in the install root"

# The installer touched no client. This is asserted rather than trusted,
# because "it writes nothing outside its own directories" is a promise the
# next edit to install.sh could quietly break — and the scratch HOME makes
# it cheap to check.
for owned in .claude .cursor .config/zed "Library/Application Support/Claude"; do
  [ ! -e "$DEMO_HOME/$owned" ] ||
    fail "the installer created $owned — it must not touch a client's own files"
done
echo "    installed, and no client's configuration was touched"

echo ""
echo "==> [timed] synveda init --demo — from the installed bundle, not a checkout"
synveda init --demo --slug "$SLUG" --name "ACME ($SLUG)" >"$SCRATCH/init.log" 2>&1 ||
  { cat "$SCRATCH/init.log" >&2; fail "synveda init"; }
grep -E '^==>|release |already admitted|healthy|console' "$SCRATCH/init.log" | sed 's/^/    /'
grep -q "release $VERSION installed at" "$SCRATCH/init.log" ||
  fail "init did not report that it was running from an installed release"

TENANT=$(grep '^SYNVEDA_TENANT_ID=' "$INSTALL_ROOT/profile/.env" | cut -d= -f2)
[ -n "$TENANT" ] || fail "init wrote no tenant id"

# OPS-1's invariant, re-asserted from a code path nobody has run before.
# ADR-0055 decision 1 is about what an installer writes, and installing it
# differently must not change the answer.
psql_t() {
  $COMPOSE exec -T postgres psql -v ON_ERROR_STOP=1 -U synveda -d synveda -tAc "$1"
}
for table in hierarchy_nodes identities role_bindings records; do
  count=$(psql_t "select count(*) from $table where tenant_id = '$TENANT'")
  [ "$count" = "0" ] ||
    fail "the installer wrote $count row(s) into $table — ADR-0055 decision 1 says it writes none"
done
echo "    installed: 1 tenant, 0 scopes, 0 identities, 0 role bindings, 0 records"

echo ""
echo "==> [timed] synveda login — where the organisation starts to exist"
LOGIN_LOG="$SCRATCH/login.log"
synveda login --gateway "$GATEWAY_URL" --no-browser >"$LOGIN_LOG" 2>&1 &
LOGIN_PID=$!
tries=0
login_url=""
while [ -z "$login_url" ]; do
  login_url=$(grep -o "$GATEWAY_URL/auth/login?[^ ]*" "$LOGIN_LOG" 2>/dev/null | head -1 || true)
  tries=$((tries + 1))
  [ "$tries" -ge 100 ] && { cat "$LOGIN_LOG" >&2; fail "synveda login printed no login URL"; }
  [ -z "$login_url" ] && sleep 0.1
done

# What the browser would do — the AUTH-1/AUTH-2/ADPT-1/OPS-1 dance, unchanged.
authorize_url=$(curl -si "$login_url" | grep -i '^location:' | sed 's/^[Ll]ocation: //' | tr -d '\r')
case "$authorize_url" in
"$RAUTHY_URL"/auth/v1/oidc/authorize\?*) ;;
*) fail "/auth/login did not redirect to the IdP: $authorize_url" ;;
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
      const digest = crypto.createHash("sha256").update(challenge).update(String(counter)).digest();
      if (zeros(digest) >= difficulty) { process.stdout.write(challenge + counter); break; }
    }
  });
')
login_response=$(curl -si -X POST "$RAUTHY_URL/auth/v1/oidc/authorize" \
  -H 'Content-Type: application/json' \
  -H "Cookie: $cookies" -H "x-csrf-token: $csrf" \
  -d "{\"email\":\"$OPERATOR\",\"password\":\"$PASSWORD\",
       \"client_id\":\"synveda\",\"redirect_uri\":\"$GATEWAY_URL/auth/callback\",
       \"state\":\"$login_state\",\"nonce\":\"$login_nonce\",
       \"code_challenge\":\"$login_challenge\",\"code_challenge_method\":\"S256\",
       \"scopes\":[\"openid\",\"profile\",\"email\",\"groups\"],
       \"pow\":\"$pow\"}")
callback_url=$(printf '%s\n' "$login_response" | grep -i '^location:' |
  sed 's/^[Ll]ocation: //' | tr -d '\r')
[ -n "$callback_url" ] || {
  printf '%s\n' "$login_response" | head -5 >&2
  fail "rauthy returned no callback location"
}
handoff=$(curl -si "$callback_url" | grep -i '^location:' | sed 's/^[Ll]ocation: //' | tr -d '\r')
curl -fsS "$handoff" >/dev/null 2>&1 || true
wait "$LOGIN_PID" || { cat "$LOGIN_LOG" >&2; fail "synveda login did not complete"; }
echo "    logged in as $OPERATOR"

ROOT=$(synveda hierarchy root 2>/dev/null)
[ -n "$ROOT" ] || fail "no org root after the first admin login"
echo "    org root provisioned by the login itself: $ROOT"

echo ""
echo "==> [timed] a scope, a turn observed, and the same turn recalled"
ENG=$(synveda hierarchy create --parent "$ROOT" --kind department --slug eng --name Engineering 2>/dev/null | awk '{print $1}')
[ -n "$ENG" ] || fail "hierarchy create"
BEARER=$(synveda auth token 2>/dev/null)
[ -n "$BEARER" ] || fail "no bearer from the stored profile"
observed=$(curl -fsS -X POST "$GATEWAY_URL/v1/observe" \
  -H "Authorization: Bearer $BEARER" -H 'Content-Type: application/json' \
  -d "{\"session_id\":\"ops8-demo-$$\",
       \"events\":[{\"idempotency_key\":\"ops8-$$-1\",
         \"kind\":\"transcript_delta\",
         \"occurred_at\":\"$(date -u +%Y-%m-%dT%H:%M:%SZ)\",
         \"payload\":{\"text\":\"The release ships prebuilt binaries and published images, so installing Synveda needs Docker and nothing else.\"}}]}")
accepted=$(printf '%s\n' "$observed" | json_field accepted)
[ "$accepted" = "1" ] || fail "observe accepted $accepted events, wanted 1"

found=""
tries=0
while [ -z "$found" ]; do
  hit=$(synveda recall --query "what does installing need" --quiet 2>/dev/null || true)
  case "$hit" in
  *Docker*) found=yes ;;
  *)
    tries=$((tries + 1))
    [ "$tries" -ge 60 ] && fail "the observed turn never came back as memory"
    sleep 1
    ;;
  esac
done
echo "    recalled it through the PDP, under the operator's own bearer:"
synveda recall --query "what does installing need" --quiet 2>/dev/null |
  head -6 | sed 's/^/      /'

elapsed=$(($(date +%s) - started))
echo ""
if [ "$elapsed" -gt "$BUDGET_SECS" ]; then
  fail "download → working governed memory took ${elapsed}s, over the ${BUDGET_SECS}s budget"
fi
echo "==> AC: a downloaded release → working governed memory in ${elapsed}s (budget ${BUDGET_SECS}s)"

# ── The proof (untimed) ──────────────────────────────────────────────────
echo ""
echo "==> the console, on the default host-gateway path"
# The gap OPS-8 closed. The image has set SYNVEDA_CONSOLE_DIR since CNSL-1
# and the host process never did, so `/console/` 404'd for everyone without
# a checkout and a pnpm build — quietly, because ADR-0056 decision 1 makes a
# missing bundle a 404 rather than a boot failure.
console_status=$(curl -s -o "$SCRATCH/console.html" -w '%{http_code}' "$GATEWAY_URL/console/")
[ "$console_status" = "200" ] ||
  fail "GET /console/ returned $console_status — the host gateway is not serving the bundle"
grep -qi '<title' "$SCRATCH/console.html" || fail "/console/ served something that is not the console"
echo "    GET /console/ → 200, $(wc -c < "$SCRATCH/console.html" | tr -d ' ') bytes"

echo ""
echo "==> the Claude Code plugin, installed into the harness that runs it"
# What this feature exists for. The gap OPS-8 opened with was that the
# release shipped no plugin at all; the gap it *found* was that
# `~/.claude/plugins/synveda/` — the path the adapter README and ADPT-1's
# demo use — is not a location Claude Code reads. Plugins come from
# marketplaces, so the assertion is the vendor's own view of its state, not
# the presence of files.
if command -v claude >/dev/null 2>&1; then
  synveda plugin install --client claude-code >"$SCRATCH/plugin.log" 2>&1 ||
    { cat "$SCRATCH/plugin.log" >&2; fail "synveda plugin install"; }
  grep -E 'claude plugin|installed' "$SCRATCH/plugin.log" | sed 's/^/    /'

  claude plugin list >"$SCRATCH/plugin-list.txt" 2>&1 || fail "claude plugin list"
  grep -q 'synveda@synveda' "$SCRATCH/plugin-list.txt" ||
    { cat "$SCRATCH/plugin-list.txt" >&2; fail "Claude Code does not have the plugin"; }
  # "Installed" is not "loaded". A manifest naming `hooks/hooks.json` — which
  # this one did — installs perfectly and then reports `✘ failed to load`,
  # so the status line is the assertion and the install is not.
  grep -q 'enabled' "$SCRATCH/plugin-list.txt" ||
    { cat "$SCRATCH/plugin-list.txt" >&2; fail "the plugin installed but did not load"; }

  claude plugin details synveda@synveda >"$SCRATCH/plugin-details.txt" 2>&1 ||
    fail "claude plugin details"
  # The four ADPT-1 seams and the ADPT-2 server. `mcpServers` inline in
  # plugin.json — which this one also had — registers nothing and says
  # nothing, so the count is what catches it.
  for seam in SessionStart Stop PreCompact SessionEnd; do
    grep -q "$seam" "$SCRATCH/plugin-details.txt" ||
      fail "the loaded plugin has no $seam hook"
  done
  grep -qE 'MCP servers \(1\)' "$SCRATCH/plugin-details.txt" ||
    { sed -n '/Component inventory/,/^$/p' "$SCRATCH/plugin-details.txt" >&2
      fail "the loaded plugin registers no MCP server"; }
  sed -n '/Component inventory/,/LSP/p' "$SCRATCH/plugin-details.txt" | sed 's/^/    /'

  # Idempotent, and it says so rather than reinstalling.
  synveda plugin install --client claude-code 2>&1 | grep -q 'already installed' ||
    fail "a second install did not report the plugin as already there"
  echo "    a second install leaves it alone"
else
  # Not a silent skip: the assertion that matters most in this section is
  # the one that cannot run here.
  echo "    SKIPPED — the \`claude\` CLI is not on PATH."
  echo "    The bundle is installed at $INSTALL_ROOT/plugin and"
  echo "    \`synveda plugin install\` is what points Claude Code at it, but"
  echo "    nothing here has proven this release's plugin LOADS. That is the"
  echo "    assertion, and on this machine it did not run."
  synveda plugin install --client claude-code --dry-run 2>&1 | sed 's/^/    /' || true
fi

echo ""
echo "==> a profile from another release is refused rather than run"
# ADR-0065 decision 5. The failure this prevents does not look like a
# version problem — it looks like a service that will not start, or a
# variable the gateway does not read.
cp "$INSTALL_ROOT/profile/version" "$SCRATCH/version.real"
printf '0.0.1-not-this-one\n' > "$INSTALL_ROOT/profile/version"
if synveda init --slug "$SLUG" --name x >"$SCRATCH/mismatch.log" 2>&1; then
  cp "$SCRATCH/version.real" "$INSTALL_ROOT/profile/version"
  fail "init ran against a profile from a different release"
fi
grep -q '0.0.1-not-this-one' "$SCRATCH/mismatch.log" ||
  { cat "$SCRATCH/mismatch.log" >&2; fail "the refusal does not name the version it found"; }
cp "$SCRATCH/version.real" "$INSTALL_ROOT/profile/version"
sed -n '1,3p' "$SCRATCH/mismatch.log" | sed 's/^/    /'

echo ""
echo "==> the chain: who wrote what, in order"
synveda audit tail --tenant "$TENANT" --limit 40 >"$SCRATCH/tail.json" 2>/dev/null ||
  fail "audit tail"
node -e '
  const fs = require("fs");
  const events = fs.readFileSync(process.argv[1], "utf8")
    .split("\n").filter((line) => line.trim()).map((line) => JSON.parse(line));
  const ordered = [...events].reverse();
  for (const e of ordered) {
    const actor = e.actor.kind === "break_glass" ? "BREAK-GLASS" : (e.actor.subject || e.actor.kind);
    console.log("    " + String(e.seq).padStart(4) + "  " + e.action.padEnd(28) + "  " + actor);
  }
  const glass = ordered.filter((e) => e.actor.kind === "break_glass");
  if (glass.length !== 1 || glass[0].action !== "tenant.created") {
    console.error("the installer must leave exactly one break-glass event, the tenant; found " +
      JSON.stringify(glass.map((e) => e.action)));
    process.exit(1);
  }
  for (const action of ["identity.provisioned", "role.bound"]) {
    const e = ordered.find((x) => x.action === action);
    if (!e) { console.error("no " + action + " on the chain"); process.exit(1); }
    if (e.actor.kind === "break_glass") {
      console.error(action + " was written by the installer, not by a login");
      process.exit(1);
    }
  }
  const created = ordered.filter((e) => e.action === "hierarchy.node.created");
  if (!created.length || created.some((e) => !e.payload || !e.payload.authz)) {
    console.error("a scope was created with no authz decision recorded");
    process.exit(1);
  }
  console.log("");
  console.log("    exactly 1 break-glass event (tenant.created) — everything else has an actor");
  console.log("    installing from a release changed none of it");
' "$SCRATCH/tail.json" || fail "the chain does not show what ADR-0055 claims"

echo ""
echo "==> the chain verifies"
synveda audit verify --tenant "$TENANT" | sed 's/^/    /'

echo ""
echo "==> what a tester now has, from three commands and a Docker daemon"
echo "    gateway   $GATEWAY_URL   ($(curl -fsS $GATEWAY_URL/healthz))"
echo "    console   $GATEWAY_URL/console/"
if command -v claude >/dev/null 2>&1; then
  echo "    plugin    synveda@synveda, enabled in Claude Code"
else
  echo "    plugin    installed at $INSTALL_ROOT/plugin (not verified — no claude CLI)"
fi
echo "    tenant    $SLUG ($TENANT)"
echo "    operator  $OPERATOR / $PASSWORD"
echo "    installed $INSTALL_ROOT"
echo ""
echo "    This one is a throwaway: it is torn down below, because it is a"
echo "    scratch compose project under a temp directory rather than the"
echo "    deployment OPS-1 leaves you. Re-run with OPS8_KEEP=1 to keep it."
echo ""
echo "OPS-8 acceptance criterion: PASS (${elapsed}s of ${BUDGET_SECS}s)"
