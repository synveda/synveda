# MEM-10: Governed scheduled consolidation

## Problem and evidence

At `8a22b0e5` Synveda keeps Knowledge consistent at write time and at read
time, but nothing revisits the existing corpus on a schedule.

- **Write time.** ADR-0096's deterministic classifier compares a new capture
  or Knowledge write with content hashes and a bounded lexical candidate set,
  and records duplicates, contradictions, supersessions and transitions as
  conflict sets. Unresolved writes become `transitional` until a reviewer
  resolves them through a VedaFlow command.
- **Read time.** An item past its freshness interval is excluded from context
  and appears in the staleness queue; verification appends a revision.
- **Outcome evidence.** ADR-0151 turns authenticated task outcomes into small,
  reviewable Procedure improvements.

The classifier sees only the item being written and only lexical matches.
Two items written months apart that say the same thing in different words,
a later item that quietly contradicts an earlier one, or several episodes that
together establish a fact are never compared again. A repository search for a
consolidation pass or job finds none.

Offline consolidation is now common. The external facts below come from a
survey checked on 2026-11-01. Six offerings consolidate memory in the
background: Anthropic Dreams, Mem0, Letta, Honcho, Mastra and AWS AgentCore's
episodic reflections. Anthropic Dreams, a research preview since 2026-05-06,
reads a memory store and up to 100 session transcripts and writes a new store;
the input is untouched, and a person reviews the output before attaching or
discarding it ([Dreams](https://platform.claude.com/docs/en/managed-agents/dreams)).
Letta's background process has no approval step, and Google's Memory Bank
deletes a contradicted memory. Of the vendors surveyed, only Supermemory,
Cognee and Anthropic Dreams document human review of memories.

## Scope

1. **ADR.** Decide the cadence and bounds, which proposal kinds are in the
   first release, whether a model may be used to find candidates or draft a
   merged item, and whether any kind may auto-apply under governed
   Configuration.
2. A bounded scheduled pass, per tenant and scope, that finds:
   - near-duplicates the write-time classifier missed;
   - contradictions and supersessions between existing active items;
   - groups of episodes that support a proposed fact or procedure.
3. Each finding becomes an existing reviewable object: a conflict set, a merge
   or supersession proposal, or a Capture candidate, with the evidence it was
   drawn from.
4. Console and API views that show what the pass proposed, what reviewers
   accepted and what they rejected.

## Non-goals

- Changing, merging or deleting Knowledge without a VedaFlow command.
- Comparing items across tenants, or across scopes in a way that would expose
  an item to someone who cannot read it or lower its sensitivity.
- Running reconciliation on the read path; ADR-0096 rejected that.
- A second job system.

## Architecture seam

The pass runs in the core worker beside its existing sweeps, as ordinary
tenant transactions under a service principal that Cedar authorizes for
proposal only. Candidate finding is pure and bounded (items, pairs, time and
any model tokens per run). Outputs use the existing conflict-set, Knowledge
proposal and Capture candidate types, so review, audit and auto-apply rules
need no new path. Any model use goes through the governed external-provider
Configuration.

## Acceptance criteria

- The ADR is accepted before implementation.
- On a seeded corpus with known paraphrased duplicates, silent contradictions
  and supporting episodes, the pass proposes them within stated precision and
  recall floors and proposes nothing across a scope boundary.
- Every proposal cites its exact source revisions and events, and nothing
  reaches context until a reviewer, or an explicitly configured auto-apply
  rule, accepts it.
- A run stops at its bounds, resumes without duplicate proposals and leaves
  content-free audit.

## Required tests

- Pure candidate-finding tests with bounded inputs.
- Database tests for scope and tenant confinement, idempotent re-runs and
  interrupted runs.
- Policy tests showing the pass's principal can propose but not apply.
- An evaluation fixture with a reviewed answer key, run in the product
  evaluation suite.

## Rollout and rollback

Ship off by default and run as a dry report first, then as proposals for an
opted-in scope. Rollback disables the schedule; open proposals stay
reviewable and can be rejected in bulk.

## Dependencies

The owner chooses cadence, proposal kinds, model use and any auto-apply.
Model-assisted finding needs an approved provider and budget, which the
[request limits and tenant usage budgets](OPS-14.md) work should account for.
