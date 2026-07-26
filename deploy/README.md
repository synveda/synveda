# Deployment

- `docker-compose/` — SMB single-node profile (Postgres+pgvector+AGE+PGMQ,
  Rauthy, Temporal, TEI, Jaeger). Lands with FND-2.
- `helm/` — enterprise multi-region profile. Lands with OPS-2 (Phase 3).

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
