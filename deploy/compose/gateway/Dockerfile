# The product image every deployment shape uses (OPS-1; CPR-36, ADR-0095;
# CPR-45, ADR-0102). Build context is the repository root.
#
# Two stages, and the split is the point: dependencies compile in their own
# layer so that editing this workspace's own sources does not recompile
# Cedar and sqlx. A cold build is minutes and is what ADR-0055
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
# The adapter is a workspace member the lockfile resolves against.
COPY adapters/claude-code/package.json adapters/claude-code/
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
COPY crates/synveda-crypto/Cargo.toml      crates/synveda-crypto/
COPY crates/synveda-eval/Cargo.toml        crates/synveda-eval/
COPY crates/synveda-gateway/Cargo.toml     crates/synveda-gateway/
COPY crates/synveda-identity/Cargo.toml    crates/synveda-identity/
COPY crates/synveda-ingest/Cargo.toml      crates/synveda-ingest/
COPY crates/synveda-okf/Cargo.toml         crates/synveda-okf/
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
RUN cargo build --release -p synveda-gateway -p synveda-cli

# The real sources. Touching them invalidates only from here down.
COPY .sqlx .sqlx
COPY adapters/registry.json adapters/registry.json
COPY crates crates
RUN set -eux; \
    find crates -name '*.rs' -newermt '1970-01-01' -exec touch {} +; \
    cargo build --release -p synveda-gateway -p synveda-cli

FROM debian:bookworm-slim AS runtime
# ca-certificates for OIDC discovery against an external issuer over TLS;
# curl for the healthcheck compose waits on.
RUN apt-get update \
 && apt-get install --no-install-recommends --yes ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*
# Two binaries and no data: the Cedar schema, the base layer and all three
# product packs are `include_str!`d into synveda-policy, so there is no
# policy directory to ship and no way for the image's policy to differ from
# the binary's.
#
# The CLI joined the image with OPS-2 (ADR-0062 amendment). In the SMB
# single-node deployment it is on the operator's laptop and only the gateway is a
# container, so one binary was right; in a cluster there is no laptop with
# a route to the database, and the two things an enterprise install has to
# run — `synveda db migrate` under an admin identity, and
# `synveda scim token issue` to wire Entra or Okta (AUTH-4, ADR-0059) —
# are CLI verbs. A second image for them would double the build for one
# `cargo build -p` that shares this stage's dependency compilation anyway.
COPY --from=build /src/target/release/synveda-gateway /usr/local/bin/synveda-gateway
COPY --from=build /src/target/release/synveda /usr/local/bin/synveda
COPY --chmod=0755 deploy/compose/gateway/synveda-container /usr/local/bin/synveda-container

# The console bundle: static files the binary serves from its own origin
# (CNSL-1, ADR-0056 decision 1). No Node in this stage — the build stage
# above is where it lived, and the runtime is still one process.
COPY --from=console /src/console/dist /usr/share/synveda/console
ENV HOME=/tmp \
    XDG_CACHE_HOME=/tmp/.cache \
    SYNVEDA_CONSOLE_DIR=/usr/share/synveda/console

WORKDIR /app
EXPOSE 8120

# Compose supplies the operator's validated non-zero UID/GID so mode-0600
# secret bind mounts remain readable. This default also makes an unconfigured
# image non-root and preserves arbitrary-UID compatibility for later OpenShift.
USER 65532:65532
STOPSIGNAL SIGTERM

# Health is role-specific. Compose and Kubernetes call the closed `probe`
# command instead of inheriting a gateway-only OCI HEALTHCHECK.
ENTRYPOINT ["/usr/local/bin/synveda-container"]
CMD ["gateway"]
