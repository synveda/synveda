#!/usr/bin/env sh
# OPS-1 acceptance demo: the SMB profile (ADR-0055).
# AC (docs/backlog/OPS-1.md): laptop → working governed memory in
# <10 minutes, documented.
#
# The criterion is a stopwatch, so the demo is split where ADPT-1's is —
# at the person, for the same reason (ADR-0055 decision 6):
#
#   the images (untimed)   `docker compose pull` and one local build of
#                          the gateway image. A release ships this; an
#                          operator being timed has already got it.
#
#   the install (TIMED,    everything a person does, on a laptop with no
#   budget 600s)           Synveda state at all: `synveda init`, one
#                          browser login, five `hierarchy create`s, a
#                          turn observed and the same turn recalled —
#                          governed and audited end to end.
#
#   the proof (untimed)    the chain, read in order, showing what wrote
#                          what: exactly ONE break-glass event (the
#                          tenant), then the org root and the operator's
#                          own org-admin binding arriving under the
#                          operator's subject at first login, then every
#                          scope carrying its own PDP decision. That
#                          ordering IS ADR-0055 decisions 1 and 2 — an
#                          installer that seeds an org would show the
#                          hierarchy under a break-glass actor instead.
#
# The one thing here that is not a `synveda` verb is the observed turn,
# which goes to /v1/observe with curl: observing is a harness's job
# (ADPT-1's plugin does it on every session) and OPS-1 does not claim to
# add an operator verb for it. Everything else is the product's own CLI.
#
# Needs docker, node, and a Rust toolchain. On Windows, run via Git Bash.
set -eu

cd "$(dirname "$0")/.."
REPO=$(pwd)

COMPOSE="docker compose -f deploy/compose/docker-compose.yml"
RAUTHY_URL=http://localhost:8100
GATEWAY_URL=http://127.0.0.1:8120
SLUG=ops1-$$
OPERATOR="operator@$SLUG.localhost"
PASSWORD='Synveda-Demo-Passw0rd!'
BUDGET_SECS=600

DEMO_HOME=$(mktemp -d "${TMPDIR:-/tmp}/ops1-home-XXXXXX")
cleanup() {
  # Only the scratch HOME goes.
  #
  # The deployment is deliberately left running — this demo's whole subject
  # is an instance that survives, and a demo that tore it down would be
  # asserting the opposite of its own feature. That extends to the tenant,
  # the org root and the operator identity in the IdP: the last thing this
  # script prints is those credentials and an invitation to go and use
  # them, so removing any of it on exit would make the demo lie.
  #
  # The cost is that each run leaves one more tenant and one more operator
  # behind, and nothing reaps them — see the teardown line at the end,
  # which exists because there was no documented way to remove a demo
  # deployment you were finished with. (This comment used to claim "this
  # run's tenant" was cleaned up here. It never was.)
  rm -rf "$DEMO_HOME"
  return 0
}
trap cleanup EXIT INT TERM

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

fail() {
  echo "" >&2
  echo "demo FAILED: $1" >&2
  exit 1
}

# ── The images (untimed) ─────────────────────────────────────────────────
echo "==> the images: what a release ships and an operator pulls"
cargo build -q -p synveda-cli
SYNVEDA="$REPO/target/debug/synveda"
$COMPOSE pull --quiet postgres rauthy jaeger 2>/dev/null || true
$COMPOSE build --quiet gateway
echo "    gateway image built; the clock starts now"

# ── The install (TIMED) ──────────────────────────────────────────────────
started=$(date +%s)

# A scratch HOME: no credentials, no profile, no configuration. This is
# what "laptop" means in the acceptance criterion — no *Synveda* state.
#
# Docker's CLI plugins live under the real home, and `docker compose` is a
# plugin: without this, moving HOME hides the compose subcommand and the
# installer fails at step 1 with `unknown shorthand flag: -f`. Docker being
# installed is a prerequisite of the demo in the same way node and cargo
# are, so its own configuration stays where it is.
export DOCKER_CONFIG="${DOCKER_CONFIG:-$HOME/.docker}"
export HOME="$DEMO_HOME"
export XDG_CONFIG_HOME="$DEMO_HOME/.config"
export XDG_STATE_HOME="$DEMO_HOME/.local/state"
mkdir -p "$XDG_CONFIG_HOME" "$XDG_STATE_HOME"

echo ""
echo "==> [timed] synveda init"
"$SYNVEDA" init --slug "$SLUG" --name "ACME ($SLUG)" >"$DEMO_HOME/init.log" 2>&1 ||
  { cat "$DEMO_HOME/init.log" >&2; fail "synveda init"; }
grep -E '^==>|already admitted|healthy' "$DEMO_HOME/init.log" | sed 's/^/    /'

TENANT=$(grep '^SYNVEDA_TENANT_ID=' deploy/compose/.env | cut -d= -f2)
[ -n "$TENANT" ] || fail "init wrote no tenant id"

# The invariant ADR-0055 decision 1 is about, asserted before anything
# else happens: at this point the installer has run to completion, and the
# tenant must be the ONLY thing it wrote. No scope, no identity, no role
# binding, no record.
export DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
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
LOGIN_LOG="$DEMO_HOME/login.log"
"$SYNVEDA" login --gateway "$GATEWAY_URL" --no-browser >"$LOGIN_LOG" 2>&1 &
LOGIN_PID=$!
tries=0
login_url=""
while [ -z "$login_url" ]; do
  login_url=$(grep -o "$GATEWAY_URL/auth/login?[^ ]*" "$LOGIN_LOG" 2>/dev/null | head -1 || true)
  tries=$((tries + 1))
  [ "$tries" -ge 100 ] && { cat "$LOGIN_LOG" >&2; fail "synveda login printed no login URL"; }
  [ -z "$login_url" ] && sleep 0.1
done

# What the browser would do: Rauthy session, proof-of-work, credentials,
# then the gateway callback. Straight from the AUTH-1/AUTH-2/ADPT-1 demos.
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

# AUTH-2 provisioned the org root from the tenant's own slug and name, and
# AUTHZ-3 bound tenant-wide org-admin from the `synveda-admins` group —
# neither of them written by the installer (ADR-0055 decision 2).
ROOT=$("$SYNVEDA" hierarchy root 2>/dev/null)
[ -n "$ROOT" ] || fail "no org root after the first admin login"
echo "    org root provisioned by the login itself: $ROOT"

echo ""
echo "==> [timed] synveda hierarchy — ACME's shape, one PDP decision per node"
create() { "$SYNVEDA" hierarchy create --parent "$1" --kind "$2" --slug "$3" --name "$4" 2>/dev/null; }
ENG=$(create "$ROOT" department eng "Engineering" | awk '{print $1}')
SALES=$(create "$ROOT" department sales "Sales" | awk '{print $1}')
PLATFORM=$(create "$ENG" team platform "Platform" | awk '{print $1}')
create "$ENG" team payments "Payments" >/dev/null
create "$SALES" team emea "EMEA" >/dev/null
"$SYNVEDA" hierarchy list 2>/dev/null | sed 's/^/    /'

echo ""
echo "==> [timed] a turn observed, and the same turn recalled"
BEARER=$("$SYNVEDA" auth token 2>/dev/null)
[ -n "$BEARER" ] || fail "no bearer from the stored profile"
observed=$(curl -fsS -X POST "$GATEWAY_URL/v1/observe" \
  -H "Authorization: Bearer $BEARER" -H 'Content-Type: application/json' \
  -d "{\"session_id\":\"ops1-demo-$$\",
       \"events\":[{\"idempotency_key\":\"ops1-$$-1\",
         \"kind\":\"transcript_delta\",
         \"occurred_at\":\"$(date -u +%Y-%m-%dT%H:%M:%SZ)\",
         \"payload\":{\"text\":\"We settled on blue-green rollouts for the payments service, because a rollback has to be one DNS change and not a redeploy.\"}}]}")
accepted=$(printf '%s\n' "$observed" | json_field accepted)
[ "$accepted" = "1" ] || fail "observe accepted $accepted events, wanted 1"
echo "    observed 1 event; the extraction worker is asynchronous, so we wait for the record"

# MEM-3/MEM-4: extract, scan, embed, commit. Poll the governed read rather
# than the table — what matters is that it comes back through the PDP.
found=""
tries=0
while [ -z "$found" ]; do
  hit=$("$SYNVEDA" recall --query "how do we roll out payments" --quiet 2>/dev/null || true)
  case "$hit" in
  *blue-green*) found=yes ;;
  *)
    tries=$((tries + 1))
    [ "$tries" -ge 60 ] && fail "the observed turn never came back as memory"
    sleep 1
    ;;
  esac
done
echo "    recalled it through the PDP, under the operator's own bearer:"
"$SYNVEDA" recall --query "how do we roll out payments" --quiet 2>/dev/null |
  head -8 | sed 's/^/      /'

elapsed=$(($(date +%s) - started))
echo ""
if [ "$elapsed" -gt "$BUDGET_SECS" ]; then
  fail "laptop → working governed memory took ${elapsed}s, over the ${BUDGET_SECS}s budget"
fi
echo "==> AC: laptop → working governed memory in ${elapsed}s (budget ${BUDGET_SECS}s)"

# ── The proof (untimed) ──────────────────────────────────────────────────
echo ""
echo "==> the chain: who wrote what, in order"
"$SYNVEDA" audit tail --tenant "$TENANT" --limit 40 >"$DEMO_HOME/tail.json" 2>/dev/null ||
  fail "audit tail"
node -e '
  const fs = require("fs");
  // `audit tail` prints JSON lines, newest first.
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
  // Decision 2: the root and the org-admin binding are the operator s,
  // not an installer s.
  for (const action of ["identity.provisioned", "role.bound"]) {
    const e = ordered.find((x) => x.action === action);
    if (!e) { console.error("no " + action + " on the chain"); process.exit(1); }
    if (e.actor.kind === "break_glass") {
      console.error(action + " was written by the installer, not by a login");
      process.exit(1);
    }
  }
  // Decision 1/3: every authored scope carries its own PDP decision.
  const created = ordered.filter((e) => e.action === "hierarchy.node.created");
  if (created.length < 5) {
    console.error("expected 5 authored scopes on the chain, found " + created.length);
    process.exit(1);
  }
  for (const e of created) {
    if (!e.payload || !e.payload.authz) {
      console.error("a scope was created with no authz decision recorded: " + JSON.stringify(e.payload));
      process.exit(1);
    }
  }
  console.log("");
  console.log("    exactly 1 break-glass event (tenant.created) — everything else has an actor");
  console.log("    " + created.length + " authored scopes, each carrying its own PDP decision");
' "$DEMO_HOME/tail.json" || fail "the chain does not show what ADR-0055 claims"

echo ""
echo "==> the chain verifies"
"$SYNVEDA" audit verify --tenant "$TENANT" | sed 's/^/    /'

echo ""
echo "==> the deployment is still running — that is the feature"
echo "    gateway   $GATEWAY_URL   ($(curl -fsS $GATEWAY_URL/healthz))"
echo "    tenant    $SLUG ($TENANT)"
echo "    operator  $OPERATOR / $PASSWORD"
echo "    traces    http://localhost:16686"
echo ""
echo "    docker compose -f deploy/compose/docker-compose.yml down    # stops it; state persists"
echo ""
echo "    when you are finished with this deployment, its tenant and its"
echo "    operator outlive the container — and there is no way to remove"
echo "    just this tenant yet. 32 foreign keys reference \`tenants\`, every"
echo "    one ON DELETE NO ACTION, so \`delete from tenants\` succeeds only for"
echo "    a tenant that holds nothing — never one somebody has logged into."
echo "    Per-tenant erasure is TEN-5. What works today:"
echo ""
echo "      synveda tenant export --tenant $TENANT   # keep it first (TEN-4)"
echo "      docker compose -f deploy/compose/docker-compose.yml down -v   # wipes ALL tenants"
echo "      curl -X DELETE \$RAUTHY/auth/v1/users/<id-of-$OPERATOR> -H \"Authorization: \$RAUTHY_API_KEY\""
echo ""
echo "OPS-1 acceptance criterion: PASS (${elapsed}s of ${BUDGET_SECS}s)"
