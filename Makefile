# Mirrors .github/workflows/ci.yml exactly — `make ci` locally == CI green.
# dev-up/dev-down/smoke manage the docker-compose dev environment (FND-2);
# state persists in named volumes — wipe with `$(COMPOSE) down -v`.

COMPOSE = docker compose -f deploy/compose/docker-compose.yml
# Dev-compose credentials (FND-2); tests that need Postgres read DATABASE_URL
# and skip when it is unset — CI runs without a database.
DATABASE_URL ?= postgres://synveda:synveda-dev@localhost:5432/synveda

.PHONY: fmt lint test build deny check-deps ts-build ts-test ci dev-up dev-down smoke db-test eval eval-check

dev-up:
	$(COMPOSE) up --build --detach --wait

dev-down:
	$(COMPOSE) down

smoke:
	bash scripts/smoke.sh

# The eval harness (EVAL-1, ADR-0028): a scenario suite against a live
# stack on a scratch database, five axes, gated by evals/baseline.json.
# Needs the dev compose (postgres) and node. Exit status is the gate's.
eval:
	sh evals/run.sh

# Parses the suite and the baseline with no stack at all — what `ci`
# can run, and what catches a scenario that would measure nothing.
eval-check:
	cargo run -q -p synveda-eval -- check

db-test:
	DATABASE_URL=$(DATABASE_URL) cargo test --workspace

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

# The adapter suites (ADPT-1); packages without a test script are skipped.
ts-test:
	pnpm -r test

ci: fmt lint test build deny check-deps eval-check ts-build ts-test
