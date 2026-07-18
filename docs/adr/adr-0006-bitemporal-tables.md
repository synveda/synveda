# ADR-0006: Bitemporal storage as current+history table pairs with tx-time triggers

- **Status**: Accepted
- **Date**: 2026-07-18
- **Feature(s)**: FND-4
- **Deciders**: sujitn

## Context

Every record is bitemporal (seed §4.2, tech plan §1.1): *transaction time*
(`tx_from`/`tx_to` — when the database knew this version) and *valid time*
(`valid_from`/`valid_to` — when the fact held in the world). Transaction time
powers "what did the agent know on date X" (tech plan §2.5) and must be
tamper-resistant to be worth anything in an audit; valid time is domain data
the extraction pipeline and supersession logic (MEM-5) set deliberately. The
tech plan fixes the mechanism class — native tables plus triggers, no
extension dependency — but not the table layout, and the layout is the part
that is expensive to change later.

Constraints: the `inject` hot path (p99 < 150ms, seed §10) reads only current
versions; future ANN/FTS indexes must not accumulate dead versions; sqlx
compile-time-checked queries are mandatory (CLAUDE.md); prefer boring,
explicit SQL.

## Decision

Each bitemporal entity is stored as a **pair of identically-shaped tables**:
a current table (`records`) holding exactly the live versions and a history
table (`records_history`) holding closed versions. Triggers — not the
application — maintain transaction time: inserts stamp `tx_from = now()`,
updates and deletes archive the old version into history with
`tx_to = now()`, and history rejects UPDATE/DELETE outright. Valid time is
ordinary application-managed data. Open-ended bounds are `NULL`, and a
`records_versions` view (`UNION ALL` of both tables) is the single surface
for as-of queries.

## Options considered

1. **Current + history pair, triggers (chosen)** — the `temporal_tables`
   extension pattern, hand-rolled in plain SQL. Hot path scans a lean
   current table; history is append-only and can be locked down; DELETE has
   natural semantics (archive, no trigger recursion). Con: two tables to
   keep structurally identical (schema changes must alter both — a
   documented migration rule), and as-of queries need the union view.
2. **Single table holding all versions** — PK `(id, tx_from)`, partial
   unique index for the current version. One table, direct as-of queries.
   Cons: every current-state query must filter `tx_to IS NULL` (an app bug
   silently reads dead versions); future pgvector/Tantivy indexes bloat with
   history; converting DELETE into "close the row" needs recursion-guarded
   triggers. Rejected on hot-path and footgun grounds.
3. **`tstzrange` columns + exclusion constraints** — non-overlap enforced by
   the database, elegant. Cons: GiST exclusion indexes on the write path,
   range types are clumsier through sqlx's checked macros, and the feature
   spec names the four discrete columns. Rejected as the clever option.
4. **`temporal_tables` extension / pg_periods** — same semantics, less code.
   Rejected: adds a C extension dependency to every deployment (tech plan
   explicitly chose "native tables + triggers" to avoid this).

`'infinity'` timestamps were considered for open bounds and rejected: sqlx
does not decode them into `chrono` types, and `NULL` maps directly to
`Option<DateTime<Utc>>` in the checked-query workflow.

## Consequences

- Positive: current-state reads are plain scans of a small table with no
  temporal predicate; tx-time cannot be forged or edited from application
  code (defence-in-depth under seed §2.5 audit-first); the pattern is
  mechanical to repeat for later bitemporal entities (assets, hierarchy).
- Negative / accepted trade-offs: schema changes touch two tables plus the
  view, enforced by convention in migrations; two updates of one record in
  the same transaction collapse into one version (the intermediate version's
  tx period is empty — it never existed in transaction time, which is the
  correct bitemporal reading); under concurrent updates whose transaction
  clocks run backwards, the trigger raises a serialization-style error and
  the caller retries rather than recording a negative-length period.
- Reversal trigger: if as-of queries over the union view dominate and their
  latency at real history volumes breaches SLOs, revisit option 2 with
  partitioning; if a second engine must consume history, revisit logical
  decoding of the pair.

## Compliance notes

Transaction-time integrity underpins AUD-2's "what did agent A know at time
T" — with triggers as the only writer of `tx_from`/`tx_to`, a compromised or
buggy application layer cannot rewrite what was known when. This is
complementary to, not a substitute for, the AUD-1 hash chain. Tables carry
`tenant_id` from day one; TEN-2 row-level security attaches to both tables
of every pair.
