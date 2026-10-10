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
RUN mkdir -p /usr/share/licenses/synveda \
    && chmod 0555 /usr/share/licenses /usr/share/licenses/synveda
COPY --chmod=0444 LICENSE NOTICE /usr/share/licenses/synveda/
COPY --chmod=0555 deploy/compose/postgres/synveda-logical-backup /usr/local/bin/synveda-logical-backup
COPY --chmod=0555 deploy/compose/postgres/synveda-logical-restore /usr/local/bin/synveda-logical-restore
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

# OPS-5: optional physical-backup image. The reference image below remains
# the default; this MIT-licensed binary is added only to the explicit PITR
# variant so the normal reference does not acquire a backup credential path.
FROM runtime AS pitr
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
    apt-get install -y --no-install-recommends pgbackrest=2.59.1-1.pgdg12+1; \
    test "$(pgbackrest version)" = 'pgBackRest 2.59.1'; \
    test -s /usr/share/doc/pgbackrest/copyright; \
    rm -rf /var/lib/apt/lists/*
COPY --chmod=0555 deploy/compose/postgres/synveda-pgbackrest-entrypoint /usr/local/bin/synveda-pgbackrest-entrypoint

# Genuine native inputs for the isolated compatibility fixtures only.
# The copied library and bitcode use the same PostgreSQL 17/Debian 12 ABI.
FROM pgvector/pgvector:0.8.2-pg17-bookworm@sha256:feb68f4f15446397d8cac7f4fe48fe4586de83160d1fc48b46283312d1a33966 AS vector-082-source
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

FROM runtime AS vector-082-fixture
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
# Remove the reference installation scripts before copying the actual older
# native extension. Package metadata belongs to the reference base; this
# deliberately overlaid test image is never a release/provenance artifact.
RUN rm -f /usr/share/postgresql/17/extension/vector* \
    && rm -rf /usr/lib/postgresql/17/lib/bitcode/vector \
    && rm -f /usr/lib/postgresql/17/lib/bitcode/vector.index.bc
COPY --from=vector-082-source /usr/lib/postgresql/17/lib/vector.so /usr/lib/postgresql/17/lib/vector.so
COPY --from=vector-082-source /usr/lib/postgresql/17/lib/bitcode/vector/ /usr/lib/postgresql/17/lib/bitcode/vector/
COPY --from=vector-082-source /usr/lib/postgresql/17/lib/bitcode/vector.index.bc /usr/lib/postgresql/17/lib/bitcode/vector.index.bc
COPY --from=vector-082-source /usr/share/postgresql/17/extension/vector* /usr/share/postgresql/17/extension/

FROM pgvector/pgvector:0.8.3-pg17-bookworm@sha256:be400b50812ab2cc908ed78593fda2e51e3b45fe774fa637f1c7b16e68531d95 AS vector-083-source
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

FROM runtime AS vector-083-fixture
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
# Remove the reference installation scripts before copying the actual older
# native extension. Package metadata belongs to the reference base; this
# deliberately overlaid test image is never a release/provenance artifact.
RUN rm -f /usr/share/postgresql/17/extension/vector* \
    && rm -rf /usr/lib/postgresql/17/lib/bitcode/vector \
    && rm -f /usr/lib/postgresql/17/lib/bitcode/vector.index.bc
COPY --from=vector-083-source /usr/lib/postgresql/17/lib/vector.so /usr/lib/postgresql/17/lib/vector.so
COPY --from=vector-083-source /usr/lib/postgresql/17/lib/bitcode/vector/ /usr/lib/postgresql/17/lib/bitcode/vector/
COPY --from=vector-083-source /usr/lib/postgresql/17/lib/bitcode/vector.index.bc /usr/lib/postgresql/17/lib/bitcode/vector.index.bc
COPY --from=vector-083-source /usr/share/postgresql/17/extension/vector* /usr/share/postgresql/17/extension/

FROM pgvector/pgvector:0.8.4-pg17-bookworm@sha256:dd467f03ca5c5581222490e5217e48a262864ccb659be559f8491bbafdc97da0 AS vector-084-source
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

FROM runtime AS vector-084-fixture
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
# Remove the reference installation scripts before copying the actual older
# native extension. Package metadata belongs to the reference base; this
# deliberately overlaid test image is never a release/provenance artifact.
RUN rm -f /usr/share/postgresql/17/extension/vector* \
    && rm -rf /usr/lib/postgresql/17/lib/bitcode/vector \
    && rm -f /usr/lib/postgresql/17/lib/bitcode/vector.index.bc
COPY --from=vector-084-source /usr/lib/postgresql/17/lib/vector.so /usr/lib/postgresql/17/lib/vector.so
COPY --from=vector-084-source /usr/lib/postgresql/17/lib/bitcode/vector/ /usr/lib/postgresql/17/lib/bitcode/vector/
COPY --from=vector-084-source /usr/lib/postgresql/17/lib/bitcode/vector.index.bc /usr/lib/postgresql/17/lib/bitcode/vector.index.bc
COPY --from=vector-084-source /usr/share/postgresql/17/extension/vector* /usr/share/postgresql/17/extension/

FROM pgvector/pgvector:0.8.5-pg17-bookworm@sha256:d2ef61f42ef767baa5a1475393303cc235bcd92febd9d7014eddb48b41f3bad0 AS vector-085-source
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

FROM runtime AS vector-085-fixture
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
# Remove the reference installation scripts before copying the actual older
# native extension. Package metadata belongs to the reference base; this
# deliberately overlaid test image is never a release/provenance artifact.
RUN rm -f /usr/share/postgresql/17/extension/vector* \
    && rm -rf /usr/lib/postgresql/17/lib/bitcode/vector \
    && rm -f /usr/lib/postgresql/17/lib/bitcode/vector.index.bc
COPY --from=vector-085-source /usr/lib/postgresql/17/lib/vector.so /usr/lib/postgresql/17/lib/vector.so
COPY --from=vector-085-source /usr/lib/postgresql/17/lib/bitcode/vector/ /usr/lib/postgresql/17/lib/bitcode/vector/
COPY --from=vector-085-source /usr/lib/postgresql/17/lib/bitcode/vector.index.bc /usr/lib/postgresql/17/lib/bitcode/vector.index.bc
COPY --from=vector-085-source /usr/share/postgresql/17/extension/vector* /usr/share/postgresql/17/extension/

# Default final stage for development, reference, tests and release builds.
# Database creation and extensions always use the reviewed bootstrap command;
# no image-level initdb hook owns schema or role lifecycle.
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
