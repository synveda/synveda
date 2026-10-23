# ADR-0143: Derive actionable health from authorized ContextRun evidence

- **Status**: Accepted
- **Date**: 2026-10-14
- **Feature(s)**: Context Inspector and outcome feedback, Context-platform product and trust evaluation
- **Deciders**: Owner-authorized implementation; GPT-6 Astra planning review

## Context

The Inspector already reauthorizes individual retained candidates, selections,
relationships and feedback. Users still have to infer a next action from those
rows. A trace is bounded evidence of planning and an offered response, not an
oracle of every stored source, acknowledged delivery, model use or task success.
Optional retention and independent source authority limit what can be observed.

## Decision

Extend the existing authorized detail response and console with six distinct
health findings: provenance, source freshness, conflicts/applicability, budget
and transformation, outcome coverage, and pending review/cleanup. Each finding
has a closed observation status, observation time, explanation and a supported
inspection or fresh-context action. Exact evidence addresses are copied only
from the already filtered response. Retain at most one example per dimension;
never calculate denied-resource counts or compare raw and visible row counts.

Also expose five pipeline stages. Capture/representation and downstream use
remain unknown without evidence. Eligibility reports only visible exclusions;
retrieval reports observed degraded legs without calling a denial a miss;
assembly reports exact excerpts and budget exclusions without asserting semantic
completeness. Feedback is an authenticated user's assertion, not independently
verified execution. Offered context is distinct from acknowledged delivery and
actual model use. Offline fixture/oracle diagnoses remain separate from scored
production retrieval and cannot supply ranking or assembly inputs.

Findings introduce no writable health aggregate, score, unrestricted endpoint,
model service or telemetry content. Existing workflow actions and PDP decisions
remain authoritative when a user follows an action.

## Options considered

1. **Filtered trace projection (chosen).** Reuses independent authorization and
   supplies reviewable evidence and actions without another store.
2. **Tenant-wide health score.** Hides causality and would leak denied counts.
3. **Infer model success from delivery or feedback.** Unsupported observation.

## Consequences

The API can explain observed limitations and guide correction immediately.
Synchronization, physical cleanup and independently verified outcomes remain
unknown until their separate governed workflows provide admissible evidence.
Optional retention makes some dimensions unknown rather than falsely healthy.

## Compliance notes

Health is computed after current Cedar decisions under forced RLS. Detailed
evidence stays in the authorized response; logs, audit and metrics retain their
existing content-free contracts. The projection adds no SQL or migrations.

### Amendment 1: bounded operational evidence (2026-10-14)

After exact trace authorization, inspect at most 32 visible revisions and 32
associated outcome/evaluation receipts. Configured-source evidence independently
requires SourceRead; conflict evidence requires every exact member to be visible.
Use the latest complete observation's database clock for synchronization, and
keep refresh request/state separate from that clock. Authenticate runner outcome
custody without inferring causal credit or model use. Expose one exact authorized
example and a supported inspection destination per dimension. Reduced/disabled
traces gain no source addresses through enrichment. This adds static checked
queries to existing stores, without another health aggregate or migration.
