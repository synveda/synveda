# Postgres for the Kubernetes deployment (OPS-2; CPR-36, ADR-0095).
#
# CloudNativePG's standard image already carries pgvector, the only non-core
# extension used by the epoch-3 schema. This is the same extension shape as
# the single-node image: bounded graph expansion and capture work leasing use
# ordinary SQL.

ARG CNPG_BASE=ghcr.io/cloudnative-pg/postgresql:17.11-202608310816-standard-bookworm@sha256:e8ffaff9d17011fb71f264d857c3b4c54cb86ed0443a4ecb0298cb96be708d4e
# Compile against Debian 12's glibc to match the pinned CloudNativePG image.
# The final-stage probe below enforces this ABI boundary.
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

FROM ${CNPG_BASE}
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

# The base runs as uid 26. Installing Synveda's read-only support files needs
# root; the image returns to uid 26 before the operator starts it.
USER root

RUN set -eux; \
    test "$(dpkg-query -W -f='${db:Status-Abbrev} ${Version}' postgresql-17-pgvector)" = 'ii  0.8.6-1.pgdg12+1'; \
    test -x /usr/bin/timeout; \
    install -d -m 0555 /usr/local/share/synveda;

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
