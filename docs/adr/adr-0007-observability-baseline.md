# ADR-0007: Observability via the tracing facade, with OTel export and metrics owned by the gateway binary

- **Status**: Accepted
- **Date**: 2026-07-18
- **Feature(s)**: FND-5
- **Deciders**: sujitn

## Context

FND-5 requires OTel tracing through gateway→core→store and Prometheus
metrics including `tokens_per_inject`, with a single trace visible in
Jaeger spanning an end-to-end request. The tech plan (§1.3) fixes the
observability stack class — OpenTelemetry traces, Prometheus metrics —
but not how instrumentation is layered across crates, and that layering
interacts with two standing rules:

- The crate dependency rule (seed §8): nothing imports upward, and lower
  crates should stay lean — `synveda-types` has zero internal deps, and
  `synveda-store` today depends only on sqlx + serde + chrono + uuid.
- The PDP invariant (seed §2.2): no code path from harness to storage
  bypasses the PDP. In Phase 0 there is no PDP and no `inject`/`observe`/
  `recall` — those are Phase 1 features — so the "end-to-end request"
  that proves the AC must not be a prototype of a data-path endpoint
  that would later have to grow policy checks.

## Decision

1. **Instrumentation and export are separated.** Every crate that has
   something to trace instruments with the `tracing` facade only
   (`#[tracing::instrument]`, `tracing::info!`); the OpenTelemetry SDK,
   the OTLP exporter, and the subscriber wiring live **only in
   `synveda-gateway`** — the one binary that speaks to the outside.
   Lower crates never depend on any `opentelemetry-*` crate. Spans nest
   in-process through the subscriber, so a gateway request produces one
   trace spanning gateway→retrieval→store with no context-propagation
   machinery inside the workspace.
2. **Export is OTLP/gRPC** to `OTEL_EXPORTER_OTLP_ENDPOINT` (default
   `http://localhost:4317` — the Jaeger service already in the dev
   compose). Jaeger is a dev-profile consumer, not a dependency: any
   OTLP collector works.
3. **Metrics use the `metrics` facade** with
   `metrics-exporter-prometheus` installed by the gateway and rendered
   at `GET /metrics`. The `synveda_tokens_per_inject` histogram is
   registered (with description and unit) at gateway startup so the
   contract exists from day one; the composition engine (CTX-2/CTX-3)
   records into it through the facade when it lands. Baseline HTTP
   request count/duration metrics come from gateway middleware.
4. **The traced end-to-end request is an ops-plane readiness probe.**
   `GET /readyz` calls `synveda_retrieval::readiness`, which calls
   `synveda_store::ping` (a compile-checked `SELECT 1`). It traverses
   the real crate layering but reads no memory content, so it does not
   create — or normalise — a PDP-free data path. Phase 1 endpoints
   inherit the tracing pattern and add AuthN→PDP→audit in front.

## Options considered

1. **`tracing` facade + gateway-owned OTel (chosen)** — standard Rust
   layering; lower crates stay lean; exporter choice and sampling are a
   deployment concern of the one binary. Con: the OTel view of a span is
   only as rich as `tracing` metadata allows; acceptable at our scale.
2. **OpenTelemetry API used directly in every crate** — richer OTel
   semantics everywhere. Rejected: spreads a heavy, fast-moving
   dependency (0.x with breaking releases several times a year) into
   every crate, and couples storage code to an export SDK.
3. **Jaeger-native exporter** — rejected: deprecated upstream; OTLP is
   the only supported ingest going forward, and Jaeger 2.x speaks it
   natively.
4. **`prometheus` crate with hand-registered collectors** — rejected:
   registry plumbing would have to thread through every crate that
   emits a metric, exactly the coupling option 1 avoids; the `metrics`
   facade keeps emit sites dependency-free the same way `tracing` does.
5. **OTel metrics pipeline with a Prometheus exporter** — one SDK for
   both signals. Rejected for now: the OTel metrics/Prometheus bridge
   still churns; the `metrics` facade is stable and boring. Reversible
   later behind the same emit sites only if the facade becomes a limit.

W3C `traceparent` extraction from incoming requests is deliberately
deferred to Phase 1 (ADPT-1/CTX-3), when external callers exist; the
baseline emits new root traces per request.

## Consequences

- Positive: `cargo tree` for store/retrieval gains only the `tracing`
  facade (~no transitive weight); OTel version bumps touch one crate;
  every Phase 1 path gets tracing by writing `#[instrument]` and
  nothing else; the tokens-per-inject SLO metric (research digest A1)
  has a stable name and registration point before any inject exists.
- Negative / accepted trade-offs: the gateway carries the full OTel +
  axum + metrics dependency tree (it was always going to as the only
  binary); span attributes are limited to what `tracing` fields
  express; a second binary (e.g. a Temporal worker) will need the same
  init block — lift it into a shared crate only when that second binary
  actually appears.
- Reversal trigger: if the `tracing`→OTel bridge loses span fidelity we
  need (links, span events with structured bodies), revisit option 2
  for the affected crate only.

## Compliance notes

Traces are plumbing for the audit story, not a substitute for it:
AUD-1's hash-chained events remain the tamper-evident record. Span
fields on the readiness path carry no tenant data. When Phase 1 lands,
trace IDs should be recorded in audit events (cross-reference, one
direction only) so an auditor can walk from an audit row to the
operational trace — noted for AUD-1.
