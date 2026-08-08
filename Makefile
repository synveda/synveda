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

.PHONY: fmt lint test build deny check-deps check-backlog ts-build ts-test ci dev-up dev-down smoke db-test eval eval-check eval-judge eval-read eval-extraction-live eval-retrieval eval-security

dev-up:
	$(COMPOSE) up --build --detach --wait

dev-down:
	$(COMPOSE) down

smoke:
	bash scripts/smoke.sh

# The eval harness (EVAL-1, ADR-0028; EVAL-2, ADR-0046; EVAL-4, ADR-0047):
# the scenario suite, the labelled extraction corpus and the Q&A corpus
# against a live stack on a scratch database, gated by
# evals/baseline.json. Needs the dev compose (postgres) and node. Exit
# status is the gate's, and since EVAL-4 this is what `ci.yml` runs on
# every pull request.
eval:
	sh evals/run.sh

# EVAL-4's retrieval half (ADR-0047 decision 6): the same Q&A corpus with
# real embeddings, so the dense leg means something and the `semantic`
# questions are measured rather than skipped. Its own baseline, because a
# hash embedder's geometry carries none by construction and the two sets
# of numbers are not comparable. Unlike the live-*extraction* half this
# one **is** on the nightly: BGE-M3 is served locally from an image and a
# model id written in deploy/compose/docker-compose.yml, so it changes
# when someone edits that file — which is someone changing the code, and
# the thing ADR-0028 decision 6 asked a nightly failure to mean.
eval-retrieval:
	$(COMPOSE) up --detach --wait tei
	SYNVEDA_EMBEDDER=tei \
	SYNVEDA_TEI_URL=$${SYNVEDA_TEI_URL:-http://localhost:8110} \
	EVAL_DENSE_RETRIEVAL=1 \
	EVAL_BASELINE=evals/baseline-retrieval.json sh evals/run.sh

# EVAL-5's nightly (ADR-0048): the security corpus at the full variant
# budget, gated by evals/baseline-security.json. `make eval` already runs
# the same suite on every pull request at a deterministic 400-variant
# slice — a product that blocks a merge on a token count and not on a
# disclosure has recorded its priorities backwards — and this is the run
# the acceptance criterion's 10,000 belongs to.
#
# Its own baseline for a different reason than EVAL-4's split: there the
# two paths measure incomparable things, here they measure the same thing
# at different scale. The leak counts are identical in both files, because
# zero is zero; only the two coverage floors differ, and those exist
# because a one-sided gate whose denominator the run chooses passes by
# measuring less.
eval-security:
	EVAL_SECURITY_VARIANTS=10000 \
	EVAL_BASELINE=evals/baseline-security.json sh evals/run.sh

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

# The judge measured before it measures (EVAL-3, ADR-0061 decision 4):
# the configured judge over the labelled sets, with no gateway and no
# database. The default judge is the lexical rubric and reaches no
# network, so this is free and runnable anywhere; SYNVEDA_JUDGE=claude
# plus ANTHROPIC_API_KEY runs the model judge, which costs money per
# pair. Off `ci` for that reason and because it gates nothing — a judge
# measurement that failed a build would be decision 5's gate through a
# side door.
eval-judge:
	cargo run -q -p synveda-eval -- judge

# The reader measured against its probes, graded by the configured judge
# (EVAL-3, ADR-0061 decision 6). The blocks come from a file rather than
# from /v1/inject, so this measures the reader and the judge and NOT
# Synveda — the axes are named probe_* rather than qa_* to keep that
# impossible to mistake. SYNVEDA_READER=claude plus ANTHROPIC_API_KEY
# runs the model reader; the default selects a line and costs nothing.
eval-read:
	cargo run -q -p synveda-eval -- read

# The full suite against a scratch database of its own, dropped afterwards
# (kept on failure, and by KEEP_TEST_DB=1). It used to run against the
# long-lived dev database and leave every tenant it admitted behind — see
# scripts/db-test.sh for what that cost.
db-test:
	DATABASE_URL=$(DATABASE_URL) bash scripts/db-test.sh

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

# SYNVEDA_FEATURES.md, docs/backlog/<ID>.md and STATUS.md describe one feature
# set; this asserts they agree. Writes nothing — it replaced a generator that
# wrote all three and discarded their hand-written narrative doing it.
check-backlog:
	node scripts/check-backlog.mjs

# CLAUDE.md's licence rule on the npm side (CNSL-1, ADR-0056 decision 8).
# Needs the workspace installed, so it runs after ts-build in `ci`.
check-npm-licences:
	node scripts/check-npm-licences.mjs

ts-build:
	pnpm install --frozen-lockfile
	pnpm -r build

# The adapter suites (ADPT-1); packages without a test script are skipped.
ts-test:
	pnpm -r test

ci: fmt lint test build deny check-deps check-backlog eval-check ts-build check-npm-licences ts-test
