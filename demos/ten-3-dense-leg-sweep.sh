#!/usr/bin/env bash
# TEN-3 — the arm sweep behind ADR-0063.
#
# Each arm gets a **fresh database**, and that is not tidiness: the corpus
# from a previous run stays in the HNSW graph, and the ratio of a tenant's
# vectors to everyone else's is exactly the quantity under measurement.
# Reusing a database would move the thing being measured between arms.
#
# Record ids are UUIDv7, so every run builds a different graph and recall
# carries run-to-run variance — ±3 points at this corpus size, measured by
# running the shipped default twice (0.847, then 0.878). Two arms are only
# separable well outside that, which is why the ADR's gate is a margin and
# not a decimal comparison.
#
# Usage: demos/ten-3-dense-leg-sweep.sh [records] [tenants]
set -euo pipefail
cd "$(dirname "$0")/.."

RECORDS=${1:-64000}
TENANTS=${2:-8}
SCOPES=${SCOPES:-16}
QUERIES=${QUERIES:-100}
PG=${PG_CONTAINER:-synveda-postgres-1}
DB=${DB:-ten3}
OUT=${OUT:-/tmp/ten3-sweep.txt}

: >"$OUT"

arm() {
  mode=$1
  ef=$2
  docker exec "$PG" psql -U synveda -d postgres \
    -c "drop database if exists $DB;" -c "create database $DB;" >/dev/null
  docker exec "$PG" psql -U synveda -d "$DB" \
    -c "create extension if not exists vector; create extension if not exists pgmq;" >/dev/null
  echo "### iterative=$mode ef_search=$ef" | tee -a "$OUT"
  SQLX_OFFLINE=true \
    DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$DB" \
    SYNVEDA_BENCH_RECORDS="$RECORDS" SYNVEDA_BENCH_TENANTS="$TENANTS" \
    SYNVEDA_BENCH_SCOPES="$SCOPES" SYNVEDA_BENCH_QUERIES="$QUERIES" \
    SYNVEDA_BENCH_ITERATIVE="$mode" SYNVEDA_BENCH_EF_SEARCH="$ef" \
    cargo test -p synveda-store --test ann_bench -- --ignored --nocapture 2>&1 |
    grep -E "^  (broad|selective)" | tee -a "$OUT"
}

# `off` is the pre-0.8 behaviour and the only way to price what ADR-0024
# decision 5 bought; the rest is the ef_search sweep.
arm off 100
arm relaxed_order 100
arm relaxed_order 400
arm relaxed_order 1000

echo
echo "results in $OUT"
