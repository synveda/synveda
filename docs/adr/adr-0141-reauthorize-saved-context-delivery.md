# ADR-0141: Saved context delivery repeats source admission

- **Status**: Accepted (proof storage extended by ADR-0144)
- **Date**: 2026-10-14
- **Feature(s)**: CPR-20, CTX-6, MEM-8
- **Deciders**: Owner-authorized implementation; GPT-6 Astra planning review

## Context

An idempotent ContextRun receipt binds a particular saved contribution. SessionWrite
is necessary to request it, but cannot authorize Knowledge, disclosed provenance
or checkpoint evidence. The existing replay returns saved bytes after only the
Session decision. Optional diagnostic retention can omit exact delivery addresses;
authored packs and Skills also lack complete saved source proof.

## Decision

Before replay, authorize the Session and each retained exact delivered dependency
under current Cedar/RLS and effective Configuration. Replay only the original
bytes when their proof is sufficient and they remain eligible. Otherwise return
one content-free conflict instructing the caller to use a new idempotency key;
do not modify the receipt or silently substitute current revisions.

Addressed Knowledge must still name its current active, valid, fresh revision;
the exact source descriptors actually disclosed must still be independently
visible. Independently hidden descriptors that were never delivered do not revoke
an approved shared conclusion. Checkpoint/tail content additionally requires
current SessionRead and original event/hash/review proof. Graph path endpoints
retain their independent decisions.

ADR-0144 provides private content-free delivery lineage for new receipts,
independently of diagnostic retention, including authored context and Skills.
Nonempty older receipts without that producer proof fail closed. Safe empty
contributions remain replayable. Historical diagnostic reads
and explicit temporal Knowledge queries retain their own data/authority contracts.

## Options considered

1. **Repeat exact source admission (chosen).** Keeps the original receipt and
   prevents it from becoming a second authority path.
2. **Recompose under the same key.** Replaces an acknowledged identity with
   different bytes and destroys idempotency evidence.
3. **Trust original policy or expiry alone.** Cannot observe revocation,
   correction or source removal.

## Consequences

- Positive: saved delivery cannot knowingly repeat revoked or obsolete evidence.
- Negative / accepted: some retained responses cannot prove safe replay and need
  a new key. Source-level suppression and final admission ordering remain MEM-8 work.
- Reversal trigger: reviewed all-mode storage now proves authored/Skill replay
  under ADR-0144 without widening diagnostic disclosure.

## Compliance notes

All reads use current embedded Cedar and ordinary forced-RLS transactions. Audit
and errors contain no source content or denied addresses/counts. This repair does
not establish atomic ordering against in-flight removals, restartable physical
cleanup, or retraction of bytes already delivered. Those require MEM-8's separate
source-level operation, suppression and final admission decision.
