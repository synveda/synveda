# ADR-0091: one typed VedaFlow review with configurable separation of duties

- **Status**: Accepted
- **Date**: 2026-08-25
- **Feature(s)**: CPR-32
- **Deciders**: autonomous context-platform continuation

## Context

The context-platform families already open one `vedaflow_proposals` row and
bind their typed payload to an immutable object/commit. The matrix already
counts roles and distinct identities, resolves from the effective inherited
pack plus curator requirements, and an empty requirement auto-applies the same
change. That is the right substrate.

Four gaps keep it from being the product's complete review model. The common
row says only `asset_kind`; reviewers must infer the stable aggregate,
operation and revision from a family-specific projection. A verdict request
does not echo the commit the reviewer inspected. ADR-0032 deliberately allowed
self-approval and placed no constraint on who later executes the effect.
Finally, Advanced Reviews casts verdicts but cannot cancel or complete an
approved proposal. Building one queue per noun would hide those gaps behind
duplicated UI without fixing them.

## Decision

1. **Every proposal carries typed artifact references.** A bounded immutable
   JSON array on the common row names a closed artifact-family vocabulary,
   stable aggregate id, operation, exact proposed version/digest and optional
   expected revision. Typed command layers construct these references when
   they open the proposal. OKF-sourced Knowledge also names the immutable
   import artifact/job evidence. Authored multi-member proposals name one
   reference per member. The array is indexed for review filtering and is
   part of the proposal row's immutable fields.

2. **The matrix can require separation without becoming authority.** Matching
   rules may forbid the proposal author from reviewing and may require the
   effect executor to differ from both author and every counting approver.
   Restrictions merge monotonically: any matching floor or pack rule can make
   the result stricter, and none can subtract a stricter requirement. Cedar
   still decides whether a person may review or execute;
   matrix arithmetic only refuses a combination of otherwise authorised
   identities.

3. **Seeded profiles choose the separation.** Product floors for restricted,
   Skill and Tool material forbid self-review. `regulated-strict` additionally
   requires a separate effect actor for every reviewed artifact.
   `standard` forbids author self-review but lets the author execute after a
   different reviewer approves. `open-collaboration` retains its intentionally
   permissive cells; personal auto-apply remains possible only where the live
   requirement is empty.

4. **This supersedes ADR-0032 decision 7 for proposal review only.** The old
   decision rejected a universal self-approval ban because the direct
   single-actor channel route has no distinct proposer. CPR-32 adds a
   configurable restriction to recorded proposals, where author identity is
   explicit. The direct route remains the matrix's single-actor degenerate
   case until the final authored-artifact cut; it gains no fictional proposal
   identity. Distinct-approver counting remains unchanged.

5. **A verdict carries an exact commit precondition.** Approve and reject both
   require `expected_commit`; a mismatch is a conflict before a review row or
   close transition is written. Stored approvals continue to key by commit.
   Artifact command layers still repeat their own expected-revision and
   payload-integrity checks at execution, so commit approval cannot make a
   stale aggregate mutation valid.

6. **Cancellation reuses proposer withdrawal.** There is one terminal
   lifecycle, not `withdraw` and `cancel` aliases. The public wire remains the
   existing withdrawal act and Advanced Reviews labels it Cancel. A reviewer
   rejects with a reason; the author cancels under `ProposalOpen` and their
   exact identity. This is cancellation semantics without a second route,
   state or audit action.

7. **The comprehensive surface can finish the workflow.** Proposal responses
   expose typed references, separation requirements and a deterministic
   lifecycle timeline derived from the proposal plus immutable review acts.
   Advanced Reviews filters by family, sends exact commit preconditions,
   permits the author to cancel, and invokes the existing `apply` or `publish`
   effect route for approved proposals. Every attempt remains server-authorised
   and failures render the gateway's reason.

8. **Security quarantine and New Learnings stay distinct.** The former gates a
   redacted session event before extraction; the latter decides a capture
   candidate before it becomes Knowledge. Neither publishes an artifact by a
   second mutation path, so folding either into the proposal table would erase
   a real state boundary rather than remove duplication.

## Options considered

1. **Extend the one proposal with typed refs and separation (chosen).** One
   workflow, review contract and audit narrative across all artifact families.
2. **A review table and console per noun.** Easier local DTOs, but six answers
   to approval, cancellation and revision drift. Rejected.
3. **Globally ban self-approval.** Breaks intentional personal auto-apply and
   the direct channel matrix seam. Rejected in favour of monotonic rule flags.
4. **Auto-execute the deciding approval.** Runs an effect under reviewer or
   system authority without the artifact write decision. Rejected; execution
   remains a separate PDP-checked act.
5. **Treat session quarantine as another proposal family.** Confuses admission
   of a redacted event with publication of an immutable artifact. Rejected.

## Consequences

- Positive: one queue can say exactly what aggregate/version/action is under
  review; stale UI actions fail before writing; stricter profiles can prove
  author/reviewer/publisher separation without another authorisation engine.
- Negative / accepted: a regulated reviewed mutation can require a third
  authorised person to execute it, and proposal callers must now retain the
  commit they inspected.
- Reversal trigger: a reviewer assignment language needs alternatives or
  quorum groups beyond role/subject/count → version the matrix vocabulary in a
  new ADR; do not overload typed artifact references with authority.

## Compliance notes

- **PDP/VedaFlow:** every verdict and effect still crosses its Cedar action;
  separation only narrows identities after an allow and never grants.
- **RLS:** the typed reference index lives on the existing tenant-bound,
  forced-RLS proposal row; no structural exception or cross-tenant key exists.
- **Audit:** opened/reviewed/closed acts carry ids, commit, reference digests,
  separation requirements and identity ids, never artifact content or secrets.
- **Privacy:** proposal visibility remains `ProposalRead` at the target; typed
  references are returned only after that decision and reveal no denied
  artifact outside the proposal a permitted reviewer already sees.
