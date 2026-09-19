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
    # Each image retains its own intermediate layers. Cache publication is
    # best-effort and bounded; build/load errors still fail the acceptance.
    exec docker buildx build --load -t "$1" -f "$2" \
      --cache-from "type=gha,version=2,scope=kind-$3,timeout=2m" \
      --cache-to "type=gha,version=2,scope=kind-$3,mode=max,ignore-error=true,timeout=2m" .
    ;;
  *)
    echo "SYNVEDA_KIND_GHA_CACHE must be 0 or 1" >&2
    exit 64
    ;;
esac
