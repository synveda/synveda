# SKIL-5: Authentic Skill usage reporting

## Problem and evidence

ADR-0085 and the current API already provide immutable `skill_usage_events`, exact Skill version/binding/session identity, closed usage stages, `host_observed` versus `model_reported` evidence, idempotent record/list operations, audit, and per-version console evidence. The Claude support registry states that the host exposes sync/advertisement but no trustworthy activation callback. No supported client currently demonstrates end-to-end trusted activation/execution/outcome emission, and there is no scope/time-bucket dashboard; raw event presence must not be presented as success.

## Scope

- Instrument only supported host seams that can truthfully emit advertised, discovered, activated, instructions/resource loaded, script requested/executed, and explicit outcome events for an exact Skill version and binding.
- Preserve evidence class and source client/version; accept model reports as labelled claims, never as host-observed activation or execution.
- Add bounded scope, version, stage, evidence, client, and time-bucket aggregates derived from immutable events, with unique Sessions and explicit outcome counts.
- Add a console view that separates exposure, activation, execution, reported outcome, unknown outcome, and evidence quality; link back to paged raw events.
- Feed only declared, sufficiently covered evidence into VedaFlow review or evaluation reports; never auto-promote a Skill.

## Non-goals

- Guessing Skill use from prompt text, tool calls, generated prose, or advertisement alone.
- Logging instructions, resources, script contents, user content, arbitrary metadata dimensions, or identities in metric labels.
- Treating missing outcome as success, merging `model_reported` with `host_observed`, or ranking across tenants.
- Making the gateway execute Skill bundle code or changing Skill resolution/binding authority.

## Architecture seam

Clients append through the existing `/v1/skill-usage` contract; the gateway validates exact Session/binding/version identity and applies PDP/RLS/audit. Aggregation is a bounded read model over immutable events in `synveda-store`, exposed through generated public API operations and the existing Skill version console area. Prometheus retains only low-cardinality operational counters.

## Acceptance criteria

- At least one named supported client emits every lifecycle stage that its real host contract can observe, with authentic version-pinned evidence and honest `not_applicable` gaps.
- Duplicate/reordered client delivery is idempotent and cannot attach usage to the wrong Session, binding, Skill version, tenant, or evidence class.
- Scope/time reports reconcile exactly to authorized raw events and distinguish unique Sessions, events, evidence source, outcomes, unknowns, and incomplete coverage.
- Revoked/denied Skill evidence is not disclosed; historical aggregate evidence remains correctly labelled without exposing content or resource existence.
- The dashboard never labels advertisement/activation as execution or an absent/model-reported outcome as host-confirmed success.

## Required tests

- Authentic client-frame replay and live-client run for each claimed observable stage.
- Existing event validation/idempotency tests plus wrong-version, wrong-binding, cross-session, revoke, and cross-tenant cases.
- Aggregate reconciliation, pagination, time-boundary, unique-session, missing/late outcome, and evidence-separation tests.
- Console accessibility and reader-visible truthfulness tests for partial/no evidence.
- Bounded-cardinality metrics and content/secret-redaction tests.

## Rollout and rollback

Enable client emission per adapter/version with aggregates hidden until coverage is measured. Mark partial host contracts explicitly and promote dashboards only after reconciliation. Rollback disables the emitter/aggregate view while retaining immutable raw evidence; do not relabel historical model reports as host observations.

## Dependencies

The owner must select a client with trustworthy lifecycle callbacks and define outcome vocabulary, aggregation windows, minimum coverage, retention, privacy, and the evidence allowed in promotion/evaluation. Proprietary live-client access may remain external; unsupported callbacks must remain `not_applicable` rather than be simulated.
