# Mirrors .github/workflows/ci.yml exactly — `make ci` locally == CI green.
# dev-up/dev-down/smoke manage the docker-compose dev environment (FND-2);
# state persists in named volumes — wipe with `$(COMPOSE) down -v`.

COMPOSE = docker compose -f deploy/compose/docker-compose.yml

# The TEI image is per-architecture (deploy/compose/docker-compose.yml
# explains why). Upstream ships a versioned amd64 release and an unversioned
# arm64 build, so the arm64 side is pinned by commit; both serve BGE-M3 and
# agree to float32 rounding. Override SYNVEDA_TEI_IMAGE to pin something
# else.
TEI_IMAGE_amd64  = ghcr.io/huggingface/text-embeddings-inference:cpu-1.8.1
TEI_IMAGE_x86_64 = $(TEI_IMAGE_amd64)
TEI_IMAGE_arm64  = ghcr.io/huggingface/text-embeddings-inference:cpu-arm64-sha-4150561
TEI_IMAGE_aarch64 = $(TEI_IMAGE_arm64)
SYNVEDA_TEI_IMAGE ?= $(or $(TEI_IMAGE_$(shell uname -m)),$(TEI_IMAGE_amd64))
export SYNVEDA_TEI_IMAGE
# Dev-compose credentials (FND-2); tests that need Postgres read DATABASE_URL
# and skip when it is unset — CI runs without a database.
DATABASE_URL ?= postgres://synveda:synveda-dev@localhost:5432/synveda

.PHONY: fmt lint test build deny check-deps ts-build ts-test ci dev-up dev-down smoke db-test eval eval-check eval-extraction-live

dev-up:
	$(COMPOSE) up --build --detach --wait

dev-down:
	$(COMPOSE) down

smoke:
	bash scripts/smoke.sh

# The eval harness (EVAL-1, ADR-0028; EVAL-2, ADR-0046): the scenario
# suite and the labelled extraction corpus against a live stack on a
# scratch database, gated by evals/baseline.json. Needs the dev compose
# (postgres) and node. Exit status is the gate's.
eval:
	sh evals/run.sh

# The same corpus through a real model instead of the rule-based
# extractor (EVAL-2, ADR-0046 decision 12), gated by its own baseline
# because the two sets of numbers are not comparable. Deliberately not on
# the nightly: it costs money per run, it needs a key CI does not hold,
# and a gate that pages on model drift is the one ADR-0028 decision 6
# already refused. Needs ANTHROPIC_API_KEY, or SYNVEDA_EXTRACTOR=vllm
# plus SYNVEDA_VLLM_BASE_URL for the air-gapped path.
eval-extraction-live:
	SYNVEDA_EXTRACTOR=$${SYNVEDA_EXTRACTOR:-claude} \
	EVAL_BASELINE=evals/baseline-live.json sh evals/run.sh

# Parses the suite, the corpus and the baseline with no stack at all —
# what `ci` can run, and what catches a scenario that would measure
# nothing or a fixture whose label can never match.
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
