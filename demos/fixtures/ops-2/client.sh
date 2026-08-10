#!/bin/sh
# The install test's assertions, from inside the cluster, through the
# product's own client (OPS-2, ADR-0062 decision 7).
#
# What this asserts is a governed round trip, not readiness. "Every pod is
# Ready" is the shape EVAL-3's harness warned about — a validity guard that
# passes when there is nothing to validate — and an install test that never
# asks the installation to do anything is the same instrument.
#
# It stays alive at the end so the demo can `kubectl exec` back in after
# killing the database's primary. The bearer is in this pod's credential
# store, and re-acquiring one after a failover would test the IdP rather
# than the failover.
set -eu

WORK=${WORK_DIR:-/work}
GATEWAY=${SYNVEDA_GATEWAY:?SYNVEDA_GATEWAY must be set}
export SYNVEDA_GATEWAY

fail() {
  echo "client FAILED: $1" >&2
  [ $# -gt 1 ] && printf '%s\n' "$2" >&2
  echo "failed" >"$WORK/status"
  exit 1
}

# ── login ────────────────────────────────────────────────────────────────
echo "==> synveda login (the browser half is the sibling container)"
synveda login --no-browser >"$WORK/login.log" 2>&1 &
LOGIN_PID=$!

waited=0
while [ ! -f "$WORK/browser.done" ]; do
  if [ -f "$WORK/browser.failed" ]; then
    fail "the browser container gave up" "$(cat "$WORK/browser.failed"; cat "$WORK/login.log")"
  fi
  waited=$((waited + 1))
  [ "$waited" -ge 600 ] && fail "the browser never completed the login" "$(cat "$WORK/login.log")"
  sleep 1
done
wait "$LOGIN_PID" || fail "synveda login did not complete" "$(cat "$WORK/login.log")"
cat "$WORK/login.log"

# ── who the deployment thinks we are ─────────────────────────────────────
# The org root, the identity and its role binding were created by that
# login and by nothing else: `synveda init` has no equivalent here, and the
# chart's install job writes the migrations and one tenant row (ADR-0055
# decision 1). This is the first assertion because everything after it
# depends on a hierarchy that an installer never touched.
echo "==> whoami — the tenant the issuer bound us to"
whoami_json=$(synveda whoami --json) || fail "whoami failed"
printf '%s\n' "$whoami_json"
case "$whoami_json" in
*'"slug":"acme"'*) ;;
*) fail "the bearer did not resolve to the admitted tenant" "$whoami_json" ;;
esac

# Where AUTH-2 put us, which is the assertion that matters and is not in
# whoami's answer: the CLI prints the placement the login produced. Under
# `acme/` means the org root was manufactured from the tenant's own slug
# and the admin-group subject landed beneath it rather than in quarantine
# (ADR-0015 decision 6) — the whole reason this install seeds no hierarchy.
grep -q "in tenant acme at acme/" "$WORK/login.log" ||
  fail "the login did not place the operator under a manufactured org root" "$(cat "$WORK/login.log")"
echo "    $(grep -o 'logged in as .*' "$WORK/login.log" | head -1)"

echo "==> the org root's id — the parent every first create needs"
root_json=$(synveda hierarchy root --json) || fail "hierarchy root failed"
printf '%s\n' "$root_json"
scope_id=$(printf '%s' "$root_json" |
  sed -n 's/.*"id"[[:space:]]*:[[:space:]]*"\([0-9a-f-]\{36\}\)".*/\1/p' | head -1)
[ -n "$scope_id" ] || fail "could not read the org root's id" "$root_json"

# ── a governed write ─────────────────────────────────────────────────────
echo "==> synveda hierarchy create — a PDP decision, over HTTP, under our own bearer"
# A fresh slug each run. A sibling slug is unique and immutable, so a
# fixed one turns the second run of this demo into an assertion about a
# uniqueness constraint instead of about authorisation — which is what the
# first re-run proved by failing here.
team_slug="platform-$(date +%s)"
team=$(synveda hierarchy create \
  --parent "$scope_id" --kind team --slug "$team_slug" --name Platform 2>&1) ||
  fail "hierarchy create was refused" "$team"
printf '%s\n' "$team"
case "$team" in
*"$team_slug"*) ;;
*) fail "hierarchy create returned no node for $team_slug" "$team" ;;
esac

# ── the data path ────────────────────────────────────────────────────────
# `synveda auth token` is the supported way to hand the stored bearer to
# something that is not the CLI; nothing here reads the credential file.
bearer=$(synveda auth token) || fail "could not read the stored bearer"
now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
session="ops2-install-test"
fact="the release train leaves on Thursday at 14:00 UTC"

post() {
  curl -sS -o "$WORK/body" -w '%{http_code}' \
    -X POST "$GATEWAY$1" \
    -H "Authorization: Bearer $bearer" \
    -H 'Content-Type: application/json' \
    -d "$2"
}

echo "==> observe — one signal, through /v1 and the PDP"
# A fresh idempotency key each run, and `accepted` asserted rather than
# just the status. A fixed key made the second run of this demo report
# `accepted:0 duplicates:1` and pass anyway — which is the failure mode
# this whole feature is written against: a check that holds when there is
# nothing for it to hold about.
key="ops2-$(date +%s)"
status=$(post /v1/observe "{\"session_id\":\"$session\",\"events\":[
  {\"idempotency_key\":\"$key\",\"kind\":\"decision\",
   \"payload\":{\"decision\":\"$fact\"},\"occurred_at\":\"$now\"}]}")
case "$status" in
20*) ;;
*) fail "observe answered $status" "$(cat "$WORK/body")" ;;
esac
cat "$WORK/body"; echo
grep -q '"accepted":1' "$WORK/body" ||
  fail "observe admitted nothing new" "$(cat "$WORK/body")"

echo "==> inject — extraction, embedding and the sidecar, until the memory comes back"
tries=0
while :; do
  status=$(post /v1/inject "{\"task\":\"when does the release train leave\",\"session_id\":\"$session\"}")
  case "$status" in
  20*) ;;
  *) fail "inject answered $status" "$(cat "$WORK/body")" ;;
  esac
  grep -q "release train leaves" "$WORK/body" && break
  tries=$((tries + 1))
  [ "$tries" -ge 120 ] && fail "the seeded memory never came back from inject" "$(cat "$WORK/body")"
  sleep 1
done
echo "    the block carries it after ${tries}s"

# The chain is verified too, but not from here. `synveda audit verify`
# walks the chain in the database and recomputes every hash — an operator's
# check with a database connection, not an API call — and this container
# deliberately holds no database credential: everything above went through
# `/v1` under a bearer. The demo runs it in a pod of its own.

echo "ready" >"$WORK/status"
echo
echo "==> the round trip holds. staying up for the failover assertion."
# The demo execs back in here after deleting the primary. Not `sleep
# infinity`: a pod that outlives a demo somebody interrupted is litter.
sleep 1800
