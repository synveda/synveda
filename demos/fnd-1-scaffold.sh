#!/usr/bin/env sh
# FND-1 acceptance demo: the workspace scaffold passes every CI gate.
# Runs the exact checks CI runs (.github/workflows/ci.yml / Makefile).
# On Windows, run via Git Bash.
set -eu

cd "$(dirname "$0")/.."

echo "==> cargo fmt --all --check"
cargo fmt --all --check

echo "==> cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings

echo "==> cargo test --workspace"
cargo test --workspace

echo "==> cargo build --workspace   (FND-1 acceptance criterion)"
cargo build --workspace

echo "==> cargo deny check          (licence allowlist: MIT/Apache-2.0/PostgreSQL)"
cargo deny check

echo "==> crate layering rule       (seed section 8)"
node scripts/check-crate-deps.mjs

echo "==> pnpm install + build      (adapters + TS SDK stubs)"
pnpm install --frozen-lockfile
pnpm -r build

echo ""
echo "FND-1 scaffold: all checks green."
