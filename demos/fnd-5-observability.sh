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
# `sqlx::query!` expands against DATABASE_URL at compile time, and the
# database named above can still be empty at this point: a crate that needs
# a rebuild here type-checks against a schema that does not exist yet and
# fails with `relation "audit_chain_heads" does not exist` rather than with
# anything about this demo. It is invisible whenever the workspace happens
# to be built already. The checked-in `.sqlx` cache is the answer to
# "compile without a database", and it is what `make ci` and
# scripts/db-test.sh use for the same reason.
SQLX_OFFLINE=true
export SQLX_OFFLINE
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

# ADR-0007's deferred clause, landed 2026-08-05: a caller that arrives with
# a W3C `traceparent` continues *its* trace rather than starting a new one
# here. ADPT-1's hooks have been sending this header since they shipped and
# nothing read it, so until now every trace began at the gateway.
echo "==> a caller's own trace, continued across the boundary"
CALLER_TRACE=$(od -An -tx1 -N16 /dev/urandom | tr -d ' \n')
CALLER_SPAN=$(od -An -tx1 -N8 /dev/urandom | tr -d ' \n')
curl -fsS -H "traceparent: 00-$CALLER_TRACE-$CALLER_SPAN-01" \
  http://127.0.0.1:8120/readyz >/dev/null
echo "    sent traceparent: 00-$CALLER_TRACE-$CALLER_SPAN-01"

echo "==> querying Jaeger for one trace spanning all three layers"
# No apostrophes below: this is a single-quoted shell string, and one would
# close it.
node -e '
  const caller = process.argv[1];
  const url =
    "http://localhost:16686/api/traces?service=synveda-gateway&limit=20&lookback=1h";
  const want = ["GET /readyz", "retrieval.readiness", "store.ping"];
  const spans = (trace) => trace.spans.map((span) => span.operationName);
  const complete = (trace) => want.every((op) => spans(trace).includes(op));
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  (async () => {
    for (let attempt = 0; attempt < 30; attempt++) {
      try {
        const body = await (await fetch(url)).json();
        const traces = body.data ?? [];
        // The baseline: a request that arrived with no caller context still
        // roots one trace across all three layers.
        const hit = traces.find((t) => t.traceID !== caller && complete(t));
        // And ADR-0007 deferred clause: the request that DID arrive with a
        // traceparent is in the trace the caller chose, not one of ours.
        // Both are polled together because Jaeger ingests them separately,
        // and asserting the second inside the iteration that found the
        // first is a race that fails about half the time.
        const joined = traces.find((t) => t.traceID === caller);
        if (hit && joined && complete(joined)) {
          console.log(
            `    single trace ${hit.traceID} contains: ${want.join(" + ")}`,
          );
          console.log(`    view it: http://localhost:16686/trace/${hit.traceID}`);
          console.log(
            `    and the caller-rooted trace ${caller} holds the same three layers`,
          );
          console.log(`    view it: http://localhost:16686/trace/${caller}`);
          console.log(
            "    the id came from the client, so a slow session start is one",
          );
          console.log(
            "    trace from hook through plan, embed, search and compose",
          );
          process.exit(0);
        }
      } catch {
        // Jaeger may still be ingesting; keep polling.
      }
      await sleep(1000);
    }
    console.error(
      `demo FAILED: wanted one gateway-rooted trace with ${want.join(", ")} and the ` +
        `caller-rooted trace ${caller} holding the same, within 30s`,
    );
    process.exit(1);
  })();
' "$CALLER_TRACE"

echo ""
echo "FND-5 observability baseline: acceptance criterion passes."
