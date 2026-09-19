# Copilot CLI adapter (ADPT-9)

Context and observations over the existing authenticated Session runtime.
Use the release archive's complete `plugin/copilot-cli/` tree with Node 22+,
or build from source with the workspace's locked pnpm.
Captured native text/tool records use the shared bounded reader, event mapper
and spool; no retrieval, policy or protocol implementation is added.

See [setup and evidence](../../docs/integrations/copilot-cli.md) and
[ADR-0107](../../docs/adr/adr-0107-copilot-context-adapter.md).
`src/hook.test.mts` covers authored contracts and captured native new/resume
payloads; `src/fixtures.test.mts` preserves digest-pinned evidence for the first
failed marker and successful fresh-marker resume in the same native Session.
`src/transcript.test.mts` checks captured text/tool translation and rejected
input. All 31 Copilot tests and the full 144-test adapter regression suite pass
on macOS Node 24 and offline Docker Node 22. `agentStop` records locally;
`sessionEnd` flushes within a two-second credential/delivery budget and retains
the task for resume. The clean native start/resume and shared SDK workflow
verify authentication, exact approved Skill activation, context, proposals,
workspace denial, Capture/end, Knowledge reuse and audit correlation.
The registry's verified level is limited to CLI 1.0.83 / gpt-5.6-luna on macOS
arm64 with the documented source-build/Docker-Keycloak setup. Native outage,
compaction, other result shapes/platforms and published installation remain
unqualified. Extracted archive replay is a separate packaging check.
