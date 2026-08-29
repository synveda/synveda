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

.PHONY: fmt lint test build deny check-deps check-adr-status check-adapters check-api-types check-backlog check-benchmarks check-chart-images check-compose-contract check-context-hard-cut check-context-security check-corpus-licences check-demos check-deploy check-docs check-npm-licences check-product-eval chart-lint compose-config compose-secrets ts-build ts-test ci dev-up dev-down smoke db-test claude-acceptance claude-acceptance-live eval eval-check eval-product eval-judge eval-read eval-longmemeval eval-longmemeval-full eval-longmemeval-judged eval-extraction-live eval-retrieval eval-security

dev-up:
	$(COMPOSE) up --build --detach --wait

dev-down:
	$(COMPOSE) down

smoke:
	bash scripts/smoke.sh

# CPR-45's additive canonical topology is static-only until database, realm and
# issuer convergence land. This renders all eight runtime/provider rows and
# starts or pulls nothing; it is not an alias for the legacy dev lifecycle.
compose-config: check-compose-contract

compose-secrets:
	deploy/compose/scripts/generate-secrets.sh

# The eval harness (EVAL-1, ADR-0028; EVAL-2, ADR-0046; EVAL-4, ADR-0047):
# the scenario suite, the labelled extraction corpus and the Q&A corpus
# against a live stack on one fresh exact-role database fixture, gated by
# evals/baseline.json. The target owns that database lifecycle and needs Docker
# plus node. Exit status is the gate's, and since EVAL-4 this is what `ci.yml`
# runs on every pull request.
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
	node scripts/product-evaluation.mjs --check
	node --test scripts/product-evaluation.test.mjs

# CPR-40's deterministic product/trust suite. It executes exact
# database-backed acceptance cases on a fresh migrated scratch database,
# rejects skipped DB tests, and writes one machine-readable and one
# human-readable report under target/. The existing corpus targets below
# remain separate because semantic/model measurements are not comparable with
# the deterministic path.
eval-product:
	SYNVEDA_DB_TEST_TASK=product-evaluation \
		bash scripts/db-test.sh

check-product-eval:
	node scripts/product-evaluation.mjs --check
	node --test scripts/product-evaluation.test.mjs

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

# LongMemEval's deterministic retrieval tier (EVAL-3, ADR-0061
# decision 5): did the block bind the evidence sessions the instance names
# in `answer_session_ids`? Record identity — the predicate EVAL-4 already
# grades and the one reproducible from bytes — so it gates, against
# `evals/baseline-longmemeval.json`, and it reaches no model and costs
# nothing per run.
#
# The declared slice, per decision 7: a suite that bounds its coverage
# says what it bounded, and every report states the corpus digest, the
# instance count, the slice rule and the abstention instances excluded.
# The actor pool is sized to the slice — one actor per instance is what
# keeps forty-session haystacks from landing inside each other.
#
# Needs the corpus fetched into evals/fixtures/longmemeval (NOTICE.md says
# why it is not committed). The model-judged tier — the published QA
# accuracy, gated by nothing — is the other half of decision 5 and lands
# with the reader and the judge already built.
eval-longmemeval:
	sh evals/run-longmemeval.sh

# All 500 instances. A target somebody schedules rather than one they wait
# on: seeding an instance is ~40 sessions through the whole pipeline, and
# ADR-0061's reversal trigger (f) is already written for the day this
# outgrows a single ordered pass.
eval-longmemeval-full:
	EVAL_LONGMEMEVAL_INSTANCES=500 sh evals/run-longmemeval.sh

# The model-judged tier (decision 5): the same run, plus a reader that
# answers each question out of the block and a judge that grades the
# answer against the corpus's reference. This is the published figure and
# the marketing artefact — and it gates nothing, deliberately. A gate that
# fails when a model changes rather than when the code changes is the
# alarm ADR-0028 decision 6 already refused; breaches print, the exit
# status stays success, and `eval-longmemeval` is where a regression stops
# a build.
#
# It costs money per instance and the reader is the expensive half: its
# prompt is a whole governed block. The judge's own agreement is measured
# inside the run rather than beside it (decision 4), so a score cannot be
# published without the number that bounds what it can claim. Needs
# ANTHROPIC_API_KEY; SYNVEDA_READER/SYNVEDA_JUDGE=extractive/lexical runs
# the whole shape for free, which is what a dry run wants and not what a
# published figure is.
eval-longmemeval-judged:
	SYNVEDA_READER=$${SYNVEDA_READER:-claude} \
	SYNVEDA_JUDGE=$${SYNVEDA_JUDGE:-claude} \
	EVAL_LONGMEMEVAL_JUDGED=1 sh evals/run-longmemeval.sh

# The reader measured against its probes, graded by the configured judge
# (EVAL-3, ADR-0061 decision 6). The blocks come from a file rather than
# from a live ContextRun, so this measures the reader and the judge and NOT
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
	bash scripts/db-test.sh

# CPR-14's deterministic tier: authentic Claude Code frames through the built
# hook, the real gateway/PDP/schema, persisted events, timeline and audit chain.
# A fresh scratch database is created and dropped by db-test.sh.
CLAUDE_ACCEPTANCE_TEST := a_claude_code_session_is_a_governed_run_from_start_to_end
claude-acceptance:
	pnpm --filter @synveda/claude-code-adapter build
	cargo test -q -p synveda-gateway --test claude_lifecycle -- --list | \
		grep -Fqx '$(CLAUDE_ACCEPTANCE_TEST): test'
	bash scripts/db-test.sh \
		-p synveda-gateway --test claude_lifecycle \
		$(CLAUDE_ACCEPTANCE_TEST) \
		-- --exact --nocapture --test-threads=1

# Tier 3 is never substituted by replay. The wrapper exits 77, with the exact
# missing prerequisite, when Claude Code or authentication is unavailable.
claude-acceptance-live:
	bash scripts/claude-acceptance-live.sh

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

# The frontend's types are generated from the OpenAPI document, and the
# document is generated from the gateway's own handlers (CPR-4, ADR-0071
# decision 7). The Rust half of that chain is checked by
# `crates/synveda-gateway/tests/openapi.rs`, which `test` already runs; this is
# the TypeScript half. Needs nothing but node, so it runs early.
#
# To refresh both after changing a DTO or a handler annotation:
#   SYNVEDA_WRITE_OPENAPI=1 cargo test -p synveda-gateway --test openapi
#   node scripts/generate-api-types.mjs
check-api-types:
	node scripts/generate-api-types.mjs --check

# STATUS.md is the concise feature inventory. This enforces unique IDs/counts
# and requires an implementation-ready brief for open work only; delivered
# history stays in git rather than duplicate Markdown diaries.
check-backlog:
	node --test scripts/check-backlog.test.mjs
	node scripts/check-backlog.mjs

# Demos are executable documentation. CPR-13 derives the accepted command
# vocabulary from Clap's recursive help and the route vocabulary from the
# generated OpenAPI document, then checks every shell script without executing
# it. The fixture test deliberately adds one dead command and one dead path.
check-demos:
	node --test scripts/check-demos.test.mjs
	node scripts/check-demos.mjs

# CPR-39: a config recipe, captured protocol and a fully verified client are
# deliberately different support levels. This also checks the fixture hashes
# and the generated public support/onboarding surfaces plus README summary.
check-adapters:
	node --test scripts/check-adapter-conformance.test.mjs
	node scripts/check-adapter-conformance.mjs

# CPR-42: the Rust/TypeScript suites execute each adversarial case; this
# inventory prevents a refactor from deleting a whole security boundary while
# leaving unrelated tests green, and enforces the non-execution/client/logging
# seams that are visible directly in source.
check-context-security:
	node --test scripts/check-context-security.test.mjs
	node scripts/check-context-security.mjs

# CPR-43: active runtime code/config carries no retired route, aggregate,
# table, sidecar, hidden alias or old telemetry name; storage is exactly one
# epoch-3 baseline with the pgvector-only extension shape.
check-context-hard-cut:
	node --test scripts/check-context-hard-cut.test.mjs
	node scripts/check-context-hard-cut.mjs

# check-backlog does not read ADR headers. This closes the useful half of that
# gap: a delivered feature must not retain a Proposed ADR. Accepted decisions
# may precede delivery because ADRs are written first.
check-adr-status:
	node scripts/check-adr-status.mjs

# Current documentation, including open briefs, must resolve repository-local
# links and code-path references. Historical ADR/spike prose stays link-checked
# without becoming a claim about the current product.
check-docs:
	node --test scripts/check-docs.test.mjs
	node scripts/check-docs.mjs

# The repository licence rule on the npm side (CNSL-1, ADR-0056 decision 8).
# Needs the workspace installed, so it runs after ts-build in `ci`.
check-npm-licences:
	node scripts/check-npm-licences.mjs

# The same rule on the corpus side (EVAL-3, ADR-0061 compliance notes).
# `cargo deny` governs crates and check-npm-licences governs packages; a
# corpus is data, which is how a CC BY-NC one reached a feature
# specification, the phase demo goal and AGENTS.md before anyone read its
# LICENSE.txt. Needs nothing but node, so it runs early in `ci` — and it
# also fires on a developer's machine that fetched a corpus, which is
# where the licence file actually lands.
check-corpus-licences:
	node scripts/check-corpus-licences.mjs

# docs/BENCHMARKS.md's table is generated from evals/scores/*.json (EVAL-3,
# ADR-0061 decision 11); this asserts the two still agree. "Tracked per
# release is a file that accumulates rows, not a number somebody edits" —
# and this is what makes the second half of that sentence enforceable. To
# publish a row: `node scripts/publish-benchmark.mjs <report.json>`.
check-benchmarks:
	node scripts/publish-benchmark.mjs

# The same rule again, one artefact class further out (OPS-2, ADR-0062
# decision 11). cargo-deny governs crates, check-npm-licences packages and
# check-corpus-licences corpora; a Helm chart references container images,
# which none of those look at. Tags are matched too, so bumping an image is
# a diff somebody reads — which is the point, because an inference server's
# licence is exactly the kind that changes between releases.
check-chart-images:
	node scripts/check-chart-images.mjs

# The enterprise chart renders, in both of the shapes CI covers: the
# minimum a real install must state, and every optional path at once.
# Needs helm. The chart's defaults deliberately do not render — five values
# have no default because each is a decision somebody has to make on
# purpose — so the lint values are also the list of those decisions.
chart-lint:
	helm lint deploy/helm/synveda --strict -f deploy/helm/synveda/ci/lint-values.yaml
	helm lint deploy/helm/synveda --strict -f deploy/helm/synveda/ci/full-values.yaml
	node scripts/check-helm-contract.mjs

# CPR-36: source/release Compose, Helm, generated API and the packaged profile
# are one runtime; a repeat package cannot retain a removed asset. CPR-44's
# scratch-HOME test keeps the local KEK exactly when this deployment keeps its
# volumes, and proves the explicit purge and dry-run paths separately.
check-deploy:
	node --test scripts/check-deploy-convergence.test.mjs
	node --test scripts/uninstall.test.mjs
	node --test scripts/check-compose-contract.test.mjs
	node scripts/check-deploy-convergence.mjs
	node scripts/check-compose-contract.mjs

check-compose-contract:
	node --test scripts/check-compose-contract.test.mjs
	node scripts/check-compose-contract.mjs

ts-build:
	pnpm install --frozen-lockfile
	pnpm -r build

# The adapter suites (ADPT-1); packages without a test script are skipped.
ts-test:
	pnpm -r test

ci: fmt lint test build deny check-deps check-api-types check-backlog check-demos check-adapters check-context-security check-context-hard-cut check-adr-status check-docs check-corpus-licences check-chart-images check-benchmarks chart-lint check-deploy eval-check ts-build check-npm-licences ts-test
