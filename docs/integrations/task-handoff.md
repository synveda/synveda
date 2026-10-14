# Grounded task state and handoff

CTX-6 derives working state from exact, currently authorized source bytes.
TaskState is an immutable Session event. It is working context; publishing
Knowledge still requires Capture and VedaFlow review.

Read an admitted event with `GET /v1/sessions/{session}/events/{event}`. Build a
request using its server ID, payload hash and exact UTF-8 byte range:

```json
{
  "schema_version": 1,
  "client_event_id": "retry-guard-state-1",
  "statements": [{
    "kind": "objective",
    "evidence": {
      "source_kind": "session_event",
      "event_id": "SOURCE_EVENT_ID",
      "payload_hash": "SOURCE_PAYLOAD_HASH",
      "field": "text",
      "span": {"start": 0, "end": 24}
    }
  }]
}
```

The placeholders must be replaced with inspected addresses, hashes and byte
offsets. The server copies the text and derives its evidence class. Submit via
`POST /v1/sessions/{session}/task-state` or:

```sh
synveda session task-state SOURCE_SESSION_ID --file state-request.json
```

The response contains `event.id` and `event.payload_hash`. An identical
`client_event_id` and body retries the same derivation. Replacement requires
`expected_previous: {"event_id": "...", "payload_hash": "..."}` while the
current head remains admissible. Concurrent replacements serialize; a stale
precondition fails. An unavailable or disposed head permits a new independent
state without inheriting its old evidence.

Supported statement kinds are objective, constraint, decision, proposal,
observation, work, question, next_action, artifact and verification. Optional
`declared_outcome` is planned, attempted, succeeded, failed or unknown. A tool
report or a claimed successful test remains client-reported evidence. It does
not prove authenticated execution or authorize repeating an external action.

Sources can also address an exact current Knowledge item/revision/content hash
or an actual ImportArtifact/content hash with a body byte span. Each source is
independently authorized. Optional `context_snapshot` identifies a same-Session
ContextRun and block hash; snapshots with restart/handoff ancestry are refused.
Optional `checkout` addresses the event containing native
`payload.context.checkout`. Checkout facts remain client reports.

Open the receiving Session normally under its own client identity. An explicit
handoff request to its existing context-run route uses this body:

```json
{
  "budget_tokens": 2000,
  "handoff": {
    "source_session_id": "SOURCE_SESSION_ID",
    "snapshot_event_id": "TASK_STATE_OR_CHECKPOINT_ID",
    "payload_hash": "SNAPSHOT_PAYLOAD_HASH"
  }
}
```

The receiving caller must currently read both the source Session and every
source. Foreign snapshot addresses remain in the private delivery manifest;
receiver-local checkpoint/tail columns retain their original meaning. Preview
the same request without recording delivery:

```sh
synveda context preview RECEIVER_SESSION_ID \
  --handoff SOURCE_SESSION_ID@SNAPSHOT_EVENT_ID@SNAPSHOT_PAYLOAD_HASH \
  --budget-tokens 2000 --json
```

An optional fresh `checkout_observation` contains `ref`, `observed_at` and the
known branch, commit, tree, dirty state and dirty-byte digest. Changed facts
require revalidation. Omitted facts remain unknown; historical verification
never establishes that a new or dirty tree passes.

Native Claude/Codex compaction uses the existing durable spool and compact/start
interfaces. The latest verified TaskState produces checkpoint schema/method v2;
retained v1 checkpoints still use their original verifier. Complete replacement
state excludes earlier objectives and their excerpts. An unavailable newer head
cannot cause fallback to an older objective. Fresh native checkout observations
do not rewrite the spool's historical checkout.

Limits are 32 statements, 1024 copied UTF-8 bytes per statement, a 64 KiB
request/event envelope and 16 KiB rendered handoff. Explicit handoff is atomic
within the shared Synveda allowance: insufficient room returns HTTP 422.
Native optional restart may be omitted with an explicit budget status.
Sensitivity narrowing also applies to copied Knowledge.

Withdrawal immediately makes dependent state and saved deliveries ineligible.
Forget retains permanent suppression and content-free headers, while durable
cleanup scrubs copied Session events and delivery text. Artifact cleanup reaches
its final `session_copies` phase before reporting success. Item Forget clears
indexed Knowledge copies without deleting independent primary Session events.
Retained but inaccessible text before cleanup is distinct from physically
scrubbed service copies. Content already delivered to an external host is
outside that cleanup boundary.

Run controlled acceptance with:

```sh
bash scripts/db-test.sh -p synveda-gateway --test task_handoff \
  --test context_runs --test sessions_api --test source_withdrawals \
  --test artifact_withdrawals --test knowledge_lifecycle -- --test-threads=1
```

Captured protocol tests and ordinary-role database acceptance qualify these
public paths. A new proprietary-client loading run, model task correctness and
provider usage remain separate qualifications; see `evals/product/README.md`.
