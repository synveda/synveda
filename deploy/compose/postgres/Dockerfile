# Dev Postgres for Synveda: PostgreSQL 17 + pgvector + Apache AGE + PGMQ
# (tech plan §1.1 — one database engine for Knowledge, vectors, graph, queues).
#
# pgvector comes from the PGDG apt repo already configured in the base image.
# Apache AGE has no release image for PG17 (only dev snapshots), so it is
# built from its release tag. PGMQ is a pure-SQL extension; its Makefile just
# installs the control + SQL files.
FROM postgres:17-bookworm

ARG AGE_TAG=PG17/v1.7.0-rc0
ARG PGMQ_TAG=v1.10.1

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
        postgresql-17-pgvector \
        build-essential postgresql-server-dev-17 bison flex git ca-certificates; \
    git clone --depth 1 --branch "$AGE_TAG" https://github.com/apache/age.git /tmp/age; \
    make -C /tmp/age -j"$(nproc)" install; \
    git clone --depth 1 --branch "$PGMQ_TAG" https://github.com/pgmq/pgmq.git /tmp/pgmq; \
    make -C /tmp/pgmq/pgmq-extension install; \
    # pgmq's install target ships the control + upgrade scripts but not the
    # base install script; provide it so CREATE EXTENSION pgmq works.
    cp /tmp/pgmq/pgmq-extension/sql/pgmq.sql \
       "$(pg_config --sharedir)/extension/pgmq--${PGMQ_TAG#v}.sql"; \
    apt-get purge -y --auto-remove build-essential postgresql-server-dev-17 bison flex git; \
    rm -rf /tmp/age /tmp/pgmq /var/lib/apt/lists/*

COPY initdb/ /docker-entrypoint-initdb.d/
