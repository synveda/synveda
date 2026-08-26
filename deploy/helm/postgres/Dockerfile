# Postgres for the Kubernetes deployment (OPS-2; CPR-36, ADR-0095).
#
# CloudNativePG's base image plus pgvector, the only non-core extension used
# by the epoch-3 schema. This is the same extension shape as the single-node
# image: bounded graph expansion and capture work leasing use ordinary SQL.

ARG CNPG_BASE=ghcr.io/cloudnative-pg/postgresql:17
FROM ${CNPG_BASE}

# The base runs as uid 26. apt and `make install` need root; the image has
# to go back, or the operator's own entrypoint fails on a permission it
# expects to have.
USER root

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends postgresql-17-pgvector; \
    rm -rf /var/lib/apt/lists/*

USER 26
