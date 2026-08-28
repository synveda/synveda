# Dev Postgres for Synveda: PostgreSQL 17 + pgvector. Knowledge, bounded graph
# adjacency, capture leasing and durable jobs all live in ordinary tenant-bound
# tables; pgvector is the epoch-3 schema's only non-core extension.
FROM postgres:17.11-bookworm@sha256:051f7b7b3abdd564d5d1bd1e8c4b9c1b6e77087d1dd22020ede611c096a272e0

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends postgresql-17-pgvector; \
    rm -rf /var/lib/apt/lists/*

COPY initdb/ /docker-entrypoint-initdb.d/
