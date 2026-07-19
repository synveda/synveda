# ADR-0021: Redaction & secret scanning — scan-at-admission, per-pack modes, signal-less quarantine

- **Status**: Accepted
- **Date**: 2026-07-19
- **Feature(s)**: MEM-2
- **Deciders**: sujitn

## Context

MEM-2 is the redaction pipeline (seed §6): PII and secret detection on
`observe` **before persistence**, with modes deny / redact /
quarantine-for-review per policy pack. The AC: seeded secrets never
reach storage in any mode, and the quarantine review queue works. The
tech plan fixes the implementation home (`synveda-ingest`, §1.2:
"Rust regex+ML pipeline; gitleaks ruleset port for secrets") and MEM-1
(ADR-0020) built the buffer this feature guards, recording the honest
debt: *"staging holds pre-redaction content until MEM-2 inserts itself
between buffer and extraction — redaction-before-persistence is
honestly not yet true."*

Forces at play:

- **"Never reach storage" names the staging table too.** ADR-0020's
  consumer contract (decision 7) put the pipeline behind the buffer —
  but `observe_events` *is* storage: tenant-isolated and app-immutable,
  yet dumped by a backup, readable by an operator, retained until
  MEM-6/TEN-5. A scanner running as a queue consumer would satisfy the
  diagram and fail the AC. The only position that discharges "in any
  mode" structurally is before the insert, in the ack path.
- **The ack path has a budget.** The ack is enqueue-only, <20ms
  (seed §10). Scanning is O(payload bytes) CPU; a worst-case batch is
  256 × 64 KiB = 16 MiB. Regex scanning must neither stall the async
  reactor nor silently regress the MEM-1 load AC.
- **Redaction must be able to differ per pack, and per finding class.**
  The seed pins `regulated-strict` to "PII redaction on ingest" and
  names quarantine-for-review as a mode; a leaked credential and a
  customer email are different severities. Packs today are pure Cedar
  bundles — Cedar cannot express "run this regex", so the mode is pack
  *configuration*, not pack *policy*, and it must ride the same
  resolution (nearest assignment → tenant default → embedded default,
  ADR-0014 decision 3) or two sources of truth diverge.
- **Quarantine needs review authority over personal scopes.** Observe
  writes land at the caller's home leaf (ADR-0020 decision 4) — a
  user-kind scope the content-role grants deliberately exclude (the
  privacy floor, ADR-0015 decision 4). Somebody other than the owner
  must adjudicate a quarantined event, or "review" means self-release
  by the identity whose session leaked the secret.
- **Idempotency must survive the fork.** ADR-0020 decision 2 made the
  staging table the single admission point. If quarantined events lived
  in a separate holding table, a redelivered quarantined event would
  miss the unique key and quarantine twice.
- **The audit seam is binding.** One chained event per audited
  operation (ADR-0019 decision 4); review decisions are mutations and
  must chain in their own transaction.

## Decision

Scanning runs in the observe ack path, before the staging insert, in
`synveda-ingest`; findings are redacted from the payload
unconditionally; the triggered categories' pack-configured modes pick
each event's disposition (deny > quarantine > redact); quarantined
events stage redacted but signal-less, gated by a review table and a
new PDP-governed review plane.

1. **Scan before persistence; the raw payload dies at the seam.** The
   gateway resolves the effective pack for the caller's home scope
   (the same resolution the `MemoryWrite` decision used), scans every
   event's payload, and hands the store **only** redacted content.
   Matched spans are replaced with `[REDACTED:<rule-id>]`; the finding
   record is rule id + category + count — never the matched text, in
   no table, no audit payload, no response, no log line. This is the
   AC discharged structurally: in every mode the secret's only
   possible representations downstream are the placeholder and the
   rule id. Raw content is unrecoverable after admission by design —
   the client is the system of record for its own transcript. The scan
   runs under `spawn_blocking`: worst-case 16 MiB batches are CPU work
   that must not stall the reactor; typical batches cost microseconds.
2. **A curated, validated ruleset in `synveda-ingest`.** Rules are
   data: id, category (`secret` | `pii`), regex, optional redaction
   capture group, optional validator. Secrets are gitleaks-derived
   (MIT) high-signal patterns — private-key blocks, AWS/GitHub/GitLab/
   Slack/Stripe/Google/OpenAI/Anthropic token grammars, JWTs,
   URL-embedded credentials — plus a keyword-anchored generic rule
   gated by Shannon entropy (the gitleaks discipline: keyword + regex
   + entropy). PII is deliberately conservative: email, dashed US SSN,
   IBAN (mod-97 verified), payment-card candidates (Luhn verified),
   international-format phone numbers. Validators run in code on the
   candidate match, so "16 digits" alone is not a card and a prose
   sentence after `token:` is not a secret. Detection walks JSON
   string values (structure preserved; serde_json's recursion limit
   bounds depth). One compiled `RegexSet` prefilters all rules per
   string. The regex+ML split from the tech plan is honoured as a
   seam: MEM-3+ can add an ML/NER pass behind the same `Ruleset`
   surface; EVAL-2's fixture set owns precision tuning.
3. **`RedactionConfig { secrets, pii }`, each `deny | redact |
   quarantine`, resolved per pack.** The config lives on the loaded
   pack inside the PDP and hot-reloads with it. Embedded packs carry
   compiled-in configs: `regulated-strict` = secrets **quarantine**,
   PII **redact** (the seed's own words: PII redaction on ingest;
   quarantine-for-review for the toxic class); `standard` and
   `open-collaboration` = redact both. Stored packs gain an optional
   `redaction` jsonb column (`synveda policy apply --redaction-secrets
   deny …`); a stored pack without one gets the strict config — fail
   safe. `Pdp::effective` exposes the config beside name/version/origin,
   so the observe handler reads the mode from exactly the pack that
   authorized the write.
4. **Disposition per event: the strictest triggered mode.** Redaction
   itself is unconditional (decision 1); the mode decides *flow*.
   Events whose findings trigger categories with different modes take
   the strictest (`deny` > `quarantine` > `redact`). `redact`: the
   event admits normally — staged, enqueued, `status: accepted` with a
   `redactions` summary. `deny`: the event is refused per event
   (`status: denied`, rule ids named), siblings admit; content
   findings are not envelope malformation — the client cannot shape
   what a transcript contains, so whole-batch 422 (ADR-0020
   decision 5) is wrong here, and per-event refusal keeps the
   at-least-once ack idempotent. A denied event leaves no row, so no
   idempotency record: a retry re-scans deterministically; a
   mutated retry under the same key is the client defect ADR-0020
   already names, now with the mild consequence that a cleaned retry
   may admit.
5. **Quarantine = staged, redacted, signal-less.** Quarantined events
   insert into `observe_events` like any admission — the idempotency
   point stays single (redelivery of a quarantined event reports
   `duplicate`, quarantines nothing twice) — but **no PGMQ signal is
   sent**. Migration 0013 adds `observe_quarantine`: `event_id`,
   `tenant_id`, `scope_id`, `findings` jsonb, `state`
   (`pending`/`released`/`rejected`), reviewer subject, review time,
   reason. RLS-forced with the standard policy; grants are SELECT,
   INSERT, and **column-level UPDATE** on the review columns only, and
   a trigger admits exactly the `pending → released|rejected`
   transition — findings and provenance are immutable, review is
   one-shot, schema-enforced (the AUD-1 doctrine). `observe_events`
   itself gains a `redactions` jsonb column stamped at insert
   (append-only preserved): the finding summary is provenance the
   reviewer and MEM-3 both need.
6. **The review plane: two Cedar actions, packs bump to `@5`.**
   `QuarantineRead` (list — tenant-wide on the Tenant resource, or a
   subtree on a Scope) and `QuarantineReview` (release/reject — the
   event's scope, never the tenant). All three product packs grant
   both to `steward`, `org-admin`, and `security-reviewer` — the
   marker role's first live action (seed §5), entirely at home
   adjudicating secret-scan findings before SKIL-2 widens it.
   Pack-uniform, like `MemoryWrite`: how a security control is
   reviewed does not loosen per pack. `auditor` is excluded: quarantine
   rows are content, and the auditor reads everything *but* content.
   This is a deliberate, recorded carve-out of the personal-scope
   privacy floor: quarantined events live at user-kind home scopes,
   and subtree review authority is exactly the oversight function —
   the redaction in decision 1 bounds what that authority can see, and
   the owner deliberately holds no self-release right. No base-layer
   change: the token-scope confinement forbid already fences service
   identities to their anchor subtree for the new actions.
7. **Release re-joins the pipeline; reject ends it.** Release flips
   the state and sends the standard `{tenant_id, event_id}` signal in
   the same tenant transaction — the consumer contract (ADR-0020
   decision 7) is unchanged and cannot distinguish a released event
   from an admitted one. Reject flips the state and sends nothing; the
   staging row remains immutable provenance that never enters the
   pipeline. Both chain their audit event in-transaction
   (`memory.quarantine.released` / `memory.quarantine.rejected`, with
   the authorizing decision context and reason); queue listings chain
   the standalone allowed-read `authz.decision` like every admin-plane
   read (ADR-0019 decision 4). The batch's `memory.observed` payload
   gains quarantined/denied counts and the rule-id summary.

## Options considered

1. **Scan at admission, per-category modes, staging-integrated
   quarantine (chosen)** — the AC holds structurally, idempotency
   stays single-point, the pipeline contract is untouched. Con: CPU on
   the ack path and irreversible redaction; both accepted below.
2. **Scan in the queue consumer (the tech-plan diagram read
   literally)** — keeps the ack path untouched. Rejected: staging is
   storage; the AC fails in every mode the moment the raw row commits,
   and seed §6 says *before persistence*.
3. **Store raw content for quarantined events (reviewer fidelity)** —
   lets the reviewer see exactly what was caught. Rejected: that is
   precisely "secrets reach storage", now concentrated in the table
   whose whole purpose is suspicion; the reviewer adjudicates whether
   the *event* may flow, not whether the secret was real — rule id,
   category, and redacted context suffice.
4. **Whole-batch deny (the 422 validation precedent)** — one shape for
   all refusals. Rejected: envelope shape is client-controlled,
   transcript content is not; failing 255 clean events for one finding
   teaches adapters to shrink batches to size 1 and punishes the
   at-least-once retry contract.
5. **One mode knob per pack (not per category)** — simpler config.
   Rejected: the seed itself splits them for `regulated-strict` (PII
   *redaction* on ingest, quarantine-for-review as escalation); one
   knob forces PII quarantine (review queues full of emails) or
   secret-redact-only (no review at all) under strict.
6. **A separate quarantine holding table (skip staging)** — cleaner
   "not admitted" semantics. Rejected: splits the idempotency point
   (ADR-0020 decision 2); redelivered quarantined events would
   re-quarantine, and release would need a second admission path into
   staging with its own conflict handling.
7. **Curator as the reviewing role** — content approval is curator
   territory. Rejected for now: quarantine is a security control, not
   content curation; curators hold no authority over personal scopes
   and should not gain it as a side effect. `security-reviewer` exists
   for exactly this judgment; FLOW-3 may add curator paths where
   review is genuinely curatorial.
8. **ML/NER-based PII detection now** — better recall on prose PII
   (names, addresses). Deferred: an inference dependency on the ack
   path breaks the <20ms budget and the enqueue-only doctrine; the
   `Ruleset` seam is where an async ML pass (MEM-3's extraction side)
   plugs in later without moving the enforcement point.

## Consequences

- Positive: redaction-before-persistence (seed §6) is now true — the
  ADR-0020 debt is paid; the AC is structural (no code path carries a
  raw finding past the scan seam); the pipeline (MEM-3+) inherits a
  buffer whose content is already policy-clean, so extraction never
  handles a live credential; `security-reviewer` stops being a marker;
  pack configuration gains its first non-Cedar dimension through the
  same resolution and hot-reload machinery packs already use.
- Negative / accepted trade-offs: scanning taxes every observe ack in
  proportion to payload bytes — the MEM-1 load-AC shape (100-event
  small-payload batches) stays the asserted bound, worst-case maximal
  batches may exceed the nominal budget, and EVAL-6 owns
  percentile-complete SLO enforcement; redaction is irreversible, so a
  false positive permanently mutates a stored observation (the
  validator/entropy gates and conservative PII grammar bound the rate;
  EVAL-2's labelled fixtures own measuring it); regex rules drift as
  providers mint new token formats — the ruleset is versioned data and
  a rule addition is a normal PR, but a rotation we miss is a miss
  (defence in depth: the generic entropy rule backstops known
  grammars); `observe_quarantine` retention joins the MEM-6/TEN-5
  disposal obligation at the same horizon as staging rows.
- Reversal trigger: if EVAL-2's fixtures show the regex/validator
  ruleset's precision or recall is inadequate on real transcripts,
  add the ML pass behind `Ruleset` (option 8's seam) rather than
  loosening rules; if scan cost dominates real ack latency at MEM-1's
  load shape, move the *scan* (never the enforcement point) onto a
  pre-staged pending state — an ADR-level change, since it would
  reopen the "raw content at rest" question this ADR closes.

## Compliance notes

Seed §2.2 holds: the quarantine surfaces authorize through
`Pdp::authorize` with versioned actions (`QuarantineRead` /
`QuarantineReview`), and observe's `MemoryWrite` gate is untouched —
scanning adds no bypass, it runs inside the already-authorized
operation. The RLS structural rule (ADR-0009) is satisfied in
migration 0013 (forced RLS + policy + least-privilege grants in the
creating migration; the completeness guard extends to
`observe_quarantine`). ADR-0019's emission obligation: review
mutations chain in their own transaction with decision context;
admission counts ride the existing per-batch `memory.observed` event;
denials stay on the `respond` seam. The AC is demonstrated by
`crates/synveda-gateway/tests/observe_redaction.rs` (seeded secrets
swept for across every storage surface under all three modes;
quarantine review E2E) and `demos/mem-2-redaction.sh`.
