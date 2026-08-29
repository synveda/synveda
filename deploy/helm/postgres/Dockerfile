# Postgres for the Kubernetes deployment (OPS-2; CPR-36, ADR-0095).
#
# CloudNativePG's base image plus pgvector, the only non-core extension used
# by the epoch-3 schema. This is the same extension shape as the single-node
# image: bounded graph expansion and capture work leasing use ordinary SQL.

ARG CNPG_BASE=ghcr.io/cloudnative-pg/postgresql:17@sha256:fa6e2b2e14d19a109cc142cf857d328420bb7f1656b08c96e08be377692247ab
# Compile against Debian 11's glibc because the pinned CloudNativePG image is
# bullseye-based. The final-stage probe below enforces this ABI boundary.
FROM rust:1.96.0-bullseye@sha256:7069898d5edfc11b0ba498ecefbcc5438f6390b3ce0be11a9750cf39cab7e02f AS snapshot-builder

COPY deploy/compose/postgres/synveda-input-snapshot.c /tmp/synveda-input-snapshot.c

RUN set -eux; \
    cc -std=c11 -O2 -Wall -Wextra -Werror \
       /tmp/synveda-input-snapshot.c -o /usr/local/bin/synveda-input-snapshot; \
    strip /usr/local/bin/synveda-input-snapshot; \
    chmod 0555 /usr/local/bin/synveda-input-snapshot

FROM ${CNPG_BASE}

# The base runs as uid 26. apt and `make install` need root; the image has
# to go back, or the operator's own entrypoint fails on a permission it
# expects to have.
USER root

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends postgresql-17-pgvector=0.8.6-1.pgdg11+1; \
    test -x /usr/bin/timeout; \
    install -d -m 0555 /usr/local/share/synveda; \
    rm -rf /var/lib/apt/lists/*

# One audited, content-free database bootstrap command is shared with the
# canonical Compose PostgreSQL image. Build this Dockerfile from the repository
# root so Helm and Compose cannot drift into separate role/ACL implementations.
COPY --chmod=0555 deploy/compose/postgres/synveda-database-bootstrap /usr/local/bin/synveda-database-bootstrap
COPY --from=snapshot-builder --chmod=0555 /usr/local/bin/synveda-input-snapshot /usr/local/bin/synveda-input-snapshot
RUN printf 'snapshot-probe\n' > /tmp/synveda-snapshot-input \
    && /usr/local/bin/synveda-input-snapshot \
       /tmp/synveda-snapshot-input /tmp/synveda-snapshot-output \
    && test -s /tmp/synveda-snapshot-output \
    && rm -f /tmp/synveda-snapshot-input /tmp/synveda-snapshot-output
COPY --chmod=0444 deploy/compose/postgres/synveda-cluster-authority-contract.sql /usr/local/share/synveda/cluster-authority-contract.sql
COPY --chmod=0444 deploy/compose/postgres/synveda-local-authority-contract.sql /usr/local/share/synveda/local-authority-contract.sql
COPY --chmod=0444 deploy/compose/postgres/synveda-extension-contract.sql /usr/local/share/synveda/extension-contract.sql
COPY --chmod=0444 crates/synveda-store/sql/extension_fingerprint.sql /usr/local/share/synveda/extension-fingerprint.sql
COPY --chmod=0444 deploy/compose/postgres/synveda-credential-log-contract.sql /usr/local/share/synveda/credential-log-contract.sql

# Keep the bootstrap assertion byte-for-byte derived from the Rust query. The
# \gset footer must be in the same psql include because psql clears the query
# buffer when an included file reaches EOF.
RUN set -eux; \
    cp /usr/local/share/synveda/extension-fingerprint.sql \
       /usr/local/share/synveda/extension-fingerprint-assert.psql; \
    printf '%s\n' '\gset synveda_extension_' \
        >> /usr/local/share/synveda/extension-fingerprint-assert.psql; \
    chmod 0444 /usr/local/share/synveda/extension-fingerprint-assert.psql

USER 26
