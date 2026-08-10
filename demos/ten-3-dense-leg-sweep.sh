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
# **Hence REPEATS, and hence a repeat re-seeds.** ADR-0063's first table
# is n=1 in every row and says so in its own last paragraph: "repeats, and
# a max_scan_tuples axis, come before the gate in decision 3 is applied to
# anything." The variance lives in the graph, so repeating only the
# queries would measure the query generator instead — a repeat throws the
# database away like an arm does.
#
# Usage: demos/ten-3-dense-leg-sweep.sh [records] [tenants]
#   REPEATS  runs per arm (3)
#   RUNS     directory for the per-run JSON reports (/tmp/ten3-runs)
#   OUT      the live transcript (/tmp/ten3-sweep.txt)
#
# Cost: one run is a seed plus 200 measured queries plus 200 exact ones.
# The whole grid below is 8 arms × REPEATS, which is a thing somebody
# schedules rather than a thing somebody waits for — the reports land as
# they finish, so an interrupted sweep keeps everything already measured.
set -euo pipefail
cd "$(dirname "$0")/.."

RECORDS=${1:-64000}
TENANTS=${2:-8}
SCOPES=${SCOPES:-16}
QUERIES=${QUERIES:-100}
REPEATS=${REPEATS:-3}
PG=${PG_CONTAINER:-synveda-postgres-1}
DB=${DB:-ten3}
OUT=${OUT:-/tmp/ten3-sweep.txt}
RUNS=${RUNS:-/tmp/ten3-runs}

: >"$OUT"
mkdir -p "$RUNS"

arm() {
  local mode=$1
  local ef=$2
  local bound=$3
  local plan=${4:-auto}
  local run
  for run in $(seq 1 "$REPEATS"); do
    docker exec "$PG" psql -U synveda -d postgres \
      -c "drop database if exists $DB;" -c "create database $DB;" >/dev/null
    docker exec "$PG" psql -U synveda -d "$DB" \
      -c "create extension if not exists vector; create extension if not exists pgmq;" >/dev/null
    echo "### iterative=$mode ef_search=$ef max_scan_tuples=$bound plan_cache=$plan" \
      "  (run $run/$REPEATS)" | tee -a "$OUT"
    SQLX_OFFLINE=true \
      DATABASE_URL="postgres://synveda:synveda-dev@localhost:5432/$DB" \
      SYNVEDA_BENCH_RECORDS="$RECORDS" SYNVEDA_BENCH_TENANTS="$TENANTS" \
      SYNVEDA_BENCH_SCOPES="$SCOPES" SYNVEDA_BENCH_QUERIES="$QUERIES" \
      SYNVEDA_BENCH_ITERATIVE="$mode" SYNVEDA_BENCH_EF_SEARCH="$ef" \
      SYNVEDA_BENCH_MAX_SCAN_TUPLES="$bound" SYNVEDA_BENCH_PLAN_CACHE="$plan" \
      SYNVEDA_BENCH_REPORT="$RUNS/$mode-ef$ef-mst$bound-$plan-$run.json" \
      cargo test -p synveda-store --test ann_bench -- --ignored --nocapture 2>&1 |
      grep -E "^  (broad|selective)" | tee -a "$OUT"
  done
}

# The bound axis. `default` is pgvector's 20,000 and is what every
# deployment of this product has ever run; the raised value is twice the
# corpus rather than a round number, because the HNSW index holds *every*
# tenant's vectors and a bound above the whole index is the only value
# that cannot stop a scan early. That is the arm measurement 4 needs: a
# scan that was faster *and* worse at ef_search 1000 either stopped
# against this bound or did not, and one number settles it.
UNBOUNDED=$((RECORDS * 2))

# The grid is in two halves, and the split is the finding that forced it.
#
# Under `plan_cache_mode = auto` — what the product runs — PostgreSQL
# plans against real parameters for five executions and may switch to a
# generic plan on the sixth, and at this corpus shape the generic plan
# drops `record_embeddings_hnsw_1024` and scans the tenant's whole allowed
# slice exactly. So under `auto` the HNSW GUCs govern only the first few
# executions of each pooled connection, and sweeping ef_search there
# measures the pool, not the index.
#
# Half one therefore holds the tuning still and moves only the plan mode:
# three arms that price what the product does today against what it would
# do if it kept its own index.
ARMS=(
  "relaxed_order 100 default auto"               # arm A: exactly what ships
  "relaxed_order 100 default force_custom_plan"  # ...the same, keeping HNSW
  "relaxed_order 100 default force_generic_plan" # ...the steady state a warm pool reaches

  # Half two is arm B proper, and it runs under force_custom_plan because
  # that is the only mode where an HNSW knob is the thing being varied.
  "off           100 default  force_custom_plan"   # what ADR-0024 decision 5 bought
  "relaxed_order 400 default  force_custom_plan"
  "relaxed_order 1000 default force_custom_plan"   # the row that ran backwards
  "strict_order  100 default  force_custom_plan"   # decision 2 names it; never measured
  "relaxed_order 100 $UNBOUNDED force_custom_plan"
  "relaxed_order 400 $UNBOUNDED force_custom_plan"
  "relaxed_order 1000 $UNBOUNDED force_custom_plan"
)

# The manifest, written **before** the first arm runs and not as they go.
#
# It is what makes an interrupted sweep detectable. Publishing enforces a
# minimum run count per arm, and that alone cannot see a sweep that stopped
# half way: arms run to completion one at a time, so a directory caught mid
# sweep holds nothing but *complete* arms, and the publisher cheerfully
# published six of ten as though they were the sweep. A declaration made up
# front is the only version of this file that can be short of what actually
# ran — one written as arms finish would agree with any prefix, which is
# the property that made the guard useless.
#
# Boring string assembly rather than a JSON tool: every value here comes
# from the literal array above and is alphanumeric, so there is nothing to
# quote and nothing to escape.
{
  echo '{'
  echo '  "benchmark": "ten3-dense-leg",'
  echo "  \"repeats\": $REPEATS,"
  echo '  "arms": ['
  for i in "${!ARMS[@]}"; do
    read -r a_mode a_ef a_bound a_plan <<<"${ARMS[$i]}"
    if [ "$a_bound" = default ]; then a_bound_json=null; else a_bound_json=$a_bound; fi
    if [ "$i" -eq $((${#ARMS[@]} - 1)) ]; then a_comma=; else a_comma=,; fi
    echo "    {\"iterative_scan\": \"$a_mode\", \"ef_search\": $a_ef," \
      "\"max_scan_tuples\": $a_bound_json, \"plan_cache_mode\": \"$a_plan\"}$a_comma"
  done
  echo '  ]'
  echo '}'
} >"$RUNS/sweep.json"

for spec in "${ARMS[@]}"; do
  # Deliberately unquoted: each entry is four whitespace-separated fields.
  # shellcheck disable=SC2086
  arm $spec
done

echo | tee -a "$OUT"
node scripts/summarise-ann-bench.mjs "$RUNS" | tee -a "$OUT"
echo | tee -a "$OUT"
echo "transcript in $OUT, per-run reports in $RUNS" | tee -a "$OUT"
