# ADR-0079: Evidence tiers for the installed Claude Code session lifecycle

- **Status**: Accepted
- **Date**: 2026-08-23
- **Feature(s)**: CPR-14
- **Deciders**: Live Claude Code session acceptance gate

## Context

CPR-12 moved the Claude Code adapter onto the session plane and made its local
delivery durable. Unit tests proved the adapter functions and a mock driver
proved recorded hook inputs, but neither proved the installed plugin,
Claude Code's current private hook frames, the real gateway, persistence and
the audit chain in one run. Earlier evidence belongs to the deleted
observe/inject plane and cannot be replayed as proof of the replacement.

The strongest test requires a current authenticated `claude` executable.
Ordinary CI deliberately has no Claude credential, and a test that quietly
substituted manufactured hook JSON would make "live" mean two incompatible
things. Conversely, making all lifecycle persistence depend on paid,
non-deterministic model access would leave every pull request without a gate.

The acceptance environment also has a destructive temptation: insert a tenant,
identity, grant, record and session directly so the interesting part starts
quickly. That would prove a path the product does not offer and, for governed
assets, bypass the PDP this product exists to enforce.

## Decision

### 1. Evidence has three named tiers

1. **Captured/mock** runs the built hook child process over committed
   real-client frames against a scripted gateway in the adapter suite.
2. **Replay/live-gateway** runs those same bytes through the built hook, public
   session routes, current schema, embedded PDP, Postgres and audit chain. It
   is the deterministic CI gate and is always labelled replay.
3. **Live-client** installs the packaged marketplace through
   `synveda plugin install`, asks Claude Code to report the plugin, four hooks
   and MCP server, then runs `claude -p` against the same gateway harness.
   Only this tier may close the live acceptance criterion.

A lower tier never claims the tier above it. If the executable or
authentication is absent, the live runner exits 77 and says which prerequisite
is missing (`make` surfaces recipe `Error 77` as its own non-zero status);
replay success does not turn that into a live pass.

### 2. Replay bytes carry provenance

`adapters/claude-code/fixtures/manifest.json` binds every hook and transcript
fixture to a genuine client version, capture provenance, sanitisation
statement and SHA-256. A committed JSON Schema names the manifest contract and
the adapter suite checks the schema invariants, hashes, coverage and
credential/personal-path denylist.

The live hook accepts an opt-in `SYNVEDA_CAPTURE_DIR` only for an absolute
scratch path. It writes the raw client frame as mode 0600 under a mode 0700
directory. The live runner deletes those raw frames with its isolated
configuration; committing a new capture requires explicit sanitisation and a
manifest hash.

### 3. Acceptance setup uses product paths

Tenant admission remains the operator/system path. The principal scope and
first administrator grant are minted through the production JIT provisioning
seam using the `synveda-admins` claim. Workspaces, projects, sessions and
events are created through their public HTTP routes under the PDP. A seed
memory, when needed to prove a non-empty context block, is appended to a seed
session and processed by the production ingestion worker; no governed record
is inserted by the test.

The adapter can name both `workspace_id` and `project_id` through
`.synveda/config.json`, `SYNVEDA_WORKSPACE` and `SYNVEDA_PROJECT`. A project
cannot be inferred from list order. Once the server opens a run, the stored
placement wins over later configuration.

### 4. Recovery is proved as a lost acknowledgement, not a second happy path

The replay takes the gateway down after a turn is durable, asserts two pending
entries and their attempt counts, then restores the gateway. Before the hook
retries, it appends the first pending event through the public route and
deliberately leaves the local acknowledgement unchanged. The next
`SessionStart` therefore sends an overlapping two-event batch: the first
answer is `duplicate` at its original server sequence and the second is
`appended` at the next sequence. All client ids remain unique.

Acknowledged purge safety remains owned by the CLI spool tests: purge can
remove acknowledged entries and cannot remove a pending one. A host killed
before any hook writes the in-flight tail remains outside the guarantee.

### 5. Existing budgets are the performance gate

The acceptance target reports SessionStart, Stop, SessionEnd, append,
context-run and backlog-recovery duration. It asserts against the existing
hook/request ceilings (8s start, 5s Stop, 8s SessionEnd, the configured
context deadline and bounded 2s backlog work). Limits are not raised from this
test. The dedicated append load test continues to own the sub-20ms product SLO.

### 6. Payload evidence and operational evidence stay separate

Spool directories and files are explicitly 0700/0600 in both the Node writer
and Rust CLI writer, independent of umask. The server computes its own BLAKE3
payload hash; the spool's SHA-256 remains only a local corruption check.
Timeline assertions name event type, length, order and lateness without
message text. Audit assertions scan the whole tenant chain and reject known
fixture content. Raw payload expansion remains separately decided under
`SessionDiagnostics`.

Logs and the live report contain versions, ids, counts, outcomes, hashes and
durations, never message bodies or credentials. The live report is written
under `target/`; isolated Claude/Synveda configuration and raw frames are
removed after the run.

### 7. Stable protocol is asserted, model prose is not

The live prompt requests a real `Read` tool call so the persisted protocol can
prove user, tool invocation, tool result and assistant activity. The test does
not compare the assistant's words. It asserts one opened/resumed session,
context-run persistence, ordered events, normal close, timeline and verifying
audit actions, together with exact client/plugin/Synveda/platform versions.

## Consequences

- Ordinary CI gains a database-backed job explicitly named
  `claude-replay`; it requires no Claude credential and cannot claim the
  installed client ran.
- `make claude-acceptance` reproduces that tier locally and
  `make claude-acceptance-live` is the separately runnable real-client gate.
- This ADR changes no schema or public HTTP API. `project_id` was already in
  the public session contract and durable spool; CPR-14 closes the missing
  adapter configuration seam.
- A replay can ship while live verification remains pending, but feature
  counts and support statements stay pending until the real executable
  completes the gate. That is an explicit state, not a degraded pass.
