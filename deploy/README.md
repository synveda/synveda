# Deployment

- `compose/` — SMB single-node profile (Postgres+pgvector+AGE+PGMQ, Rauthy,
  Temporal, TEI, Jaeger; FND-2) plus the gateway itself (`gateway/Dockerfile`,
  OPS-1). Installed with `synveda init` — see [docs/INSTALL.md](../docs/INSTALL.md).
- `helm/` — enterprise profile (OPS-2, ADR-0062): the gateway, an HA
  Postgres cluster under CloudNativePG, and the wiring for a customer's
  IdP. `helm/synveda/` is the chart, `helm/postgres/Dockerfile` builds its
  Postgres image, and `helm/IMAGES.md` is the inventory every image in it
  has to appear in (`make check-chart-images`).

## The enterprise profile installs what has a consumer

CloudNativePG is a dependency and ships. **Temporal and Qdrant do not**:
nothing in this workspace links a Temporal client (VedaFlow went into
Postgres, ADR-0003, and the Rust SDK's licence graph is inadmissible), and
`VectorIndex` — the trait a Qdrant would sit behind — is OPS-4's and does
not exist yet. Both are named in OPS-2's feature text; ADR-0062 decision 1
records the triggers that put them in the chart.

The **CloudNativePG operator is not installed by this chart**. It is
cluster-scoped, and a product chart that owns cluster-scoped CRDs fights
the next chart that wants them. Install it first; the chart renders a
`Cluster` for it.

## One gateway replica, and the chart refuses to render a second

Not a default — a rendering error, with the reason. Two things in the
gateway are process-local and both fail *silently* with more than one:
pending logins live in memory (a callback landing on another pod is a 401
for a login the IdP completed), and the scope-chain cache is invalidated
in-process with no TTL (a hierarchy move handled by one replica leaves the
others composing against the ancestry the mover left, which reads as a
policy decision rather than a stale cache). OPS-7 is the feature that
fixes both.

What needed no work, because it was already done in the database: the
audit chain's per-tenant head lock, the promotion sweep's watermark lock,
the lapse sweep's idempotency stamp, PGMQ's archive-lock, and console
sessions in a table. ADR-0062 decision 4 has the inventory.

## The gateway stops being a superuser here

The compose profile's gateway connects as `POSTGRES_USER`, which is the
bootstrap superuser, and a superuser bypasses row-level security even where
it is FORCED — so TEN-2's isolation backstop has been inert in every
deployment that exists. In this profile the install job migrates under
CNPG's superuser (migrations create a role and an extension) and then
grants the gateway's own login role membership of `synveda_app`, the
least-privilege role every migration has been granting since 0003. The
gateway never holds an admin credential, and `values.yaml` has no key that
can give it one.

## The gateway service is behind a profile

`gateway` carries `profiles: ["deployed"]`, so `make dev-up` brings up the
dependencies and not the product: that target is the contributor's loop,
where the gateway runs from `cargo run` against whatever is checked out.
`synveda init` starts it explicitly, and naming a profiled service on the
command line is enough to activate it.

Which of the two start paths `init` uses depends on the issuer, not on
taste — with the bundled Rauthy the gateway runs as a host process, because
an OIDC issuer identifier must be one URL both the browser and the gateway
can reach and RFC 6761 makes `http://localhost:8100/...` unreachable from
inside any container. ADR-0055 decision 8 has the measurements and the
alternatives that were tried.

## The TEI image is per-architecture

Upstream publishes two text-embeddings-inference builds and versions only
one of them: `cpu-<version>` is amd64, and the arm64 side ships as
`cpu-arm64-latest` plus per-commit `cpu-arm64-sha-<commit>` tags. There is
no versioned arm64 release, so compose pins the arm64 build **by commit**
rather than following `latest`.

`make dev-up` selects the image from `uname -m`; the compose default is the
amd64 release, which is what CI runs. Override with `SYNVEDA_TEI_IMAGE` to
pin something else.

The two builds are interchangeable where it matters. Measured 2026-07-26 on
the same inputs: same model (BAAI/bge-m3), same dimension (1024), vectors
agreeing to float32 rounding — cosine `1.000000000`, max `|diff|` 7e-8 —
and CTX-1's recorded live-TEI quality numbers reproduce exactly
(sparse-only recall@6 0.500, hybrid 0.792, MRR 1.0). That is the property
that has to hold: `record_embeddings` stores a model and a dim, so a corpus
embedded on one architecture must stay comparable to one embedded on the
other.

## GPU

Not available through compose on macOS, and not a configuration gap.
Docker on a Mac runs a Linux VM: Metal is a host-only framework with no
passthrough into containers, and there is no NVIDIA runtime to use instead
(`docker info` lists `runc` alone). MLX is Metal-backed on macOS — its
Linux wheels target CUDA — so an MLX embedder would have to run as a
**native host process**, outside compose, and would then be a second
embedder implementation that only some developers exercise.

The CPU path is adequate for dev: BGE-M3 on the arm64 build measures ~22 ms
for a single input and ~13 ms/text at batch 32 (~81 texts/sec sustained),
against an inject budget of p99 150 ms. If embedding throughput ever
becomes the constraint, `mlx-community/bge-m3-mlx-fp16` keeps the same
model and would slot behind the existing `Embedder` seam
(`synveda-ingest`), with the caveat above about dev/prod divergence.
