# Copilot CLI adapter (ADPT-9)

Experimental context and observations over the existing authenticated Session
runtime. Build from source with Node 22+ and the workspace's locked pnpm.
Captured native text/tool records use the shared bounded reader, event mapper
and spool; no retrieval, policy or protocol implementation is added.

See [setup and evidence](../../docs/integrations/copilot-cli.md),
[the open acceptance work](../../docs/backlog/ADPT-9.md), and
[ADR-0107](../../docs/adr/adr-0107-copilot-context-adapter.md).
`src/hook.test.mts` covers authored contracts and captured native new/resume
payloads; `src/fixtures.test.mts` preserves digest-pinned evidence for the first
failed marker and successful fresh-marker resume in the same native Session.
`src/transcript.test.mts` checks captured text/tool translation and rejected
input. All 23 Copilot tests and the full 136-test adapter regression suite pass
on macOS Node 24 and offline Docker Node 22. `agentStop` records locally;
`sessionEnd` flushes within a two-second credential/delivery budget and retains
the task for resume. Native Synveda authentication, governed Skill attribution
and the complete lifecycle remain unverified.
The registry's experimental level applies specifically to Copilot CLI.
