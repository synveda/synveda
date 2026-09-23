# Developing Synveda

For trying the released product, use [prebuilt Docker](../deploy/compose/PREBUILT.md).
This guide is for changing the checkout. The [contribution guide](../CONTRIBUTING.md)
covers choosing work and submitting a PR.

## Source setup

Start in your cloned checkout. Git, GNU Make and Node are enough for the fast
static checks. Install the other tools only for the area you are changing:

| Tool | Repository contract | Needed for |
| --- | --- | --- |
| Rust via rustup | **1.96.0**, rustfmt and Clippy in [rust-toolchain.toml](../rust-toolchain.toml) | Rust builds and tests |
| Node.js | Root manifest declares 22+; use **22.13+** for pnpm 11 (CI uses Node 22 and SDK/site checks also use 24) | Static checks, console, adapters, website |
| pnpm | **11.13.1** in [package.json](../package.json) | Frozen frontend workspace installation |
| C compiler, linker, pkg-config, OpenSSL development libraries | Native TLS dependencies in [Cargo.toml](../Cargo.toml) | Rust source builds; Debian/Ubuntu packages: `build-essential pkg-config libssl-dev` |
| Docker / Compose / Buildx | Engine **28+**, Compose **2.33.1+**, local Unix socket; embedded `default` builder for source images | Database tests and full source deployment |
| Python | **3.11+**, locked development requirements | SDK work and `make ci`; see [SDK setup](../sdks/README.md#install-locally-and-check) |
| Helm | **4.2.3** in CI | Chart/deployment checks; no cluster for rendering |

Linux on GitHub-hosted runners is the CI reference. Source development also
has macOS Apple Silicon/OrbStack evidence. On macOS install Command Line Tools
and the native build prerequisites. Windows/WSL2 and Docker Desktop installation
qualification remain open; do not infer support from a successful Compose render.
Internet access is needed to obtain toolchains, packages and container images;
basic validation needs no maintainer secret or paid service.

```sh
# First feedback: no package install, database or Rust compilation.
make check-fast

# For Rust changes (rustup reads rust-toolchain.toml):
rustup toolchain install
export SQLX_OFFLINE=true
cargo test -p synveda-types

# For console, adapters or website changes, with the pinned pnpm installed:
pnpm --version                    # 11.13.1
pnpm install --frozen-lockfile
```

Keep Cargo.lock and pnpm-lock.yaml. Do not regenerate them to solve an unrelated
setup failure. SQLx compiles against the committed `.sqlx` cache in offline
mode; this does not provide a running database for integration tests.

## Running your changes

The [source Compose guide](../deploy/compose/README.md) owns environment settings,
file-backed secrets, exact host/DNS prerequisites and lifecycle commands.
No root `.env`, host database or manually seeded administrator is required.
Source development uses `app.synveda.test` and `auth.synveda.test`; the released
loopback bundle uses localhost and is a separate installation.

Follow [development hostname setup](../deploy/compose/README.md#development-hostname-setup)
first. The privileged hosts edit is explicit. If another project owns that
block, do not overwrite it or stop its services: use a disposable development
host or arrange a deliberate handoff with its operator. Then, from the checkout:

```sh
export SYNVEDA_COMPOSE_PROFILES=demo
make compose-up
make compose-smoke
```

`compose-up` builds the current checkout, creates distinct database roles, runs
the epoch-3 migration and tenant bootstrap, and converges Keycloak. It prints
the actual console URL (default **http://app.synveda.test:8080/console/**),
demo usernames and protected password-file paths. Retrieve the generated
password from the printed file in your own terminal and sign in as
`synveda-demo-admin`. Create a workspace and project through Getting started;
the project should appear in the selector. No shared default password exists.
The optional [source product walkthrough](../deploy/compose/README.md#governed-ingestion-retry-walkthrough)
uses ordinary authenticated APIs and distinct reviewers for synthetic learnings.

Backend and console changes use this same image build and authentication path.
After edits, rerun `make compose-up` and `make compose-smoke` with the same
selectors; compilation reuses Docker's cache. For fast console feedback, use
its unit tests and production build below. `pnpm --filter @synveda/console dev`
starts Vite, but the current Vite configuration has no API/auth proxy: it is
not a complete signed-in development environment. The gateway serves the
production bundle at `/console/` with its real same-origin cookie/CSP boundary.

```sh
make compose-down                 # preserve volumes, keys and issuer
make compose-up                   # reuse that same state
```

`make compose-reset` is destructive and requires the lifecycle's exact
project-bound confirmation. It is not a routine troubleshooting or migration
step. Never remove existing volumes or key files to get a test green.
Pre-epoch-3 databases are deliberately refused; no compatibility migrator exists.
For a **new disposable database**, let the normal bootstrap apply
[the single baseline migration](../crates/synveda-store/migrations/0001_context_platform.sql).

## Validation

Run from the repository root. Choose focused checks in addition to the fast
path; `check-fast` composes existing static gates and does not execute product
or database behavior.

| Change | Commands |
| --- | --- |
| All changes / docs | `make check-fast` and `git diff --check` |
| Rust crate | `cargo fmt --all --check`; `SQLX_OFFLINE=true cargo clippy -p synveda-types --all-targets -- -D warnings`; `SQLX_OFFLINE=true cargo test -p synveda-types` (replace the crate as appropriate) |
| Console | `pnpm --filter @synveda/console test`; `pnpm --filter @synveda/console build` |
| Claude hooks | `pnpm --filter @synveda/claude-code-adapter test` |
| Codex / Copilot hooks | `pnpm --filter @synveda/claude-code-adapter build`, then `pnpm --filter @synveda/codex-adapter test` or `pnpm --filter @synveda/copilot-cli-adapter test` |
| Public website / brand | `pnpm --filter @synveda/website check` ([preview and asset workflow](../website/README.md)) |
| SDKs | `make sdk-check sdk-package-check` after [SDK prerequisites](../sdks/README.md#check-local-package-archives) |
| Deployment / packaging | `make check-deploy chart-lint` (Docker Compose and Helm installed; no live application implied) |
| CI / release automation | `make check-ci`; `actionlint -shellcheck=` with actionlint 1.7.7; then the affected packaging checks in the [CI/release guide](CI.md) |

The following checks need real local services, but no proprietary client or
model credential:

```sh
# One focused gateway suite on fresh exact-role PostgreSQL fixtures:
bash scripts/db-test.sh -p synveda-gateway --test sessions_api -- --test-threads=1

make db-test                      # full fresh database suite
make claude-acceptance            # captured real frames replayed through gateway
make eval                        # deterministic evaluation, isolated PostgreSQL
make compose-acceptance           # fresh named project; source guide prerequisites
```

The database wrapper generates private credentials, migrates disposable clusters,
uses ordinary tenant transactions, and refuses skipped required database tests.
Successful fixtures are removed; failures retain their task-owned state and
print recovery locations. It never targets your deployment database.

`SQLX_OFFLINE=true make ci` is the broad **local aggregate**, after installing
Rust, the pnpm workspace, cargo-deny, Helm, Docker Compose and the SDK development
requirements. It does not reproduce all hosted CI: `.github/workflows/ci.yml`
also runs database-backed evaluation, SDK interoperability, Claude replay,
native client archives and exact Docker/Helm candidate qualification. The
[CI/release guide](CI.md) maps every job and the stable CI Result gate.
Avoid `--all-features`: the normal build and
test-support feature configurations have separate purposes.

Live `make claude-acceptance-live` needs an installed authenticated Claude client;
exit 77 means unavailable. `make eval-extraction-live` needs model credentials
or a configured local inference endpoint. Semantic retrieval downloads a model.
These are separate from a basic first contribution; report exactly which tier ran.

## Generated contracts

After intentionally changing public DTOs or routes:

```sh
SQLX_OFFLINE=true SYNVEDA_WRITE_OPENAPI=1 cargo test -p synveda-gateway --test openapi
node scripts/generate-api-types.mjs
```

For the SDK slice, follow [its generator/check workflow](../sdks/README.md).
For changed SQL, install the SQLx CLI matching the workspace's SQLx 0.8 series,
then regenerate against the wrapper's fresh current database:

```sh
SYNVEDA_DB_TEST_TASK=sqlx-prepare bash scripts/db-test.sh
```

Never hand-edit `.sqlx/query-*.json`, `docs/api/openapi.json` or
`console/src/generated/api.ts`. Review every changed query hash; no unrelated
cache churn. The route catalogue, OpenAPI and console client must remain peers.

## Code map

| Area | Starting point |
| --- | --- |
| Types / scopes / roles | [synveda-types](../crates/synveda-types/src) |
| Gateway and separate core worker | [routes.rs](../crates/synveda-gateway/src/routes.rs), [app.rs](../crates/synveda-gateway/src/app.rs), [worker.rs](../crates/synveda-gateway/src/worker.rs) |
| Identity, Cedar and encryption | [synveda-identity](../crates/synveda-identity/src), [synveda-policy](../crates/synveda-policy/src), [policies](../policies/README.md), [synveda-crypto](../crates/synveda-crypto/src) |
| Persistence / forced RLS / migrations | [synveda-store](../crates/synveda-store/src), [migrations](../crates/synveda-store/migrations) |
| Reviewed changes and audit | [synveda-vedaflow](../crates/synveda-vedaflow/src), [synveda-audit](../crates/synveda-audit/src) |
| Capture and retrieval | [synveda-ingest](../crates/synveda-ingest/src), [synveda-retrieval](../crates/synveda-retrieval/src) |
| CLI, MCP and exchange | [synveda-cli](../crates/synveda-cli/src), [synveda-okf](../crates/synveda-okf/src) |
| React UI / API types | [console/src](../console/src), [generated client](../console/src/generated/api.ts) |
| Client integrations | [adapters](../adapters), [support registry](../adapters/registry.json), [SDKs](../sdks/README.md) |
| Tests and executable acceptance | Each crate's `tests/`, [gateway tests](../crates/synveda-gateway/tests), [demos](../demos), [evaluation suite](../evals/product/README.md) |
| Deployment / optional queue experiment | [Compose](../deploy/compose/README.md), [Helm](../deploy/helm/synveda/README.md), [synveda-apalis](../crates/synveda-apalis) |
| Public site / brand | [website guide](../website/README.md), [brand assets](../assets/brand) |

### Trace a session event append

1. [routes.rs](../crates/synveda-gateway/src/routes.rs) binds
   `POST /v1/sessions/{session_id}/events` to `sessions::append_events`.
   Authentication and tenant middleware are assembled in `app.rs`.
2. [gateway sessions.rs](../crates/synveda-gateway/src/sessions.rs) opens a
   tenant transaction, loads the Session with `SessionWrite`, checks ownership
   and validates/redacts the bounded batch. Cedar authorizes the operation.
3. [store sessions.rs](../crates/synveda-store/src/sessions.rs) locks the
   Session and appends immutable events idempotently by `client_event_id`
   under forced RLS. The gateway appends content-free audit evidence and commits.
4. [sessions_api.rs](../crates/synveda-gateway/tests/sessions_api.rs) tests the
   complete path: retry and partial redelivery, foreign-tenant isolation,
   unauthorized calls and content-free audit. Run it with the database command
   above. Session evidence is not active Knowledge: later Capture creates
   candidates, and reviewed acceptance enters the typed Knowledge/VedaFlow path.

## Troubleshooting

- **SQLx asks for DATABASE_URL during compilation:** use `SQLX_OFFLINE=true`.
  If a query changed, regenerate metadata with the fresh fixture; do not point
  compilation/tests at a retained deployment database.
- **pnpm/version or frozen-lockfile failure:** compare Node/pnpm against the
  pins above; install the pinned tools before changing a lockfile.
- **Docker socket, remote context or builder refusal:** check `docker context
  show`, `docker info`, `docker compose version` and `docker buildx ls`. Source
  lifecycle requires the local embedded default builder and no ambient builder
  override. See the Compose guide for the exact refusal.
- **Issuer, hosts ownership, TLS or missing-key refusal:** follow the
  [source lifecycle diagnostics](../deploy/compose/README.md). Preserve the
  original issuer and matching key set; do not disable auth, PDP or RLS.
- **Tests pass suspiciously quickly:** check for database skips. Only the
  explicit database wrapper establishes database acceptance; a static Compose
  render establishes configuration validity, not startup or login.
