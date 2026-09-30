#!/usr/bin/env bash
# OPS-7: fresh chart-rendered CNPG bootstrap and a restarted streaming replica.
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

namespace=synveda-ops7-acl
cluster=ops7-acl-pg
image=synveda/cnpg-postgres:17.11-ops7-acl
if kubectl get namespace "$namespace" >/dev/null 2>&1; then
  echo "OPS-7 ACL namespace already exists; refusing to reuse or delete it" >&2
  exit 78
fi
scratch=$(mktemp -d "${TMPDIR:-/tmp}/synveda-ops7-acl.XXXXXX")
chmod 700 "$scratch"
owned=false
cleanup() {
  status=$?
  trap - EXIT
  if [ "$owned" = true ]; then
    kubectl delete namespace "$namespace" --wait=true --timeout=180s >/dev/null || status=1
  fi
  rm -f "$scratch/synveda_migrator_password" "$scratch/synveda_gateway_password" \
    "$scratch/synveda_worker_password"
  rmdir "$scratch" || status=1
  exit "$status"
}
trap cleanup EXIT
bash scripts/build-kind-image.sh "$image" deploy/helm/postgres/Dockerfile cnpg-postgres >/dev/null
kind load docker-image --name synveda-ops7 "$image" >/dev/null
kubectl create namespace "$namespace" >/dev/null
owned=true

helm template synveda deploy/helm/synveda -n "$namespace" \
  -f demos/fixtures/ops-2/values.yaml --set fullnameOverride=ops7-acl \
  --set "postgres.image=$image" \
  --api-versions postgresql.cnpg.io/v1 | \
  ruby -rjson -ryaml -e '
    documents = YAML.load_stream(STDIN.read).select { |document| document.is_a?(Hash) }
    cluster = documents.find { |document| document["kind"] == "Cluster" }
    roles = documents.find { |document| document["kind"] == "ConfigMap" && document.dig("metadata", "labels", "app.kubernetes.io/component") == "database-contract" }
    abort "chart did not render exactly the expected CNPG Cluster" unless cluster && cluster.dig("metadata", "name") == "ops7-acl-pg"
    abort "chart did not render the database role contract" unless roles && roles.dig("metadata", "name") == "ops7-acl-database-roles"
    puts JSON.generate({"apiVersion" => "v1", "kind" => "List", "items" => [roles, cluster]})
  ' | kubectl -n "$namespace" apply -f - >/dev/null

primary=
for ((attempt = 0; attempt < 120; attempt++)); do
  primary=$(kubectl -n "$namespace" get "cluster/$cluster" \
    -o jsonpath='{.status.currentPrimary}' 2>/dev/null || true)
  if [ -n "$primary" ]; then
    ready=$(kubectl -n "$namespace" get "pod/$primary" -o jsonpath='{.status.containerStatuses[0].ready}' 2>/dev/null || true)
    [ "$ready" = true ] && break
  fi
  sleep 2
done
[ -n "$primary" ] && [ "${ready:-}" = true ] || {
  echo "fresh CNPG primary did not become ready" >&2
  exit 1
}

umask 077
for name in synveda_migrator_password synveda_gateway_password synveda_worker_password; do
  openssl rand -hex 32 >"$scratch/$name"
done
kubectl -n "$namespace" create secret generic ops7-acl-bootstrap-inputs \
  --from-file="$scratch/synveda_migrator_password" \
  --from-file="$scratch/synveda_gateway_password" \
  --from-file="$scratch/synveda_worker_password" >/dev/null
cat <<EOF | kubectl -n "$namespace" apply -f - >/dev/null
apiVersion: batch/v1
kind: Job
metadata:
  name: ops7-acl-bootstrap
spec:
  backoffLimit: 0
  activeDeadlineSeconds: 600
  template:
    spec:
      restartPolicy: Never
      automountServiceAccountToken: false
      securityContext:
        runAsNonRoot: true
        runAsUser: 26
        runAsGroup: 26
        fsGroup: 26
        seccompProfile:
          type: RuntimeDefault
      containers:
        - name: bootstrap
          image: $image
          imagePullPolicy: Never
          command: ["/bin/sh", "-ec"]
          args:
            - |
              umask 077
              cp /run/superuser/password /run/secrets/postgres_bootstrap_password
              for name in synveda_migrator_password synveda_gateway_password synveda_worker_password; do
                cp "/run/credentials/\$name" "/run/secrets/\$name"
              done
              cp /run/roles/roles.json /run/secrets/database_roles.json
              chmod 0600 /run/secrets/*
              exec /usr/local/bin/synveda-database-bootstrap synveda
          env:
            - name: SYNVEDA_POSTGRES_BOOTSTRAP_USER
              value: postgres
            - name: SYNVEDA_POSTGRES_BOOTSTRAP_URL
              value: postgresql://postgres@$cluster-rw:5432/postgres
            - name: SYNVEDA_POSTGRES_BUNDLED_CLUSTER
              value: "true"
            - name: SYNVEDA_POSTGRES_CNPG_CLUSTER
              value: "true"
            - name: SYNVEDA_DATABASE_BOOTSTRAP_PRIVATE_DIR
              value: /run/secrets
            - name: SYNVEDA_DATABASE_ROLES_FILE
              value: /run/secrets/database_roles.json
          securityContext:
            allowPrivilegeEscalation: false
            readOnlyRootFilesystem: true
            capabilities:
              drop: [ALL]
          volumeMounts:
            - { name: superuser, mountPath: /run/superuser, readOnly: true }
            - { name: credentials, mountPath: /run/credentials, readOnly: true }
            - { name: roles, mountPath: /run/roles, readOnly: true }
            - { name: private, mountPath: /run/secrets }
            - { name: tmp, mountPath: /tmp }
      volumes:
        - name: superuser
          secret:
            secretName: $cluster-superuser
        - name: credentials
          secret:
            secretName: ops7-acl-bootstrap-inputs
        - name: roles
          configMap:
            name: ops7-acl-database-roles
        - name: private
          emptyDir:
            medium: Memory
        - name: tmp
          emptyDir:
            medium: Memory
EOF
completed=false
for ((attempt = 0; attempt < 180; attempt++)); do
  succeeded=$(kubectl -n "$namespace" get job/ops7-acl-bootstrap -o jsonpath='{.status.succeeded}')
  if [ "$succeeded" = 1 ]; then completed=true; break; fi
  failed=$(kubectl -n "$namespace" get job/ops7-acl-bootstrap -o jsonpath='{.status.failed}')
  [ -z "$failed" ] || [ "$failed" = 0 ] || break
  sleep 2
done
[ "$completed" = true ] || {
  kubectl -n "$namespace" logs job/ops7-acl-bootstrap -c bootstrap --tail=50 >&2 || true
  echo "fresh CNPG administrator bootstrap failed" >&2
  exit 1
}
kubectl -n "$namespace" wait "cluster/$cluster" --for=condition=Ready --timeout=300s >/dev/null
[ "$(kubectl -n "$namespace" get "cluster/$cluster" -o jsonpath='{.status.readyInstances}')" = 2 ] || {
  echo "fresh CNPG bootstrap did not make both instances ready" >&2
  exit 1
}
privileges=$(kubectl -n "$namespace" exec "$primary" -c postgres -- \
  psql -X -qAt -v ON_ERROR_STOP=1 -U postgres -d postgres -c \
  "select has_database_privilege('streaming_replica','postgres','CONNECT'),
          has_database_privilege('synveda_migrator','postgres','CONNECT'),
          exists (select 1 from pg_catalog.aclexplode((select datacl from pg_database where datname='postgres')) where grantee=0 and privilege_type='CONNECT'),
          exists (select 1 from pg_catalog.aclexplode((select datacl from pg_database where datname='template1')) where grantee=0 and privilege_type='CONNECT');")
[ "$privileges" = 't|f|f|f' ] || {
  echo "CNPG bootstrap did not retain only the reserved replica's maintenance CONNECT" >&2
  exit 1
}

replica=$(kubectl -n "$namespace" get pods -l "cnpg.io/cluster=$cluster,role=replica" \
  -o jsonpath='{.items[0].metadata.name}')
[ -n "$replica" ] || { echo "fresh CNPG replica is absent" >&2; exit 1; }
old_uid=$(kubectl -n "$namespace" get "pod/$replica" -o jsonpath='{.metadata.uid}')
kubectl -n "$namespace" delete "pod/$replica" --wait=false >/dev/null
restarted=false
for ((attempt = 0; attempt < 180; attempt++)); do
  new_uid=$(kubectl -n "$namespace" get "pod/$replica" -o jsonpath='{.metadata.uid}' 2>/dev/null || true)
  ready=$(kubectl -n "$namespace" get "pod/$replica" -o jsonpath='{.status.containerStatuses[0].ready}' 2>/dev/null || true)
  count=$(kubectl -n "$namespace" get "cluster/$cluster" -o jsonpath='{.status.readyInstances}')
  if [ -n "$new_uid" ] && [ "$new_uid" != "$old_uid" ] &&
      [ "$ready" = true ] && [ "$count" = 2 ]; then
    restarted=true
    break
  fi
  sleep 2
done
[ "$restarted" = true ] || {
  kubectl -n "$namespace" logs "pod/$replica" -c postgres --tail=20 >&2 || true
  echo "fresh CNPG replica did not restart and reconnect" >&2
  exit 1
}
echo "OPS-7: chart-rendered fresh CNPG booted two instances; reserved replica has maintenance CONNECT, product role and PUBLIC remain denied, and replica restarted ready"
