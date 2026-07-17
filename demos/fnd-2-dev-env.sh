#!/usr/bin/env sh
# FND-2 acceptance demo: the dev environment comes up and passes the smoke test.
# AC (docs/backlog/FND-2.md): `make dev-up && make smoke` passes.
# On Windows, run via Git Bash. First run pulls images and the BGE-M3 model.
set -eu

cd "$(dirname "$0")/.."

make dev-up && make smoke

echo ""
echo "FND-2 dev environment: acceptance criterion green."
