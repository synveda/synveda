---
title: "EVAL-5: Security evals"
labels:
  - epic:EVAL
  - phase:2
size: M
---

# EVAL-5: Security evals

**Epic:** EVAL — Evaluation (functional requirement) · **Phase:** 2 · **Size:** M

## Description

Policy-leak suite (restricted content never crosses sensitivity/scope under 10k generated query variants); cross-tenant fuzz (TEN-6); prompt-injection-via-memory suite (a memory containing instructions must not alter agent behaviour when injected — content is data, wrapped and labelled).

## Acceptance criteria

Written 2026-07-31 (ADR-0048). The feature arrived with four words — "nightly;
zero-tolerance gate" — which name a cadence and a posture and no axis, no
surface and no artefact. Fourth time (EVAL-1/ADR-0028, EVAL-2/ADR-0046,
EVAL-4/ADR-0047), same precedent.

- **One security corpus** under `evals/fixtures/security/` in which every
  (record, reader) pair declares `readable` or `forbidden`, refused at parse
  time when a pair is undeclared or declared twice. An undeclared pair is an
  unmeasured boundary, and a security suite that skips one silently is the
  failure mode it exists to prevent.
- **The corpus is governed into place, never seeded.** Material enters at its
  author's leaf through `/v1/observe`, climbs through `POST /v1/proposals` and
  each level's real approvers, and reaches `restricted` through a classify
  proposal the author opens at their own home scope and two distinct approvers
  sign, one of them holding `compliance` — the only mechanism in the product
  that mints the tier (ADR-0038 decisions 8 and 9).
- **Every read surface, not one.** Each generated variant is asked over
  `POST /v1/inject` and `POST /v1/recall`'s query form; each reader is
  additionally asked the sweep form and the **ids form naming every record it
  must not have**. Recall's universe is wider than inject's by design
  (ADR-0024), and the ids form needs no retrieval to succeed and only a refusal
  to fail.
- **Counts, not rates.** `security_leaks_sensitivity`, `security_leaks_scope`
  and `security_leaks_tenant` are integers gated at zero. A rate divides a leak
  by a denominator the run chooses, and three decimal places then round one leak
  in ten thousand to zero.
- **Floors under the denominator.** `security_probes` and `security_variants`
  are gated with minima — 10k variants on the nightly. A one-sided gate with a
  free denominator passes by measuring less, and nothing in the report would
  look wrong.
- **A positive control.** `security_controls` is gated at 1.0: every
  declared-readable pair actually reaches its reader, so a run of zeros is a
  measurement rather than an empty corpus, a dead pipeline or an expired bearer.
- **Two predicates.** A leak is graded by record identity *and* by distinctive
  phrase; either counts, and a disagreement between them is reported as its own
  defect, because a block whose text carries material its watermark does not
  name is not the same failure as one that served the wrong record.
- **The cross-tenant half runs here**, against a second admitted tenant with its
  own hierarchy, actors and corpus. TEN-6's remaining scope — the store seam
  TEN-2 already fuzzes, and graph traversal, which has no caller-facing surface
  until GRPH-3 — is recorded rather than left to be rediscovered.
- **The prompt-injection half is an invariant about lines.**
  `security_unattributed_lines` is gated at zero: every non-empty line of a
  composed block is the preamble, a section header, the index legend, the
  watermark or an entry line, and the entry lines number exactly
  `record_ids.len()`. A record's content cannot forge a scope header, an entry
  no record backs, a marker on a line of its own, or a watermark. This takes a
  renderer that folds whitespace in rendered content rather than an extractor
  that happens to, plus the preamble line saying the entries are recorded
  material and not instructions — labelled in the ADR as a mitigation addressed
  to the guest, not counted as a control.
- **`security_marker_echoes`** — content reproducing ` [confidential]` or
  `(recall <id>)` inline, which needs no newline — is measured and gated by
  nothing on the first run.
- **A real, governed relaxation fails it by name.** A lapse — proposed on the
  disclosing side, approved by two distinct stewards, time-boxed and audited
  (AUTHZ-4) — granting a sibling team read of the vault team's material, on a
  fresh tenant. It fails the gate naming the axis, the baseline, the measurement
  and the delta, while `security_leaks_sensitivity` and `security_leaks_tenant`
  hold at zero on the same run — and they hold for two *different* reasons,
  which is why they are separate axes: the confidential record is withheld by
  the grant's own declared tier ceiling, and the `restricted` one by something
  no grant can reach at all, since it lives at a personal leaf and the base
  layer's one permit carries `resource.kind != "user"`.

  A lapse rather than a pack flip, and the reason is worth more than the demo: a
  pack cannot put a sibling team's material into anybody's block, because the
  candidate universe is the caller's placement chain and "widens by lapse and by
  nothing else" (ADR-0037 decision 13). `open-collaboration` at the org
  discloses nothing here — which is a good product property and a demo that
  would have proved nothing.
- **Nightly at the full variant budget** against `evals/baseline-security.json`,
  and a deterministic every-k-th slice on the **pull-request** path against
  `evals/baseline.json`: a product that blocks a merge on a token count and not
  on a disclosure has recorded its priorities backwards.
- **Demo script.**

Deferred with a recorded trigger: the behavioural half of the injection suite —
whether a model reading the block obeys an instruction inside it — because it
measures a joint property of the product's framing and one model's
susceptibility, and would fail when a model changed rather than when the code
did. It rides the model-backed judge EVAL-3 must build and ADR-0046 option 6
already deferred.
