# ADR-0099: Product evaluation separates delivery, use and trust outcomes

- **Status**: Accepted
- **Date**: 2026-08-25
- **Feature(s)**: CPR-40
- **Deciders**: Autonomous continuation of the context-platform programme

## Context

The inherited evaluation harness still described the deleted global runtime
and Record projection. Three live suites intentionally refused to report after
that cut because a budgeted ContextRun is not an enumeration benchmark. The
new session-scoped Knowledge query/evaluation lenses now exist, but the product
also needs a higher-level gate that proves capture, review, reuse, isolation,
supersession, provenance, Skills, Tools, OKF and adapter recovery together.

A retrieved item is not necessarily selected, a selected item is not
necessarily used, and use is not automatically helpful. Collapsing those
events into one score would hide both quality failures and unsafe leakage. The
model-backed extraction and BGE-M3 tiers also depend on external services and
must not be presented as the deterministic merge gate.

## Decision

1. **One declarative deterministic product suite names every required case.**
   `evals/product/suite.json` maps eighteen product/trust scenarios to exact
   acceptance tests. CI validates that inventory, its evidence paths and exact
   test names; `make eval-product` executes the database-backed suite and
   refuses skipped database evidence.
2. **Outcome stages stay separate.** Reports persist `retrieved`, `selected`,
   `injected`, `referenced_by_agent`, `accepted_by_user`, `helpful`,
   `unhelpful` and `caused_correction` as independent measurements tied to an
   exact ContextRun selection and immutable Knowledge revision.
3. **Trust invariants are zero-count gates.** Cross-tenant leakage,
   principal-private leakage, superseded current injection, selection without
   provenance, unversioned Skill/Tool activation and plaintext-secret leakage
   have maximum zero. Coverage comes from named positive controls and exact
   scenarios, so an empty/dead path cannot pass as isolation.
4. **The existing deterministic corpora are re-cut, not emulated.** Session
   events create capture candidates; acceptance uses VedaFlow; query and
   enumeration use the separately authorised session-scoped Knowledge lenses;
   context composition remains budgeted. The evaluator selects explicit
   workspace/project runs and models the former four-tier corpus as principal,
   project, workspace and tenant scopes.
5. **Rendered context is one JSON data entry per selection.** Newlines and old
   watermark syntax inside supplied Markdown cannot forge structural lines or
   evaluation attribution. Each entry carries immutable addresses, type,
   sensitivity and source-evidence availability. Token baselines include that
   evidence cost.
6. **External/model tiers remain distinct.** BGE-M3 retrieval, live-model
   extraction, full security variants and LongMemEval Stage H keep their own
   reports and baselines. A missing service or credential is a named blocker,
   not a substituted deterministic claim.
7. **A live provider needs governed runtime admission.** Each evaluation
   tenant creates and reviews an immutable strict-profile Configuration,
   permits only its local TEI dependency, and binds it at the tenant root.
   Environment selection cannot bypass that artifact. Dense precision reads
   only explicit caller-budget probes that also bind in fact; default-budget
   blocks made smaller by the provenance envelope still measure the scope
   gradient and remain outside the within-scope ranking denominator.

## Options considered

1. **Separate deterministic product and specialised live tiers (chosen).** A
   merge gate remains reproducible while every external result stays honest.
2. **Use ContextRun as the enumeration benchmark.** Rejected: its token budget
   and selection policy make absence ambiguous by design.
3. **Infer helpfulness from selection or injection.** Rejected: it would turn
   delivery telemetry into an outcome claim and make negative feedback
   impossible to measure.
4. **Keep the old evaluator behind adapters.** Rejected: that would preserve a
   second Record-era application path and make policy results incomparable.

## Consequences

- Positive: one revision-labelled machine report and one human report cover
  the full product loop; trust failures are unroundable counts; the original
  deterministic questions execute on current public contracts.
- Negative / accepted trade-offs: the evidence-bearing JSON envelope costs
  more tokens than the old prose block, and the deterministic extractor still
  emits one candidate per event. Both limits are measured explicitly.
- Reversal trigger: a specialised live tier becomes deterministic and
  hermetic on supported CI infrastructure → it may join the default product
  run only while retaining its own version and evidence fields.

## Compliance notes

Every scenario uses the public gateway or an exact acceptance seam exercising
the embedded PDP, tenant RLS, VedaFlow and hash-chained audit. Reports contain
counts, hashes, versions and synthetic timing only; no secret, raw credential
or denied Knowledge address is emitted.
