---
title: "CPR-1: Implementation baseline & locked decisions"
labels:
  - epic:CPR
  - phase:5
size: M
---

# CPR-1: Implementation baseline & locked decisions

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** M

## Description

The first prompt of a 33-prompt programme that re-cuts Synveda for an
individual and a small team without producing a second product. It writes
down what the repository *is* at the commit the programme starts from,
records the eight decisions the programme may not reopen, and changes no
runtime behaviour.

The artefacts are `docs/implementation/synveda-context-platform.md` — the
programme's running record, and required reading for every prompt after this
one — and ADR-0068.

## Why this exists

A redesign that deletes 38 migrations and re-cuts every noun needs two things
before it starts, and neither of them is code.

The first is an **inventory**. Sixty-five features of accumulated shape is
more than anyone holds in their head, and a hard-cut redesign that has not
written down what it is cutting will discover the parts it forgot one
compile error at a time, in the middle of something else. The inventory is
therefore exhaustive and boring on purpose: every route, every CLI verb,
every table, every RLS policy, every Cedar action, every console screen, and
— the part that turned out to be worth the most — every adapter's *actual*
verification level, which for three of them is lower than the feature list
implies.

The second is a **lock**. Eight decisions that later prompts implement and
may not relitigate, because each of them is the kind of decision that gets
quietly reversed under pressure at prompt 19: one domain model rather than
two; profiles rather than editions; a fresh epoch rather than a migrator;
generic scopes rather than five ranks; sessions as a real aggregate; the
candidate/knowledge boundary as a table boundary rather than a column;
immutable versions; and external formats at the boundary. They are ADR-0068,
and the ADR argues each one against the option that would have been cheaper.

The baseline also states the **MVP checkpoint** — what must be true after
Prompt 20 — so that "are we there yet" is a list somebody can read rather
than a judgement somebody makes.

## What this prompt deliberately does not do

It does not touch a migration, a type, a route, a DTO, a CLI command or a
console component. It adds no test, because it adds no behaviour: the
existing suite is run and its state recorded, which is a measurement rather
than a change. It does not file the other 32 prompts as features — each is
filed by the prompt that runs it, which is this repository's habit and the
only way the backlog stays a record of what was found rather than a forecast.

## Acceptance criteria

- `docs/implementation/synveda-context-platform.md` exists and records: base
  commit SHA; CI status; migration head; the public HTTP route inventory;
  the CLI command inventory; the console route and navigation inventory;
  domain entities and tenant-bound tables; the RLS-protected table
  inventory; the Cedar/PDP entity and action model; the observe, inject and
  recall paths; the hierarchy and role-binding implementation; the record,
  proposal, quarantine, skill and context-pack models; client adapters with
  their **actual** verification level; an explicit deletion map from old
  concepts to target concepts; the ordered programme of Prompts 1–33; and
  the MVP checkpoint after Prompt 20.
- ADR-0068 records the eight locked decisions, each with the options
  considered and the consequence accepted, and names a reversal trigger.
- The complete test suite is run and its result recorded accurately,
  pre-existing failures included — **`make ci` and `make db-test` both pass
  at the base commit, and there are no pre-existing failures**.
- **No product runtime behaviour changes.** The diff touches documentation
  only, and `make ci` is green after it as it was before it.
