# ADR-0119: Session checkpoints derive from the immutable event ledger

- **Status**: Accepted
- **Date**: 2026-09-25
- **Feature(s)**: CTX-6
- **Deciders**: Synveda implementation review

## Context

The Claude `PreCompact` hook can durably record transcript events before the host rewrites them, and `SessionStart` can reinject context after compaction. The server holds redacted, immutable, ordered Session events under forced tenant RLS, but no typed checkpoint or bounded restart composition. A generated summary of arbitrary transcript text would invent authority and evidence. A separate checkpoint database or model service would duplicate the Session runtime and introduce another retention rule before this path has quality evidence.

## Decision

1. The adapter records a distinct compaction-boundary event in its existing local spool after the transcript delta. Its stable client event ID identifies a retry. It may declare bounded expected client event IDs and its local sequence high-water mark; these are coverage claims, not server authority. `PreCompact` still returns after the durable local write, without network or model work.
2. After the ordinary append scan and SessionWrite decision, the gateway builds a typed checkpoint event from the already-redacted event ledger up to that exact boundary. The store persists it as an immutable Session event with a server-assigned sequence and source-event ID/hash list. A bounded source range, digest, coverage status and previous-checkpoint address are immutable. The checkpoint's idempotency key derives from the boundary event. Unknown or missing client evidence is marked incomplete; server sequence continuity alone never proves a complete host transcript.
3. The first method is deterministic, with no summariser or external egress. It keeps bounded labelled excerpts from user-authored events. Goals, constraints, decisions, unresolved questions, artefacts and verification are separate typed fields; this method leaves them empty because the adapter does not supply trustworthy typed observations. Model-authored text and unverified test claims cannot become observed verification. Original events remain primary evidence.
4. Restart composition is requested only by a supported compact/restart hook and resolved for its exact Synveda session, which already binds tenant, principal, client installation, native conversation, repository and project. It reauthorises the Session and current Knowledge, rechecks referenced event hashes, selects one current checkpoint and a bounded later event tail, and counts their rendered contribution with the shared context budget. A missing source withholds the affected derivative. No global latest-session selection or opaque host-state rewrite exists.
5. Checkpoint content is Session working state, never approved Knowledge. Capture may consider its frozen source events through the ordinary candidate workflow; only a typed VedaFlow command may publish Knowledge. The event ledger's existing retention and future disposal own checkpoint events, so this increment creates no second retention schedule. The method and bounded windows are versioned; unsupported adapters do not claim checkpoint delivery.

## Options considered

1. **Typed derived Session event (chosen).** It reuses the immutable, tenant-isolated ledger, idempotent append, audit and retention boundaries while preserving an explicit provenance chain.
2. **A new checkpoint table and global latest row.** It adds an avoidable disposal and RLS surface, and a global lookup can cross native task/fork boundaries.
3. **Pass through the host's native summary.** It cannot prove source coverage, redaction, or permissions and might turn a probabilistic host artefact into Synveda authority.

## Consequences

- Positive: compact callbacks stay local and fast; restart can use exact redacted evidence without a model dependency.
- Negative / accepted: this method is an evidence excerpt, not a semantic summary. It may omit older useful facts; coverage remains explicitly partial when the host or adapter did not expose them. An authorised Session read remains necessary even for checkpoint diagnostics.
- Reversal trigger: probe-tested representative restarts show that a versioned summariser improves task success beyond bounded excerpts while respecting source dependency, redaction, retention and security gates. Such a method needs a separate decision before promotion.

## Compliance notes

The tenant and Session PDP boundary applies to creation and every read; event storage retains forced RLS. Scanning occurs before checkpoint derivation, and audit records only IDs, digests, counts, method and coverage. Derived text is never logged. Knowledge publication still requires VedaFlow review. A source event that is purged or withheld by quarantine invalidates its derivative at use, rather than rewriting historical checkpoint bytes. A SessionRead denial withholds the entire restart contribution.
