# ADPT-4: Python & TS SDKs

## Problem and evidence

The initial slice in `sdks/` now generates 15 operations and 52 schemas from the checked OpenAPI, with bounded HTTPX/Fetch clients, nine tests per language and a shared real-gateway acceptance workflow. Claude hook replay and both SDKs share authenticated Session identity, approved Skill bytes, pending proposals, workspace denial and correlated content-free audit. Installed packages expose verified build metadata, with two measured runtime pairs. Public publication, wider operation coverage and the public runtime/server support policy remain open. The shared Keycloak workflow passes. Framework-specific shims are not a substitute for supported base clients.

## Scope

The authorised initial slice and execution checkpoints are in
[the interoperability plan](../INTEROPERABILITY_PLAN.md), under ADR-0106.
It adds approved Skill retrieval and typed Knowledge proposal submission to
the shared authenticated scenario without closing the wider release scope.

- Generate or mechanically derive typed Python and TypeScript clients from the checked OpenAPI contract, then add a small maintained ergonomic layer.
- Cover OIDC bearer injection/refresh hooks, sessions and ordered events, context runs, Knowledge query, capture, keyset pagination, idempotency keys, typed errors, timeouts, cancellation, and user-agent/version reporting.
- Retry only operations whose public idempotency contract makes replay safe; expose response/audit correlation identifiers without logging content or credentials.
- Ship equivalent cross-session Knowledge examples and contract fixtures for both languages.
- Define supported language/runtime versions, package versioning, contract compatibility, release provenance, and deprecation policy.

## Non-goals

- Embedding Cedar, SQL, Synveda domain storage, model/tool execution, or adapter lifecycle logic in an SDK.
- Direct database access, static API-key invention, or retries for non-idempotent calls.
- Claiming LangGraph, LlamaIndex, Semantic Kernel, or gRPC support from the base clients alone.
- Hand-maintaining DTOs that can be generated from the authoritative contract.

## Architecture seam

Both SDKs are public REST clients over the generated OpenAPI schema and generated operation identifiers. Language-specific convenience types wrap, but do not fork, wire DTOs. Authentication stays pluggable at the HTTP boundary; the gateway remains the only PDP, RLS, VedaFlow, and audit enforcement point.

## Acceptance criteria

- Python and TypeScript complete the same create-session, append-events, run-context, capture/accept, end-session, and cross-session Knowledge scenario.
- Golden fixtures prove byte-equivalent paths, query encoding, pagination, errors, and idempotency semantics for both clients.
- Token refresh, timeout, cancellation, rate-limit, transient failure, and retry-after handling never expose credentials or duplicate governed effects.
- Package builds are reproducible, signed/provenanced, and tied to a checked OpenAPI digest and compatible server range.
- Published documentation states supported runtimes, authentication prerequisites, limitations, and upgrade policy.

## Required tests

- Generated-source drift and API-operation coverage checks.
- Unit tests for auth injection, redaction, pagination, time/date/enum encoding, error mapping, timeout, cancellation, and safe-retry classification.
- Shared mock-server golden suite run by Python and TypeScript.
- Live gateway end-to-end scenario with ordinary OIDC identities, deny/revoke, and idempotent replay.
- Package install/import smoke tests on every supported runtime and platform.

## Rollout and rollback

Publish pre-1.0 prereleases against a pinned server version, run the shared conformance suite, then promote only supported combinations. Rollback yanks or deprecates a broken package version without deleting it, publishes the fixed successor, and leaves the server contract unchanged; server-side feature discovery must let older clients fail clearly.

## Dependencies

The owner must approve Synveda's repository/package licence, PyPI/npm namespaces, signing/provenance custody, supported runtime matrix, release ownership, and compatibility window before public distribution. The clients depend on stable generated OpenAPI and test OIDC credentials. gRPC support, if accepted under ADPT-3, is separate.

### Release decision proposal

Prepared on 2026-09-19 against `09ff50a05a526e0405460e46cb126fe1bbe9ee2e`. This is a proposal
for the first SDK prerelease, not an accepted support policy or permission to
publish. Local packaging and the authenticated workflow already work; reuse
them unchanged. No additional adapter or public API expansion is required for
this release slice.

Read-only inspection confirms that `synveda/synveda` is public. Its latest
published product release is `v0.2.0`, whose tag resolves to
`92ffa890ee330eb31bce71d5fba08624dcd88a22`. That tree has neither
`docs/api/openapi.json` nor the current `session_api.rs`/`context_api.rs` modules.
The checkout also calls its API `0.2.0`; that number alone cannot identify a
compatible server. No test against the published product was run.

| Decision | Proposed smallest choice | Evidence / remaining owner input | Acceptance before public distribution |
| --- | --- | --- | --- |
| First-party terms | Apply the owner's exact SDK licence and notices, explicitly identifying whether they cover only `sdks/` or the repository. No licence is selected by this proposal. | No root `LICENSE`; `README.md` records the unresolved terms; `crates/synveda-gateway/src/openapi.rs` says `Proprietary`; neither SDK manifest specifies a licence. | Owner supplies approved terms and copyright holder. Both installed archives contain the required text and matching metadata; regenerate OpenAPI only if the approved repository terms change its source annotation. |
| Names and ownership | Retain `@synveda/sdk` and `synveda-sdk` if the owner controls their registry namespaces. | Both anonymous package metadata requests returned HTTP 404 on 2026-09-19. That establishes neither availability nor ownership. GitHub repository administration does not establish npm/PyPI rights. | Named npm scope administrator, PyPI project owner and release maintainer confirm access and the first-publication setup; verify access through the registries without recording credentials. |
| First candidate and server boundary | Use npm `0.1.0-rc.1` and Python `0.1.0rc1` as the same candidate, tied to one exact server source/image and checked OpenAPI digest. SDK versions remain independent of the product version. | Existing SDK version is `0.1.0`; generated API version is `0.2.0`. Current digest and tested environments are in the [SDK guide](../../sdks/README.md#compatibility-and-release-boundary). A releasable server candidate has not been selected. | Version/digest drift checks, installed-package tests and the shared authenticated workflow pass against the named candidate. Do not advertise compatibility with the old `v0.2.0` release from its version label. |
| Publisher identity | One SDK-only GitHub Actions workflow in this repository, named `sdk-release.yml`, with an `sdk-release` environment and registry OIDC trusted publishers. Keep its candidate tags outside the product workflow's `v*` trigger, for example `sdk-v0.1.0-rc.1`. | Existing `release.yml` publishes product artifacts and checks their version against Cargo; it has no SDK publisher. Owner must name the registry/account recovery custodian and environment reviewer. These proposed workflow/environment names are not configured. | A credential-free dry run produces the same tested archives. Only the publish jobs receive OIDC permission. Registry attestations identify the expected repository, workflow, commit and archive digest; a fresh consumer verifies and installs the downloaded bytes. |
| Runtime and change policy | Initially qualify the existing Node 22/Python 3.11 and Node 24/Python 3.14 CI pairs on native Linux amd64; keep the measured macOS/Linux arm64 results labelled as supplemental evidence. Claim only exact candidate/server combinations until more are tested. | CI declares both pairs; their remote runs remain unverified. Package minimum versions do not prove every newer runtime. Owner may adopt this deliberately small evaluation window. | Both native CI rows pass without skipped SDK checks; one ordinary Keycloak workflow passes per server candidate. Before 1.0, document breaking changes in a new minor candidate, keep patch releases compatible with the advertised contract, and deprecate/yank defective versions rather than silently replacing bytes. A stable release needs a separately approved support/deprecation window. |

Use maintained registry tooling. npm trusted publishing requires a supported
GitHub-hosted runner, npm CLI at least 11.5.1 and Node at least 22.14.0; these
are publisher requirements, not new SDK consumer minimums. Configure the exact
repository/workflow/environment and explicitly allow the selected publishing
action. OIDC publication of a public package from this public repository can
generate provenance automatically. See [npm trusted publishing](https://docs.npmjs.com/trusted-publishers/).
The npm manifest must also name the exact public repository; see
[npm provenance prerequisites](https://docs.npmjs.com/generating-provenance-statements/).

For Python, reuse PyPA's maintained publishing action with job-scoped OIDC
permission and its attestation support; do not implement token exchange or
signing. See [PyPI publishing](https://docs.pypi.org/trusted-publishers/using-a-publisher/)
and [attestation production](https://docs.pypi.org/attestations/producing-attestations/).
A PyPI pending publisher can create the first project but does not reserve its
name. See [PyPI first publication](https://docs.pypi.org/trusted-publishers/creating-a-project-through-oidc/).
The npm first-publication bootstrap must be confirmed with the scope owner;
do not assume it has PyPI's pending-publisher mechanism. No registry account,
publisher or environment was changed during this inspection.

After those decisions, use at most three implementation batches:

1. **Package metadata and terms.** Amend ADR-0106 with the accepted choices;
   add approved licence/notices, repository/readme metadata and candidate
   versions through the existing manifests/generator. Extend the existing
   archive checks to prove those files and metadata survive installation.
2. **Build and publish one candidate.** Reuse `sdk-check`, `sdk-package-check`,
   their locks and the existing shared gateway acceptance. Keep build/test
   jobs separate from the minimal OIDC publish jobs, use immutable action
   references, publish the validated bytes, and retain their digests and
   attestations. First complete the non-publishing workflow run; registry
   configuration and actual publication follow the approved owner decisions.
3. **Verify distribution and document support.** Download the exact registry
   versions into empty consumers, verify provenance/digests, run the existing
   package suites and the shared allowed-context/approved-Skill/proposal/
   workspace-denial/audit workflow against the selected server. Publish only
   the resulting compatibility table and rollback instructions. Package
   provenance does not close product-image signing or production readiness.

**Current checkpoint (2026-09-12)**

`make sdk-check`, full CI and the fresh exact-role database suite pass at
`8f40237`; the explicit shared gateway workflow passed in that implementation
slice. Both SDKs additionally passed all eight tests on their minimum runtimes
in pinned Linux arm64 containers: Node 22.23.2 and Python 3.11.16. These are
runtime tests, not OIDC evidence or a complete platform support matrix.
Fresh Keycloak acceptance passed after the owned hosts handoff. Native Codex
and both SDKs then completed the shared authenticated workflow through the public
proxy on Session `01a096b9-e1c1-7a50-aa32-8c1db0efdba8`. Each retrieved allowed
context and the exact approved Skill, submitted an idempotent pending proposal,
received cross-workspace denial and correlated content-free audit. The gateway
returns its actual response trace ID because the proxy removes incoming trace
context; SDK success/error results prefer it. Nine tests per SDK and all 20
focused fresh exact-role DB tests pass. Capture, explicit task-owner end,
cross-session reuse and the audit chain also passed. The content-free result is
in `adapters/codex/fixtures/keycloak-qualification.json`.

CPR-33/ADPT-4 now corrects the Session audit filter through the existing shared
query. It matches lifecycle snapshots and delivery identities before pagination;
42 focused exact-role DB tests pass, including combined filters, Session/tenant
isolation and unchanged canonical hashes. Canonical Compose up/smoke and live
Keycloak queries returned all 23 saved events, including open/end, across 12
pages with unchanged prefix hashes and continued workspace denial; see
`docs/INTEROPERABILITY_PLAN.md`. Package ownership, licence, release
provenance and broader runtime/server decisions still require resolution before
publication.

The completed CPR-39 run at `8e90358` adds automatic compaction and native
outage/recovery on the same task used by both SDKs, plus live public-API grant
revocation/re-authorisation with one unchanged Python client and bearer.
The fixture `adapters/codex/fixtures/live-qualification.json` records 19 unique
events, 17 Capture candidates, explicit end, reuse and audit verification
through sequence 858. Existing product policy and SDK code needed no change.

The package check from `265eb6b` found that a clean `npm pack` included only
`package.json`: importing the installed SDK failed with `ERR_MODULE_NOT_FOUND`.
The standard `prepack` hook now runs the existing compiler and packages only
runtime/type outputs. The Python wheel already worked; its existing Hatchling
backend now joins the hash-locked development dependencies for offline checks.
`make sdk-package-check` builds each archive twice, installs into isolated
consumers, verifies public types/resources and reuses all nine tests per SDK.
It passed on pinned Docker Linux arm64 Node 22.23.2/Python 3.11.16 and macOS
arm64 Node 24.18.0/Python 3.14.6, with identical archive hashes and no skips.
CI now invokes the same check; that remote job and the broader CI/database
suites were not rerun for this packaging-only batch. Exact hashes, corrected
environment failures and reproduction are in `docs/INTEROPERABILITY_PLAN.md`
and `sdks/README.md`.

**Compatibility increment (2026-09-19; starting at `06bcc096f7d56d5728e2522c21265c94fa07836e`)**

The generated contracts already carry the OpenAPI digest, but both HTTP clients
hard-code SDK version `0.1.0` in their identification header. Public imports do
not expose a package/API/contract tuple, and CI tests only Node 22/Python 3.11.
A package version bump could therefore misidentify the installed client.

Under the amended ADR-0106, derive the two package versions and OpenAPI version
through the existing generator, expose the three metadata constants and verify
the header through the installed-package wire suites. Add Node 24/Python 3.14
to the existing CI job while keeping real gateway acceptance on the minimum
pair. Record exact measured runtimes/platforms; no compatibility range or public
support promise is inferred. Reuse the package checks, their hash-locked inputs
and the documented offline Docker path.

Acceptance passed: isolated TypeScript/Python package-version and API-version
mutations each fail the existing generator drift check; unchanged inputs pass.
Installed public metadata equals each manifest and current OpenAPI, and actual
HTTP headers use the SDK version. Nine tests per SDK pass on pinned offline
Docker Linux arm64 Node 22.23.2/Python 3.11.16 and host macOS arm64 Node
24.18.0/Python 3.14.6, with zero skips and identical archive hashes. No API
expansion, protocol negotiation or framework adapter was needed.

CI declares both runtime pairs on Linux amd64, but its remote execution remains
unverified. SDK drift, formatting, dependency direction, adapter conformance,
generated console types, docs, backlog, ADR and CI YAML checks pass. Full CI,
fresh database, live Keycloak and Compose acceptance were
not rerun for this metadata/header change; prior authenticated evidence remains
dated to its original source. No Rust changed, so strict Clippy is inapplicable.
The [SDK guide](../../sdks/README.md#compatibility-and-release-boundary) records
the exact contract and measured matrix, separate from public support policy.

**Release preparation checkpoint (2026-09-19; starting at `09ff50a`)**

Both existing archive checks now also pass in pinned offline emulated Linux
amd64 containers: Node 22.23.2 and Python 3.11.16, nine tests each, zero skips.
Installed metadata, types/resources, headers and two clean builds pass; all
three archive hashes match the native arm64 results above. This is supplemental
package evidence, not a native CI, live OIDC or published-install qualification.
The public repository, latest product tag and anonymous registry-name reads
were inspected for the proposal; no registry or workflow settings were changed.
Exact images and reproduction are in the SDK guide; the interoperability plan
records the check limits. The owner question for licence terms and named
registry/release ownership remains pending.

Next: resolve the concrete choices in the release decision proposal above,
starting with licence terms and registry/release ownership, then execute only
the selected release path and support policy. No public package was published.
ADPT-4 remains open for those decisions and its wider scope.
