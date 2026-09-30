#!/usr/bin/env bash
# OPS-7: recover a real Capture claim after policy expiry or owner-pod loss.
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
mode=${1:-policy-expiry}
if [ "$#" -gt 1 ]; then
  echo "usage: $0 [policy-expiry|pod-loss]" >&2
  exit 78
fi
case "$mode" in
  policy-expiry|pod-loss) ;;
  *)
    echo "usage: $0 [policy-expiry|pod-loss]" >&2
    exit 78
    ;;
esac
scratch=$(mktemp -d "${TMPDIR:-/tmp}/synveda-ops7-claim.XXXXXX")
chmod 700 "$scratch"
fault_binary=target/debug/examples/ops7_invalid_policy
forward_pid=
upgraded=false
cleanup() {
  status=$?
  trap - EXIT
  if [ -s "$scratch/database-url" ] && [ -x "$fault_binary" ]; then
    OPS7_DATABASE_URL_FILE="$scratch/database-url" "$fault_binary" clear >/dev/null 2>&1 || status=1
  fi
  kubectl -n "$namespace" delete pod/ops7-claim-probe service/ops7-worker-probe \
    configmap/ops7-claim-scripts --ignore-not-found --wait=true >/dev/null 2>&1 || true
  if [ "$upgraded" = true ]; then
    helm upgrade synveda deploy/helm/synveda -n "$namespace" \
      -f "$scratch/values.json" --server-side=true --force-conflicts \
      --wait --wait-for-jobs --timeout 15m >/dev/null || status=1
    if [ "$(kubectl -n "$namespace" get deployment/synveda -o jsonpath='{.spec.replicas}')" != 1 ] ||
        [ "$(kubectl -n "$namespace" get deployment/synveda-worker -o jsonpath='{.spec.replicas}')" != 1 ] ||
        ! kubectl -n "$namespace" rollout status deployment/synveda --timeout=120s >/dev/null ||
        ! kubectl -n "$namespace" rollout status deployment/synveda-worker --timeout=120s >/dev/null; then
      status=1
    fi
  fi
  if [ -n "$forward_pid" ]; then
    kill "$forward_pid" >/dev/null 2>&1 || true
    wait "$forward_pid" >/dev/null 2>&1 || true
  fi
  rm -f "$scratch/database-url" "$scratch/port.log" "$scratch/values.json" \
    "$scratch/before.json" "$scratch/closed.json"
  rmdir "$scratch"
  exit "$status"
}
trap cleanup EXIT

kubectl -n "$namespace" get deployment/synveda deployment/synveda-worker \
  secret/ops2-keycloak secret/synveda-gateway-db >/dev/null
read -r wanted_instances ready_instances <<< "$(kubectl -n "$namespace" get cluster/synveda-pg \
  -o jsonpath='{.spec.instances}{" "}{.status.readyInstances}')"
[ "$wanted_instances" = "$ready_instances" ] || {
  echo "OPS-7 CNPG replicas are not fully ready; repair their reported cause before this drill" >&2
  exit 78
}
helm -n "$namespace" get values synveda -o json >"$scratch/values.json"
if [ "$mode" = policy-expiry ]; then
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

  kubectl -n "$namespace" get secret/synveda-gateway-db -o jsonpath='{.data.DATABASE_URL}' | \
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
        // The isolated Kind database is reached only over this loopback forward.
        url.searchParams.set("sslmode", "disable");
        fs.writeFileSync(process.env.OPS7_DSN_FILE, url.toString(), { mode: 0o600, flag: "wx" });
      });
    '
  OPS7_DATABASE_URL_FILE="$scratch/database-url" "$fault_binary" clear >/dev/null
fi

upgraded=true
helm upgrade synveda deploy/helm/synveda -n "$namespace" -f "$scratch/values.json" \
  --set extractor.kind=vllm --set extractor.model=ops7-interruption \
  --set extractor.baseUrl=http://ops7-worker-probe:8088 \
  --server-side=true --force-conflicts \
  --wait --wait-for-jobs --timeout 15m >/dev/null
kubectl -n "$namespace" rollout status deployment/synveda-worker --timeout=120s >/dev/null
kubectl -n "$namespace" scale deployment/synveda --replicas=3 >/dev/null
kubectl -n "$namespace" rollout status deployment/synveda --timeout=300s >/dev/null
if [ "$mode" = pod-loss ]; then
  kubectl -n "$namespace" scale deployment/synveda-worker --replicas=2 >/dev/null
  kubectl -n "$namespace" rollout status deployment/synveda-worker --timeout=300s >/dev/null
fi
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

kubectl -n "$namespace" create configmap ops7-claim-scripts \
  --from-file=login-cross-pod.mjs=demos/fixtures/ops-7/login-cross-pod.mjs \
  --from-file=claimed-capture.mjs=demos/fixtures/ops-7/claimed-capture.mjs \
  --from-file=blocked-extractor.mjs=demos/fixtures/ops-2/blocked-extractor.mjs \
  --dry-run=client -o yaml | kubectl apply -f - >/dev/null
kubectl -n "$namespace" delete pod/ops7-claim-probe service/ops7-worker-probe \
  --ignore-not-found --wait=true >/dev/null
cat <<EOF | kubectl -n "$namespace" apply -f - >/dev/null
apiVersion: v1
kind: Service
metadata:
  name: ops7-worker-probe
spec:
  selector:
    app: ops7-worker-probe
  ports:
    - port: 8088
      targetPort: 8088
---
apiVersion: v1
kind: Pod
metadata:
  name: ops7-claim-probe
  labels:
    app: ops7-worker-probe
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
    - name: provider
      image: ghcr.io/synveda/product:$image_tag
      imagePullPolicy: Never
      command: ["node", "/scripts/blocked-extractor.mjs"]
      ports:
        - containerPort: 8088
      securityContext:
        allowPrivilegeEscalation: false
        readOnlyRootFilesystem: true
        capabilities:
          drop: [ALL]
      volumeMounts:
        - name: scripts
          mountPath: /scripts
          readOnly: true
    - name: probe
      image: ghcr.io/synveda/product:$image_tag
      imagePullPolicy: Never
      command: ["node", "-e", "setInterval(() => {}, 60000)"]
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
        - name: work
          mountPath: /work
  volumes:
    - name: scripts
      configMap:
        name: ops7-claim-scripts
    - name: work
      emptyDir: {}
    - name: login-secret
      secret:
        secretName: ops2-keycloak
        items:
          - key: keycloak_demo_admin_password
            path: keycloak_demo_admin_password
EOF

kubectl -n "$namespace" wait pod/ops7-claim-probe --for=condition=Ready --timeout=120s >/dev/null
worker_ready() {
  kubectl -n "$namespace" exec deployment/synveda-worker -- \
    curl --silent --max-time 5 --output /dev/null --write-out '%{http_code}' \
      http://127.0.0.1:8121/readyz
}
worker_metrics() {
  kubectl -n "$namespace" exec deployment/synveda-worker -- \
    curl --fail --silent --max-time 5 http://127.0.0.1:8121/metrics | node -e '
      let value = "";
      process.stdin.on("data", chunk => value += chunk);
      process.stdin.on("end", () => {
        const lines = value.split("\n");
        const number = name => {
          const line = lines.find(item => item.startsWith(`${name} `));
          return line ? Number(line.split(" ").at(-1)) : 0;
        };
        console.log(JSON.stringify({
          authorityReady: number("synveda_worker_authority_ready"),
          authorityUnavailable: number("synveda_worker_authority_checks_total{outcome=\"unavailable\"}") +
            number("synveda_worker_authority_checks_total{outcome=\"timeout\"}"),
          refreshError: number("synveda_policy_pack_refresh_sweeps_total{outcome=\"error\"}"),
          reloadError: number("synveda_policy_pack_reloads_total{outcome=\"error\"}"),
        }));
      });
    '
}
provider_stats() {
  kubectl -n "$namespace" exec pod/ops7-claim-probe -c probe -- \
    node -e "fetch('http://127.0.0.1:8088/stats').then(r=>r.text()).then(console.log)"
}
provider_counts() {
  provider_stats | node -e '
    let value = "";
    process.stdin.on("data", chunk => value += chunk);
    process.stdin.on("end", () => {
      const stats = JSON.parse(value);
      console.log(`${stats.calls} ${stats.active} ${stats.cancelled}`);
    });
  '
}

kubectl -n "$namespace" exec pod/ops7-claim-probe -c probe -- \
  node /scripts/claimed-capture.mjs start
if [ "$mode" = policy-expiry ]; then
  [ "$(worker_ready)" = 200 ] || { echo "worker was not ready with claimed Capture" >&2; exit 1; }
  worker_metrics >"$scratch/before.json"
  node -e 'if (require(process.argv[1]).authorityReady !== 1) process.exit(1)' \
    "$scratch/before.json"

  OPS7_DATABASE_URL_FILE="$scratch/database-url" "$fault_binary" apply
  fault_start=$SECONDS
  closed=false
  for ((attempt = 0; attempt < 45; attempt++)); do
    ready=$(worker_ready)
    read -r calls active cancelled <<< "$(provider_counts)"
    [ "$calls" = 1 ] || { echo "another provider call began before policy recovery" >&2; exit 1; }
    if [ "$ready" = 503 ] && [ "$active" = 0 ] && [ "$cancelled" = 1 ]; then
      closed=true
      break
    fi
    sleep 1
  done
  [ "$closed" = true ] || { echo "worker did not close and cancel claimed Capture" >&2; exit 1; }
  closed_after=$((SECONDS - fault_start))
  [ "$closed_after" -le 36 ] || { echo "worker policy closure exceeded the provisional lease" >&2; exit 1; }
  worker_metrics >"$scratch/closed.json"
  node - "$scratch/before.json" "$scratch/closed.json" <<'JS'
const fs = require("node:fs");
const before = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const after = JSON.parse(fs.readFileSync(process.argv[3], "utf8"));
if (after.authorityReady !== 1 || after.authorityUnavailable !== before.authorityUnavailable ||
    after.refreshError <= before.refreshError || after.reloadError <= before.reloadError) {
  throw new Error("policy expiry was not isolated from database authority");
}
JS
  echo "OPS-7: worker policy-only closure cancelled the claimed extractor after ${closed_after}s"
  while [ $((SECONDS - fault_start)) -lt 42 ]; do
    [ "$(worker_ready)" = 503 ] || { echo "worker reopened before fault clear" >&2; exit 1; }
    read -r calls active cancelled <<< "$(provider_counts)"
    [ "$calls $active $cancelled" = "1 0 1" ] || {
      echo "worker retried Capture while policy was stale" >&2
      exit 1
    }
    sleep 1
  done
  OPS7_DATABASE_URL_FILE="$scratch/database-url" "$fault_binary" clear
  recovered=false
  for ((attempt = 0; attempt < 75; attempt++)); do
    if [ "$(worker_ready)" = 200 ]; then recovered=true; break; fi
    sleep 1
  done
  [ "$recovered" = true ] || { echo "worker did not recover after policy repair" >&2; exit 1; }
else
  first_peer=$(provider_stats | node -e '
    let input = "";
    process.stdin.on("data", chunk => input += chunk);
    process.stdin.on("end", () => {
      const stats = JSON.parse(input);
      if (stats.calls !== 1 || stats.active !== 1 || stats.peers?.length !== 1) process.exit(1);
      console.log(stats.peers[0].replace(/^::ffff:/, ""));
    });
  ')
  read -r owner survivor <<< "$(kubectl -n "$namespace" get pods \
    -l app.kubernetes.io/component=worker -o json | node -e '
      let input = "";
      process.stdin.on("data", chunk => input += chunk);
      process.stdin.on("end", () => {
        const pods = JSON.parse(input).items;
        const owner = pods.find(pod => pod.status.podIP === process.argv[1]);
        const survivor = pods.find(pod => pod.metadata.name !== owner?.metadata.name);
        if (pods.length !== 2 || !owner || !survivor ||
            pods.some(pod => !pod.status.containerStatuses?.[0]?.ready)) process.exit(1);
        console.log(`${owner.metadata.name} ${survivor.metadata.name}`);
      });
    ' "$first_peer")"
  [ -n "$owner" ] && [ -n "$survivor" ] || {
    echo "blocked provider peer did not identify one of two ready workers" >&2
    exit 1
  }
  kubectl -n "$namespace" delete "pod/$owner" --wait=false >/dev/null
  cancelled=false
  for ((attempt = 0; attempt < 40; attempt++)); do
    read -r calls active dropped <<< "$(provider_counts)"
    [ "$calls" = 1 ] || { echo "another worker called the provider before the claim expired" >&2; exit 1; }
    if [ "$active $dropped" = "0 1" ]; then cancelled=true; break; fi
    sleep 1
  done
  [ "$cancelled" = true ] || { echo "lost worker's provider call did not cancel" >&2; exit 1; }
  [ "$(kubectl -n "$namespace" exec "pod/$survivor" -- \
    curl --silent --max-time 5 --output /dev/null --write-out '%{http_code}' \
      http://127.0.0.1:8121/readyz)" = 200 ] || {
    echo "surviving worker was not ready after owner loss" >&2
    exit 1
  }
  echo "OPS-7: one of two ready workers owned Capture; losing that pod cancelled its provider call"
fi
retried=false
for ((attempt = 0; attempt < 120; attempt++)); do
  read -r calls active cancelled <<< "$(provider_counts)"
  [ "$calls" -le 2 ] || { echo "Capture called the provider more than twice" >&2; exit 1; }
  if [ "$calls $active $cancelled" = "2 1 1" ]; then retried=true; break; fi
  sleep 1
done
[ "$retried" = true ] || { echo "Capture did not reclaim its expired lease" >&2; exit 1; }
if [ "$mode" = pod-loss ]; then
  second_peer=$(provider_stats | node -e '
    let input = "";
    process.stdin.on("data", chunk => input += chunk);
    process.stdin.on("end", () => {
      const stats = JSON.parse(input);
      if (stats.peers?.length !== 2) process.exit(1);
      console.log(stats.peers[1].replace(/^::ffff:/, ""));
    });
  ')
  [ "$second_peer" != "$first_peer" ] || {
    echo "the deleted worker made the retry provider call" >&2
    exit 1
  }
fi
kubectl -n "$namespace" exec pod/ops7-claim-probe -c probe -- \
  node /scripts/claimed-capture.mjs pre-release
kubectl -n "$namespace" exec pod/ops7-claim-probe -c probe -- \
  node -e "fetch('http://127.0.0.1:8088/release',{method:'POST'}).then(r=>{if(!r.ok)process.exit(1)})"
kubectl -n "$namespace" exec pod/ops7-claim-probe -c probe -- \
  node /scripts/claimed-capture.mjs verify
