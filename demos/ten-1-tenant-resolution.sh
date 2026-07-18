#!/usr/bin/env sh
# TEN-1 acceptance demo: tenant model & resolution.
# AC (docs/backlog/TEN-1.md): request without resolvable tenant -> 401;
# traces carry tenant_id.
#
# Flow: migrate -> admit a tenant (CLI) -> mint a dev token (CLI, ADR-0008)
# -> boot the gateway -> prove the uniform 401, the resolved /v1/whoami via
# the task-local, the resolution metrics, and a Jaeger trace whose request
# span carries tenant.id.
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
# Dev-mode token secret (ADR-0008); demo-only, never reuse outside compose.
SYNVEDA_DEV_JWT_SECRET=ten-1-demo-secret
export SYNVEDA_DEV_JWT_SECRET

cargo build -p synveda-gateway -p synveda-cli

echo "==> migrate + admit a tenant + mint a dev token (synveda CLI)"
./target/debug/synveda db migrate
tenant_json=$(./target/debug/synveda tenant create \
  --slug "ten1-demo-$(date +%s)-$$" --name "TEN-1 Demo Tenant")
tenant_id=$(echo "$tenant_json" | node -e '
  let d = "";
  process.stdin.on("data", (c) => (d += c));
  process.stdin.on("end", () => console.log(JSON.parse(d).id));
')
echo "    tenant: $tenant_id"
token=$(./target/debug/synveda token issue --tenant "$tenant_id" --subject demo-user)

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

echo "==> AC: request without a resolvable tenant -> 401"
code=$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8120/v1/whoami)
if [ "$code" != "401" ]; then
  echo "demo FAILED: no-token request returned HTTP $code, want 401" >&2
  exit 1
fi
code=$(curl -s -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer not-a-real-token" http://127.0.0.1:8120/v1/whoami)
if [ "$code" != "401" ]; then
  echo "demo FAILED: garbage-token request returned HTTP $code, want 401" >&2
  exit 1
fi
unknown=$(./target/debug/synveda token issue \
  --tenant 00000000-0000-7000-8000-000000000000 --subject nobody)
code=$(curl -s -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer $unknown" http://127.0.0.1:8120/v1/whoami)
if [ "$code" != "401" ]; then
  echo "demo FAILED: unknown-tenant request returned HTTP $code, want 401" >&2
  exit 1
fi
echo "    401 without token, with a garbage token, and for an unknown tenant"

echo "==> resolved request: /v1/whoami reports the tenant from the task-local"
curl -fsS -H "Authorization: Bearer $token" http://127.0.0.1:8120/v1/whoami |
  TENANT_ID="$tenant_id" node -e '
    let d = "";
    process.stdin.on("data", (c) => (d += c));
    process.stdin.on("end", () => {
      const body = JSON.parse(d);
      if (body.subject !== "demo-user" || body.tenant.id !== process.env.TENANT_ID) {
        console.error("unexpected whoami body: " + d);
        process.exit(1);
      }
      console.log(`    subject ${body.subject} resolved to tenant ${body.tenant.slug}`);
    });
  '

echo "==> /metrics counts both outcomes"
metrics=$(curl -fsS http://127.0.0.1:8120/metrics)
echo "$metrics" | grep 'synveda_tenant_resolutions_total' | grep -q 'outcome="resolved"' || {
  echo "demo FAILED: resolved outcome missing from /metrics" >&2
  exit 1
}
echo "$metrics" | grep 'synveda_tenant_resolutions_total' | grep -q 'outcome="rejected"' || {
  echo "demo FAILED: rejected outcome missing from /metrics" >&2
  exit 1
}
echo "    synveda_tenant_resolutions_total: resolved + rejected present"

echo "==> AC: the Jaeger trace for GET /v1/whoami carries tenant.id"
TENANT_ID="$tenant_id" node -e '
  const url =
    "http://localhost:16686/api/traces?service=synveda-gateway&limit=20&lookback=1h";
  const want = process.env.TENANT_ID;
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  (async () => {
    for (let attempt = 0; attempt < 30; attempt++) {
      try {
        const body = await (await fetch(url)).json();
        for (const trace of body.data ?? []) {
          const span = trace.spans.find(
            (s) =>
              s.operationName === "GET /v1/whoami" &&
              s.tags.some((t) => t.key === "tenant.id" && t.value === want),
          );
          if (span) {
            console.log(`    span "GET /v1/whoami" carries tenant.id=${want}`);
            console.log(`    view it: http://localhost:16686/trace/${trace.traceID}`);
            process.exit(0);
          }
        }
      } catch {
        // Jaeger may still be ingesting; keep polling.
      }
      await sleep(1000);
    }
    console.error(
      `demo FAILED: no GET /v1/whoami span with tenant.id=${want} in Jaeger`,
    );
    process.exit(1);
  })();
'

echo ""
echo "TEN-1 tenant model & resolution: acceptance criteria pass."
