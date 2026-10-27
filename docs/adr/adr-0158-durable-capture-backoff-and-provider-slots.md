# ADR-0158: Durable Capture backoff, bounded manual retry and installation-wide provider slots

- **Status**: Accepted. The schedule, classification, slot protocol and manual
  retry are source- and exact-role-tested, and the slot ceiling passed a
  one-node source-image Kind drill; the numeric values below are a
  conservative proposal until the owner adopts them
- **Date**: 2026-10-26
- **Feature(s)**: Capture retry and provider backpressure
- **Deciders**: Owner-authorized implementation

## Context

Capture already fences each claim with a database-time lease and an
incremented attempt counter, and closes a batch after five attempts
(ADR-0083). [ADR-0129](adr-0129-scale-fenced-capture-with-singleton-maintenance.md)
admits one combined worker plus at most two Capture-only workers. Before this
decision a failed nonterminal attempt returned straight to `pending`, and a
claim chose the oldest pending batch with no deadline. Each worker polls every
second and may claim eight batches per tenant per sweep, so a provider outage
could spend all five attempts in seconds, and every process could call the
provider at once. Every failure, including a policy denial or a malformed
model response, was retried. A terminal batch also had no recovery: freezing
the same session snapshot again replays the failed row, because the snapshot
digest is unique per session.

The external provider is the scarce shared resource. An in-process semaphore
bounds one process, not the three the chart permits. PostgreSQL is the only
coordination authority the product runs; a second queue or broker would
create another source of truth for Capture state.

## Decision

1. **Database-time retry schedule on the existing row.** Migration `0016`
   adds `next_attempt_at`, `terminal_reason`, `manual_retries` and
   `manual_retried_at` to `capture_batches`. A pending batch is due when
   `coalesce(next_attempt_at, created_at) <= statement_timestamp()`; a
   pending row without a deadline (including every row from before `0016`) is
   due now. A claim prefers a running row whose lease expired, otherwise the
   earliest-due pending row, both under `FOR UPDATE SKIP LOCKED` and tenant
   RLS. Worker clocks never decide eligibility. Two partial indexes replace
   the old pending index: due pending session rows ordered by eligibility,
   and running rows. Completion and failure keep the exact owner, attempt and
   live-lease fence.

2. **Closed failure classification.** Extractors return a closed
   `ProviderFailure` instead of a free-text dependency message, and the
   worker persists one `CaptureFailureCode`:

   | Code | Cause | Disposition |
   | --- | --- | --- |
   | `provider_timeout`, `provider_unreachable`, `provider_interrupted` | client deadline, refused connection, transport loss | retry |
   | `provider_rate_limited` | HTTP 429 | retry, honouring `Retry-After` |
   | `provider_unavailable` | HTTP 503, 529 | retry, honouring `Retry-After` |
   | `provider_transient` | HTTP 408, 500, 502, 504 | retry |
   | `provider_unauthorized`, `provider_rejected` | 401/403; other statuses and unfollowed redirects | close |
   | `provider_response_invalid`, `provider_misconfigured` | output outside the extraction contract; client set-up | close |
   | `source_unavailable`, `procedure_content_invalid` | withdrawn or changed frozen evidence | close; never manually retried |
   | `configuration_invalid`, `capture_disabled`, `provider_not_allowed`, `principal_unavailable`, `session_*_denied`, `session_*_revoked`, `configuration_revoked`, `candidate_invalid`, `too_many_candidates`, `candidate_limit` | policy, Configuration or validation | close |
   | `lease_expired` | the final attempt of a cycle lost its lease | close |

   Provider bodies, header values and credentials are never retained.
   `Retry-After` is untrusted: delta-seconds or an HTTP-date parsed with
   `httpdate`, admitted only for 429/503/529, ignored when malformed or in the
   past, capped at one hour and then at the policy's maximum delay.

3. **Bounded deterministic backoff.** A retryable failure of attempt *n*
   waits `min(max_delay, base × 4^(n−1)) × (0.5 + j)`, where `j ∈ [0, 1)` is a
   BLAKE3 digest of the batch id and attempt, lengthened (never shortened) by
   an admitted provider hint and capped at `max_delay`. Every worker computes
   the same delay for the same durable attempt. The store adds it to
   `statement_timestamp()`. A failure closes as `attempts_exhausted` when the
   cycle's five attempts are spent and as `retry_age_exhausted` when the next
   eligibility would fall more than `max_age` after the cycle began (its first
   claim, or the manual retry that opened it); a closing classification is
   `permanent`. The causal code stays in `error_code`. Process loss or
   cancellation leaves the claim running until its lease expires; nothing
   releases the fence early.

4. **Installation-wide provider slots.** A deployment-scoped table
   `capture_provider_slots` holds fixed slots `0..limit−1` per provider
   identity. The identity key is a BLAKE3 digest of the extractor method and
   its normalised endpoint, so every model and credential at one endpoint
   shares one pool; the deterministic extractor has none. A worker reserves a
   free or expired slot with `FOR UPDATE SKIP LOCKED` inside the same
   transaction that commits its claim, and only when the attempt will call a
   provider. If none is free the transaction rolls back: no attempt is spent
   and the batch keeps its place. The reservation carries the process's
   opaque lease owner and a fresh fencing token, renews with the batch lease
   in one transaction (a failed slot renewal is a lost lease that cancels the
   call), and is released in the transaction that completes or fails the
   attempt. A crashed holder's slot becomes free when its database-time
   expiry passes. Rows name no tenant, batch or endpoint, so the table needs
   no tenant RLS, like the login ledger. Lowering the limit stops new
   reservations above it at once; existing holders finish or expire.

5. **Bounded manual retry.** `POST /v1/capture-batches/{id}/retry` with an
   `Idempotency-Key` reopens a terminal session batch for one more five-attempt
   cycle. The caller needs `SessionWrite` on the source session, the same
   embedded-Cedar decision that freezes a batch. The gateway refuses a batch
   that is not failed, whose frozen source no longer counts its events or was
   withdrawn, whose effective Configuration version or digest has changed
   since it was frozen, or whose Configuration disables explicit Capture. The
   attempt counter is never reset: `0016` bounds it at
   `5 × (manual_retries + 1)`, the transition trigger admits `failed →
   pending` only with `manual_retries + 1` and unchanged attempts, and
   attempts may never decrease. The update repeats every predicate in
   database time. One audited `capture.batch.retry_requested` event records the
   prior code, terminal reason and fence; prior failure events remain in the
   chain. The next attempt re-decides everything as the batch principal.
   Reopening extraction changes no reviewable state, so it is not a VedaFlow
   change.

6. **Observability with closed labels.** `synveda_capture_batches_total`
   gains `retry_scheduled`, `permanent_failure`, `attempts_exhausted`,
   `retry_age_exhausted` and `provider_saturated` outcomes. New signals:
   `synveda_capture_queue_wait_seconds{attempt}` (eligibility to claim),
   `synveda_capture_retry_cycle_seconds{outcome}` (cycles that needed a retry),
   `synveda_capture_provider_slots_held{method}` and
   `synveda_capture_provider_slot_limit{method}`, and the unlabeled
   `synveda_capture_saturated_wait_seconds`. None carries a tenant, principal,
   batch or endpoint label. The public batch view exposes the schedule,
   terminal reason, retry count and attempt budget.

7. **Provisional values.** Base 30 s, maximum delay 30 min, maximum cycle
   age 2 h, five attempts per cycle, provider concurrency 3 (the chart's one
   combined plus two Capture-only workers), two manual retries within seven
   days of the terminal transition, `SessionWrite` authority. The worker
   reads `SYNVEDA_CAPTURE_RETRY_BASE_SECS`,
   `SYNVEDA_CAPTURE_RETRY_MAX_DELAY_SECS`, `SYNVEDA_CAPTURE_RETRY_MAX_AGE_SECS`
   and `SYNVEDA_CAPTURE_PROVIDER_CONCURRENCY` (1–64) and refuses unordered or
   out-of-bound values; every process sharing a provider identity must use one
   concurrency value, because the ceiling is the largest configured.

## Options considered

1. **In-process semaphore per worker.** Bounds one process; three processes
   would reach three times the intended concurrency. Rejected.
2. **PostgreSQL advisory locks.** Transaction-scoped locks cannot span an
   HTTP call made outside the transaction; session locks pin one pooled
   connection per call, are lost behind a transaction pooler and have no
   database-time bound for a hung holder. Rejected for the lease-shaped
   table the claim already uses.
3. **Reserve capacity after the claim commits.** A saturated provider would
   spend attempts without ever calling it. Rejected.
4. **Reset the attempt counter or freeze a new batch on manual retry.**
   Resetting destroys the fence; the snapshot digest is unique, so a new
   freeze cannot exist. Rejected for bounded retry cycles.
5. **A scheduler, PGMQ or the Apalis leaf as retry authority.** Creates a
   second source of truth for Capture state. Rejected.
6. **Retry every failure.** Spends provider capacity and attempts on
   failures that cannot change. Rejected for the closed classification.

## Consequences

- Positive: an outage spaces five attempts over minutes rather than seconds;
  permanent failures close after one attempt; the provider ceiling holds
  across every worker process; operators can inspect the schedule and reopen
  exhausted work without a database edit.
- Negative: a saturated provider leaves due work waiting, which the queue-wait
  and saturated-wait signals expose. Each provider-bound claim adds two
  statements (ensure and reserve slots), each renewal one, and each terminal
  transition one release. An attempt may still repeat an external call: the
  fence guarantees one durable candidate set, not exactly-once provider
  effects. Installations sharing one provider account are not coordinated.
- Rollback: the schema is forward-only. Rollback is the restore-based path of
  [ADR-0156](adr-0156-quiesced-release-upgrade-and-restore-rollback.md); an
  older binary refuses the `0016` head.
- Reversal trigger: owner-selected values differ, a provider requires
  per-model or per-credential pools, or shared-account capacity across
  installations needs coordination; change the key or values in a new
  decision rather than in place.

## Compliance notes

Capture rows keep forced tenant RLS and the exact worker role. The slot table
is listed with the deployment tables and holds no tenant data. Manual retry
crosses embedded Cedar at the source session, records content-free audit and
cannot reach another tenant's batch. No runtime role gains a privilege beyond
column updates on the new columns and the slot table.
