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
echo "==> whoami — the org root was manufactured by logging in"
whoami_json=$(synveda whoami --json) || fail "whoami failed"
printf '%s\n' "$whoami_json"
scope_path=$(printf '%s' "$whoami_json" | sed -n 's/.*"scope_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
scope_id=$(printf '%s' "$whoami_json" | sed -n 's/.*"scope_id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
[ -n "$scope_path" ] || fail "whoami named no scope path" "$whoami_json"
[ -n "$scope_id" ] || fail "whoami named no scope id" "$whoami_json"
case "$whoami_json" in
*'"quarantined":true'*) fail "the operator was quarantined — the admin group mapping did not hold" "$whoami_json" ;;
esac
echo "    placed at $scope_path"

# ── a governed write ─────────────────────────────────────────────────────
echo "==> synveda hierarchy create — a PDP decision, over HTTP, under our own bearer"
team=$(synveda hierarchy create \
  --parent "$scope_id" --kind team --slug platform --name Platform 2>&1) ||
  fail "hierarchy create was refused" "$team"
printf '%s\n' "$team"

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
status=$(post /v1/observe "{\"session_id\":\"$session\",\"events\":[
  {\"idempotency_key\":\"ops2-1\",\"kind\":\"decision\",
   \"payload\":{\"decision\":\"$fact\"},\"occurred_at\":\"$now\"}]}")
case "$status" in
20*) ;;
*) fail "observe answered $status" "$(cat "$WORK/body")" ;;
esac
cat "$WORK/body"; echo

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

# ── the chain ────────────────────────────────────────────────────────────
echo "==> audit verify — everything above is on the tenant's hash chain"
verify=$(synveda audit verify 2>&1) || fail "the audit chain did not verify" "$verify"
printf '%s\n' "$verify"

echo "ready" >"$WORK/status"
echo
echo "==> the round trip holds. staying up for the failover assertion."
# The demo execs back in here after deleting the primary. Not `sleep
# infinity`: a pod that outlives a demo somebody interrupted is litter.
sleep 1800
