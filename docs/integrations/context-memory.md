# Context and memory lifecycle

Source lineage governs exact Session and imported-artifact roots across Capture,
Knowledge, context delivery, TaskState/checkpoints and Procedure learning. Cedar
checks every read/write, tenant transactions use forced RLS, and typed mutations
retain content-free VedaFlow/audit evidence. Additive schema head `0010` preserves
the published baseline, immutable migrations `0001`–`0009` and their exact prefix
catalogue guards.

## Current implementation matrix

| Phase | Current contract | Runnable acceptance |
| --- | --- | --- |
| A | Exact lineage, scoped withdrawal/Forget, permanent suppression, admission ordering and bounded fenced cleanup; all current producers participate | `bash scripts/db-test.sh -p synveda-gateway --test source_withdrawals --test artifact_withdrawals --test knowledge_lifecycle --test task_handoff --test procedure_learning -- --test-threads=1` |
| B | Six authorized health findings and five pipeline observations, exact inspection actions and a separate frozen private oracle | `bash scripts/db-test.sh -p synveda-gateway --test context_health --test source_reconciliation -- --test-threads=1` |
| C | Closed optional claims, significant values/units/versions/ranges/applicability and conservative unparsed review | `bash demos/cpr-37-conflict-freshness.sh` |
| D | Governed local/private Git transport, exact provider/repository identity, atomic complete observations and targeted path/hash freshness | `bash scripts/db-test.sh -p synveda-gateway --test source_reconciliation -- --test-threads=1`; live private-source qualification remains separate |
| E | Grounded TaskState, replacement checkpoints, explicit handoff and reported checkout comparison | `bash scripts/db-test.sh -p synveda-gateway --test task_handoff -- --test-threads=1`; live vendor/model quality remains separate |
| F | Exact-reference dedup, atomic qualifier preservation, required-input failure and complete optional host accounting | `bash scripts/db-test.sh -p synveda-gateway --test context_runs -- --test-threads=1` |
| G | Authenticated scoped outcomes, exact deterministic append, frozen independent cases and separate publication review | `bash scripts/db-test.sh -p synveda-gateway --test procedure_learning -- --test-threads=1` |
| H | Six fictional Git-backed domain records, review, real CLI preview, fixed runner cases and removal | `bash demos/cpr-41-iris-context.sh` |

## Boundaries and retained content

| Store or derivative | Exact dependency | Withdrawal / logical removal | Forget / physical cleanup |
| --- | --- | --- | --- |
| Originating Session event | Event and admitted payload hash | Explicit event control suppresses its copies and replay identity | Payload is scrubbed under the event's retention contract |
| Actual imported artifact and managed aliases | Tenant/scope/source/path/content digest, with provider identity where applicable | All matching physical aliases become ineligible | Service-owned artifact/frontmatter/worker copies are scrubbed through durable bounded work |
| Capture inputs, candidates and pending decisions | Frozen event/artifact identities and flat roots | Fresh source checks refuse stale extraction and decisions | Affected payload and pending proposal copies are scrubbed |
| Knowledge revisions, sources, relations and embeddings | Exact immutable source/revision identities | Affected derivatives leave future delivery; independent clean heads remain eligible | Bodies/source/relation payloads and embeddings are erased or invalidated under their contracts |
| TaskState, checkpoint and handoff copies | Exact event/Knowledge/artifact spans and source hashes | Current proof refuses restoration/delivery without falling back to an older state | Indexed copied event/ContextRun text is scrubbed |
| Saved ContextRun, including hashes-only/disabled trace | Private immutable complete producer manifest | Replay and expansion reauthorize current exact sources in every diagnostic mode | Rendered/query/candidate copies are cleared; minimal proof remains |
| Procedure outcomes, candidates and learned revisions | Flat historical Knowledge, Session/artifact roots and exact Skill versions | Current authority and root removal gates every read/apply | Content-bearing copies are scrubbed; closed evaluation identities/digests/statuses remain |

Withdrawal retains inaccessible content. A queued or blocked cleanup is not a
claim that bytes were erased. Inspect the source-control and operation receipts
for logical `removed_at`, cleanup state/stage and a content-free hold/failure code.
Source Forget can remain logically removed while physical cleanup is held. Item
Forget can be refused before its removal boundary when a dependent revision is
held. Remote Git history and export receipts are outside generic memory cleanup.

The tenant admission gate serializes logical-removal commits against delivery
admission. Delivery admitted before removal can already have reached its caller;
Synveda cannot retract those bytes. If removal wins, final admission/replay
checks refuse previously assembled content. Cleanup is idempotent, paged and
owner/attempt fenced; stale workers cannot revive removed roots. An unchanged
suppressed source/path/hash stays suppressed through retries, duplicate imports,
force pushes and reindex. Explicitly authorized new source bytes have a new
suppression identity; configuration cannot override an existing suppression.

Lifecycle controls preserve separate meanings for access revocation, expiry,
staleness, supersession, archive, withdrawal and forgetting. Ordinary Knowledge
queries with omitted `as_known_at` still select current database heads; explicit
historical reads retain independent Cedar checks. Current delivery copies and
historical outcome derivations have different contracts: a head change invalidates
an old exact delivery, while a permitted historical observation can still explain
which earlier revision it used.

## Bounds and inspection

TaskState has at most 32 exact statements, 1,024 UTF-8 bytes per statement,
64 KiB requests/events and a 16 KiB handoff contribution. Learning appends are
bounded to 1,024 bytes, eight outcome/support/Skill inputs, 32 fixed cases and
eight frozen attempts. Flat historical Knowledge and Skill authority fan-in is
at most 64; source/artifact root and per-item copy guards cap work at 4,096.
Source descriptors, file/object bytes, fetch deadlines and concurrency retain
[the inbound Git bounds](git-sources.md). Cleanup uses the existing fixed page,
lease and attempt contracts, including fifth-attempt continuation/fencing.

Health is assessed after independent source, conflict-member, proposal, outcome
and evaluation authorization. Its bounded visible scan prefers established
warnings over neutral evidence. A source's latest complete database snapshot
clock is distinct from its latest refresh operation. A changed live descriptor
or detached repository does not qualify an old observation as current.
Hashes-only/disabled traces expose no detailed evidence addresses.

Production health does not know total representation coverage, recall, host
acknowledgment or causal task success. The offline owned-fixture diagnostic freezes
the actual delivery before using its private source/exact-ID oracle. Its assembly
negative control is explicitly injected into an in-memory copy; the actual
planner preserves the qualifier. A failed authenticated task with correct context
still has unknown cause. Oracle labels never enter production ranking.

A complete `host_allowance` report uses `o200k_base` and names
`context_window_tokens`, `occupied_input_tokens`, `reserved_output_tokens` and
`host_overhead_tokens`. Checked subtraction only narrows requested/governed
Synveda text allowance. Missing quantities remain unknown; exhaustion fails
explicitly. This is a caller report, not provider billing or a whole-prompt fit
attestation. See [context optimisation](../adr/adr-0118-governed-context-optimisation.md),
[handoff](task-handoff.md) and [Procedure learning](procedure-learning.md).

## Schema authority

Migration `0010_verified_procedure_improvements.sql` has SHA-256
`61de10ae5ad1cad3a57f101691eb1d8cb353765a66d6fafd79043c41da0e7ab7`.
An isolated raw migration measured the following catalogue, which runtime
authority verifies before serving. Prefix guards retain each prior head's
catalogue; these values do not replace upgrade or retained-data acceptance.

| Catalogue | Rows | SHA-256 |
| --- | --- | --- |
| ACL | 425 | `78b4f6c9d546d81039a53b6aa21612fd75d348c4bc2a32ce60213aef2b1008ea` |
| Routine | 106 | `3debb1befaab1c6c8b0c8b126728157e8a139bf27ef7b754c95a3d2d87c85b1a` |
| Trigger | 182 | `1ead88e36331edac79585b1e063c759fe7abbdaa11ca87d4bb3418fddfe5ece9` |
| RLS | 134 | `e06e5a9bd3fce61c8268d945ff602231f7c5f04ccdc5691bec429beb8f3d0c2c` |
