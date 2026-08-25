# Postgres for the Kubernetes deployment (OPS-2; CPR-36, ADR-0095).
#
# CloudNativePG's base image plus the two extensions this product's schema
# needs: pgvector (migration 0015) and PGMQ (migration 0012). It differs
# from deploy/compose/postgres/Dockerfile in one way that matters:
#
#   **No Apache AGE.** Nothing in `crates/*/src` calls `cypher()` — GRPH-1
#   named that as the condition for dropping it and the count is still
#   zero. It is also the one extension here that needs
#   `shared_preload_libraries`, and it has no PG17 release: the dev image
#   builds `PG17/v1.7.0-rc0` from source, which would mean recompiling a
#   release candidate against the operator's base image on every minor
#   upgrade, for an extension with no caller. The dev image keeps it —
#   `crates/synveda-store/tests/graph_spike.rs` is GRPH-4's evidence and
#   `scripts/smoke.sh` runs a cypher() health check — so the two images
#   diverge here deliberately.
#
# Creating the extensions is the chart's job, not this image's: PGMQ is
# created by the Cluster's postInitApplicationSQL (it has to exist before
# migration 0012 runs) and `vector` creates itself in 0015.

ARG CNPG_BASE=ghcr.io/cloudnative-pg/postgresql:17
FROM ${CNPG_BASE}

ARG PGMQ_TAG=v1.10.1

# The base runs as uid 26. apt and `make install` need root; the image has
# to go back, or the operator's own entrypoint fails on a permission it
# expects to have.
USER root

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
        postgresql-17-pgvector \
        build-essential postgresql-server-dev-17 git ca-certificates; \
    git clone --depth 1 --branch "$PGMQ_TAG" https://github.com/pgmq/pgmq.git /tmp/pgmq; \
    make -C /tmp/pgmq/pgmq-extension install; \
    # pgmq's install target ships the control + upgrade scripts but not the
    # base install script; provide it so CREATE EXTENSION pgmq works.
    cp /tmp/pgmq/pgmq-extension/sql/pgmq.sql \
       "$(pg_config --sharedir)/extension/pgmq--${PGMQ_TAG#v}.sql"; \
    apt-get purge -y --auto-remove build-essential postgresql-server-dev-17 git; \
    rm -rf /tmp/pgmq /var/lib/apt/lists/*

USER 26
