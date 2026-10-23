#!/usr/bin/env sh
# The development deployment is the canonical Compose contract, not a
# second product topology.
set -eu

cd "$(dirname "$0")/.."

echo "==> canonical development Compose render matrix"
make compose-config

echo "==> deployment lifecycle and image contracts"
make check-deploy

echo ""
echo "Development environment: canonical Compose contracts pass."
