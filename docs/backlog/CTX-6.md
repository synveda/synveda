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

ADR-0119 fixes the first checkpoint method as a server-derived typed Session event. It is persisted in the existing immutable event ledger, after the ordinary SessionWrite decision and redaction scan. The Claude compact hook spools a typed boundary locally; the normal delivery path derives the checkpoint. Restart composition reads only the same Session after SessionRead, verifies source hashes and review state, and combines bounded excerpts and current Knowledge under one Synveda allowance. Capture continues to freeze the checkpoint's original eligible source events; the derived event cannot publish Knowledge. Adapter hooks remain public-API clients and retain the atomic spool/replay boundary from ADR-0078.

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

### Current implementation and open gates (2026-09-25)

The source checkout records `session.compaction_boundary` locally at Claude `PreCompact`, derives one `session.checkpoint` per newly admitted boundary, and reuses Session RLS, PDP, scan, audit and event retention. The deterministic method keeps at most 64 recent source events, eight user-authored excerpts per checkpoint, four checkpoints at restart and 16 tail events. It does not call a model or fill unsupported typed summary fields. Checkpoint coverage distinguishes an observed window from a missing or truncated one; host transcript completeness is not claimed. Restart uses current Knowledge through the existing planner, and a missing, deleted or withheld source fails closed for the affected derivative. The real ContextRun retains its checkpoint and tail dependencies for later trace checks.

The focused exact-role gateway tests pass for two compact boundaries in one delayed batch, duplicate replay, a later event tail, concurrent boundary delivery, a missing event and a late arrival. A scoped retention disposal withholds both checkpoint diagnostic expansion and a retained ContextRun block; a direct capture probe freezes the checkpoint's primary user event as a review-only candidate, never active Knowledge. A combined-budget probe includes checkpoint evidence beside required Knowledge when it fits and keeps the required body exact when restart must be omitted. The 122 Claude adapter tests pass with a loopback mock gateway, including repeated local compaction windows. SQLx generation and check, generated OpenAPI/console types, strict Clippy and repository static gates pass. `demos/ctx-6-checkpoint-restart.sh` replays the public API cases. These tests do not establish a live proprietary Claude run or provider task quality.

The exact-role gateway policy matrix now passes for checkpoint creation, diagnostic read, restart use and capture under allow, read-only and write-only Cedar packs. A binding change after checkpoint creation withholds restart text when SessionRead is revoked, denies diagnostics without SessionDiagnostics, and denies new checkpoint/capture work when SessionWrite is revoked. A foreign tenant's checkpoint session answers like an unknown session across timeline, diagnostic, preview, append and capture routes.

The separate synthetic held-out corpus compares four task previews with restart off and on against the same authorised Knowledge snapshot after two compact boundaries. The 23-scenario fresh-database product suite includes the combined CTX-6/CTX-8 gate in both governed optimisation modes and passes: all 12 declared Session facts reappear with exact checkpoint or tail-event attribution, all four required Knowledge bodies remain exact, and every assisted block stays within a 1,400-token local `o200k_base` allowance. The predeclared tolerance is zero fact/provenance loss and a 500 ms assisted p95 local preview ceiling after one warmup per path. One four-sample run measured 28.96 ms p95. This is in-process Synveda text and latency evidence, not a model answer, provider usage, monetary saving or production latency distribution. The exact samples are in `target/product-evaluation/report.json` after `make eval-product`.

The combined gate separately retains the exact required Knowledge body and checkpoint source attribution with room, then omits the optional restart and its source ID when the same exact rendered-text allowance is tightened. The brief stays open for a live proprietary Claude compact/resume run and model-observed task outcome/usage evidence with an approved synthetic fixture and bounded spend. The isolated source Compose hostname handoff and resolver checks passed without resetting the retained interop project, but fresh image construction is still blocked by Cargo index/archive endpoint timeouts recorded in CTX-8 before browser acceptance. The original interop hosts ownership was restored. The next action is to restore source-build egress, complete fresh Compose/browser acceptance, then run a bounded live synthetic Claude compact/resume probe when credentials and spend permission are available.
