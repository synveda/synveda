#!/usr/bin/env bash
# OPS-7: cross-pod login and policy-decision acceptance after the OPS-2 Kind install.
# The chart stays pinned to one gateway; this script scales only its isolated
# Kind deployment and restores that pin on exit.
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
kubectl -n "$namespace" get deployment/synveda >/dev/null
kubectl -n "$namespace" get secret/ops2-keycloak >/dev/null

cleanup() {
  status=$?
  trap - EXIT
  kubectl -n "$namespace" delete job/ops7-login --ignore-not-found --wait=true >/dev/null 2>&1 || true
  kubectl -n "$namespace" delete job/ops7-policy-lag --ignore-not-found --wait=true >/dev/null 2>&1 || true
  kubectl -n "$namespace" delete job/ops7-decision-lag --ignore-not-found --wait=true >/dev/null 2>&1 || true
  kubectl -n "$namespace" delete configmap/ops7-login-script --ignore-not-found >/dev/null 2>&1 || true
  kubectl -n "$namespace" delete configmap/ops7-policy-scripts --ignore-not-found >/dev/null 2>&1 || true
  kubectl -n "$namespace" delete configmap/ops7-decision-scripts --ignore-not-found >/dev/null 2>&1 || true
  kubectl -n "$namespace" scale deployment/synveda --replicas=1 >/dev/null 2>&1 || true
  exit "$status"
}
trap cleanup EXIT

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

kubectl -n "$namespace" create configmap ops7-login-script \
  --from-file=login-cross-pod.mjs=demos/fixtures/ops-7/login-cross-pod.mjs \
  --dry-run=client -o yaml | kubectl apply -f - >/dev/null
kubectl -n "$namespace" delete job/ops7-login --ignore-not-found --wait=true >/dev/null
cat <<EOF | kubectl -n "$namespace" apply -f - >/dev/null
apiVersion: batch/v1
kind: Job
metadata:
  name: ops7-login
spec:
  backoffLimit: 0
  activeDeadlineSeconds: 120
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
          image: node:22-bookworm-slim@sha256:83f487e0a63425e5b4d146fb5e5be574bcbe1b7b843d3ebafdd95eaf7767a7e5
          imagePullPolicy: IfNotPresent
          command: ["node", "/scripts/login-cross-pod.mjs"]
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
            name: ops7-login-script
        - name: login-secret
          secret:
            secretName: ops2-keycloak
            items:
              - key: keycloak_demo_admin_password
                path: keycloak_demo_admin_password
EOF

if kubectl -n "$namespace" wait --for=condition=complete job/ops7-login --timeout=150s >/dev/null; then
  kubectl -n "$namespace" logs job/ops7-login -c probe
else
  kubectl -n "$namespace" logs job/ops7-login -c probe >&2 || true
  echo "OPS-7 cross-pod login probe failed" >&2
  exit 1
fi

image_tag=$(awk -F'"' '/^appVersion:/ { print $2; exit }' deploy/helm/synveda/Chart.yaml)
[ -n "$image_tag" ] || { echo "chart appVersion is absent" >&2; exit 1; }
kubectl -n "$namespace" create configmap ops7-policy-scripts \
  --from-file=policy-lag.mjs=demos/fixtures/ops-7/policy-lag.mjs \
  --from-file=lag-pack.cedar=demos/fixtures/ops-7/lag-pack.cedar \
  --dry-run=client -o yaml | kubectl apply -f - >/dev/null
kubectl -n "$namespace" delete job/ops7-policy-lag --ignore-not-found --wait=true >/dev/null
cat <<EOF | kubectl -n "$namespace" apply -f - >/dev/null
apiVersion: batch/v1
kind: Job
metadata:
  name: ops7-policy-lag
spec:
  backoffLimit: 0
  activeDeadlineSeconds: 120
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
          command: ["node", "/scripts/policy-lag.mjs"]
          env:
            - name: POD_URLS
              value: '$pod_urls'
            - name: DATABASE_URL_FILE
              value: /run/db/DATABASE_URL
          securityContext:
            allowPrivilegeEscalation: false
            readOnlyRootFilesystem: true
            capabilities:
              drop: [ALL]
          volumeMounts:
            - name: scripts
              mountPath: /scripts
              readOnly: true
            - name: gateway-database
              mountPath: /run/db
              readOnly: true
      volumes:
        - name: scripts
          configMap:
            name: ops7-policy-scripts
        - name: gateway-database
          secret:
            secretName: synveda-gateway-db
            items:
              - key: DATABASE_URL
                path: DATABASE_URL
EOF

if kubectl -n "$namespace" wait --for=condition=complete job/ops7-policy-lag --timeout=150s >/dev/null; then
  kubectl -n "$namespace" logs job/ops7-policy-lag -c probe
else
  kubectl -n "$namespace" logs job/ops7-policy-lag -c probe >&2 || true
  echo "OPS-7 three-pod policy revision probe failed" >&2
  exit 1
fi

kubectl -n "$namespace" create configmap ops7-decision-scripts \
  --from-file=login-cross-pod.mjs=demos/fixtures/ops-7/login-cross-pod.mjs \
  --from-file=decision-lag.mjs=demos/fixtures/ops-7/decision-lag.mjs \
  --from-file=decision-allow.cedar=demos/fixtures/ops-7/decision-allow.cedar \
  --from-file=decision-deny.cedar=demos/fixtures/ops-7/decision-deny.cedar \
  --dry-run=client -o yaml | kubectl apply -f - >/dev/null
kubectl -n "$namespace" delete job/ops7-decision-lag --ignore-not-found --wait=true >/dev/null
cat <<EOF | kubectl -n "$namespace" apply -f - >/dev/null
apiVersion: batch/v1
kind: Job
metadata:
  name: ops7-decision-lag
spec:
  backoffLimit: 0
  activeDeadlineSeconds: 180
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
          command: ["node", "/scripts/decision-lag.mjs"]
          env:
            - name: POD_URLS
              value: '$pod_urls'
            - name: DATABASE_URL_FILE
              value: /run/db/DATABASE_URL
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
            - name: gateway-database
              mountPath: /run/db
              readOnly: true
      volumes:
        - name: scripts
          configMap:
            name: ops7-decision-scripts
        - name: login-secret
          secret:
            secretName: ops2-keycloak
            items:
              - key: keycloak_demo_admin_password
                path: keycloak_demo_admin_password
              - key: keycloak_demo_approver_password
                path: keycloak_demo_approver_password
        - name: gateway-database
          secret:
            secretName: synveda-gateway-db
            items:
              - key: DATABASE_URL
                path: DATABASE_URL
EOF

if kubectl -n "$namespace" wait --for=condition=complete job/ops7-decision-lag --timeout=190s >/dev/null; then
  kubectl -n "$namespace" logs job/ops7-decision-lag -c probe
else
  kubectl -n "$namespace" logs job/ops7-decision-lag -c probe >&2 || true
  echo "OPS-7 three-pod authorization decision probe failed" >&2
  exit 1
fi
