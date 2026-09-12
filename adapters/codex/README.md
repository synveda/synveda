# Codex lifecycle translation (CPR-39)

This internal workspace adapter translates captured Codex CLI 0.152.0 start,
PreCompact, Stop and runtime-exit hooks. Its lifecycle is **verified** on
macOS arm64 with GPT-5.5/low and the documented Keycloak setup. Follow
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

`Stop` and `PreCompact` record locally. `SessionEnd` gives credentials and delivery a shared
two-second deadline, below the native client's three-second exit cap, and
keeps the Synveda task active: the native client can resume that same ID.
Namespaced external IDs keep different harnesses apart. Startup/resume retries
only spools for the same client and pinned gateway. An explicit task owner
uses the public Capture/end APIs; closing an MCP connection does neither.
`SessionStart` with `source: "compact"` reuses the context path and its configured
`compact_budget_tokens`. PostCompact has no separate write or injection path.

Input is limited to 64 KiB and transcript reads to 8 MiB/20,000 records. Wrong
Session IDs, symlinks, non-regular files, partial JSON and unrecognised tool
result status are held without advancing the spool cursor. Fixed diagnostic
reasons go to the existing adapter log. The source transcript stays available
for recovery; the hook exits successfully so it cannot block coding.

The observed transcript tags distinguish user text from injected environment
and agent instructions. Only user text, assistant text, function calls and
command results with native exit status and text-only MCP results with native
completion/error status are mapped. MCP namespaces are preserved. Reasoning
is excluded. Non-text tool results remain unqualified. Manual and automatic
compaction passed live. Host death before a recording hook can lose the
unfinished turn. Packaged installation and other versions/platforms are unqualified.

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

`fixtures/transcript-mcp.jsonl` adds seven native records from the same synthetic
task's failed and successful Synveda recall calls after real Keycloak login.
It was captured on 2026-09-12 with Codex 0.152.0/GPT-5.5 low. Only Session
identity/version/source metadata, MCP calls, native completions and model-visible
outputs were retained. The cwd was replaced with `/synveda/qualification/codex`;
native IDs, timestamps, status and synthetic returned Knowledge were preserved.
The first MCP launch lacked the isolated profile directory; allowing the
existing `XDG_CONFIG_HOME` into the MCP environment corrected authentication.

The original lifecycle capture exercised native model authentication only.
The later MCP capture also exercised ordinary Synveda Keycloak authentication.
Replay still uses synthetic HTTP responses and does not itself establish
Cedar, RLS or complete lifecycle qualification. CPR-39 tracks the live criteria.

`fixtures/compaction.json` pins the native manual PreCompact/PostCompact and
subsequent compact SessionStart sequence, captured before the filter correction.
`fixtures/transcript-compaction.jsonl` retains the last authored user/assistant
pair on each side, Session identity and compacted metadata. The opaque replacement
history, other records and unrelated Session metadata are omitted; message fields,
IDs, ordinals and timestamps remain unchanged. Hook paths use the same synthetic
path substitution described above. Automatic frames have separate provenance.

`fixtures/recovery-qualification.json` records the later Keycloak run: five
events survived a paused gateway and arrived once after native resume; manual
compaction reinjected context on the same task after the filter correction.
Both SDK workflows, explicit Capture/end, cross-session reuse and the audit
chain passed. The automatic-threshold probe completed a normal resumed turn
without compaction hooks, so it supplies no automatic-compaction qualification.

`fixtures/auto-compaction.json` captures the first completed interactive turn
with a temporary 1000-token compaction threshold: startup, approved Skill read,
automatic PreCompact/PostCompact, compact SessionStart and final Stop.
`fixtures/transcript-auto-compaction.jsonl` projects seven corresponding native
records, including command completion status and the compacted boundary. Paths
are normalised as above; instructions, reasoning, replacement history and
unrelated metadata are omitted. Both manual and automatic fixtures are replayed
through the same test, proving bounded context and four unique observations.

`fixtures/live-qualification.json` records the complete native task and shared
SDK run on unchanged product code at `8e90358`: automatic context reinjection,
authenticated MCP, four pending outage events delivered once, 19 unique events,
Capture/end, Knowledge reuse and a valid audit chain through sequence 858.
The companion Python/public-API probe uses an unchanged bearer and client for
deny/allow/revoke/re-authorise/deny, removing only its disposable grants.
The receipt states the interrupted low-threshold follow-up and corrected
test-only audit filter; neither is counted as a passing probe.
