#!/usr/bin/env sh
# CPR-2: current schema bootstrap, hard-cut refusal and explicit idempotent
# reset (ADR-0069).
set -eu

cd "$(dirname "$0")/.."

echo "==> CPR-2 — schema epoch and local reset"
make db-test

echo ""
echo "CPR-2 schema epoch: current bootstrap and hard-cut evidence pass."
