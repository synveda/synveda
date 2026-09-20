#!/bin/sh
# OPS-11: preparation writes private files only; kubectl remains an explicit step.
set -eu
umask 077
[ "$(id -u)" -ne 0 ] || { echo 'run as an ordinary account' >&2; exit 78; }
if [ -n "${DOCKER_CONTEXT:-}" ] || [ -z "${DOCKER_HOST:-}" ]; then
  endpoint=$(docker context inspect "${DOCKER_CONTEXT:-$(docker context show)}" --format '{{.Endpoints.docker.Host}}')
else
  endpoint=$DOCKER_HOST
fi
case "$endpoint" in unix:///*) ;; *) echo 'preparation requires a local Unix-socket Docker engine' >&2; exit 78 ;; esac
export DOCKER_HOST=$endpoint
unset DOCKER_CONTEXT DOCKER_TLS_VERIFY DOCKER_CERT_PATH
chart=$(CDPATH= cd "$(dirname "$0")/.." && pwd -P)
output=${1:?usage: prepare-local.sh /absolute/private/output-directory}
case "$output" in /*) ;; *) echo 'output must be absolute' >&2; exit 64 ;; esac
[ ! -L "$output" ] || exit 78
mkdir -p "$output"
version=$(awk '/^appVersion:/ {gsub(/"/, "", $2); print $2}' "$chart/Chart.yaml")
image=${SYNVEDA_PRODUCT_IMAGE:-ghcr.io/synveda/product:$version}
docker run --rm --network none --read-only --cap-drop ALL \
  --security-opt no-new-privileges:true --pids-limit 64 --memory 256m \
  --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,nosuid,nodev,size=32m \
  --mount "type=bind,source=$chart/examples,target=/scripts,readonly" \
  --mount "type=bind,source=$output,target=/output" \
  --entrypoint /usr/bin/timeout "$image" 60s node /scripts/prepare-local.mjs
printf 'Private configuration: %s\nInspect the Kubernetes context, then follow examples/README.md.\n' "$output"
