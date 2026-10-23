#!/usr/bin/env sh
# Local, release and Helm deployment shapes converge on one runtime.
set -eu

cd "$(dirname "$0")/.."

echo "==> One context platform across deployment shapes"

make check-deploy
make db-test

echo ""
echo "Deployment convergence: current contract and exact-role evidence pass."
