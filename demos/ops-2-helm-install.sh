#!/usr/bin/env bash
# OPS-2 — the acceptance criterion: a kind-cluster install test.
#
# What it asserts is deliberately not "every pod is Ready". EVAL-3's first
# complete run reported a passing retrieval score over blocks the pipeline
# had not filled, because its validity guard passed precisely when there
# was nothing to validate; an install test that never asks the installation
# to do anything is the same instrument. So this asserts three things a
# readiness check cannot (ADR-0062 decision 7):
#
#   1. a governed round trip — a real OIDC login, AUTH-2 manufacturing the
#      org root from it, a PDP-decided hierarchy write, observe →
#      extraction → embed → inject returning the memory, and the audit
#      chain verifying over all of it;
#   2. a failover — delete the CNPG primary and do it again;
#   3. a live backstop — the gateway's own database role is not a
#      superuser and holds no BYPASSRLS, so TEN-2's forced RLS is actually
#      enforced against it. Every deployment before this one connected as
#      the compose superuser and bypassed it.
#
# It is also the first thing that ever asks the gateway *image* to serve.
# ADR-0055 built it, could not exercise it — the bundled IdP's issuer is a
# `localhost` URL and RFC 6761 makes that the caller's own loopback — and
# recorded this test as where that becomes true. A Service DNS name is a
# real name, so the gateway pod and the client pod resolve the issuer to
# the same place and the comparison ADR-0010 makes holds.
#
# Usage:  demos/ops-2-helm-install.sh            create, assert, tear down
#         KEEP=1 demos/ops-2-helm-install.sh     leave the cluster up
#         REUSE=1 demos/ops-2-helm-install.sh    reuse an existing cluster
set -euo pipefail

CLUSTER=${CLUSTER:-synveda-ops2}
NS=synveda-test
RELEASE=synveda
CNPG_VERSION=${CNPG_VERSION:-1.25.0}
FIXTURES=demos/fixtures/ops-2
IMAGE_TAG=0.1.0
KEEP=${KEEP:-0}
REUSE=${REUSE:-0}

cd "$(dirname "$0")/.."

fail() {
  echo >&2
  echo "demo FAILED: $1" >&2
  shift || true
  [ $# -gt 0 ] && printf '%s\n' "$@" >&2
  diagnostics
  exit 1
}

diagnostics() {
  echo >&2
  echo "--- what the cluster looked like ---" >&2
  kubectl get pods -n "$NS" -o wide 2>&1 | sed 's/^/  /' >&2 || true
  kubectl get cluster -n "$NS" 2>&1 | sed 's/^/  /' >&2 || true
}

cleanup() {
  [ -n "${PORT_FORWARD_PID:-}" ] && kill "$PORT_FORWARD_PID" 2>/dev/null || true
  if [ "$KEEP" = "1" ]; then
    echo
    echo "KEEP=1: the cluster stays. Delete it with:  kind delete cluster --name $CLUSTER"
  else
    kind delete cluster --name "$CLUSTER" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

for tool in docker kind kubectl helm node; do
  command -v "$tool" >/dev/null 2>&1 || fail "$tool is not installed"
done

echo "==> the cluster"
if kind get clusters 2>/dev/null | grep -qx "$CLUSTER"; then
  [ "$REUSE" = "1" ] || fail "a kind cluster named $CLUSTER already exists (REUSE=1 to use it, or delete it)"
  echo "    reusing $CLUSTER"
else
  kind create cluster --name "$CLUSTER" --config "$FIXTURES/kind-cluster.yaml"
fi
kubectl config use-context "kind-$CLUSTER" >/dev/null

# ── the images ───────────────────────────────────────────────────────────
# Built from the Dockerfiles a release builds, not from a thin wrapper
# around a CI-compiled binary: the point of this test is that *this*
# artefact serves (ADR-0062 decision 9).
echo "==> building the product image (this is the slow part; layers cache)"
docker build -t "synveda/gateway:$IMAGE_TAG" -f deploy/compose/gateway/Dockerfile .
echo "==> building the enterprise Postgres image (CNPG base + pgvector + PGMQ, no AGE)"
docker build -t synveda/enterprise-postgres:17 deploy/helm/postgres
echo "==> loading both into the cluster"
kind load docker-image --name "$CLUSTER" "synveda/gateway:$IMAGE_TAG" synveda/enterprise-postgres:17

# ── the operator ─────────────────────────────────────────────────────────
# Cluster-scoped, and installed separately for the reason ADR-0062
# decision 1 gives: a product chart that owns cluster-scoped CRDs fights the
# next chart that wants them.
echo "==> CloudNativePG $CNPG_VERSION"
kubectl apply --server-side -f \
  "https://raw.githubusercontent.com/cloudnative-pg/cloudnative-pg/release-${CNPG_VERSION%.*}/releases/cnpg-${CNPG_VERSION}.yaml" >/dev/null
kubectl wait --for=condition=Available --timeout=300s \
  -n cnpg-system deployment/cnpg-controller-manager ||
  fail "the CloudNativePG operator never became available"

# ── the test issuer ──────────────────────────────────────────────────────
echo "==> the test issuer (Rauthy, at a Service DNS name)"
kubectl apply -f "$FIXTURES/idp.yaml" >/dev/null
kubectl rollout status -n "$NS" deployment/idp --timeout=300s ||
  fail "the test issuer never became ready"

# Administration over a port-forward; the *login* is what has to happen
# inside the cluster, and does.
kubectl port-forward -n "$NS" svc/idp 18080:8080 >/dev/null 2>&1 &
PORT_FORWARD_PID=$!
for _ in $(seq 1 60); do
  curl -fsS http://127.0.0.1:18080/auth/v1/health >/dev/null 2>&1 && break
  sleep 1
done
curl -fsS http://127.0.0.1:18080/auth/v1/health >/dev/null 2>&1 ||
  fail "could not reach the test issuer over the port-forward"

PUBLIC_URL=http://synveda.synveda-test.svc.cluster.local:8120
echo "==> provisioning the client, the admin group and one operator"
ISSUER=$(node "$FIXTURES/idp-bootstrap.mjs" http://127.0.0.1:18080 "$PUBLIC_URL") ||
  fail "could not provision the test issuer"
echo "    issuer, as the discovery document states it: $ISSUER"

# ── the chart ────────────────────────────────────────────────────────────
echo "==> helm install"
helm upgrade --install "$RELEASE" deploy/helm/synveda \
  --namespace "$NS" -f "$FIXTURES/values.yaml" --wait=false ||
  fail "helm install failed"

echo "==> the install job: migrate under the admin identity, grant, admit a tenant"
for _ in $(seq 1 120); do
  phase=$(kubectl get job -n "$NS" -l app.kubernetes.io/component=install \
    -o jsonpath='{.items[0].status.succeeded}' 2>/dev/null || true)
  [ "${phase:-0}" = "1" ] && break
  sleep 5
done
[ "${phase:-0}" = "1" ] || fail "the install job did not succeed" \
  "$(kubectl logs -n "$NS" -l app.kubernetes.io/component=install --all-containers --tail=50 2>&1 || true)"

# `tenant create` prints the created tenant as JSON; the trust entry needs
# its id. This is the ordering an operator meets too — an issuer binds to a
# tenant, and the tenant is admitted by the install (NOTES.txt says so).
TENANT_JSON=$(kubectl logs -n "$NS" -l app.kubernetes.io/component=install -c tenant --tail=20)
TENANT_ID=$(printf '%s' "$TENANT_JSON" | node -e '
  let d = ""; process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => {
    const m = d.match(/"id"\s*:\s*"([0-9a-f-]{36})"/i);
    if (!m) { console.error(d); process.exit(1); }
    console.log(m[1]);
  });') || fail "could not read the admitted tenant id" "$TENANT_JSON"
echo "    tenant $TENANT_ID"

echo "==> the trust entry, now that there is a tenant to bind it to"
kubectl create secret generic synveda-oidc -n "$NS" \
  --from-literal=SYNVEDA_OIDC_ISSUERS="[{\"issuer\":\"$ISSUER\",\"client_id\":\"synveda\",\"tenant\":{\"static\":{\"tenant_id\":\"$TENANT_ID\"}}}]" \
  --dry-run=client -o yaml | kubectl apply -f - >/dev/null

echo "==> the gateway"
kubectl rollout status -n "$NS" deployment/synveda --timeout=600s ||
  fail "the gateway never became ready" \
    "$(kubectl logs -n "$NS" deployment/synveda --all-containers --tail=50 2>&1 || true)"

# ── assertion 3, first because it is cheap and unconditional ─────────────
# The backstop. Decision 2 is worth nothing if the chart can be
# misconfigured back to a superuser DSN with nothing noticing, and until
# this deployment every gateway connected as the compose superuser — which
# bypasses row-level security even where it is FORCED.
echo "==> the backstop is live: the gateway's role is not a superuser"
# Two halves, so the claim is about the gateway and not about a role that
# merely exists: who the gateway's own credential says it is, and what that
# role may do. Asked over the instance's local socket rather than by handing
# a throwaway pod the application password.
GATEWAY_ROLE=$(kubectl get secret -n "$NS" synveda-pg-app -o jsonpath='{.data.username}' |
  base64 -d) || fail "could not read the gateway's database identity"
PG_POD=$(kubectl get pods -n "$NS" -l cnpg.io/cluster=synveda-pg,role=primary \
  -o jsonpath='{.items[0].metadata.name}') || fail "no Postgres primary to ask"
ROLE_FACTS=$(kubectl exec -n "$NS" "$PG_POD" -c postgres -- \
  psql -U postgres -d synveda -tAc \
  "select rolsuper, rolbypassrls, pg_has_role('$GATEWAY_ROLE','synveda_app','member')
     from pg_roles where rolname = '$GATEWAY_ROLE'" 2>/dev/null | tr -d '\r ') ||
  fail "could not ask the database about $GATEWAY_ROLE"
echo "    the gateway connects as $GATEWAY_ROLE — superuser|bypassrls|synveda_app: $ROLE_FACTS"
[ "$ROLE_FACTS" = "f|f|t" ] ||
  fail "$GATEWAY_ROLE is a superuser, holds BYPASSRLS, or is not in synveda_app: $ROLE_FACTS"

# ── assertion 1: the governed round trip ─────────────────────────────────
echo "==> the test client: a real login, and everything downstream of it"
kubectl create configmap install-test-scripts -n "$NS" \
  --from-file="$FIXTURES/client.sh" --from-file="$FIXTURES/browser.mjs" \
  --dry-run=client -o yaml | kubectl apply -f - >/dev/null
kubectl delete pod synveda-install-test -n "$NS" --ignore-not-found >/dev/null
kubectl apply -f "$FIXTURES/client-pod.yaml" >/dev/null

status=""
for _ in $(seq 1 180); do
  status=$(kubectl exec -n "$NS" synveda-install-test -c client -- \
    sh -c 'cat /work/status 2>/dev/null || true' 2>/dev/null || true)
  [ "$status" = "ready" ] && break
  [ "$status" = "failed" ] && break
  sleep 5
done
kubectl logs -n "$NS" synveda-install-test -c client --tail=100 2>/dev/null | sed 's/^/    /' || true
[ "$status" = "ready" ] || fail "the governed round trip did not complete" \
  "$(kubectl logs -n "$NS" synveda-install-test --all-containers --tail=60 2>&1 || true)"

# ── assertion 2: the failover ────────────────────────────────────────────
# The gateway's pool is `connect_lazy` so that a database outage is a
# /readyz report rather than a crash-loop. Nothing has ever tested that
# claim against a real failover.
echo "==> deleting the primary"
PRIMARY=$(kubectl get pods -n "$NS" -l cnpg.io/cluster=synveda-pg,role=primary \
  -o jsonpath='{.items[0].metadata.name}') || fail "no primary to delete"
echo "    $PRIMARY"
kubectl delete pod -n "$NS" "$PRIMARY" --wait=false >/dev/null

for _ in $(seq 1 60); do
  NEW=$(kubectl get pods -n "$NS" -l cnpg.io/cluster=synveda-pg,role=primary \
    -o jsonpath='{.items[0].metadata.name}' 2>/dev/null || true)
  [ -n "$NEW" ] && [ "$NEW" != "$PRIMARY" ] && break
  sleep 2
done
[ -n "${NEW:-}" ] && [ "$NEW" != "$PRIMARY" ] || fail "CloudNativePG did not elect a new primary"
echo "    new primary: $NEW"

echo "==> the same inject, on the other side of the failover"
kubectl exec -n "$NS" synveda-install-test -c client -- sh -ec '
  bearer=$(synveda auth token)
  for i in $(seq 1 60); do
    code=$(curl -sS -o /work/failover-body -w "%{http_code}" \
      -X POST "$SYNVEDA_GATEWAY/v1/inject" \
      -H "Authorization: Bearer $bearer" -H "Content-Type: application/json" \
      -d "{\"task\":\"when does the release train leave\",\"session_id\":\"ops2-failover\"}")
    if [ "${code%${code#2}}" = "2" ] && grep -q "release train leaves" /work/failover-body; then
      echo "    inject succeeded after the failover (attempt $i)"
      exit 0
    fi
    sleep 2
  done
  echo "inject never recovered:"; cat /work/failover-body; exit 1
' || fail "the deployment did not survive losing its primary"

echo
echo "================================================================"
echo "OPS-2 AC: a kind-cluster install of the enterprise profile"
echo "  the chart installed, the job migrated under the admin identity,"
echo "  and the gateway came up as a non-superuser role"
echo "  a real authorization-code + PKCE login provisioned the org root"
echo "  a governed write, observe → extraction → inject, audit verified"
echo "  the primary was deleted and the same inject succeeded again"
echo
echo "  what this does not prove: no browser was involved. This is the"
echo "  protocol path — discovery, JWKS, PKCE, iss and nonce — driven by"
echo "  a script, which is ADPT-2's honesty applied to this feature's"
echo "  own claim."
echo "================================================================"
