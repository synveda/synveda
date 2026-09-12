# Authenticated harness and SDK interoperability

Execution plan accepted on 2026-09-12. Audit baseline:
`7fa7d56cbc6baf0b7ace631826efcb3215ea1435`. Feature state remains in
`backlog/STATUS.md`; client support remains in `adapters/registry.json`.

## Boundaries

Keep the governed context, Skill and Knowledge platform, Docker-first
validation and Keycloak through generic OIDC. All consumers use public APIs;
the gateway retains Cedar, forced RLS, VedaFlow and content-free audit.
Reuse `rmcp`, Sessions, retrieval, Skill materialisation and the checked OpenAPI.
Do not add orchestration, an external MCP execution service, a generic plugin
system or speculative framework adapters. Public package publication is outside
this implementation slice; package ownership/licensing remains ADPT-4 work.

Apply NASA/JPL Power-of-Ten principles where they fit Rust: explicit control
flow, no new recursion or unsafe code, bounded inputs/retries/body reads,
small functions, narrow visibility, checked results and strict Clippy.
Boundary failures return errors, not assertions that terminate the process.
This is a Rust engineering discipline, not a flight-software certification claim.
See [the original rules](https://spinroot.com/gerard/pdf/Power_of_Ten.pdf)
and repository `AGENTS.md`.

## Batch 1 — repair existing integration (ADPT-1, ADPT-2, CPR-12, CPR-23)

Status: implemented and tested. Native Claude revalidation is separately blocked by its expired upstream OAuth session.

- `syncSkills` in `adapters/claude-code/src/skills.mts` omitted the mandatory CLI
  `--scope`. Resolve the configured project's scope, otherwise the authenticated
  principal scope, then delegate to the existing CLI sync. Never fall back from
  a refused project to a different scope.
- `mcp-server.mts` dropped project/workspace selection and `Server::new` in
  `crates/synveda-cli/src/mcp.rs` invented identity per process. Forward target settings; accept an explicit
  Synveda Session ID at launch or per tool call, or an explicit stable task key
  for a dedicated process. The host's context hook supplies its existing
  Session ID. Missing identity produces actionable guidance; discovery still
  works. Never infer task completion from disconnect.
- Correct `render_remember` and tool help: a successful append is an
  observation, not extraction or Knowledge publication.
- Acceptance: real CLI/gateway Skill materialisation and revocation; MCP
  reconnect/interleaving/target mismatch tests; no duplicate host writes;
  unchanged authentic client inputs, regenerated server responses and
  content-free audit checks. All these acceptance checks pass.

## Batch 2 — matched base SDK slice (ADPT-4)

Status: initial base slice and shared live Keycloak workflow pass. ADPT-4
remains open for release and wider compatibility obligations.

- Generate types and operation metadata from the checked OpenAPI contract;
  use maintained HTTP libraries with small ergonomic clients.
- Cover Session open/get/events, context/query, approved Skill retrieval,
  Knowledge proposals and audit queries. Expose bearer providers, deadlines,
  typed errors, explicit pagination and correlation. Bound retries and only
  replay operations with a documented idempotency contract.
- Add equivalent Python and TypeScript examples and shared wire fixtures.
- Acceptance: the coding harness and both applications retrieve permitted
  context, load an approved exact Skill version, submit a proposal that remains
  pending under the test policy, deny cross-workspace access and correlate
  content-free audit using Session/artifact IDs and W3C trace identifiers.
  Test authentication refresh, malformed input, safe replay and cancellation.
- Run with ordinary identities against the documented Docker/Keycloak path.
  Record environment blockers separately from product failures. This slice
  does not close ADPT-4's wider publication/runtime support obligations.

## Batch 3 — qualify Codex (CPR-39)

Status: delivered. Codex CLI 0.152.0 is verified on the recorded macOS arm64,
GPT-5.5/low and Keycloak setup. The existing Session runtime passes native
automatic/manual compaction, MCP recall, outage/recovery, SDK handoff,
Capture/end, reuse and audit. Live public-API revoke/re-authorisation also passes.
Packaging, non-text results and other versions/platforms remain unqualified.

- Inspect the installed Codex version and capture its actual lifecycle/MCP
  frames before writing host translation. Verify current Skill discovery:
  Synveda currently selects `.codex/skills`, while current vendor documentation
  specifies `.agents/skills`. The installed client loaded both roots: this
  suspected defect was disproved, and the existing Skill target is unchanged.
- Reuse the public API/CLI and existing delivery semantics. Implement only
  the concrete event translation supported by captured frames, preserving task
  identity across reconnect, resume and multiple conversations.
- Acceptance: authenticated context delivery, events, Capture, task end,
  replay/restart, Skill discovery and persisted audited outcomes under
  ADR-0098. Promote the exact client version only after live evidence passes.
- Copilot CLI and Pi have documented integration surfaces but no Synveda
  adapter. They remain later qualification candidates. VS Code's hook contract
  is separate from Copilot CLI's. No support claims follow from configuration.

## Evidence and outcomes

Baseline findings refer to the recorded commit above. This revision contains
the corrective code and acceptance evidence; package publication remains open.
Existing retrieval, policy, RLS, audit-chain, orchestration and registry
implementations were reused. No external MCP execution service was added.

| Priority / gap | Evidence and user-visible failure at baseline | Existing component reused | Smallest correction and acceptance | Outcome |
| --- | --- | --- | --- | --- |
| P1: harness Skill sync | `adapters/claude-code/src/skills.mts`, `syncSkills`; `crates/synveda-cli/src/main.rs`, Skill Sync arguments. Missing required scope made the launched sync fail. | Existing `skill::sync`, `/v1/me` project/principal anchors and approved bindings. | Forward the exact distribution scope; refuse a missing explicit project. Actual CLI/gateway test in `crates/synveda-gateway/tests/skills.rs` proves pending exclusion, immutable version installation, rollback and removal. | Fixed; tests pass. |
| P1: task/target handoff | `adapters/claude-code/src/mcp-server.mts` and `crates/synveda-cli/src/mcp.rs`, `Server::new` / `resolve_session`. Dropped project selection and a new transport identity fragmented task context and audit. | Public Sessions, authorised Session reads, existing idempotency and `rmcp`. | Forward target/profile; pass host Session ID in context; accept launch/per-call Session IDs or a bounded stable task key. Re-authorise every use and reject conflicting placement. `scripts/interop-mcp.test.mjs` proves reconnect, Unicode keys, interleaving, denial and write ownership. | Fixed; tests pass. |
| P1: base SDKs absent | Inspected baseline `sdks/`, workspace manifests, generated OpenAPI and console API client. Python/TS applications otherwise hand-built public HTTP calls. | Checked OpenAPI, existing TS generator, maintained Python type generator, HTTPX/Fetch and public domain APIs. | Generate only 15 operations/52 schemas; add bounded auth/error/correlation clients and equivalent examples. `crates/synveda-gateway/tests/support/sdk_interop.rs` runs both SDKs with real gateway policy, Claude hook context and a valid audit chain. | Initial slice and live Keycloak workflow pass; package release remains open. |
| P2: observation acknowledgement | `crates/synveda-cli/src/mcp.rs`, `render_remember` and `remember_tool`. Successful append incorrectly promised extraction/recallable Knowledge. | Existing Session observation disposition and separate Capture/VedaFlow paths. | Correct acknowledgement/help only; unit and real stdio tests distinguish recorded, duplicate, quarantined and refused observations. | Fixed; tests pass. |
| P1: background delivery gateway binding | `adapters/claude-code/src/deliver.mts`, `retryBacklog`, inspected after `8f40237`. The active hook checks `bindGateway`, but its retry of other conversations did not: a later login could send stored transcript events to a different deployment. | Existing spool `gateway_url`, `client_name` and ordinary authenticated public event append. | Match the saved gateway/client before any retry or retirement. The two-gateway regression in `hook.test.mts` leaves the original backlog pending and sends no events to the second deployment. | Fixed; no spool format or policy change. |
| P2: Codex qualification | Baseline registry had no lifecycle entry. Native 0.152.0 subsequently emitted `SessionEnd` and resumed the same native ID: treating that hook as final completion would close a still-resumable task. | Existing stdio server, credential/Session/event/spool runtime and conformance registry. | Pin native hook/transcript frames; translate only observed shapes in `adapters/codex`; namespace IDs, persist at Stop, flush at runtime exit and retain task identity. Replay asserts outage/retry, duplicate hooks and a single open with no implicit end. | Complete live qualification passes for the named setup; registry level is verified. Both Skill roots work, so no Skill-path change. |
| P1: response trace correlation | `deploy/compose/configs/caddy/Caddyfile`, `synveda_upstream`, strips incoming tracing headers; both SDK clients returned their sent trace ID. The real public-proxy workflow failed its audit correlation assertion. | Existing OpenTelemetry request span, Axum middleware, audit API and bounded SDK response handling. | Return `X-Synveda-Trace-Id` from the actual request span; prefer a valid response ID in SDK success/error results. Existing observability tests assert the exported span matches the header; both SDKs test edge replacement and invalid headers. | Fixed; fresh DB and live public-proxy workflows pass. Proxy sanitisation is unchanged. |
| P1: native exit budget | `adapters/claude-code/src/turn.mts`, `credentials.mts`, `deliver.mts`; installed Codex 0.152.0 clamps SessionEnd to three seconds. Credential work plus an in-flight append could overrun that cap. | Existing CLI bearer resolution, durable spool and bounded Fetch request. | Share a two-second absolute deadline across Codex credentials/delivery and bound each append by remaining time. A hung CLI and stalled append leave six durable events that are delivered on retry. | Fixed; regression and native exit pass. |
| P2: native MCP result omission | `adapters/codex/src/transcript.mts`, `translate`, previously accepted only string outputs. Captured MCP output is an array; tool results were omitted while the cursor advanced. | Existing Session event mapper and native `McpToolCall` completion metadata. | Translate captured text-only arrays, preserve namespace and native error status; hold unknown shapes. `transcript-mcp.jsonl` pins real failed/successful calls. | Fixed; replay and persisted native result assertions pass. Non-text results remain unqualified. |
| P2: Session audit filter omits lifecycle rows | `crates/synveda-gateway/src/audit_query.rs`, `payload_filter`, filters top-level `session_id`; `sessions.rs`, `end` / `session_image`, stores the identity under `session.id`. In the live result, session filter omitted end event 422, while action filter returned it. | Existing `EventFilter`, exact resource/action filtering, immutable audit rows and tenant-scoped search. | Use a typed `EventFilter::session_id` and two exact containment shapes in `synveda_audit::search`, conjoined with the existing filters before pagination. `session_filter_covers_lifecycle_pages_without_crossing_sessions_or_tenants` checks all seven lifecycle/delivery events over four pages, combined filters, foreign/unknown Sessions, tenant AuditRead and an unchanged frozen export. | Fixed; 42 focused exact-role Docker DB tests and retained-data Compose/public-proxy verification pass. |
| P1: Codex compaction filter | `adapters/codex/src/hook.mts`, `readInput`, rejected PreCompact and compact SessionStart. `adapters/codex/fixtures/compaction.json` captures both native boundaries; compact-start output was empty. Users received no fresh governed context at that boundary. | Existing `turn` local persistence, `sessionStart`, compact token budget and durable Session identity. | Accept PreCompact with the documented trigger vocabulary and SessionStart `source: "compact"`. Captured replay proves durable writes, budgeted context and no duplicates; live manual and automatic runs prove context reinjection and preserved events. | Fixed; eight adapter tests and native Keycloak manual/automatic compaction pass. No further product change was needed. |
| P2: clean npm archive has no entry point | At `265eb6b`, `sdks/typescript/package.json` exported `dist/client.mjs` but had no pack build hook. Clean `npm pack` contained only `package.json`; installing it succeeded but importing `@synveda/sdk` raised `ERR_MODULE_NOT_FOUND`. | Existing TypeScript compiler/build script, npm lifecycle, generated API and nine wire tests. | Add `prepack`, include runtime/type outputs, then run the existing suite and consumer type checks against the installed archive using `scripts/check-sdk-package.mjs`. | Fixed; Node 22/24 installed-package tests pass. Python packaging also passes through its existing Hatchling backend; no Python runtime fix was needed. |
| P2: Codex runtime absent from release archive | At `d17bd4d`, `scripts/package-plugin.sh` produced 106 entries and no Codex runtime; the release workflow built only Claude. The verified Codex hook required a workspace checkout/compiler. | Existing Codex hook/reader, private shared Session runtime, Node package resolution and the current archive/installer. | Package the compiled runtime under `plugin/codex/`, build both adapters in release CI and reuse the eight captured tests through `scripts/check-plugin-package.mjs`. Installer tests prove upgrade replacement, preserved user configuration and pre-mutation refusal of an incomplete bundle. | Local archive replay passes on Node 22/24. Native execution from a published installation remains unqualified. |

| Area | Classification after this slice | Evidence / boundary |
| --- | --- | --- |
| Synveda MCP server | Implemented and tested | `crates/synveda-cli/src/mcp.rs`; authentic and specification corpus in `crates/synveda-cli/tests/mcp_corpus.rs`. Stdio legacy/modern protocol and recall/remember remain maintained-library paths. This does not add a remote HTTP MCP endpoint. |
| HTTP contracts and language SDKs | Partial | Public catalogue/OpenAPI/console peer tests, the Python/TS base slice and installed archives pass. Broader ADPT-4 release coverage is open. |
| Skills import, validation, approval, export/install | Implemented and tested | Existing public Skill service, CLI and `crates/synveda-gateway/tests/skills.rs`; actual CLI materialisation/revocation now covered. No registry rebuild. |
| Context, observations and proposals | Implemented and tested | Existing Session/Context/Knowledge routes; Session suite, Claude lifecycle replay and shared SDK acceptance. Pending proposals remain outside Knowledge. |
| Authentication, policy and audit | Implemented and tested for the prioritised authenticated workflow | Fresh Keycloak browser acceptance and native Codex/both SDKs pass. Workspace denial, pending proposals, public-edge trace correlation and a valid audit chain are asserted. Session lifecycle and delivery filtering pass the focused exact-role DB regression and live Keycloak public-proxy verification. |
| Examples, setup and compatibility | Partial | `sdks/README.md`, equivalent runnable examples and `docs/integrations/codex.md`. Claude replay and Codex protocol pass; native lifecycle limits remain explicit. |
| External MCP servers | Implemented and tested for catalogue/discovery; gateway execution deliberately unsupported | Existing trusted server/version/binding catalogue and bounded discovery: `docs/INSTALL.md` and `crates/synveda-gateway/tests/tools.rs`, now run in the full exact-role database suite. The gateway does not execute imported commands. External server management is not a prerequisite for Synveda serving MCP. |

## Validation checkpoint (2026-09-12)

The branch began this continuation at `0a6dfb5770c153fbe49f3118be97329378c703f7`.
The content-free native result and source digests are retained in
[`adapters/codex/fixtures/keycloak-qualification.json`](../adapters/codex/fixtures/keycloak-qualification.json).
It is partial live evidence, not a promotion to `verified`.

| Check | Result |
| --- | --- |
| Full `make ci` and `make db-test` at `8f40237` | Passed. The full gates were not repeated for this follow-up; changed paths received the focused checks below. |
| Fresh canonical `make compose-acceptance` | Passed on macOS 26.6.2 arm64, OrbStack Docker Engine 29.4.0 / Compose 5.1.2 after the administrator completed the owned hosts handoff. Real Keycloak browser login/product seed, all six service restarts and final API verification passed. |
| Rebuilt deployment after response-trace correction | Canonical down with `demo,browser-acceptance`, then up/smoke with `demo`, passed while retaining PostgreSQL and project secrets. Public unauthenticated response was 401 with a fresh gateway trace ID, not either forged caller header. |
| Live native Codex plus both SDKs | Passed on one Session `01a096b9-e1c1-7a50-aa32-8c1db0efdba8`. Codex 0.152.0/GPT-5.5 low consumed injected context, read the approved exact Skill, recalled via authenticated MCP and resumed the same native/task identity after both SDK applications ran. Each SDK retrieved allowed context/Skill, submitted an idempotent pending proposal, denied an existing foreign-workspace Session and correlated content-free audit. |
| Persisted public-API outcomes | 14 unique events: two user, four assistant, three tool calls/results and two SDK Skill-load observations. Capture froze 12 eligible events and completed with 12 reviewable candidates. The task owner explicitly ended the Session. A separate application Session reused approved Knowledge. Audit chain valid through sequence 427; the separate action query confirmed end event 422. |
| SDK checks | Generated drift and nine tests per language pass on local Node 24.18.0 / Python 3.14.6. Earlier eight-test suites and Python generated imports passed on pinned Linux arm64 Node 22.23.2 / Python 3.11.16; the new correlation case was not repeated on that minimum-runtime matrix. |
| Adapter checks | All 105 Claude tests and six Codex tests pass. Includes two-second total exit budget, stalled credentials/append, retry, namespace preservation and authentic MCP success/failure. Native outage/recovery is still separate from deterministic replay. |
| Focused exact-role Docker database suite | All 20 tests pass: 16 observability tests and four Skill/SDK tests, including the explicitly requested shared harness/Python/TypeScript workflow. No ignored/skipped cases in this invocation; owned database fixtures were removed after success. |
| OpenAPI, Rust discipline and repository gates | Six OpenAPI tests, strict gateway Clippy, formatting, generated API/SDK drift, dependency direction, registry/fixture digests, backlog, accepted ADR and documentation gates pass. No SQL/schema or dependency changes. |
| Explicitly blocked or not run | Native Claude revalidation remains blocked by expired upstream OAuth. Native Codex outage/recovery, compaction/reinjection, non-text MCP results, packaged installation, other client versions/platforms and Copilot CLI/Pi qualification remain unverified. The Copilot CLI/Pi executables were unavailable. `live_precision`, real-corpus Skill/rubric tests and `report_live_catalog_fingerprints` were not requested in this focused invocation. |

The initial live SDK assertion failed because it compared the sent trace ID with
the proxy's new server trace; the final rerun above passed after the correction.
The initial native MCP launch lacked the isolated XDG profile directory; native
`env_vars` forwarding corrected setup, and the same task then authenticated.
A first generic up with the browser profile was refused before mutation because
that profile requires fresh assets; the documented down/demo-up transition
succeeded. These refusals are not counted as passing tests.

The earlier no-database observability invocation exercised 13 cases and skipped
three database-dependent cases. All three subsequently ran in the fresh exact-
role suite above. Original retained `acceptance-e2e` data and secrets were not
reset. No lifecycle contract or policy gate was weakened.

## Session audit-filter correction (2026-09-12)

CPR-33/ADPT-4 starts from `8ef2e722d01a7007a72d00043ad0e805bb5ea2e9` on
`codex/authenticated-client-interoperability`. ADR-0092 clarifies the two current
payload shapes before implementation. The correction changes one existing
SQLx-checked query and its typed selector; it adds no storage model, dependency,
public operation or authority path.

- All 42 focused exact-role Docker DB tests pass: 17 audit-query, three audit-
  event and 22 Session API tests. The first attempt failed because the new test
  omitted the two already-emitted retrieval-stage events from its expected
  history; the corrected test checks all seven. The rerun used the retained
  disposable fixture with the same ordinary gateway role. Its two databases,
  four reserved networks, image tag and private credentials were then removed.
- All 24 audit unit tests and six OpenAPI tests pass. Strict Clippy for
  `synveda-audit` and `synveda-gateway`, formatting and contract/document gates
  pass. OpenAPI and the console client remain current without regeneration.
- `SYNVEDA_DB_TEST_TASK=sqlx-prepare bash scripts/db-test.sh` regenerated and
  checked metadata against a fresh Docker database. Only query hash `610bda2`
  changes to `5f528cf`; result columns/nullability are identical, with two
  bounded JSON parameters added. No schema or unrelated metadata changed.
- Ordinary Keycloak refresh was refused; both synthetic profiles completed
  browser/CLI PKCE login again using the existing browser-acceptance code and
  image. The pre-rebuild public API reproduced missing open/end events 359 and
  422. After canonical `make compose-up` and `make compose-smoke`, the public
  API returned all 23 saved Session audit events across 12 pages, including
  both rows. The exact frozen export through sequence 431 remained unchanged
  (hash `19bb2a12a53b62730a617958618fbd63a8f6a419d4dc24793de0a661ebba9277`).
  Combined resource filtering stayed empty for another Session; the workspace-
  scoped principal still received 403 for AuditRead and foreign-workspace
  Session access. At 18:47:45 UTC the chain verified through sequence 452
  (hash `46e849292bcc29bd9ad55527ab469ad6faadefdb3ec22631a9c08680d6c36947`).

No tests were ignored or skipped in the focused successful invocations.
Full workspace CI/database gates and full fresh-project Compose acceptance
were not repeated for this correction. The earlier dated client qualification
receipt remains unchanged; the native client qualification limits still apply.

## Native qualification checkpoint (2026-09-12)

Starting from `8ff38a6a944f7e17ed2b21e3957bff0ba1140a7d`, the owner explicitly
approved the synthetic Skill/context/transcript payload for OpenAI GPT-5.5.
The installed native client remains 0.152.0 with normal project/hook trust;
credentials remain local. No client update or trust bypass was used.

- Native outage/recovery passed: eight baseline observations, five pending
  events after a turn with only the owned gateway paused, and 15 persisted
  events after resume. All pending IDs appeared once on the same Session.
  The gateway was restored unconditionally. Existing delivery code was reused.
- Authentic manual PreCompact/PostCompact and compact SessionStart frames
  exposed the filter omission before implementation. After the small filter
  correction, compact-start output included fresh allowed context and the
  original Session ID. The original observations survived compaction; replay
  also proves the configured compact budget and local-only PreCompact writes.
- Both existing SDK examples passed again, followed by native resume on the
  same Session `01a09702-10aa-79b2-ae1a-fca231deb5af`. All 25 unique events
  persisted, including two SDK Skill observations. Capture produced 23
  candidates; explicit task-owner end and cross-session Knowledge reuse passed.
  At 19:22:16 UTC the audit chain verified through sequence 655, hash
  `5b87c20f708fe946584865b42e5b7ead041021b9905dec682f4735b1c206c849`.
  The Session-filtered history included open, Capture and end events.
- Seven Codex adapter tests pass with no skipped cases using the installed
  TypeScript compiler and Node runner. New manual fixtures and the content-free
  `adapters/codex/fixtures/recovery-qualification.json` are pinned in the registry;
  the earlier captures remain unchanged. No Rust, dependency, schema or public
  API changes were needed.
- Canonical retained-data Compose smoke passed after the fault was restored.
  All nine conformance/fixture checks, formatting, dependency direction,
  generated support, backlog, ADR and documentation gates pass. Strict Clippy
  was not rerun because no Rust crate changed.
- The initial SDK invocation stopped before running when the auditor's
  Keycloak session had expired. Both synthetic profiles completed ordinary
  browser/CLI PKCE renewal using the existing Compose acceptance image; the
  subsequent SDK invocation passed.

The automatic-compaction attempt used the documented
`model_auto_compact_token_limit=1000` override on `exec resume`. It completed a
normal turn but emitted no compaction hooks, so it is not a passing automatic
compaction test. Live revoke/re-authorisation, non-text results, packaging and
other versions/platforms were not exercised. Full CI/database and fresh Compose
acceptance were not repeated for this adapter-only correction. Codex remains
`captured` pending complete qualification.

## Completed Codex qualification (2026-09-12)

Product code stayed at `8e903583e58e85bce49cc2e8146ddbe6fba5b2c7` throughout
the qualifying run. The new digest-pinned
[`live-qualification.json`](../adapters/codex/fixtures/live-qualification.json)
records native Codex 0.152.0/GPT-5.5 low on macOS arm64 with ordinary Keycloak
profiles and the retained canonical Docker deployment.

- The first interactive Skill-read turn with a temporary 1000-token threshold
  emitted automatic PreCompact/PostCompact, compact SessionStart and Stop.
  Fresh allowed context and the original task ID were returned. Eight native
  frames and seven transcript records are pinned and replayed alongside manual
  compaction; all eight adapter tests pass without skips.
- On the same task, authenticated MCP recall, native exit/resume and outage
  recovery passed. Four pending events survived the paused gateway and arrived
  once after restoration. Both SDKs then retrieved context/approved Skill,
  submitted idempotent pending proposals, denied the foreign workspace and
  correlated audit. Native resume after the SDK handoff kept the task identity.
- All 19 unique events persisted. Capture completed with 17 candidates, the
  task owner explicitly ended the Session and a new Session reused Knowledge.
  Audit verification passed through sequence 858 with hash
  `6ba873fee702f21d7b2107c6d8ed094133453ff4d85c820616c13f8700540697`.
- A companion live Python/public-API probe kept its bearer and client unchanged
  through deny/allow/revoke/re-authorise/deny for an existing synthetic foreign
  Session. Only new disposable grants were revoked; original grants are
  unchanged. Grant and denial responses correlate with content-free audit.
- A second turn at the artificial threshold repeatedly compacted and was
  interrupted after 58 seconds. Normal-settings resume then completed recall;
  the interrupted turn is not a passing MCP probe. The first revocation run
  restored grants but used an incorrect test-only audit filter (`since` instead
  of `from`); the corrected run passed. No product change addressed either probe.
- Strict TypeScript compilation, all eight adapter tests, all ten conformance/
  fixture checks, formatting, dependency direction, generated support, backlog,
  ADR and documentation gates pass. The first projection check caught the
  stale README summary; rendering it from the registry corrected the drift.
  Canonical retained-data Compose smoke passes after gateway restoration.
  Full CI/database and fresh-project Compose acceptance were not repeated.
  Strict Clippy was not rerun because no Rust crate changed; no focused test
  was skipped in the successful invocations.

This completes CPR-39's applicable lifecycle criteria for the named setup;
the registry and generated support views now say `verified`. Non-text results,
automatic Skill activation, packaged installation and other versions/platforms
are not inferred. Reproduction steps are in
[the Codex guide](integrations/codex.md#reproduce-the-qualification-boundaries).

### ADPT-4 installed archive checkpoint (2026-09-12)

Started from `265eb6b7ff9f3c902558f0cc4fdaf71a5b159390` on
`codex/authenticated-client-interoperability`, after committing the native
qualification. The npm failure above was reproduced without generated `dist`
files. The correction uses npm's existing build lifecycle. Python retains its
existing Hatchling backend; adding it to the development lock enables offline
checks. New build-only dependencies are Hatchling (MIT), Pluggy (MIT) and Trove
Classifiers (Apache-2.0); existing locked versions/hashes remain unchanged.

`make sdk-package-check` runs bounded clean builds and separate offline consumers.
Python builds a wheel from its sdist and checks `py.typed`, generated imports
and the OpenAPI digest without the generator/backend in the consumer. TypeScript
checks the installed public exports, positive/negative consumer typing and a
seven-file archive that excludes tests/workflow code. Both reuse their existing
nine wire tests. Each archive is built twice and compared byte-for-byte.

| Executed environment | Result |
| --- | --- |
| Pinned Docker Linux arm64 Node 22.23.2 / Python 3.11.16 | Nine tests per installed SDK passed, zero skips; exports/types/resources, offline installs and matching clean builds passed. Source and prepared wheels were mounted read-only; package checks ran with networking disabled. |
| Local macOS arm64 Node 24.18.0 / Python 3.14.6 | `make sdk-package-check` passed the same 18 tests and archive assertions, zero skips. Hashes also matched the minimum-runtime containers. |

Archive SHA-256 values for this source and fixed Python build epoch:

| Archive | SHA-256 |
| --- | --- |
| npm | `938bc0053e861691090ee0e1725049cdfe786c256c8457147af1895befc74b43` |
| Python sdist | `dc78f473e2ecf199abe3e282d13de5f07c7361fde9ac721c8fdc2a1045f284af` |
| Python wheel | `96bee3f2668726d793a052bf4c60b883563456585ef7ae64dd789e268c9b1776` |

The initial offline Python build was blocked by an uncached Hatchling dependency.
The prepared hash-locked wheelhouse resolved that prerequisite. The first local
Node suite was blocked by sandbox loopback restrictions; the first Python Docker
install was blocked by a non-executable temporary mount. Both were rerun with
the required test permissions and passed. Those failed attempts are not passes.
The new staging harness also initially resolved a pnpm bin shim relative to its
temporary location; it now references the locked compiler directly.

SDK generator drift (15 operations, 52 schemas), repository formatting,
dependency direction, generated console types, backlog, ADR status and docs
checks passed. Python formatting, JavaScript syntax and the CI YAML/invocation
check also passed. Reproduction and minimum-runtime Docker commands are in
[the SDK guide](../sdks/README.md).
The existing CI SDK job now prepares locked wheels and invokes the same target;
that remote Linux amd64 job was not run here. Full CI, fresh database, live
Keycloak and Compose lifecycle suites were not rerun for packaging-only changes;
their prior evidence remains dated above. No Rust changed in this batch.

### OPS-8/CPR-39 harness archive checkpoint (2026-09-12)

Local harness packaging started at
`d17bd4d3e37bc65d2e416e43e8b374d3306d8b43`. The existing release archive contains
the unchanged Claude marketplace and now adds 17 Codex runtime/manifest files,
including its existing shared Session runtime. The runtime resolves inside the
extracted archive with no checkout dependencies, compiler or package install.
ADR-0065 amendment 9 records the packaging boundary.

`make plugin-package-check` passed all eight captured lifecycle/reader tests on
macOS arm64 Node 24.18.0 and pinned Docker Linux arm64 Node 22.23.2. Both had
zero skips; Docker mounted source read-only and ran with networking disabled.
The tests exercise task identity, context, local persistence, outage/recovery,
manual/automatic compaction, MCP text results and the bounded native-exit budget.
They are captured replay against a synthetic HTTP fixture, not a new native
Codex or live Keycloak run. Production module bytes are compared with the built
adapter before test helpers are added outside the archive. An initial staging
error omitted a dependency of the transcript test helper; both corrected runs
passed. The failed attempts were not counted as passes.

The existing installer tests now include Codex bytes in initial/repeated installs
and upgrades, preserved user-owned Codex configuration, and pre-mutation refusal
of a checksum-valid archive missing the shared runtime entry point. CI and the
release job invoke archive replay after building both adapters. No dependency,
protocol, domain API or Rust implementation was added. Publication, native
execution from an installed release, broader client/platform versions and the
remote CI/release jobs remain unverified in this batch.

Validation: all 16 release-parity/installer tests, ten adapter-conformance
tests, formatting, dependency direction, generated types, backlog, ADR status,
docs and shell/JavaScript/YAML checks pass. The full `make check-deploy` did
not finish: both runs stopped reporting before the interrupted-build result
and were terminated. The case alone and its timeout/interruption pair pass;
this does not substitute for a passing full gate. The exact blocker and next
action are in [CPR-45](backlog/CPR-45.md#deterministic-gate-progress-2026-09-12).
No full CI/database or live Keycloak/Compose lifecycle was rerun for this batch.
The remaining deployment convergence/uninstall suite passed 48 checks in the
restricted invocation; its one loopback-listener case was blocked by `EPERM`
and then passed with that permission. The final static convergence check passed.
Those results did not establish a passing full lifecycle gate.

### CPR-45 deployment gate progress (2026-09-12)

At `f06926a`, the unchanged lifecycle suite passed all 89 tests in 360 seconds
on macOS arm64 Node 24.18.0, using a 30-second per-test deadline. The apparent
stall was delayed reporting: the interrupted-build case exited in 2.5 seconds,
while later synchronous fixtures kept the child-exit callback occupied and
buffered its result. A process sample and progressing backup/restore/upgrade
children distinguished this from a stuck Compose process. Both interruption
cases also passed together. The earlier terminated gates remain incomplete.

A single `afterEach` event-loop yield now lets the report flush between
fixtures. No product script, assertion, test selector, deadline or cleanup
contract changes. The complete unfiltered `make check-deploy` passes all 342
tests, including the 169 combined script checks, with zero failures,
cancellations or skips. Compose rendering and final deployment convergence
pass, and the report advances between cases while the suite is running.
Formatting and backlog/ADR/docs checks also pass. No Rust changed; strict
Clippy is not applicable. Full CI, database tests and live deployment
acceptance were not rerun for this test-only fix.

The four focused build/failure/interruption cases also pass with zero skips on
pinned Node 22.23.2/Linux arm64 in Docker, using the image's ordinary `node`
user, no network, a read-only checkout and disposable temporary state. The
first root-user invocation failed all four checks at or after Compose's existing
non-zero UID/GID refusal; it is not counted as a pass. No runtime-user rule was
changed to obtain the successful rerun.

## Remaining actions

The CPR-45 deterministic-gate checkpoint is closed. Wider live reference
acceptance remains in its open brief.

1. Qualify the installed Copilot CLI 1.0.83 from its actual contract and authentic
   frames. `copilot --version` confirmed it during the archive batch; no Copilot
   authentication, protocol or lifecycle test has run yet and no registry entry
   is inferred. Pi was absent from PATH. Reuse the current public API and MCP
   implementation; add translation only when authentic evidence requires it.
2. Resolve ADPT-4's existing package ownership/licence, signing/provenance,
   runtime/server matrix and release ownership before public distribution.
   Local archive build/install/import is verified; packages remain unpublished.
3. Native Codex execution from a published installation, non-text results and
   other client versions/platforms require their own evidence. Generic
   MCP/Skills compatibility and archive replay do not qualify those workflows.

These continue the original client batches; no new orchestration, plugin
system or protocol implementation is proposed.

### Current local deployment

The owned hosts handoff is complete. `synveda-development-acceptance-interop`
uses `10.231.46.0/24` and the `demo` profile after the documented retained-data
rebuild. Its public UI is `http://app.synveda.test:8080/console/`. Use:

```sh
SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-interop SYNVEDA_COMPOSE_IPV4_POOL=10.231.46.0/24 SYNVEDA_COMPOSE_PROFILES=demo make compose-smoke
SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-interop SYNVEDA_COMPOSE_IPV4_POOL=10.231.46.0/24 SYNVEDA_COMPOSE_PROFILES=demo make compose-down
```

Follow `deploy/compose/README.md` for any later profile/hosts handoff. Retained
profile-owned volumes must remain in the shutdown contract even when a profile's
container is absent. No `compose-reset` is needed for an ordinary transition.

Minimum-runtime image digests used for SDK runtime and installed-archive evidence:
`node@sha256:7725a5c2c83eed1d36258c66efae14b1ceccd021db9ed1d9559d3335ed3d68ed`
and `python@sha256:9534e5a8e315485d4061ed659af0fd78a284c015f9b73661b41d6bab25604534`.
No package publication, HA, SaaS readiness or broader certification follows
from this single-host workflow.
