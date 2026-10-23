#!/usr/bin/env sh
# Current schema bootstrap, hard-cut refusal and explicit idempotent
# reset (ADR-0069).
set -eu

cd "$(dirname "$0")/.."

echo "==> Schema epoch and local reset"
make db-test

echo ""
echo "Schema epoch: current bootstrap and hard-cut evidence pass."
