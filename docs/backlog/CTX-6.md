# CTX-6: Session compression assist

## Problem and evidence

The Claude adapter's `PreCompact` hook durably spools the remaining transcript and returns, while delivery occurs at `SessionEnd` or the next `SessionStart` under ADR-0078. It does not create a typed checkpoint or summary for the compacted session. Consequently a restarted client can use recorded Knowledge and new events, but Synveda cannot provide bounded, provenance-preserving evidence of the compacted working context or test whether critical facts survived compression.

## Scope

- Define an immutable session checkpoint tied to one session, an exact event-sequence range, source digests, creation time, summarizer/model identity when used, and sensitivity/provenance metadata.
- Keep `PreCompact` local-durable and fast; upload and optional summarisation happen after the spool acknowledgement through the session delivery path.
- Build restart context from the latest authorized checkpoint, a bounded recent-event window, and current authorized Knowledge, with explicit source attribution and budget accounting.
- Permit checkpoint evidence to seed a capture candidate, but require normal VedaFlow review before any fact becomes active Knowledge.
- Apply boundary validation, redaction/scanning, tenant RLS, per-resource PDP decisions, content-free audit, bounded metrics, idempotency, and retention.

## Non-goals

- Reintroducing observe events, Records, implicit memory publication, or a second adapter runtime plane.
- Letting a model-authored summary overwrite the original session evidence or masquerade as verified Knowledge.
- Blocking the host compaction hook on a model or network call.
- Solving host cache invalidation or guaranteeing recovery of transcript bytes the host never exposed.

## Architecture seam

Add a typed checkpoint contract beside the Session aggregate, persisted by `synveda-store` and reached through session-scoped gateway application services. Capture consumes the checkpoint as immutable source evidence; the context planner may select it as a separately labelled source after PDP authorization. Adapter hooks remain public-API clients and retain the atomic spool/replay boundary from ADR-0078.

## Acceptance criteria

- Duplicate or reordered spool delivery creates at most one checkpoint for the exact source range and never loses an acknowledged local payload.
- A compact/restart scenario reconstructs a bounded context containing the expected probe facts with exact checkpoint/event/Knowledge attribution.
- Revoked or denied checkpoint and Knowledge sources are absent from the served block without leaking their existence; RLS and audit evidence remain intact.
- Capture from a checkpoint produces a candidate, not active Knowledge, and acceptance uses a typed VedaFlow Knowledge command.
- Summary failure leaves original session evidence usable and produces a causal, non-secret status rather than blocking compaction.

## Required tests

- Unit tests for checkpoint canonicalization, bounds, digests, attribution, and budget accounting.
- Database tests for forced RLS, immutability, idempotency, ordering, retention, and concurrent replay.
- Gateway policy matrix for checkpoint create/read/use/capture with allow, deny, revoke, and cross-tenant cases.
- Authentic Claude compact/restart frame replay plus a live-client run when the proprietary client is available.
- Probe-based evaluation comparing restart facts, provenance, and token budget with and without the assist.

## Rollout and rollback

Ship storage and ingestion dark, then enable checkpoint planning per adapter/profile after probe and privacy review. Rollback disables checkpoint creation and selection while leaving stored checkpoints inert for their governed retention period; the existing event-spool and Knowledge paths continue unchanged.

## Dependencies

An accepted ADR must fix checkpoint identity, summary provenance, planner priority, and capture interaction. The owner must approve retention, sensitivity/redaction rules, summarizer policy and budget, recent-window bounds, and the probe corpus. Authentic hook evidence depends on access to a supported Claude client and credential.
