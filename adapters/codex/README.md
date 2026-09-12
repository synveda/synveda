# Codex lifecycle translation (CPR-39)

This internal workspace adapter translates captured Codex CLI 0.152.0 start,
Stop and runtime-exit hooks. It is **captured**, not live verified. Follow
[`docs/integrations/codex.md`](../../docs/integrations/codex.md) for setup,
write ownership and the remaining qualification limits.

```sh
pnpm install --frozen-lockfile
pnpm --filter @synveda/codex-adapter... build
pnpm --filter @synveda/codex-adapter... test
```

The dependency on the existing Claude adapter exposes only its internal
Session runtime. It shares credential resolution, configuration, public HTTP
calls, event mapping and the atomic spool consumed by `synveda session flush`.
There is no new delivery engine or adapter/plugin registry.

`Stop` records locally. `SessionEnd` flushes within the existing deadline and
keeps the Synveda task active: the native client can resume that same ID.
Namespaced external IDs keep different harnesses apart. Startup/resume retries
only spools for the same client and pinned gateway. An explicit task owner
uses the public Capture/end APIs; closing an MCP connection does neither.

Input is limited to 64 KiB and transcript reads to 8 MiB/20,000 records. Wrong
Session IDs, symlinks, non-regular files, partial JSON and unrecognised command
result status are held without advancing the spool cursor. Fixed diagnostic
reasons go to the existing adapter log. The source transcript stays available
for recovery; the hook exits successfully so it cannot block coding.

The observed transcript tags distinguish user text from injected environment
and agent instructions. Only user text, assistant text, function calls and
command results with native exit status are mapped. Reasoning is excluded.
Other tool-result formats and compaction need authentic qualification before
translation. Host death before Stop/SessionEnd can lose the unfinished turn.

## Fixture provenance

`fixtures/lifecycle.json` contains ten real hook inputs from normal `exec` and
`exec resume` in one synthetic repository on Darwin arm64, 2026-09-12. The
installed 0.152.0 client used GPT-5.5/low; all six capture hooks were reviewed
and trusted through the normal `/hooks` UI. No hook-trust bypass was used.

`fixtures/transcript.jsonl` is a subset of that same native transcript: Session
metadata, non-developer response items and the completed command event. Other
metadata/event records and `base_instructions` were omitted. The synthetic
repository path was replaced with `/synveda/qualification/codex` throughout;
hook `transcript_path` was replaced with the corresponding fixture path.
Native IDs, timestamps, prompts, tool outputs and ordering were preserved.
The registry pins both fixture digests. Unit mutations are separate test inputs.

The capture exercised native model authentication, not Synveda authentication.
Replay uses a synthetic HTTP responder and proves translation/durability, not
Cedar, RLS, Keycloak or complete native context consumption. Those require the
remaining public-API live qualification described in CPR-39.
