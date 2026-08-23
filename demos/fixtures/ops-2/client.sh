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
# depends on a scope tree that an installer never touched.
echo "==> whoami — the tenant the issuer bound us to"
whoami_json=$(synveda whoami --json) || fail "whoami failed"
printf '%s\n' "$whoami_json"
case "$whoami_json" in
*'"slug":"acme"'*) ;;
*) fail "the bearer did not resolve to the admitted tenant" "$whoami_json" ;;
esac

# Where AUTH-2 put us, which is the assertion that matters and is not in
# whoami's answer: the CLI prints the placement the login produced. `acme`
# means the tenant root was manufactured from the tenant's own slug and the
# admin-group subject was granted at it rather than left ungranted
# (ADR-0074 decision 4) — the whole reason this install seeds no tree.
grep -q "in tenant acme" "$WORK/login.log" ||
  fail "the login did not grant the operator at the tenant root" "$(cat "$WORK/login.log")"
echo "    $(grep -o 'logged in as .*' "$WORK/login.log" | head -1)"

# ── the data path ────────────────────────────────────────────────────────
# `synveda auth token` is the supported way to hand the stored bearer to
# something that is not the CLI; nothing here reads the credential file.
bearer=$(synveda auth token) || fail "could not read the stored bearer"
now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
fact="the release train leaves on Thursday at 14:00 UTC"

# post <path> <body> [idempotency-key] — the key is required on the two
# routes that create (CPR-4), and sending one where it is not asked for is
# harmless, so it is passed only where the contract wants it.
post() {
  curl -sS -o "$WORK/body" -w '%{http_code}' \
    -X POST "$GATEWAY$1" \
    -H "Authorization: Bearer $bearer" \
    -H 'Content-Type: application/json' \
    ${3:+-H "Idempotency-Key: $3"} \
    -d "$2"
}

# ── a governed write ─────────────────────────────────────────────────────
# A workspace, which since CPR-4 is a person's first act: it mints the
# tenant root's child and an `owner` grant for its creator in the same
# transaction, and nobody is asked to declare an organisation first.
#
# A fresh slug each run. A slug is unique and immutable, so a fixed one
# turns the second run of this demo into an assertion about a uniqueness
# constraint instead of about authorisation — which is what the first
# re-run of the hierarchy version of this proved by failing here.
echo "==> POST /v1/workspaces — a PDP decision, over HTTP, under our own bearer"
ws_slug="platform-$(date +%s)"
status=$(post /v1/workspaces \
  "{\"slug\":\"$ws_slug\",\"display_name\":\"Platform\"}" "ops2-ws-$ws_slug")
case "$status" in
20*) ;;
*) fail "creating a workspace was refused with $status" "$(cat "$WORK/body")" ;;
esac
cat "$WORK/body"; echo
workspace_id=$(sed -n 's/.*"id"[[:space:]]*:[[:space:]]*"\([0-9a-f-]\{36\}\)".*/\1/p' \
  "$WORK/body" | head -1)
[ -n "$workspace_id" ] || fail "could not read the workspace's id" "$(cat "$WORK/body")"

# ── a run ────────────────────────────────────────────────────────────────
# Since CPR-10 an observation belongs to a run, and the governed scope it
# is decided at is DERIVED from the workspace rather than sent — so this
# body names a workspace and nothing else about placement.
echo "==> POST /v1/sessions — the run everything below is recorded against"
status=$(post /v1/sessions \
  "{\"workspace_id\":\"$workspace_id\",\"client_name\":\"ops2-install-test\",\
\"external_session_id\":\"ops2-install-test\"}" "ops2-run-$ws_slug")
case "$status" in
20*) ;;
*) fail "opening a run answered $status" "$(cat "$WORK/body")" ;;
esac
session_id=$(sed -n 's/.*"id"[[:space:]]*:[[:space:]]*"\([0-9a-f-]\{36\}\)".*/\1/p' \
  "$WORK/body" | head -1)
[ -n "$session_id" ] || fail "could not read the run's id" "$(cat "$WORK/body")"
echo "    run $session_id"
# The failover half of this demo execs back in and needs the same run.
printf '%s\n' "$session_id" >"$WORK/session-id"

echo "==> append one event — through /v1 and the PDP"
# A fresh client event id each run, and `appended` asserted rather than
# just the status. A fixed id made the second run of this demo report
# `appended:0 duplicates:1` and pass anyway — which is the failure mode
# this whole feature is written against: a check that holds when there is
# nothing for it to hold about.
key="ops2-$(date +%s)"
status=$(post "/v1/sessions/$session_id/events" "{\"events\":[
  {\"client_event_id\":\"$key\",\"event_type\":\"message.assistant\",
   \"payload\":{\"text\":\"$fact\"},\"occurred_at\":\"$now\"}]}")
case "$status" in
20*) ;;
*) fail "appending answered $status" "$(cat "$WORK/body")" ;;
esac
cat "$WORK/body"; echo
grep -q '"appended":1' "$WORK/body" ||
  fail "the append admitted nothing new" "$(cat "$WORK/body")"

echo "==> a context run — extraction, embedding and the sidecar, until the memory comes back"
tries=0
while :; do
  status=$(post "/v1/sessions/$session_id/context-runs" \
    '{"query":"when does the release train leave"}' "ops2-run-$tries-$$")
  case "$status" in
  20*) ;;
  *) fail "the context run answered $status" "$(cat "$WORK/body")" ;;
  esac
  grep -q "release train leaves" "$WORK/body" && break
  tries=$((tries + 1))
  [ "$tries" -ge 120 ] &&
    fail "the seeded memory never came back from a context run" "$(cat "$WORK/body")"
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
