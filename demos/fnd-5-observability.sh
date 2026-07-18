#!/usr/bin/env sh
# FND-5 acceptance demo: a single trace visible in Jaeger spanning an
# end-to-end request — gateway→core→store→Postgres — plus the Prometheus
# contract (synveda_tokens_per_inject) on /metrics.
# AC (docs/backlog/FND-5.md): single trace visible in Jaeger spanning an
# end-to-end request.
# On Windows, run via Git Bash. Needs the postgres and jaeger services, not
# the full dev stack.
set -eu

cd "$(dirname "$0")/.."

docker compose -f deploy/compose/docker-compose.yml up --detach --wait postgres jaeger

DATABASE_URL=postgres://synveda:synveda-dev@localhost:5432/synveda
export DATABASE_URL
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317
export OTEL_EXPORTER_OTLP_ENDPOINT
# Export spans quickly instead of on the 5s batch default; demo-only.
OTEL_BSP_SCHEDULE_DELAY=500
export OTEL_BSP_SCHEDULE_DELAY
SYNVEDA_LISTEN_ADDR=127.0.0.1:8120
export SYNVEDA_LISTEN_ADDR

cargo build -p synveda-gateway

./target/debug/synveda-gateway &
GATEWAY_PID=$!
trap 'kill "$GATEWAY_PID" 2>/dev/null || true' EXIT INT TERM

echo "==> waiting for the gateway on $SYNVEDA_LISTEN_ADDR"
tries=0
until curl -fsS http://127.0.0.1:8120/healthz >/dev/null 2>&1; do
  tries=$((tries + 1))
  if [ "$tries" -ge 30 ]; then
    echo "demo FAILED: gateway did not become healthy" >&2
    exit 1
  fi
  sleep 1
done

echo "==> end-to-end request: GET /readyz (gateway -> retrieval -> store -> postgres)"
body=$(curl -fsS http://127.0.0.1:8120/readyz)
if [ "$body" != "ready" ]; then
  echo "demo FAILED: /readyz returned '$body'" >&2
  exit 1
fi
echo "    readyz: OK"

echo "==> /metrics exposes the Prometheus contract"
metrics=$(curl -fsS http://127.0.0.1:8120/metrics)
echo "$metrics" | grep -q '^# TYPE synveda_tokens_per_inject histogram' || {
  echo "demo FAILED: synveda_tokens_per_inject histogram missing from /metrics" >&2
  exit 1
}
echo "$metrics" | grep -q 'synveda_http_requests_total' || {
  echo "demo FAILED: synveda_http_requests_total missing from /metrics" >&2
  exit 1
}
echo "    metrics: OK (tokens_per_inject registered before any inject exists)"

echo "==> querying Jaeger for one trace spanning all three layers"
node -e '
  const url =
    "http://localhost:16686/api/traces?service=synveda-gateway&limit=20&lookback=1h";
  const want = ["GET /readyz", "retrieval.readiness", "store.ping"];
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  (async () => {
    for (let attempt = 0; attempt < 30; attempt++) {
      try {
        const body = await (await fetch(url)).json();
        const hit = (body.data ?? []).find((trace) =>
          want.every((op) =>
            trace.spans.some((span) => span.operationName === op),
          ),
        );
        if (hit) {
          console.log(
            `    single trace ${hit.traceID} contains: ${want.join(" + ")}`,
          );
          console.log(`    view it: http://localhost:16686/trace/${hit.traceID}`);
          process.exit(0);
        }
      } catch {
        // Jaeger may still be ingesting; keep polling.
      }
      await sleep(1000);
    }
    console.error(
      `demo FAILED: no single Jaeger trace contains all of: ${want.join(", ")}`,
    );
    process.exit(1);
  })();
'

echo ""
echo "FND-5 observability baseline: acceptance criterion passes."
