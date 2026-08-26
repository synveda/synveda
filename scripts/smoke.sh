#!/usr/bin/env bash
# FND-2 smoke test: end-to-end health check of the dev environment
# (deploy/compose). Run via `make smoke` after `make dev-up`.
#
# Every check exercises the service, not just the port: SQL round-trips for
# pgvector and the epoch-3 baseline, an OIDC health probe, a Temporal cluster-health RPC, a
# real embedding, and an OTLP ingest. Retries are generous because on a cold
# cache TEI first downloads BGE-M3 (~2.3 GB).
set -euo pipefail

cd "$(dirname "$0")/.."

compose() { docker compose -f deploy/compose/docker-compose.yml "$@"; }

fail() {
  echo "" >&2
  echo "smoke FAILED: $1" >&2
  exit 1
}

# retry <service> <timeout-seconds> <label> <command...> — run until success
# or timeout; fail immediately if the service's container stops running
# (e.g. OOM-killed) instead of burning the whole timeout.
retry() {
  local service=$1 timeout=$2 label=$3 start elapsed state
  shift 3
  start=$(date +%s)
  until "$@" >/dev/null 2>&1; do
    state=$(compose ps -a --format '{{.State}}' "$service" 2>/dev/null || true)
    if [ "$state" != "running" ]; then
      compose logs --tail 10 "$service" >&2 || true
      fail "$label: service '$service' is ${state:-gone}, not running (last logs above)"
    fi
    elapsed=$(($(date +%s) - start))
    if [ "$elapsed" -ge "$timeout" ]; then
      fail "$label not healthy after ${timeout}s (check: $*)"
    fi
    printf '\r    waiting for %s (%ss/%ss)' "$label" "$elapsed" "$timeout"
    sleep 3
  done
  printf '\r'
}

http_ok() { [ "$(curl -s -o /dev/null -w '%{http_code}' "$1")" = "200" ]; }

psql_synveda() {
  compose exec -T postgres psql -U synveda -d synveda -v ON_ERROR_STOP=1 -qAt "$@"
}

echo "==> postgres: server + epoch-3 extensions"
retry postgres 120 "postgres" compose exec -T postgres pg_isready -U synveda -d synveda
psql_synveda -c "SELECT '    ' || extname || ' ' || extversion FROM pg_extension
                 WHERE extname IN ('vector','btree_gin') ORDER BY extname"
count=$(psql_synveda -c "SELECT count(*) FROM pg_extension WHERE extname IN ('vector','btree_gin')")
[ "$count" = "2" ] || fail "expected extensions vector+btree_gin, found $count of 2"

echo "==> postgres: pgvector distance query"
psql_synveda -c "SELECT '[1,2,3]'::vector <-> '[1,2,4]'::vector" >/dev/null
echo "    vector: OK"

echo "==> rauthy: OIDC provider health"
retry rauthy 120 "rauthy" http_ok http://localhost:8100/auth/v1/health
echo "    rauthy: OK"

echo "==> temporal: cluster health + default namespace"
retry temporal 300 "temporal server" \
  compose exec -T temporal-admin-tools temporal operator cluster health --address temporal:7233
retry temporal 120 "temporal default namespace" \
  compose exec -T temporal-admin-tools temporal operator namespace describe --namespace default --address temporal:7233
echo "    temporal: OK"

echo "==> tei: real embedding (first run downloads BGE-M3, be patient)"
retry tei 1800 "tei model load" http_ok http://localhost:8110/health
curl -sf http://localhost:8110/embed -H 'Content-Type: application/json' \
  -d '{"inputs": "synveda dev environment smoke test"}' |
  node -e '
    let d = "";
    process.stdin.on("data", (c) => (d += c));
    process.stdin.on("end", () => {
      const v = JSON.parse(d);
      if (!Array.isArray(v) || !Array.isArray(v[0]) || v[0].length !== 1024) {
        console.error("unexpected /embed response shape (want 1024-dim dense vector)");
        process.exit(1);
      }
      console.log("    tei: OK (BGE-M3 dense, dim " + v[0].length + ")");
    });'

echo "==> jaeger: UI + OTLP/HTTP ingest"
retry jaeger 120 "jaeger ui" http_ok http://localhost:16686/
otlp_code=$(curl -s -o /dev/null -w '%{http_code}' -X POST http://localhost:4318/v1/traces \
  -H 'Content-Type: application/json' -d '{}')
[ "$otlp_code" = "200" ] || fail "jaeger OTLP/HTTP ingest returned HTTP $otlp_code"
echo "    jaeger: OK"

echo ""
echo "smoke: all services healthy — FND-2 acceptance criterion passes."
