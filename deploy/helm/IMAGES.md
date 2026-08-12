# Container images we ship

Every image the Helm chart can reference, every image the **released
single-node profile** runs, and every base image the images we build are
built from. `scripts/check-chart-images.mjs` (in `make ci`) fails the build
when one of those surfaces names an image that is not on this list, **tag
included** — so a version bump is a diff somebody reads rather than a
silent change of what is installed.

The release profile joined with OPS-8 (ADR-0065 decision 9). It is the
stronger case, not the weaker one: the chart is what a customer's platform
team installs deliberately, and `deploy/release/` is what anybody installs
with one `curl | sh`. Adding it found four images no check had looked at,
one of them an inference server pinned by commit.

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

## Images the released single-node profile runs

`deploy/release/docker-compose.yml`, the bundle `scripts/install.sh` unpacks
under `~/.synveda/profile`. `<version>` is the release tag, substituted by
`scripts/package-release.sh` — inventoried as a placeholder for the same
reason the chart's `<appVersion>` is.

| Image | Where | Licence | Why it is here |
|---|---|---|---|
| `ghcr.io/synveda/gateway:<version>` | `gateway`, `--issuer` path only | ours | The product. Same image and same Dockerfile the chart runs, published rather than built. A default install runs the binary on the host instead — ADR-0055 decision 8. |
| `ghcr.io/synveda/postgres:<version>` | `postgres` | ours (see bases) | Postgres 17 with pgvector, AGE and PGMQ, from `deploy/compose/postgres/Dockerfile`. The same image the dev compose builds as `synveda/dev-postgres`; the published name drops the `dev-` because this one is what a customer installs. **Keeps AGE**, unlike the chart's `enterprise-postgres` — ADR-0062 decision 3 dropped it there for a reason that does not apply to a single node. |
| `ghcr.io/sebadob/rauthy:0.35.2` | `rauthy` | Apache-2.0 | The bundled OIDC provider. Dev-shaped credentials, and the reason the default install's gateway is a host process. |
| `jaegertracing/jaeger:2.19.0` | `jaeger` | Apache-2.0 | Traces on port 16686. FND-5's exporter targets it; the profile starts it because an install nobody can see inside is harder to trust. |
| `ghcr.io/huggingface/text-embeddings-inference:cpu-1.8.1` | `tei`, optional | **read on every bump** | The amd64 embedder, when `--embedder tei`. See the arm64 row below. |

## The per-architecture embedder pins

Upstream publishes two TEI builds and versions only one of them. The pins
live in the `Makefile` (`TEI_IMAGE_*`), which is what `make dev-up` resolves
and what `synveda init` carries its own copy of for an operator with no
Makefile.

| Image | Where | Licence | Why it is here |
|---|---|---|---|
| `ghcr.io/huggingface/text-embeddings-inference:cpu-arm64-sha-4150561` | `TEI_IMAGE_arm64` | **read on every bump** | Apple Silicon. There are no versioned arm64 tags, so this is pinned by *commit* rather than left on `cpu-arm64-latest` — which means a bump is a deliberate act and the licence at that commit is what applies. It agrees with the amd64 release to float32 rounding (cosine 1.000000000, max abs diff 7e-8, measured 2026-07-26), which is the property that matters when `record_embeddings` stores a model and a dim. |

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

## Images the install test runs, and the chart never does

`make check-chart-images` does not scan `demos/` — the chart is what a
customer installs, and the test's scaffolding is not shipped to anyone. They
are recorded here anyway, because "not shipped" is a reason to hold a lower
bar, not none.

| Image | Where | Licence | Why it is here |
|---|---|---|---|
| `ghcr.io/sebadob/rauthy:0.35.2` | `demos/fixtures/ops-2/idp.yaml` | Apache-2.0 | The test issuer, at a Service DNS name. Same version the dev compose runs. |
| `node:22-bookworm-slim` | `demos/fixtures/ops-2/client-pod.yaml` | MIT | Plays the browser half of `synveda login`. |
| CloudNativePG operator | applied by the demo, version pinned in it | Apache-2.0 | Installed separately by design; the chart renders a `Cluster` for it. |

## Extensions compiled into `synveda/enterprise-postgres`

Not images, and not covered by `cargo-deny` either, so they are recorded
in the same place:

| Extension | Version | Licence |
|---|---|---|
| pgvector | PGDG `postgresql-17-pgvector` | PostgreSQL |
| PGMQ | `v1.10.1` | PostgreSQL |

Apache AGE is deliberately **not** in this image; the dev compose image
keeps it. `deploy/helm/postgres/Dockerfile` says why.
