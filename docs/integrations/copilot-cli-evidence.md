# GitHub Copilot CLI qualification record

This page keeps the dated evidence behind the Copilot CLI entry in the
[client support matrix](../CLIENT_SUPPORT.md). To set Copilot up, follow
[Connect an AI coding agent](../AGENT_SETUP.md) and the
[Copilot CLI guide](copilot-cli.md).

The [clean lifecycle result](../../adapters/copilot-cli/fixtures/clean-lifecycle.json)
proves both native starts use the same Synveda task, with both SDKs running
between the prompts. Earlier failures remain in their original fixtures.

## Evidence and remaining qualification

Authored tests exercise the hook process against the shared mock HTTP gateway:
authentication, stable namespaced task identity, start/resume, another task's
start, fresh context requests, denied/empty context, outages, gateway/client
isolation, missing login and malformed/oversized input. These verify adapter
behaviour, not actual Cedar/RLS enforcement or native model consumption.

The observation increment at `d22f8fd` plus its working tree passes **136 adapter
tests** (105 Claude, eight Codex, 23 Copilot), zero skips, on macOS arm64 Node
24.18.0 and offline Linux arm64 Docker Node 22.23.2. Docker uses the pinned image
`node@sha256:7725a5c2c83eed1d36258c66efae14b1ceccd021db9ed1d9559d3335ed3d68ed`,
an ordinary UID, a read-only checkout and executable temporary storage for
synthetic CLI stubs. The existing Codex package passes all eight extracted tests
on both runtimes; it contains no Copilot runtime yet. Formatting, strict
TypeScript compilation, dependency, registry, documentation, backlog and ADR
gates pass. No Rust, SQL, public contract or dependency changes are made.

Native local checks used only an owned scratch checkout and isolated
`COPILOT_HOME`. `copilot mcp add ... --json` produced the expected local command
entry, and `copilot skill list --json` listed the synthetic project Skill as
enabled. No account credential or user Skill contents are captured here.

The user-approved native probe ran at 2026-09-13T07:00:16Z on CLI 1.0.83,
auto-routed to `gpt-5.6-luna`, with a saved 30-credit Session limit. It exited
zero in 9.17 seconds and reported one premium request. That counter is retained
as reported, not converted into a monetary price. Remote export and the Synveda
MCP server were disabled for this synthetic boundary check; no Synveda login or
public-API lifecycle was exercised by the native client.

The [captured lifecycle](../../adapters/copilot-cli/fixtures/lifecycle.json)
records `userPromptSubmitted`, `sessionStart` (`source: "new"`), `preToolUse`,
`postToolUse`, `agentStop` and `sessionEnd` (`reason: "complete"`). Its native
`hook.end` confirms successful acceptance of top-level `additionalContext`.
The [transcript projection](../../adapters/copilot-cli/fixtures/transcript.jsonl)
retains stable native event IDs, user/assistant text, the correlated tool pair
and `skill.invoked` with `trigger: "agent-invoked"`. Private path prefixes are
replaced, and hidden reasoning, encrypted content, system instructions and
unrelated diagnostics are omitted with the exact projection declared in the
lifecycle file. Registry digests and replay tests check both files.

The marker assertion **failed**. A leftover synthetic project Skill instructed
the model to return `SYNVEDA_COPILOT_SKILL_OK`, which it did after invoking the
native `skill` tool. That differs from the context hook's expected marker.
The empty `--available-tools=` flag did not prevent this tool invocation. This
is useful evidence that native Skill activation is observable, but it does not
attribute that Skill to an approved Synveda binding/version or verify hook
context consumption. A successful process exit does not override the failure.

After separate approval, the corrected retry ran at 2026-09-13T07:28:35Z using
the same saved Session and 30-credit ceiling. Both owned conflicting Skills
were removed. The final command used `--available-tools=`,
`--excluded-tools=skill`, `--deny-tool=shell`, `--deny-tool=write` and
`--deny-tool=url`; no tool invocation occurred. An initial `--deny-tool=*`
attempt failed argument validation before any new hook/transcript event or
model execution. It is not counted as a passing native invocation.

The [resume lifecycle](../../adapters/copilot-cli/fixtures/resume-lifecycle.json)
and [resume transcript](../../adapters/copilot-cli/fixtures/resume-transcript.jsonl)
preserve four hooks, `source: "resume"`, the same native UUID and a native
`session.resume` event linked to the previous shutdown. The response exactly
matches an unpredictable marker present only in the latest hook output, absent
from the user prompt and prior transcript. It exited zero in 5.03 seconds.
The native premium-request counter increased from one to two cumulatively;
neither that counter nor the retained native usage units are a monetary estimate.
This verifies synthetic hook consumption on resume. It does not exercise the
real Synveda hook's Keycloak credentials or governed context retrieval.

Original private native transcripts replay offline into the same four initial
and six total events; no additional model request was made. The documented
`acceptance-interop` Compose smoke passes for the running stack. Smoke verifies
service/public-edge health; native authentication remains unqualified.

The replay tests cover durable local Stop, outage/retry, duplicate hooks,
same-task resume, denied delivery, gateway isolation, disabled observation,
malformed/foreign input and one deadline spanning credentials and a stalled
append. Native hook/transcript captures remain separately digest-pinned.

## Authenticated public-edge preflight

The [2026-09-13 result](../../adapters/copilot-cli/fixtures/public-edge-replay.json)
at `24cd02d` pins the tested source/transcript hashes and content-free outcomes.
The existing demo approver and separate author/auditor profiles needed fresh
browser logins; both completed ordinary Keycloak OIDC/PKCE S256. Configuration
and grants were unchanged. Captured transcript fixtures were driven through
the real hook using authored invocations, with private paths/current timestamps.
This is public-API replay evidence, separate from native model execution.

| Check | Measured result |
| --- | --- |
| Start context and task identity | Allowed context delivered; the same task stays active after exit/resume. The unrelated captured resume prompt did not select the ingestion Knowledge, so allowed-context resume is not claimed. |
| Durable observations | Stop records locally before HTTP delivery; six stable text/tool events delivered without duplicates. |
| Governed Skill installation | Existing sync dry run, install and unchanged reconciliation pass. Receipt binding/version/bundle/file identity match the enabled available binding; immutable API bytes equal the installed file. Copilot lists the project Skill as enabled. |
| Python and TypeScript | Both unchanged workflow examples pass on one task: allowed context, exact Skill, idempotent pending proposals, foreign-Session denial and actual response-trace correlation. |
| MCP reconnect | The existing bounded test client connects twice to `rmcp`. Both connections recall on the same explicit task, deny the foreign Session, refuse missing identity and expose only `recall` in host-write mode. Native Copilot MCP is still pending. |
| Task-owner lifecycle | Eight unique events, including two SDK Skill loads; Capture completes with six candidates, explicit end succeeds, a later Session reuses approved Knowledge, and the audit chain verifies through sequence 938. |

The six captured events include an old synthetic Skill invocation; it is not
attributed to the installed approved binding. The two typed `skill.loaded`
events come only from the SDK examples. Native governed activation must join
the observed Skill path to the installer receipt and unchanged immutable bytes;
a matching name or model assertion is insufficient.

The documented Compose smoke passed again with suffix `acceptance-interop`,
pool `10.231.46.0/24` and profile `demo`. No product or adapter code changed;
the earlier 136-test adapter results remain dated to their implementation
increment. Full CI, the database suite and full Compose acceptance were not
rerun for this preflight; strict Clippy is not applicable to these non-Rust edits.

To repeat the public checks, use the documented Compose/demo fixture and two
ordinary login profiles, an isolated trusted workspace and fresh task/run keys.
Use the hook registration and Skill sync commands in the [Copilot CLI guide](copilot-cli.md), then the existing
[SDK scenario and workflow commands](../../sdks/README.md#shared-workflow-example)
with the hook-created Session ID. Keep replay inputs labelled separately from
authentic invocations. The retained result records the exact fixture hashes;
private scratch drivers and credentials are not project state.

The approved native plan was two user prompts on one fresh native Session:

1. Load the installed ingestion-retry-review Skill for review, use the injected
   Synveda Session ID for MCP recall, and attempt the known foreign test Session.
   The Skill's ingestion steps are reviewed; no ingestion requests are needed.
2. After both SDK examples run on that native-created task, resume the same
   Copilot UUID and recall again with the injected Synveda identity. Verify
   persisted native observations and tool results, exact Skill attribution,
   explicit Capture/end, later Knowledge reuse and content-free audit.

Both earlier synthetic prompts are complete. The new pair was approved with
one shared 30-credit Session soft limit and no automatic paid retry. The CLI's
soft limit can overrun within a response. Restrict tools
to the approved Skill and Synveda recall workflow, disable
built-in MCP servers, remote export and unrelated custom instructions, and keep
shell/write/URL access denied using the
[vendor permission patterns](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference#tool-permission-patterns).
Retain native output privately and project only the needed authenticated
frames; never commit credentials, reasoning or system instructions.

### Native denial and recovery checkpoint

The first invocation ran at `d71641e` on 2026-09-13 using CLI 1.0.83,
auto-selected gpt-5.6-luna. The
[projected transcript](../../adapters/copilot-cli/fixtures/governed-transcript.jsonl)
shows approved Skill activation, allowed MCP recall using the hook-injected
Synveda Session ID, and a denied call for the foreign Session. The prompt did
not supply the allowed Synveda Session ID. The preflight matched the enabled
binding, immutable Skill bytes and installer receipt; native activation named
that exact installed path and its SHA-256 remained unchanged before/after.
No typed Skill-usage event is inferred from its name.

The [partial result](../../adapters/copilot-cli/fixtures/governed-probe.json)
records a 10.17-second invocation and one reported premium request, with the
30-credit ceiling retained. It also preserves the failed observation delivery:
the native denied completion used a text `error` object instead of `result`.
The old parser held the full transcript, leaving zero recorded/acknowledged
events. The correction accepts only the demonstrated failure shape and reuses
the existing event mapper and spool. Regression replay preserves all eight
observations, including out-of-order tool results, and drains them once through
Stop/exit without implicitly ending the application task.

All 140 adapter tests pass, zero skips, on macOS arm64 Node 24.18.0 and the
pinned offline Docker Node 22.23.2 image: 105 Claude, eight Codex and 27 Copilot.
Strict TypeScript, formatting, dependency, registry/digest, docs, backlog and
ADR checks pass. No Rust changed; strict Clippy is not applicable. Full CI,
fresh database tests, full Compose acceptance and unchanged package checks
were not rerun for this correction.

On the 2026-09-19 retry, the temporary raw captures, saved native Session,
credentials and spool were absent. The retained projection is regression
evidence, not a backup that can recreate native Session state. Docker was
stopped, and the documented Compose startup refused the hosts ownership state:
hosts bytes still matched the recorded digest, but the backup witness's `dev`
field had changed. The ownership check was preserved. Startup/public smoke,
native resume, both SDKs on this native task, post-fix live delivery/audit and
Capture/end could not be completed; the earlier public-edge replay results
belong to their separately identified task. No paid prompt ran on retry.

A later confirmed hosts-ownership renewal restored the same Docker
project and ordinary Keycloak login. The replacement below used durable private
state and its own explicit cost allowance. The second prompt of the original
lost pair was not executed.

Native outage recovery, applicable compaction and unknown/non-text result shapes
remain unqualified. Vendor `preCompact` is notification-only; no post-compaction
reinjection is claimed. Require applicable native acceptance before registry
promotion and extending the existing release archive/installation checks.

Full CI, the database suite, full Compose acceptance, a packaged Copilot
installation and complete native lifecycle qualification were not run for this increment.

### Shared native and SDK workflow — 2026-09-19

At `8336c22`, a separately approved pair ran on CLI 1.0.83 / gpt-5.6-luna. Its
first prompt did not discover the nested project's hooks. Checking the native
Git root exposed the setup error; `git init` in the owned scratch project fixed
discovery. The second prompt's real `sessionStart` with `source: "resume"`
created the Synveda task, injected allowed context and led to exact approved
Skill activation, allowed MCP recall and foreign-Session denial. This partial
run did not establish a clean initial-start/resume pair.

The [result](../../adapters/copilot-cli/fixtures/shared-workflow.json) pins the
[native projection](../../adapters/copilot-cli/fixtures/shared-workflow-transcript.jsonl),
actual hook receipts, approved binding/file identity, SDK proposal IDs and audit
traces. It records these measured boundaries:

| Check | Result |
| --- | --- |
| Native delivery | 15 observations at prompt exit; a later no-prompt reopen/exit delivered the final assistant message, giving 16. The task remained active. |
| Shared SDK workflow | Both existing examples retrieved allowed context and approved Skill bytes, submitted pending proposals, denied the foreign Session and matched actual response traces to audit. |
| Durable outcomes | 18 unique events including two SDK `skill.loaded` events; native payloads match the spool; exact public-API replay appended zero duplicates. No native outage test was run. |
| Explicit lifecycle | Capture completed with 16 reviewable candidates. The task owner ended the Session; a later Session reused approved Knowledge. |
| Audit | Content-free correlation and chain verification passed through sequence 1059. |
| Usage | Two prompts in one native Session, shared 30-credit soft limit, two cumulative premium requests and 847087000 nano-AIU. No-prompt reopens did not increase usage. These counters are not price estimates. |
| Regression | 142 adapter tests passed on macOS arm64 Node 24.18.0 and pinned offline Docker Linux arm64 Node 22.23.2, zero skips; 29 are Copilot tests. |

Native activation names the exact installed approved Skill but does not infer
a typed Skill-usage event. Candidate Capture and SDK proposals do not publish
Knowledge. The first missing-hook prompt remains failed in its captured result.
The separate clean pair below provides the registry's current context evidence.
Both approved prompts in this partial run were used. Full CI, fresh database tests, full Compose acceptance, native
outage/compaction and packaging were not run for this evidence increment.
Strict TypeScript and repository metadata gates passed; the documented Compose
smoke passed again. Rust is unchanged, so strict Clippy is not applicable.

### Clean lifecycle qualification — 2026-09-19

At `c86d5c6c11e1a7d2085ca776b4b88750a8febb02`, the fresh project passed Git-root,
approved Skill and workspace-denial preflight. Initial credential resolution
failed; ordinary browser PKCE renewal restored both profiles. Automatic approval
review initially refused the external invocation pending payload-disclosure
evidence. Public-API inspection proved that both selected Knowledge items and
the installed Skill exactly matched the repository's synthetic demo source; the
reviewed retry was permitted. No model prompt ran before that resolution.

Two approved prompts then passed in one native Session. Actual `sessionStart`
hooks with sources `new` and `resume` delivered allowed context and the same
Synveda task ID. Neither user prompt supplied that ID. Both prompts activated
the exact approved Skill, recalled allowed Knowledge and received the expected
foreign-workspace denial. Both existing SDK examples ran between the prompts
on the same task and verified pending proposals and actual audit trace IDs.

The [result](../../adapters/copilot-cli/fixtures/clean-lifecycle.json) and
[transcript projection](../../adapters/copilot-cli/fixtures/clean-lifecycle-transcript.jsonl)
pin seven authentic hook receipts and 16 native observations. Seven observations
arrived at first exit and 15 at resumed exit. A later native reopen with no user
prompt and `/exit` delivered the last assistant event. Its recorded usage stayed
at two cumulative premium requests and 575693000 nano-AIU under the shared
30-credit soft limit. Native interactive startup emitted a separate auxiliary
gpt-4o-mini failure; it did not add a user prompt or increase those counters.

All 18 events, including the two SDK Skill events, persisted with unique IDs;
native payloads matched the spool, and exact public-API replay appended zero
duplicates. Runtime exit retained the active task. Explicit Capture completed
with 16 reviewable candidates, task-owner end and later Knowledge reuse passed,
and the content-free audit chain verified through sequence 1179. All ten
registry lifecycle criteria pass for this named source-build setup. Public-API
replay establishes idempotency; it is not native outage-recovery evidence.

No product implementation changed. Native outage/compaction, other result
formats and client platforms, published installation, full CI, fresh database
tests and full Compose restart acceptance were not exercised in that run.
Archive and installer validation is recorded separately below.

All 144 adapter tests passed on macOS arm64 Node 24.18.0 and pinned offline
Docker Linux arm64 Node 22.23.2, zero skips: 105 Claude, eight Codex and 31
Copilot. Strict TypeScript, formatting, dependency direction, registry/digests,
docs, backlog and ADR gates passed. Rust is unchanged, so strict Clippy is not
applicable.

## Release archive validation

On 2026-09-19, starting at `571a6c6a185fe73efd4d04fccc4d38a3df3d7a91`, the Copilot
adapter joined the existing archive and installer under ADR-0065 amendment 10. Each Codex/Copilot
tree contains 17 regular runtime/manifest files, with release-pinned private
dependencies and no source, test, fixture or workspace symlink. Production
bytes match the build and resolve dependencies inside the extracted archive
before any test helpers are added. No adapter or server behaviour changed.

| Environment | Extracted lifecycle replay | Installer fixtures |
| --- | --- | --- |
| macOS arm64, Node 24.18.0 | 8 Codex + 23 Copilot pass | 12 pass |
| Offline Docker Linux arm64, Node 22.23.2 | 8 Codex + 23 Copilot pass | 2 pass, 10 fail at the existing unsupported-platform refusal; not a passing installer run |
| Offline Docker Linux x86_64 under emulation, Node 22.23.2 | 8 Codex + 23 Copilot pass | 12 pass |

All passing suites report zero skips. Installer fixtures exercise initial
installation, reinstall, upgrade, preserved Codex/Copilot configuration and
project hooks, and missing hook/shared-runtime refusal before mutation. They
use synthetic release binaries; they do not qualify native binaries or a
published release. Linux arm64 remains outside the installer's release matrix.

After building all three adapters with the frozen workspace dependencies, run
`make plugin-package-check` and `node --test scripts/install.test.mjs`. The
supported Linux installer check uses the immutable x86_64 child of the Node
image index above:

```sh
docker run --rm --platform linux/amd64 --network none --user 1000:1000 \
  --cap-drop ALL --security-opt no-new-privileges --read-only \
  --tmpfs /tmp:rw,exec,nosuid,nodev,size=256m \
  --mount "type=bind,src=$PWD,dst=/workspace,readonly" --workdir /workspace \
  node@sha256:b6ca9eabea5fc816699178b5dd4842270e1c42f90739afc93bbcfdf91a13512e \
  sh -c 'node scripts/check-plugin-package.mjs && node --test scripts/install.test.mjs'
```

The full 144-test adapter regression passes again on the host. Strict
TypeScript, shell syntax, formatting, dependency direction, registry, docs,
backlog and ADR checks pass. All 347 deployment tests pass across completed
component runs, with zero skips; the Compose render matrix and deployment
convergence check also pass.

Neither complete `make check-deploy` invocation passed uninterrupted. The first
failed the unchanged TERM-responsive-leader fixture: timeout 124 instead of
forced-cleanup 125. All eight deadline tests passed unchanged in isolation.
The second passed all 298 tests before deployment convergence, including all
169 lifecycle cases, then failed when the sandbox denied an ordinary loopback
socket. The remaining 49 convergence/uninstall tests and static convergence
check passed with that required socket permission. No runtime or gate was
changed to obtain those results.

No additional paid native prompt was run. Full CI, fresh database tests,
live Compose restart acceptance and native
execution from a published installation were not run; Rust Clippy is not
applicable because no Rust changed.
