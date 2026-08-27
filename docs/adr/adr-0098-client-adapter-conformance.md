# ADR-0098: Client support is generated from criterion-level evidence

- **Status**: Accepted
- **Date**: 2026-08-25
- **Feature(s)**: CPR-39
- **Deciders**: Autonomous continuation of the context-platform programme

## Context

The repository had three incompatible notions of a supported client. The CLI
contained a JSONC table of files it could edit; onboarding hand-wrote four
friendly client cards; and protocol fixtures proved only selected MCP frames.
None could say whether a named client had created a session, delivered events,
received context, recovered an outage and ended the same run. Cursor was named
in product goals despite no authentic Cursor frame; conversely genuine CPR-14
Claude Code evidence was a long feature record rather than machine-readable
support data.

Current official contracts also invalidate the old static assumption. Cursor
Hooks v1 now exposes a complete local-IDE lifecycle, while VS Code 1.133's
Preview hooks expose Stop but no SessionEnd and explicitly distinguish Stop
from session inactivity. A documented contract is useful targeting evidence,
but neither it nor a generated MCP config is a client run.

## Decision

1. **One strict registry owns support claims.** `adapters/registry.json`
   records the five closed levels, tested versions, connection generator,
   lifecycle source/events, limitations, authentic content-addressed fixtures
   and ten criterion results. The CLI projects only MCP configuration; console
   onboarding and `docs/CLIENT_SUPPORT.md` are generated projections. User
   registry extensions remain an external-adapter escape hatch and confer no
   product support level.

2. **Levels describe distinct evidence.** `configured` is a documented recipe;
   `captured` is authentic digest-pinned protocol traffic; `verified` is a
   named real version completing every applicable lifecycle criterion with
   persisted/audited outcomes; `experimental` has a plausible contract with
   missing evidence; `unsupported` lacks a required contract. The CI gate
   refuses missing criteria, non-live verified claims, missing evidence paths,
   forged captured labels and fixture digest drift.

3. **Fixtures cannot self-promote.** Captured frames remain deterministic
   regression evidence, not live execution. The authored MCP `--writes host`
   case is renamed to a vendor-neutral repository contract. Only the separately
   run real-client harness may establish `verified`; its version and instant
   must be recorded.

4. **Cursor remains experimental until driven.** Its current local Hooks v1
   contract appears sufficient, but this environment has no Cursor executable,
   authenticated account or authentic capture. VS Code cannot be promoted as a
   fallback on the installed bits alone because its documented Preview seam
   lacks an end boundary. These are external evidence blockers, not a reason to
   invent support or stop independent platform work.

5. **Adapters use public APIs and evidence is capability-shaped.** A registry
   entry grants no Cedar authority and a declared Tool is metadata. The
   conformance harness observes only the same public session, context,
   Knowledge, capture, Skill and Tool-binding APIs available to any guest.

## Options considered

1. **One generated evidence registry (chosen).** Support labels, config and UI
   cannot silently diverge, while third-party clients stay guests.
2. **Treat every working MCP config as supported.** Rejected: it proves process
   launch, not capture, close, recovery or cross-session reuse.
3. **Call authentic replay verified.** Rejected: replay proves compatibility
   with recorded bytes and is not execution by the proprietary client.
4. **Promote installed VS Code as the fallback.** Rejected: installed files and
   an unauthenticated profile do not satisfy the lifecycle, and the current
   official contract lacks SessionEnd.

## Consequences

- Positive: the support matrix is reproducible, every claim has a level and
  byte-addressed evidence, and adding a client is data plus a conformance run
  rather than a vendor branch in product code.
- Negative / accepted trade-offs: Cursor remains visibly experimental and the
  programme retains one externally blocked acceptance criterion even though
  its documented contract improved.
- Reversal trigger: a client exposes an authoritative remote conformance result
  that can be independently verified → add a signed/result-verification
  evidence kind without weakening the real-client criterion.

## Compliance notes

The registry contains product metadata only: no credential, prompt, response or
secret. Configuration generation grants no Skill, Tool or Knowledge authority;
all runtime operations remain public-API calls behind the gateway PDP, tenant
RLS, VedaFlow where mutated and the hash-chained audit path.
