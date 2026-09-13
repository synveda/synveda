# Copilot CLI context adapter (ADPT-9)

Experimental start/resume context over the existing authenticated Session
runtime. Build from source with Node 22+ and the workspace's locked pnpm.
No new runtime dependency, transcript parser, retrieval or policy implementation.

See [setup and evidence](../../docs/integrations/copilot-cli.md),
[the open acceptance work](../../docs/backlog/ADPT-9.md), and
[ADR-0107](../../docs/adr/adr-0107-copilot-context-adapter.md).
`src/hook.test.mts` covers authored contracts and the captured native start
payload; `src/fixtures.test.mts` preserves digest-pinned native evidence and its
failed marker assertion. Production start translation needed no change.
The registry's experimental level applies specifically to Copilot CLI.
