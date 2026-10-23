# ADR-0150: Ground task state and handoff in current source authority

- **Status**: Accepted
- **Date**: 2026-10-14
- **Feature(s)**: Session compression assist, Source lineage and lifecycle invalidation, One-command realistic product demo
- **Deciders**: Owner-authorized implementation; GPT-6 Astra max planning

## Context

ADR-0119 checkpoints retain bounded user excerpts, with intentionally empty
structured fields. Its verifier must continue to verify retained valid data.
Codex exposes real compact/start interfaces but checkpoint admission still gates
only Claude. A second Session cannot reuse the receiver-local restart columns.
New copied Knowledge needs exact item-erasure coverage even when its original
source is manual and has no Session or ImportArtifact root.

## Decision

Add a bounded public TaskState derivation command on the existing Session plane.
It accepts closed statement/status classifications and exact source pointers;
the server loads and authorizes sources, copies exact spans from admitted bytes,
and derives provenance classes. User declarations, model/client reports,
reviewed Knowledge and imported source remain distinct. No client field can
claim authenticated verification. Checkouts remain labelled client observations;
historical tests never prove a newer or dirty tree.

TaskState is a reserved server-derived immutable Session event. At most 32
statements of 1024 bytes, 64 source references and a 16 KiB rendered handoff enter
one snapshot. Checkpoint method/schema v2 incorporates its latest verified
TaskState; retained v1 verification remains exact. A separate handoff reference
addresses a source Session, server TaskState or Checkpoint event, and payload
hash. Current independent source SessionRead and every copied source decision
precede disclosure. Receiver ownership and client identity remain unchanged.

Private delivery Manifest v3 retains the cross-session reference separately
from receiver-local checkpoint/tail fields. Preserve retained Manifest v2 proof.
Explicit handoff is an atomic required component of the shared Synveda budget;
insufficient room fails with a budget error rather than a partial task state.
Replayed state is historical evidence under newer instructions and current
policy, and grants no permission to repeat external effects. Unknown outcomes
retain their exact status and existing idempotency/receipt semantics.

Add normalized content-free exact Knowledge-copy dependencies, inherit both
Session and ImportArtifact roots through TaskState/checkpoint/delivery, and
extend every disclosure and cleanup path before enabling the producer. Root
withdrawal hides derivatives immediately. Existing checked inline item Forget
scrubs indexed copied events and rendered runs before deletion; source Forget
keeps bounded durable cleanup. Add the missing final artifact-cleanup event
phase. Bound copied derivatives per item at admission so inline erasure remains
finite; immutable header evidence remains after content removal.

Enable checkpoint creation/restart only for the explicit implemented Claude and
Codex capability set. Reuse their actual compact/start frames and public Session
runtime, preserving spool ordering and idempotency. Registry claims distinguish
protocol replay, manual public-API handoff and native vendor qualification.

## Options considered

1. Populate v1 fields: its retained verifier cannot substantiate those fields.
2. Copy private events into a receiver or spoof its client: loses authority and
   exact provenance.
3. Derive typed state and address the source separately: preserves the ledger,
   current policy decisions, source controls and bounded restore.

## Consequences

The deterministic path needs no model, autonomous reflection worker or new
transcript store. Additive `0009` preserves all prior migration bytes and measured
catalogues. Task/provenance qualification is separate from live vendor loading
and model task success. Revisit the capability set only with real supported
interfaces and independently qualified fixtures.

## Compliance notes

Embedded Cedar, forced tenant RLS, tenant admission, content-free audit, exact
source preconditions and existing VedaFlow publication remain mandatory. Derived
state is working context and cannot publish Knowledge or expand authority.
