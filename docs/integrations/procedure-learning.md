# Qualified Procedure improvements

SKIL-6 publishes small edits through the normal Capture and VedaFlow workflow.
A registered, explicitly configured service can attest an observed outcome;
ordinary callers produce `client_reported` evidence. Authentication identifies
the reporter. It does not prove that included memory caused a success, that a
Skill executed, or that an external side effect is safe to repeat. Unknown
outcomes remain unknown.

The initial contract uses an exact active Procedure in the Session's project
and scope. Each observation names its immutable revision and content hash,
reported task/environment identity, result digest and optional exact Skill
versions, earlier Session evidence and delivered ContextRun. An outcome-linked
ContextRun must contain only Knowledge/graph snapshots: restart, handoff,
authored channels and Skill advertisements are outside this initial shape.
Skill references require current independent read authority and never activate
or install a version.

## Configure custody and fixed qualification

Use the public People/service identity, access and Configuration surfaces. Admit
reporters with their normal scoped Cedar authority, and publish the complete
`procedure_learning` field through the existing Configuration version and review
workflow. It contains `reporters` and `evaluation_suites`, both sorted and unique,
with at most eight entries each. A suite has these fields:

```json
{
  "suite_id": "owned-held-out-v1",
  "evaluator_id": "<registered independent service UUID>",
  "harness": "closed-iris-retry",
  "harness_version": "v1",
  "configuration_digest": "<SHA-256 of the fixed harness configuration>",
  "cases": [
    {"case_id": "held-out-delayed-ack", "digest": "<SHA-256 of the fixed case>", "required": true}
  ]
}
```

There are at most 32 cases, with unique labels, fixed digests and at least one
required case. The evaluator must be an admitted reporter, distinct from the
requesting identity and Session principal. The normal live Knowledge approval
matrix must require at least one distinct reviewer and forbid author approval.
Configuration and Cedar both apply; an allowlist does not grant permission.

Freeze the candidate, governing Configuration hash and case specification before
running qualification. Keep development examples separate from the fixed cases.
Changed configurations or specifications require a new attempt; a candidate has
at most eight attempts. Retrying the exact eighth attempt remains idempotent.
Each result is recorded once by its frozen evaluator, with complete fixed case
identities, baseline/candidate `passed|failed|unknown` statuses and result digests.
Required failures, regressions from a passing baseline and unknown baseline
results keep the candidate unqualified. Review does not override this gate.

## Public workflow

The CLI uses the public API and existing credential profiles:

```sh
synveda procedure outcome SESSION --file outcome.json
synveda procedure improve SESSION --file improvement.json
synveda procedure evaluate CANDIDATE --candidate-hash HASH --suite owned-held-out-v1
synveda procedure results ATTEMPT --file fixed-results.json
synveda procedure inspect ATTEMPT
synveda procedure submit CANDIDATE --candidate-hash HASH --key stable-review-request
synveda proposal show CHANGE
synveda proposal approve CHANGE
synveda proposal apply CHANGE
```

The equivalent routes are `POST /v1/sessions/{id}/outcomes`,
`POST /v1/sessions/{id}/procedure-improvements`,
`POST /v1/capture-candidates/{id}/procedure-evaluations`,
`POST /v1/procedure-evaluations/{id}/results`,
`GET /v1/procedure-evaluations/{id}` and
`POST /v1/capture-candidates/{id}/procedure-submit`. The generated OpenAPI owns
exact request shapes; CLI JSON files are bounded to 64 KiB.

An outcome input names `client_event_id`, `procedure` (`item_id`, `revision_id`,
`content_hash`), `environment` (`task_class`, `environment_id`,
`environment_digest`, optional `commit`, `tree`, `dirty_digest`), `outcome`,
`result_digest`, `supporting_events`, `skill_versions` and optional `context`
(`context_run_id`, `block_hash`). A small improvement names the exact `procedure`,
`outcomes` (`event_id`, `payload_hash`), `proposed_append`, `uncertainty` and
`development_case`. Neither input accepts a caller-selected evidence class.

The append and uncertainty are each bounded to 1,024 UTF-8 bytes. There are at
most eight supporting events, eight Skill versions and eight outcome references.
The server copies the current baseline, preserves its exception clauses and
classification floor, freezes the complete resulting content/hash and creates
one exact deterministic Capture request. The worker does not call a model for
this typed request. Concurrent identical requests reuse the same event/batch and
candidate. Arbitrary content and metadata still pass the secret/PII scanner;
validated UUID/hash protocol fields are not scanned as free text.

Generic Capture creation/replacement cannot bypass the private improvement
binding. Submission creates an exact same-item Knowledge edit. Inspect old/new
content, source events and the fixed evaluation, then have another authorized
reviewer approve and an authorized actor apply. Already applied receipts replay
with fresh source/write/proposal authority, including recovery of a durable
Capture intent after the Knowledge effect committed.

## Retention and revocation

These are historical derivation references. Advancing the Procedure head does
not erase the observation that used its prior revision. Every read/publication
still checks the exact retained Knowledge, Session, artifact and Skill authority.
Revocation affects the revoked actor; an independently permitted reviewer can
still inspect the same evidence. Source withdrawal/suppression and authorized
forgetting make affected learning copies ineligible. Forget also scrubs the
content-bearing event, candidate and proposal/revision copies under the existing
retention contract. Immutable evaluation records contain closed labels, identities,
hashes and statuses; SQL rejects extra plaintext fields.

A legal hold on a derived revision can block erasure of its historical input
before any scrub. Independent replacement and item control use header authority;
Verify/Merge/conflict operations that interpret retained content require its
current source authority. See [memory lifecycle](context-memory.md),
[ADR-0151](../adr/adr-0151-qualified-procedure-improvements-from-outcome-evidence.md)
and the [Iris walkthrough](iris-domain.md).
