# Container images in the enterprise profile

Every image the chart can reference, and every base image the two images we
build are built from. `scripts/check-chart-images.mjs` (in `make ci`) fails
the build when the chart names one that is not on this list, **tag
included** — so a version bump is a diff somebody reads rather than a
silent change of what is installed.

That is the modest half of the job. The check proves the list is complete;
it cannot prove a licence is admissible, because a container image carries
no machine-readable licence. A human reads those and records what they
read, here.

## Why this file exists

CLAUDE.md's licence rule is enforced by `cargo-deny` over crates,
`check-npm-licences` over packages and — since ADR-0061 —
`check-corpus-licences` over corpora. A Helm chart introduces a fourth
kind of artefact, and until this file nothing in the repository looked at
it. That is the same gap in the same shape as the one that let a CC BY-NC
corpus reach a published phase demo goal untouched by any check (EVAL-7).

The entry to read first on any bump is **text-embeddings-inference**: an
inference server's licence is exactly the kind that changes between
releases, and that image carries both a binary and a model.

## Images the chart runs

| Image | Where | Licence | Why it is here |
|---|---|---|---|
| `synveda/gateway:<appVersion>` | `image.repository` | ours | The product. Both binaries: the gateway serves, the CLI migrates and issues SCIM credentials. Built from `deploy/compose/gateway/Dockerfile`. |
| `synveda/enterprise-postgres:17` | `postgres.image` | ours (see bases) | Postgres for CloudNativePG, plus pgvector and PGMQ. Built from `deploy/helm/postgres/Dockerfile`. No AGE — ADR-0062 decision 3. |
| `ghcr.io/huggingface/text-embeddings-inference:cpu-1.8.1` | `tei.image`, optional | **read on every bump** | The embedder, when `embedder: tei` and `tei.enabled`. Serves BAAI/bge-m3, whose weights are a separate licence from the server's. |

## Base images we build on

| Image | Built into | Licence | Notes |
|---|---|---|---|
| `ghcr.io/cloudnative-pg/postgresql:17` | enterprise-postgres | Apache-2.0 (CNPG) over PostgreSQL-licensed Postgres | The operator's own base. Pinned by tag family here; a release pins the digest. |
| `rust:1.96.0-bookworm` | gateway (build stage) | MIT/Apache-2.0 | Matches `rust-toolchain.toml`; a mismatch is a build error rather than a silent upgrade. |
| `node:22-bookworm-slim` | gateway (console stage) | MIT | Builds the console bundle. Never in the runtime stage. |
| `debian:bookworm-slim` | gateway (runtime stage) | various, all Debian-main | Runtime: `ca-certificates` for OIDC discovery, `curl` for the healthcheck. |

## Extensions compiled into `synveda/enterprise-postgres`

Not images, and not covered by `cargo-deny` either, so they are recorded
in the same place:

| Extension | Version | Licence |
|---|---|---|
| pgvector | PGDG `postgresql-17-pgvector` | PostgreSQL |
| PGMQ | `v1.10.1` | PostgreSQL |

Apache AGE is deliberately **not** in this image; the dev compose image
keeps it. `deploy/helm/postgres/Dockerfile` says why.
