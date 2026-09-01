# PostgreSQL 17 + pgvector shared by the reference, development and test
# topologies. Knowledge, bounded graph adjacency, capture leasing and durable
# jobs all live in ordinary tenant-bound tables; pgvector is the epoch-3
# schema's only non-core extension.
FROM rust:1.96.0-bookworm@sha256:5e2214abe154fe26e39f64488952e5c991eeed1d6d6da7cc8381ae83927f0cfc AS snapshot-builder
ARG HTTP_PROXY
ARG http_proxy
ARG HTTPS_PROXY
ARG https_proxy
ARG NO_PROXY
ARG no_proxy
ARG FTP_PROXY
ARG ftp_proxy
ARG ALL_PROXY
ARG all_proxy
COPY --chmod=0555 deploy/compose/scripts/assert-build-proxy-closed /usr/local/bin/assert-build-proxy-closed
RUN /usr/local/bin/assert-build-proxy-closed

COPY deploy/compose/postgres/synveda-input-snapshot.c /tmp/synveda-input-snapshot.c

RUN set -eux; \
    cc -std=c11 -O2 -Wall -Wextra -Werror \
       /tmp/synveda-input-snapshot.c -o /usr/local/bin/synveda-input-snapshot; \
    strip /usr/local/bin/synveda-input-snapshot; \
    chmod 0555 /usr/local/bin/synveda-input-snapshot

FROM postgres:17.11-bookworm@sha256:051f7b7b3abdd564d5d1bd1e8c4b9c1b6e77087d1dd22020ede611c096a272e0 AS runtime
ARG HTTP_PROXY
ARG http_proxy
ARG HTTPS_PROXY
ARG https_proxy
ARG NO_PROXY
ARG no_proxy
ARG FTP_PROXY
ARG ftp_proxy
ARG ALL_PROXY
ARG all_proxy
COPY --chmod=0555 deploy/compose/scripts/assert-build-proxy-closed /usr/local/bin/assert-build-proxy-closed
RUN /usr/local/bin/assert-build-proxy-closed

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends postgresql-17-pgvector=0.8.6-1.pgdg12+1; \
    test -x /usr/bin/timeout; \
    install -d -m 0555 /usr/local/share/synveda; \
    rm -rf /var/lib/apt/lists/*

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

# psql clears the query buffer when an included file reaches EOF. Build the
# bootstrap-only adapter as the exact shared SQL followed by \gset so the
# Boolean is captured in the same include; Rust continues to compile and run
# the pure SQL source directly.
RUN set -eux; \
    cp /usr/local/share/synveda/extension-fingerprint.sql \
       /usr/local/share/synveda/extension-fingerprint-assert.psql; \
    printf '%s\n' '\gset synveda_extension_' \
        >> /usr/local/share/synveda/extension-fingerprint-assert.psql; \
    chmod 0444 /usr/local/share/synveda/extension-fingerprint-assert.psql

# The retained contributor environment predates the exact-role bootstrap and
# still migrates one owner-operated development database directly. Keep its
# two extension prerequisites in an explicit build target so the reference,
# db-test and release images cannot execute initdb SQL behind the bootstrap.
FROM runtime AS development
ARG HTTP_PROXY
ARG http_proxy
ARG HTTPS_PROXY
ARG https_proxy
ARG NO_PROXY
ARG no_proxy
ARG FTP_PROXY
ARG ftp_proxy
ARG ALL_PROXY
ARG all_proxy
COPY --chmod=0555 deploy/compose/scripts/assert-build-proxy-closed /usr/local/bin/assert-build-proxy-closed
RUN /usr/local/bin/assert-build-proxy-closed
COPY --chmod=0444 deploy/compose/postgres/development-initdb.sql /docker-entrypoint-initdb.d/01-synveda-extensions.sql

# Default final stage: all product/reference callers build this stage unless
# they deliberately select the development target above.
FROM runtime AS reference
ARG HTTP_PROXY
ARG http_proxy
ARG HTTPS_PROXY
ARG https_proxy
ARG NO_PROXY
ARG no_proxy
ARG FTP_PROXY
ARG ftp_proxy
ARG ALL_PROXY
ARG all_proxy
COPY --chmod=0555 deploy/compose/scripts/assert-build-proxy-closed /usr/local/bin/assert-build-proxy-closed
RUN /usr/local/bin/assert-build-proxy-closed
