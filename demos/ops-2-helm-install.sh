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
# The chart's own appVersion, never a copy of it. `values.yaml` leaves
# `image.tag` empty so `_helpers.tpl` resolves it from appVersion, and this
# demo builds the image the chart will then ask for by name — with
# `pullPolicy: Never`, since nothing is published to a registry the kind
# cluster can reach.
#
# It was `IMAGE_TAG=0.1.0`, hardcoded, and the first version bump after that
# broke this demo rather than the chart: the pod stayed on
# `ErrImageNeverPull` for "synveda/gateway:0.1.1 is not present" while a
# perfectly good 0.1.0 image sat in the cluster. Two version sources for one
# artefact, and the failure surfaces ten minutes downstream of the typo.
IMAGE_TAG=$(awk -F'"' '/^appVersion:/{print $2; exit}' deploy/helm/synveda/Chart.yaml)
[ -n "$IMAGE_TAG" ] || { echo "no appVersion in deploy/helm/synveda/Chart.yaml" >&2; exit 1; }
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
  # Why a pod is not running is the question a pod list never answers. The
  # first CI failure was a Pending pod with no node, and `Pending` alone
  # does not distinguish "no CPU" from "disk pressure" from "no volume".
  for pod in $(kubectl get pods -n "$NS" \
    --field-selector=status.phase!=Running,status.phase!=Succeeded \
    -o jsonpath='{.items[*].metadata.name}' 2>/dev/null); do
    echo "--- why $pod is not running ---" >&2
    kubectl describe pod -n "$NS" "$pod" 2>&1 | sed -n '/^Events:/,$p' | sed 's/^/  /' >&2 || true
  done
  kubectl describe node 2>&1 | sed -n '/^Conditions:/,/^Addresses:/p' | sed 's/^/  /' >&2 || true
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
echo "==> building the enterprise Postgres image (CNPG base + pgvector)"
docker build -t synveda/enterprise-postgres:17 deploy/helm/postgres
echo "==> loading both into the cluster"
kind load docker-image --name "$CLUSTER" "synveda/gateway:$IMAGE_TAG" synveda/enterprise-postgres:17
# Two multi-stage Rust builds leave a build cache the size of the images
# themselves, on a runner that then has to hold a Postgres cluster's
# volumes. Reclaiming it is free here and is the difference between a
# scheduled pod and a Pending one.
docker builder prune --force >/dev/null 2>&1 || true

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

# The trust entry needs the tenant's id, and this is the ordering an
# operator meets too: an issuer binds to a tenant, and the tenant is
# admitted by the install (NOTES.txt says so).
#
# Asked of the database rather than scraped from the install job's log.
# The first draft read the log, which worked once and then failed on the
# second run of this demo for a good reason: tenant admission is an
# install-only step, so an upgrade's job correctly prints that it skipped
# it and there is no JSON to parse. A fact about the deployment should be
# read from the deployment.
PG_POD=$(kubectl get pods -n "$NS" -l cnpg.io/cluster=synveda-pg,role=primary \
  -o jsonpath='{.items[0].metadata.name}') || fail "no Postgres primary to ask"
TENANT_ID=""
for _ in $(seq 1 30); do
  TENANT_ID=$(kubectl exec -n "$NS" "$PG_POD" -c postgres -- \
    psql -U postgres -d synveda -tAc \
    "select id from tenants where slug = 'acme'" 2>/dev/null | tr -d '\r ' || true)
  [ -n "$TENANT_ID" ] && break
  sleep 2
done
[ -n "$TENANT_ID" ] || fail "the install job admitted no tenant with slug acme" \
  "$(kubectl logs -n "$NS" -l app.kubernetes.io/component=install -c tenant --tail=20 2>&1 || true)"
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
# `__IMAGE_TAG__` rather than a literal, for the reason IMAGE_TAG is
# read from the chart: three copies of one version agreed only while
# all three were hardcoded to the same value, and the first bump left
# this pod on ErrImageNeverPull for an image the demo no longer builds.
sed "s/__IMAGE_TAG__/$IMAGE_TAG/" "$FIXTURES/client-pod.yaml" |
  kubectl apply -f - >/dev/null

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

# ── and the chain over all of it ─────────────────────────────────────────
echo "==> audit verify — under the gateway's own least-privilege role"
kubectl delete job audit-verify -n "$NS" --ignore-not-found >/dev/null
sed -e "s/__TENANT_ID__/$TENANT_ID/" -e "s/__IMAGE_TAG__/$IMAGE_TAG/" \
  "$FIXTURES/audit-verify-job.yaml" |
  kubectl apply -f - >/dev/null
kubectl wait --for=condition=complete --timeout=120s -n "$NS" job/audit-verify >/dev/null 2>&1 ||
  fail "the audit chain did not verify" \
    "$(kubectl logs -n "$NS" job/audit-verify --tail=20 2>&1 || true)"
kubectl logs -n "$NS" job/audit-verify --tail=5 | sed 's/^/    /'

# ── assertion 2: the failover ────────────────────────────────────────────
# The gateway's pool is `connect_lazy` so that a database outage is a
# /readyz report rather than a crash-loop. Nothing has ever tested that
# claim against a real failover.
# A cluster that has not finished building its replicas has nothing to
# promote, and CloudNativePG's correct answer to that is to recreate the
# instance rather than fail over. The first run of this assertion killed
# the primary at `INSTANCES 2 … Creating a new replica` and then declared
# that CNPG "did not elect a new primary" — it was asserting bootstrap and
# calling it failover. The gateway is ready as soon as there is a primary,
# so nothing earlier in this demo waits for the rest.
echo "==> waiting for all ${POSTGRES_INSTANCES:-3} instances before killing one"
kubectl wait --for=condition=Ready --timeout=600s -n "$NS" cluster/synveda-pg ||
  fail "the Postgres cluster never became fully ready" \
    "$(kubectl get cluster -n "$NS" synveda-pg -o wide 2>&1 || true)"
kubectl get cluster -n "$NS" synveda-pg | sed 's/^/    /'

echo "==> deleting the primary"
PRIMARY=$(kubectl get pods -n "$NS" -l cnpg.io/cluster=synveda-pg,role=primary \
  -o jsonpath='{.items[0].metadata.name}') || fail "no primary to delete"
echo "    $PRIMARY"
kubectl delete pod -n "$NS" "$PRIMARY" --wait=false >/dev/null

# Wait for the promotion to have *happened* before asserting anything about
# serving, and take CloudNativePG's own word for it (`status.currentPrimary`)
# rather than a pod label — the label lagged the status by more than two
# minutes in an earlier run.
#
# The order matters more than it looks. A first draft asserted composition
# straight after `kubectl delete pod`, and it passed on the first attempt:
# the old primary was still `Terminating` and still serving, so the check
# succeeded precisely because nothing had failed over yet. This is the
# EVAL-3 failure mode wearing a different hat, and it was caught by the one
# line that compared the new primary to the old one.
echo "==> waiting for a different instance to be promoted"
NEW=""
for _ in $(seq 1 150); do
  NEW=$(kubectl get cluster -n "$NS" synveda-pg \
    -o jsonpath='{.status.currentPrimary}' 2>/dev/null || true)
  [ -n "$NEW" ] && [ "$NEW" != "$PRIMARY" ] && break
  sleep 2
done
[ -n "$NEW" ] && [ "$NEW" != "$PRIMARY" ] ||
  fail "CloudNativePG never promoted another instance" \
    "$(kubectl get cluster -n "$NS" synveda-pg -o wide 2>&1 || true)"
echo "    promoted $NEW (was $PRIMARY)"

echo "==> the same context run, on the other side of the failover"
# The SAME run the client opened before the primary was deleted, read back
# from the file it wrote. Opening a fresh one here would test that the
# gateway can still create a session — a weaker claim than that a run which
# existed before the failover still composes after it.
kubectl exec -n "$NS" synveda-install-test -c client -- sh -ec '
  bearer=$(synveda auth token)
  run=$(cat /work/session-id)
  for i in $(seq 1 120); do
    code=$(curl -sS -o /work/failover-body -w "%{http_code}" \
      -X POST "$SYNVEDA_GATEWAY/v1/sessions/$run/context-runs" \
      -H "Authorization: Bearer $bearer" -H "Content-Type: application/json" \
      -H "Idempotency-Key: ops2-failover-$i" \
      -d "{\"query\":\"when does the release train leave\"}")
    if [ "${code%${code#2}}" = "2" ] && grep -q '"id"' /work/failover-body; then
      echo "    the context run succeeded after the failover (attempt $i)"
      exit 0
    fi
    sleep 2
  done
  echo "the context run never recovered:"; cat /work/failover-body; exit 1
' || fail "the deployment did not survive losing its primary" \
  "$(kubectl get cluster -n "$NS" synveda-pg -o wide 2>&1 || true)"

echo
echo "================================================================"
echo "OPS-2 AC: a kind-cluster install of the one Synveda runtime"
echo "  the chart installed, the job migrated under the admin identity,"
echo "  and the gateway came up as a non-superuser role"
echo "  a real authorization-code + PKCE login provisioned the tenant root"
echo "  a governed write, session append → context run, audit verified"
echo "  the primary was deleted and the same run composed again"
echo
echo "  what this does not prove: no browser was involved. This is the"
echo "  protocol path — discovery, JWKS, PKCE, iss and nonce — driven by"
echo "  a script, which is ADPT-2's honesty applied to this feature's"
echo "  own claim."
echo "================================================================"
