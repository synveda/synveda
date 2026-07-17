# Mirrors .github/workflows/ci.yml exactly — `make ci` locally == CI green.
# dev-up/dev-down/smoke manage the docker-compose dev environment (FND-2);
# state persists in named volumes — wipe with `$(COMPOSE) down -v`.

COMPOSE = docker compose -f deploy/compose/docker-compose.yml

.PHONY: fmt lint test build deny check-deps ts-build ci dev-up dev-down smoke

dev-up:
	$(COMPOSE) up --build --detach --wait

dev-down:
	$(COMPOSE) down

smoke:
	bash scripts/smoke.sh

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
