#!/bin/sh
# OPS-11: keep the upstream dependency unchanged; patch only PodSpec user
# namespaces and the required admission policy. No cluster connection occurs.
set -eu
scratch=$(mktemp -d "${TMPDIR:-/tmp}/synveda-openshift-v3.XXXXXX")
trap 'rm -rf -- "$scratch"' EXIT HUP INT TERM
cat > "$scratch/resources.yaml"
cat > "$scratch/kustomization.yaml" <<'EOF'
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
resources: [resources.yaml]
commonAnnotations:
  openshift.io/required-scc: restricted-v3
patches:
  - target:
      kind: (Deployment|StatefulSet|DaemonSet|Job)
    patch: |-
      - op: add
        path: /spec/template/spec/hostUsers
        value: false
  - target:
      kind: Pod
    patch: |-
      - op: add
        path: /spec/hostUsers
        value: false
EOF
kubectl kustomize "$scratch"
