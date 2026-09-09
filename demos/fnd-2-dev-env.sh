#!/usr/bin/env sh
# FND-2: the development deployment is the canonical Compose contract, not a
# second product topology.
set -eu

cd "$(dirname "$0")/.."

echo "==> canonical development Compose render matrix"
make compose-config

echo "==> deployment lifecycle and image contracts"
make check-deploy

echo ""
echo "FND-2 development environment: canonical Compose contracts pass."
