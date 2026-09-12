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

Status: initial base slice implemented and tested; live Keycloak acceptance
is blocked. ADPT-4 remains open for release and wider compatibility obligations.

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

Status: authentic Codex CLI 0.152.0 MCP protocol captured and replayed.
Lifecycle qualification remains blocked; CPR-39 stays open.

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
| P1: base SDKs absent | Inspected baseline `sdks/`, workspace manifests, generated OpenAPI and console API client. Python/TS applications otherwise hand-built public HTTP calls. | Checked OpenAPI, existing TS generator, maintained Python type generator, HTTPX/Fetch and public domain APIs. | Generate only 15 operations/52 schemas; add bounded auth/error/correlation clients and equivalent examples. `crates/synveda-gateway/tests/support/sdk_interop.rs` runs both SDKs with real gateway policy, Claude hook context and a valid audit chain. | Initial slice passes; package release and Keycloak qualification remain open. |
| P2: observation acknowledgement | `crates/synveda-cli/src/mcp.rs`, `render_remember` and `remember_tool`. Successful append incorrectly promised extraction/recallable Knowledge. | Existing Session observation disposition and separate Capture/VedaFlow paths. | Correct acknowledgement/help only; unit and real stdio tests distinguish recorded, duplicate, quarantined and refused observations. | Fixed; tests pass. |
| P2: Codex qualification | Baseline registry had no Codex lifecycle entry; `.codex/skills` existed. Configuration/Skill output alone did not establish interoperability. | Existing stdio server, `capture.sh`, MCP corpus and conformance registry. | Capture real 0.152.0 frames in `crates/synveda-cli/fixtures/mcp/codex.json`; replay them and record `captured`. Keep clients without an installer out of generated installation choices. Full lifecycle still requires authentic hooks and live acceptance. | Protocol works; lifecycle unverified. Both Skill roots work, so no Skill-path change. |

| Area | Classification after this slice | Evidence / boundary |
| --- | --- | --- |
| Synveda MCP server | Implemented and tested | `crates/synveda-cli/src/mcp.rs`; authentic and specification corpus in `crates/synveda-cli/tests/mcp_corpus.rs`. Stdio legacy/modern protocol and recall/remember remain maintained-library paths. This does not add a remote HTTP MCP endpoint. |
| HTTP contracts and language SDKs | Partial | Public catalogue/OpenAPI/console peer tests pass; the new Python/TS base slice passes. Broader ADPT-4 release coverage is open. |
| Skills import, validation, approval, export/install | Implemented and tested | Existing public Skill service, CLI and `crates/synveda-gateway/tests/skills.rs`; actual CLI materialisation/revocation now covered. No registry rebuild. |
| Context, observations and proposals | Implemented and tested | Existing Session/Context/Knowledge routes; Session suite, Claude lifecycle replay and shared SDK acceptance. Pending proposals remain outside Knowledge. |
| Authentication, policy and audit | Implemented but unverified as a complete live OIDC workflow in this run | Ordinary authenticated test identities prove policy, workspace denial and audit correlation/hash chain. Maintained Keycloak/OIDC implementation remains; current Compose inventory blocks its live qualification. |
| Examples, setup and compatibility | Partial | `sdks/README.md`, equivalent runnable examples and `docs/integrations/codex.md`. Claude replay and Codex protocol pass; native lifecycle limits remain explicit. |
| External MCP servers | Implemented but unverified in this focused run; gateway execution deliberately unsupported | Existing trusted server/version/binding catalogue and bounded discovery: `docs/INSTALL.md` and inspected `crates/synveda-gateway/tests/tools.rs`. This suite was not rerun. The gateway does not execute imported commands. External server management was not introduced as a prerequisite for Synveda serving MCP. |

## Validation checkpoint (2026-09-12)

| Check | Result |
| --- | --- |
| `cargo test -p synveda-cli` | 182 unit tests, 3 CLI integration tests and 5 corpus tests pass; corpus includes the real Codex exchange. |
| `cargo clippy -p synveda-cli -p synveda-gateway --all-targets -- -D warnings`; formatting | Pass. No new production unsafe code, recursion or panic-based boundary. |
| Claude adapter test suite | 104 pass. |
| `scripts/interop-mcp.test.mjs` | 3 pass with real stdio and synthetic HTTP replies; gateway policy is tested separately. |
| `make sdk-check` | Generated drift check and eight tests per language pass: shared wire fixtures, safe refresh/replay, errors/redaction, response bounds and cancellation. |
| Docker exact-role gateway suite | 33 pass across Skills, Sessions, OpenAPI and Claude replay. Two tests are explicitly ignored by the ordinary invocation: the SDK workflow and native Claude run. The SDK workflow was then explicitly run and passed; native Claude remains blocked. |
| Shared workflow | Pass against the actual gateway and fresh exact-role Docker schema, using ordinary tenant identities and maintained policy packs. One Claude-created Session is shared with Python and TS; allowed context, approved Skill, pending/idempotent proposal, foreign-workspace denial, W3C correlation and content-free valid audit chain are asserted. Synthetic Hs256 test identities are not Keycloak evidence. |
| Local Python package | Wheel/source build and fresh-environment wheel import pass, including generated models, operations and contract metadata. |
| Repository gates | Dependency direction, adapter evidence, generated API, backlog, ADR, docs, npm licences, deployment convergence and Compose render matrix pass. Existing authentic client requests/provenance were preserved; only changed server expectations were regenerated. |
| Canonical `make compose-smoke` | Blocked, exit 78: retained `synveda-development-acceptance-e2e` container inventory is incomplete. |
| Canonical `make compose-acceptance` | Blocked, exit 78: project containers were not initially absent. Hosts, resolver, secrets and issuer preflights passed; browser/Keycloak acceptance did not run. |
| Native Claude preflight | Blocked: installed client exits 1, reporting expired OAuth that cannot refresh. Its full live lifecycle test is not reported as passing. |
| Native Codex 0.152.0 | Real initialization, discovery and recall captured; recall reached Synveda's sign-in refusal. Three isolated headless configurations emitted no lifecycle hooks. No complete authenticated lifecycle claim. |
| Not run | Full workspace `make ci` / full database suite, live Keycloak SDK scenario, other OS/runtime matrices, and Copilot CLI/Pi qualification. Their executables were unavailable. |

The exact-role fixture was reused between focused runs to avoid rebuilding it;
its two containers, two volumes, four networks and private credential directory
were removed after successful checks. The pre-existing Compose deployment was
preserved. Initial assertion/build-fixture failures were corrected and rerun;
blocked or ignored tests were never counted as passing.

## Remaining actions

1. Prepare the documented fresh Compose acceptance project with its owned hosts
   mapping and ordinary Keycloak identities. Run the same SDK scenario there;
   do not reset retained deployment assets to manufacture acceptance evidence.
2. Capture the installed Codex lifecycle through its normal trusted-hook review
   flow, then implement only the observed host translation and satisfy every
   ADR-0098 criterion. Copilot CLI/Pi remain later independent candidates.
3. Complete ADPT-4's existing package ownership/licence, release provenance and
   tested runtime/server compatibility decisions before public publication.

These are the remaining work in batches 2 and 3, not new implementation batches.
