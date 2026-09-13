# Copilot CLI context adapter (ADPT-9)

Experimental start/resume context over the existing authenticated Session
runtime. Build from source with Node 22+ and the workspace's locked pnpm.
No new runtime dependency, transcript parser, retrieval or policy implementation.

See [setup and evidence](../../docs/integrations/copilot-cli.md),
[the open acceptance work](../../docs/backlog/ADPT-9.md), and
[ADR-0107](../../docs/adr/adr-0107-copilot-context-adapter.md).
`src/hook.test.mts` covers authored contracts and captured native new/resume
payloads; `src/fixtures.test.mts` preserves digest-pinned evidence for the first
failed marker and successful fresh-marker resume in the same native Session.
All fourteen tests pass on macOS Node 24 and offline Docker Node 22. Production
start translation needed no change. Native Synveda authentication, observations
and the complete governed lifecycle remain unverified.
The registry's experimental level applies specifically to Copilot CLI.
