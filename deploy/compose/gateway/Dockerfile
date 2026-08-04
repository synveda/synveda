# The gateway, as the single binary the SMB profile deploys (OPS-1,
# ADR-0055). Build context is the repository root.
#
# Two stages, and the split is the point: dependencies compile in their own
# layer so that editing this workspace's own sources does not recompile
# Cedar, Tantivy and sqlx. A cold build is minutes and is what ADR-0055
# decision 6 puts *outside* the acceptance criterion's clock — a release
# ships this image, and the operator being timed pulls it.

# The console bundle (CNSL-1, ADR-0056 decision 1). Its own stage, because
# it shares nothing with the Rust build and must not invalidate it: editing
# a stylesheet should not recompile Cedar.
#
# This is also what keeps decision 1's promise concrete — the image gains a
# static directory, not a second runtime. Node appears here and never in
# the runtime stage.
FROM node:22-bookworm-slim AS console
WORKDIR /src
RUN corepack enable
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY console/package.json console/
# Adapters and SDKs are workspace members the lockfile resolves against;
# their manifests have to exist for a frozen install to succeed.
COPY adapters/claude-code/package.json adapters/claude-code/
COPY adapters/mcp-server/package.json  adapters/mcp-server/
COPY sdks/typescript/package.json      sdks/typescript/
RUN pnpm install --frozen-lockfile --filter @synveda/console...
COPY tsconfig.base.json ./
COPY console console
RUN pnpm --filter @synveda/console build

FROM rust:1.96.0-bookworm AS build
WORKDIR /src

# The toolchain file pins the same version this image starts from; copying
# it keeps a mismatch a build error rather than a silent upgrade.
COPY rust-toolchain.toml ./

# Dependency layer: the manifests and a stub for every crate in the
# workspace, so `cargo build` resolves and compiles the dependency graph
# with none of our code in it. `members = ["crates/*"]` means every
# directory must exist with a lib target for the resolve to succeed.
COPY Cargo.toml Cargo.lock ./
COPY crates/synveda-audit/Cargo.toml       crates/synveda-audit/
COPY crates/synveda-cli/Cargo.toml         crates/synveda-cli/
COPY crates/synveda-eval/Cargo.toml        crates/synveda-eval/
COPY crates/synveda-gateway/Cargo.toml     crates/synveda-gateway/
COPY crates/synveda-identity/Cargo.toml    crates/synveda-identity/
COPY crates/synveda-ingest/Cargo.toml      crates/synveda-ingest/
COPY crates/synveda-policy/Cargo.toml      crates/synveda-policy/
COPY crates/synveda-retrieval/Cargo.toml   crates/synveda-retrieval/
COPY crates/synveda-store/Cargo.toml       crates/synveda-store/
COPY crates/synveda-types/Cargo.toml       crates/synveda-types/
COPY crates/synveda-vedaflow/Cargo.toml    crates/synveda-vedaflow/
RUN set -eux; \
    for crate in crates/*/; do \
      mkdir -p "$crate/src"; \
      : > "$crate/src/lib.rs"; \
    done; \
    mkdir -p crates/synveda-cli/src crates/synveda-eval/src crates/synveda-gateway/src; \
    echo 'fn main() {}' > crates/synveda-cli/src/main.rs; \
    echo 'fn main() {}' > crates/synveda-eval/src/main.rs; \
    echo 'fn main() {}' > crates/synveda-gateway/src/main.rs
# sqlx macros compile against the committed query cache; this image has no
# database, exactly like CI.
ENV SQLX_OFFLINE=true
RUN cargo build --release -p synveda-gateway || true

# The real sources. Touching them invalidates only from here down.
COPY .sqlx .sqlx
COPY crates crates
RUN set -eux; \
    find crates -name '*.rs' -newermt '1970-01-01' -exec touch {} +; \
    cargo build --release -p synveda-gateway

FROM debian:bookworm-slim AS runtime
# ca-certificates for OIDC discovery against an external issuer over TLS;
# curl for the healthcheck compose waits on.
RUN apt-get update \
 && apt-get install --no-install-recommends --yes ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*
# One binary and nothing else: the Cedar schema, the base layer and all
# three product packs are `include_str!`d into synveda-policy, so there is
# no policy directory to ship and no way for the image's policy to differ
# from the binary's.
COPY --from=build /src/target/release/synveda-gateway /usr/local/bin/synveda-gateway

# The console bundle: static files the binary serves from its own origin
# (CNSL-1, ADR-0056 decision 1). No Node in this stage — the build stage
# above is where it lived, and the runtime is still one process.
COPY --from=console /src/console/dist /usr/share/synveda/console
ENV SYNVEDA_CONSOLE_DIR=/usr/share/synveda/console

# The Tantivy sidecar (CTX-1). A named volume mounts here so an index
# survives a container replacement rather than being rebuilt on every
# restart by the indexer's sweep.
ENV SYNVEDA_SEARCH_INDEX_DIR=/var/lib/synveda/search
RUN mkdir -p /var/lib/synveda/search

WORKDIR /app
EXPOSE 8120
HEALTHCHECK --interval=5s --timeout=3s --retries=24 \
  CMD curl -fsS http://localhost:8120/healthz || exit 1
ENTRYPOINT ["/usr/local/bin/synveda-gateway"]
