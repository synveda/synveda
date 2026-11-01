# ADPT-10: Harness-native memory coexistence

## Problem and evidence

The supported coding agents now keep their own memory beside Synveda's. The
external facts below come from a survey checked on 2026-11-01; each links its
primary source.

- **Claude Code** auto memory arrived in v2.1.32 (2026-02-05) and is on by
  default. The agent writes a machine-local `MEMORY.md` index and topic files
  per project; the first 200 lines or 25 KB of the index load at every session
  start ([memory docs](https://code.claude.com/docs/en/memory)).
- **Codex** documentation says memories are off by default
  ([memories](https://learn.chatgpt.com/docs/customization/memories.md)), but
  the 0.165.0 changelog (2026-10-27) says memory v2 is the default for fresh
  profiles and can import memories from other agents. The two sources
  disagree; the pinned client version decides.
- **GitHub Copilot** Memory has been in public preview since 2026-01-15
  ([changelog](https://github.blog/changelog/2026-01-15-agentic-memory-for-github-copilot-is-in-public-preview/)).

At `8a22b0e5`, all three clients are verified Synveda adapters in
`adapters/registry.json`. No adapter reads, imports or governs these memory
files, and a repository search for `MEMORY.md` or auto memory finds nothing
outside this brief. A team using Synveda therefore also has a second,
per-machine memory for each person and project that is never reviewed,
never decided by Cedar, never audited, invisible to teammates and loaded into
context beside Synveda's. ADR-0057 settled which Synveda surface owns a write
so that one turn is never recorded twice; nothing settles how Synveda relates
to a memory the harness owns. The native memory is also an unreviewed
injection surface for the memory-poisoning risk Synveda's review path exists
to close.

## Scope

1. **Owner decision and ADR.** For each verified client, choose one of:
   - **Defer to Synveda.** Document and test the client's managed or
     organisation settings that turn native memory off (or make it read-only)
     where Synveda governs a project, and have `synveda setup` report the
     effective state.
   - **Capture.** Read native memory changes at a lifecycle boundary the
     adapter already uses and submit them as Session evidence, so they reach
     ordinary Capture candidates and review. Optionally offer a one-time,
     reviewed import of existing native memories.

   The choices can differ per client and can combine (turn writes off and
   import once). The ADR also states which source wins when both supply
   context for the same project.
2. Verify, per pinned client version, what the harness exposes: file location,
   format, whether a setting disables or locks it, and whether a hook can see
   the change.
3. Implement the chosen path for Claude Code first, then Codex and Copilot as
   their evidence allows, and record the state in the generated client support
   matrix.

## Non-goals

- Writing governed Knowledge into a harness's native memory file. That would
  give the harness's own edits and Synveda's projection one file to fight
  over.
- Publishing native memory content as Knowledge without Capture review.
- Watching files with a background daemon; the adapters stay hook-driven.
- Claiming control over hosted memory a client keeps outside the machine.

## Architecture seam

The adapter reads native memory only at an existing lifecycle hook and sends
it through the existing Session event and Capture path, with a distinct event
kind or source label so reviewers can tell a harness's memory from a
transcript. No core crate learns a harness file format. Guidance-only clients
change only agent setup and the support registry.

## Acceptance criteria

- The ADR records the per-client choice and the precedence rule.
- For each client with the defer choice, a live run on the pinned version
  shows native memory off or read-only under the documented setting, and
  `synveda setup` reports it.
- For each client with the capture choice, a live run shows a native memory
  change arriving once as Session evidence, becoming a reviewable candidate,
  and reaching later context only after approval.
- The support matrix states, per client and version, whether native memory is
  disabled, captured or uncontrolled.

## Required tests

- Fixture tests for each client's memory file format and change detection,
  including oversized, malformed and secret-bearing content (redaction applies
  before delivery).
- Idempotent re-delivery of the same native memory change.
- An adversarial native memory entry that tries to instruct the agent is
  stored as evidence and never enters context without review.
- Live client runs for the claimed state; replay alone does not establish it.

## Rollout and rollback

Ship the defer guidance first, because it changes no data path. Ship capture
per client as experimental and promote it after a live run. Rollback removes
the capture hook from setup; already-captured evidence stays governed.

## Dependencies

The owner must make the per-client choice. Live runs need installed,
authenticated clients at pinned versions. The poisoning evidence gap in
[production readiness](../PRODUCTION_READINESS.md) and the
[compliance mapping](AUD-5.md) should cite the result.
