---
title: "AUD-4: SIEM streaming"
labels:
  - epic:AUD
  - phase:3
size: S
---

# AUD-4: SIEM streaming

**Epic:** AUD — Audit (functional requirement) · **Phase:** 3 · **Size:** S

## Problem and evidence

Gateway traces can use OTLP, but telemetry export is not audit delivery. The
canonical audit chain has no durable CEF/OTLP SIEM stream, delivery cursor,
retry/dead-letter state or lag alarm. Sending best-effort log lines would lose
tenant order and canonical evidence. This P1 gap is paired with AUD-3 in
[production readiness](../PRODUCTION_READINESS.md).

## Scope

- Define one versioned, content-minimised SIEM schema derived from canonical
  AuditAction fields and safe payload keys.
- Deliver tenant sequence, hash, previous hash, actor kind/subject, action,
  outcome, resource, time and approved typed references through a durable
  idempotent cursor/outbox.
- Support the owner-selected CEF and/or OTLP log destination with bounded
  batches, timeouts, retry/backoff and inspectable terminal failure.
- Report per-destination lag and health without tenant or resource identities
  in hot metric labels.
- Document how a SIEM consumer verifies continuity against a frozen export.

## Non-goals

- No conversion of traces or application logs into authoritative audit events.
- No Knowledge/session payload or secret disclosure to simplify SIEM queries.
- No at-most-once best-effort streaming.
- No vendor-specific policy engine, incident automation or silent schema drift.
- No replacement for the database chain or AUD-3 WORM retention.

## Architecture seam

Audit append remains inside the caller's tenant transaction. Delivery reads or
receives the canonical committed sequence through a durable tenant-qualified
state and maps it to a closed schema. Destination adapters own transport only;
they cannot mutate the event, authorisation decision or canonical hash.

## Acceptance criteria

- Events arrive in order with stable schema/version and enough canonical fields
  to detect gap, duplication and alteration.
- Lost acknowledgements, restarts and two workers cause harmless duplicate
  delivery under a documented idempotency key, never skipped sequence.
- Destination outage uses bounded exponential backoff, preserves database
  availability and surfaces lag/dead-letter state.
- Fixtures for the selected Splunk/Sentinel/OTLP target parse all current
  AuditAction variants without raw content.
- Cross-tenant destination credentials and filters cannot disclose another
  tenant's events.
- Release/schema changes fail compatibility checks until SIEM mappings are
  updated.

## Required tests

- Golden mapping for every AuditAction and allowed typed payload field.
- Retry, lost-ack, reorder, malformed response, rate limit and prolonged outage
  tests.
- Multi-tenant sequence/idempotency and credential-isolation tests.
- Live development acceptance for each claimed SIEM target.
- Continuity comparison with AUD-3 frozen export.

## Rollout and rollback

Start with shadow delivery to a non-authoritative sink and measure lag/
duplicates. Enable destinations tenant by tenant after schema approval.
Rollback stops transport while preserving the cursor/outbox for later replay;
it never deletes or marks canonical audit events delivered without an ack.

## Dependencies

Compliance/operations owners choose destination, transport, retention, field
allowlist, latency SLO and dead-letter response. AUD-3 may share delivery
bookkeeping. Provider credentials and live Splunk/Sentinel environments are
external dependencies.
