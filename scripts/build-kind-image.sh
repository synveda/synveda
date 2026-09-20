#!/usr/bin/env bash
# FND-1 / ADR-0108: cache the source build, never the acceptance result.
# Run from the repository root; Kind consumes the image from the local engine.
set +x
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "usage: build-kind-image.sh <image> <dockerfile> <product|cnpg-postgres|keycloak>" >&2
  exit 64
fi
case "$3" in
  product|cnpg-postgres|keycloak) ;;
  *) echo "kind image cache scope is not supported" >&2; exit 64 ;;
esac

case "${SYNVEDA_KIND_GHA_CACHE:-0}" in
  0)
    exec docker build -t "$1" -f "$2" .
    ;;
  1)
    if [ -z "${ACTIONS_RUNTIME_TOKEN:-}" ] || [ -z "${ACTIONS_RESULTS_URL:-}" ]; then
      echo "Kind's GitHub cache requires the Actions runtime environment" >&2
      exit 64
    fi
    # Contributor code may restore main's build layers, but only a main push
    # publishes reusable cache entries. Build/load failures remain fatal.
    cache_scope="kind-$3"
    set -- buildx build --load -t "$1" -f "$2" \
      --cache-from "type=gha,version=2,scope=$cache_scope,timeout=2m"
    if [ "${GITHUB_EVENT_NAME:-}" = push ] && [ "${GITHUB_REF:-}" = refs/heads/main ]; then
      set -- "$@" --cache-to "type=gha,version=2,scope=$cache_scope,mode=max,ignore-error=true,timeout=2m"
    fi
    exec docker "$@" .
    ;;
  *)
    echo "SYNVEDA_KIND_GHA_CACHE must be 0 or 1" >&2
    exit 64
    ;;
esac
