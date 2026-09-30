#!/usr/bin/env bash
# OPS-7: test-only invalid Cedar revision in the isolated Kind installation.
# The fault helper uses ordinary gateway-role RLS and chains content-free audit.
set -euo pipefail

cd "$(dirname "$0")/.."
[ "$(kubectl config current-context)" = "kind-synveda-ops7" ] || {
  echo "OPS-7 probe requires the isolated kind-synveda-ops7 context" >&2
  exit 78
}
kind get clusters | grep -qx synveda-ops7 || {
  echo "OPS-7 Kind cluster is absent" >&2
  exit 78
}

namespace=synveda-test
scratch=$(mktemp -d "${TMPDIR:-/tmp}/synveda-ops7-fault.XXXXXX")
chmod 700 "$scratch"
fault_binary=target/debug/examples/ops7_invalid_policy
forward_pid=
cleanup() {
  status=$?
  trap - EXIT
  if [ -s "$scratch/database-url" ] && [ -x "$fault_binary" ]; then
    OPS7_DATABASE_URL_FILE="$scratch/database-url" "$fault_binary" clear >/dev/null 2>&1 || status=1
  fi
  kubectl -n "$namespace" delete job/ops7-invalid-monitor --ignore-not-found --wait=true >/dev/null 2>&1 || true
  kubectl -n "$namespace" delete configmap/ops7-invalid-scripts --ignore-not-found >/dev/null 2>&1 || true
  if ! kubectl -n "$namespace" scale deployment/synveda --replicas=1 >/dev/null; then
    status=1
  elif ! kubectl -n "$namespace" rollout status deployment/synveda --timeout=120s >/dev/null; then
    status=1
  fi
  if [ -n "$forward_pid" ]; then
    kill "$forward_pid" >/dev/null 2>&1 || true
    wait "$forward_pid" >/dev/null 2>&1 || true
  fi
  rm -f "$scratch/database-url" "$scratch/port.log"
  rmdir "$scratch"
  exit "$status"
}
trap cleanup EXIT

kubectl -n "$namespace" get deployment/synveda >/dev/null
kubectl -n "$namespace" get secret/ops2-keycloak >/dev/null
kubectl -n "$namespace" get secret/synveda-gateway-db >/dev/null
SQLX_OFFLINE=true cargo build -p synveda-gateway --example ops7_invalid_policy >/dev/null

kubectl -n "$namespace" port-forward --address 127.0.0.1 svc/synveda-pg-rw :5432 \
  >"$scratch/port.log" 2>&1 &
forward_pid=$!
port=
for ((attempt = 0; attempt < 40; attempt++)); do
  port=$(sed -nE 's/^Forwarding from 127\.0\.0\.1:([0-9]+) -> 5432$/\1/p' "$scratch/port.log" | head -1)
  [ -n "$port" ] && break
  kill -0 "$forward_pid" 2>/dev/null || break
  sleep 0.25
done
[ -n "$port" ] || { echo "isolated database port-forward did not open" >&2; exit 1; }

KUBECONFIG=${KUBECONFIG:?set an isolated kubeconfig} kubectl -n "$namespace" get secret/synveda-gateway-db \
  -o jsonpath='{.data.DATABASE_URL}' | \
  OPS7_PORT="$port" OPS7_DSN_FILE="$scratch/database-url" node -e '
    const fs = require("node:fs");
    let encoded = "";
    process.stdin.on("data", chunk => encoded += chunk);
    process.stdin.on("end", () => {
      if (encoded.length < 10 || encoded.length > 4096) process.exit(1);
      const url = new URL(Buffer.from(encoded, "base64").toString("utf8"));
      if (!["postgres:", "postgresql:"].includes(url.protocol) ||
          url.username !== "synveda_gateway" || url.pathname !== "/synveda") process.exit(1);
      url.hostname = "127.0.0.1";
      url.port = process.env.OPS7_PORT;
      // This is the isolated Kind database over a loopback port-forward.
      url.searchParams.set("sslmode", "disable");
      fs.writeFileSync(process.env.OPS7_DSN_FILE, url.toString(), { mode: 0o600, flag: "wx" });
    });
  '
OPS7_DATABASE_URL_FILE="$scratch/database-url" "$fault_binary" clear >/dev/null

kubectl -n "$namespace" scale deployment/synveda --replicas=3 >/dev/null
kubectl -n "$namespace" rollout status deployment/synveda --timeout=300s >/dev/null
read -r -a pod_ips <<< "$(kubectl -n "$namespace" get pods \
  -l app.kubernetes.io/component=gateway \
  -o jsonpath='{range .items[*]}{.status.podIP}{" "}{end}')"
[ "${#pod_ips[@]}" = 3 ] || { echo "expected three gateway pods" >&2; exit 1; }
[ "${pod_ips[0]}" != "${pod_ips[1]}" ] &&
  [ "${pod_ips[0]}" != "${pod_ips[2]}" ] &&
  [ "${pod_ips[1]}" != "${pod_ips[2]}" ] || {
  echo "gateway pod IPs were not distinct" >&2
  exit 1
}
pod_urls="[\"http://${pod_ips[0]}:8120\",\"http://${pod_ips[1]}:8120\",\"http://${pod_ips[2]}:8120\"]"
image_tag=$(awk -F'"' '/^appVersion:/ { print $2; exit }' deploy/helm/synveda/Chart.yaml)
[ -n "$image_tag" ] || { echo "chart appVersion is absent" >&2; exit 1; }
kubectl -n "$namespace" create configmap ops7-invalid-scripts \
  --from-file=login-cross-pod.mjs=demos/fixtures/ops-7/login-cross-pod.mjs \
  --from-file=invalid-refresh.mjs=demos/fixtures/ops-7/invalid-refresh.mjs \
  --dry-run=client -o yaml | kubectl apply -f - >/dev/null
kubectl -n "$namespace" delete job/ops7-invalid-monitor --ignore-not-found --wait=true >/dev/null
cat <<EOF | kubectl -n "$namespace" apply -f - >/dev/null
apiVersion: batch/v1
kind: Job
metadata:
  name: ops7-invalid-monitor
spec:
  backoffLimit: 0
  activeDeadlineSeconds: 80
  template:
    spec:
      restartPolicy: Never
      automountServiceAccountToken: false
      securityContext:
        runAsNonRoot: true
        runAsUser: 65532
        runAsGroup: 65532
        seccompProfile:
          type: RuntimeDefault
      containers:
        - name: probe
          image: ghcr.io/synveda/product:$image_tag
          imagePullPolicy: Never
          command: ["node", "/scripts/invalid-refresh.mjs"]
          env:
            - name: POD_URLS
              value: '$pod_urls'
          securityContext:
            allowPrivilegeEscalation: false
            readOnlyRootFilesystem: true
            capabilities:
              drop: [ALL]
          volumeMounts:
            - name: scripts
              mountPath: /scripts
              readOnly: true
            - name: login-secret
              mountPath: /run/secrets
              readOnly: true
      volumes:
        - name: scripts
          configMap:
            name: ops7-invalid-scripts
        - name: login-secret
          secret:
            secretName: ops2-keycloak
            items:
              - key: keycloak_demo_admin_password
                path: keycloak_demo_admin_password
EOF

baseline_ready=false
for ((attempt = 0; attempt < 60; attempt++)); do
  if kubectl -n "$namespace" logs job/ops7-invalid-monitor -c probe 2>/dev/null | \
    grep -Fqx 'OPS-7: baseline ready'; then
    baseline_ready=true
    break
  fi
  failed=$(kubectl -n "$namespace" get job/ops7-invalid-monitor -o jsonpath='{.status.failed}')
  if [ -n "$failed" ] && [ "$failed" != 0 ]; then break; fi
  sleep 1
done
if [ "$baseline_ready" != true ]; then
  kubectl -n "$namespace" logs job/ops7-invalid-monitor -c probe >&2 || true
  echo "OPS-7 monitor did not establish its baseline" >&2
  exit 1
fi

OPS7_DATABASE_URL_FILE="$scratch/database-url" "$fault_binary" apply
sleep 42
OPS7_DATABASE_URL_FILE="$scratch/database-url" "$fault_binary" clear
for ((attempt = 0; attempt < 90; attempt++)); do
  succeeded=$(kubectl -n "$namespace" get job/ops7-invalid-monitor -o jsonpath='{.status.succeeded}')
  if [ "$succeeded" = 1 ]; then
    kubectl -n "$namespace" logs job/ops7-invalid-monitor -c probe
    kubectl -n "$namespace" rollout status deployment/synveda-worker --timeout=120s >/dev/null
    exit 0
  fi
  failed=$(kubectl -n "$namespace" get job/ops7-invalid-monitor -o jsonpath='{.status.failed}')
  if [ -n "$failed" ] && [ "$failed" != 0 ]; then break; fi
  sleep 1
done
kubectl -n "$namespace" logs job/ops7-invalid-monitor -c probe >&2 || true
echo "OPS-7 invalid-pack monitor failed or exceeded its deadline" >&2
exit 1
