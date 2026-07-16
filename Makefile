# Mirrors .github/workflows/ci.yml exactly — `make ci` locally == CI green.
# `make dev-up` / `make smoke` land with FND-2.

.PHONY: fmt lint test build deny check-deps ts-build ci

fmt:
	cargo fmt --all --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

build:
	cargo build --workspace

deny:
	cargo deny check

check-deps:
	node scripts/check-crate-deps.mjs

ts-build:
	pnpm install --frozen-lockfile
	pnpm -r build

ci: fmt lint test build deny check-deps ts-build
