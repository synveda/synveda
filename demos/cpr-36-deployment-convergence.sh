#!/usr/bin/env sh
# CPR-36: local, release and Helm deployment shapes converge on one runtime.
set -eu

cd "$(dirname "$0")/.."

echo "==> CPR-36 — one context platform across deployment shapes"

make check-deploy
make db-test

echo ""
echo "CPR-36 deployment convergence: current contract and exact-role evidence pass."
