# Feature inventory

146 features in this index. This file is authoritative for feature identity,
phase and delivered/open state. Delivered names identify historical slices;
current contracts live in code, generated artefacts and accepted ADRs, while git
retains their implementation evidence. Open entries link to current briefs.

114 delivered; 32 open. The inventory and open-brief shape are checked in CI.

CPR-45/OPS-11/OPS-8/FND-7 prepare the **unpublished 0.4.0 installation
increment**, separately from v0.3.0, whose publication workflow failed because
its release page already existed. The 0.4.0 source and tag are pushed at
`e59284619567d6a13b70ce3f3b3e81121b7621e6`; all 13 hosted CI jobs and the release
packaging dry run passed. The [tagged release workflow](https://github.com/synveda/synveda/actions/runs/35525132082)
passed builds, assembly and anonymous image checks on both architectures.
Completion of its native installation gates and publication remains unverified.
Next: verify the complete public assets and reports, then update installation
publication metadata, platform evidence and matching Pages copy. The existing release
bundle gains containerized loopback preparation and governed optional sample;
the existing chart gains operator-free persistent PostgreSQL and independent
identity ownership. README/site preserve the brand and label unpublished
commands. ADR-0115 records the decision. Full candidate Docker recovery and
Kubernetes dependency/operational drills now gate release announcement after
anonymous pulls on both native architectures. Local Docker lifecycle/fault
qualification and all four packaged-chart ownership modes passed, including
real loopback browser access, retained reinstall and paired recovery. Exact
measurements and remaining blockers live in one
[CPR-45 record](CPR-45.md#installation-mission-2026-09-20).
Publication and unavailable platform/N-1 qualification keep these deployment
features open; local checks do not close production readiness gaps.

CNSL-5 aligns the console with the approved brand and simplifies Home,
navigation, connection onboarding and the root README under
[ADR-0114](../adr/adr-0114-console-brand-and-navigation.md). All 256 console
tests pass. The production bundle is byte-identical on macOS/Node 24 and Linux
arm64/Node 22, including the Docker console stage. Chromium checks cover
320/390/768/1440 px, both themes, keyboard navigation, skip links, detail-page
selection, blocked/sign-in/first-run states and reconnecting the selected project.
The 38 audited states have no detected WCAG 2 A/AA or 2.1 AA violations; they
use synthetic public-API responses, not a live deployment or user study.
Website, documentation, adapter, dependency, licence and deployment-contract
checks pass. The loopback-dependent convergence suite needed a rerun outside
the filesystem/network sandbox; the other deployment suites had passed.
No backend acceptance rerun, application redeployment or GitHub push is claimed.

FND-7 delivers the approved public website, developer README and canonical
brand assets under [ADR-0113](../adr/adr-0113-static-public-site-and-brand.md).
The isolated static build, reproducible exports on macOS/Node 24 and Linux
arm64/Node 22, browser/keyboard checks at 320–1440 px, accessibility audit,
licence and documentation gates pass. The existing Compose demo passes smoke;
no fresh installation or backend acceptance rerun is claimed. The owner has now
enabled Pages with GitHub Actions. Account image uploads remain manual owner
actions in the [maintenance guide](../../website/README.md).

The first Pages AMD64 build exposed native raster edge-pixel differences that
the two earlier ARM64 checks missed. FND-7 now uses a pinned WebAssembly renderer
and checks both native Linux architectures before deployment (ADR-0113 amendment).
All 11 exports match exactly on macOS ARM64 Node 24, Linux ARM64 Node 22 and
emulated Linux AMD64 Node 22. Both stale-export regression tests, full site checks,
the unchanged licence gate, documentation checks and workflow lint pass.
[Pages run 35511266591](https://github.com/synveda/synveda/actions/runs/35511266591)
then passed on native AMD64 and ARM64 and deployed correction `a2e819e`. All 11
published page/asset files returned HTTP 200 with bytes identical to the local
build at [the live site](https://synveda.github.io/synveda/).

[OPS-11](OPS-11.md) records the Kubernetes deployment milestone under
ADR-0109. The [deployment audit](../../deploy/README.md#kubernetes-release-contract)
reuses the existing Helm chart and owner-validated Docker MVP, keeps application
replicas at one, and orders external services, starter/onboarding, OpenShift,
then operational and published-release evidence. The chart implements external
PostgreSQL/OIDC, verified TLS/private CAs, file-mounted Secrets and a bounded
normal migration/tenant Job. ADR-0110 adds independently optional persistent
Keycloak/CNPG, explicit team admission, scoped agent credentials and an
organisation Configuration target. MEM-7 now refuses overlapping issuer/tenant
bindings; durable federation and issuer replacement remain open. OpenShift and
operational/release qualification keep OPS-11 open; measured acceptance and the
exact next action are in the brief.

ADR-0112 adds the current operability work under OPS-11: existing-job
interruption, native joint logical recovery, migration locking, repeated public
workloads and digest-bound Helm release overlays. The install, configuration
and operations instructions are consolidated. Both starter and external
operational profiles passed, including fresh logical restore, fenced Capture
recovery, migration contention and retained reinstall; exact local limits and
measurements are recorded in the open brief. Public v0.2.0 lacks the current chart/reference artifacts and
predates epoch 3, so a supported published N-1 upgrade remains unavailable;
publication and actual platform qualification still block delivery.
The current installation increment and its publication boundary are recorded
in CPR-45 above. Qualify the declared N-1 pair and actual target platform after
the complete artifacts exist. No schema reset is an upgrade.

ADR-0111 adds the restricted portability increment: assigned-ID contexts,
explicit seccomp, a narrow fsGroup/setgid bootstrap fix, edge Routes with named
certificates, opt-in NetworkPolicies, CA/proxy inputs and a restricted-v3
post-renderer for the unchanged Keycloak dependency. Kubernetes/OpenShift API
schema checks and deployment/Compose regressions are separate from real platform
qualification. No OpenShift or cloud target is configured; OPS-11 remains open
for actual SCC, router, CNI/CSI and provider acceptance. The
[portability guide](../../deploy/helm/synveda/PORTABILITY.md) records the exact
constraints and the next disposable-project checks.

PR #52's Linux schema failure was reproduced as a case-sensitive filename
mismatch. The OPS-11 runner now writes lowercase `route.json` and preserves
validator output on failure. The unchanged strict schema matrix passes on
Linux Node 22 and macOS Node 24: 27/27 resources for each OpenShift target and
7/7 for Kubernetes, zero errors or skips. Chart lint and formatting also pass;
actual platform qualification remains open.

The restricted Kind 0.32.0/Kubernetes v1.36.1 run passed all four ownership
combinations with namespace PSA restricted/v1.33 and simulated UID 1000900000
for the product, migration/bootstrap and Keycloak. Each completed 400 captured
events, 80 context runs, persistence/restart/upgrade/reinstall and revocation;
audit verification reached 526 events. The
[portability report](../../demos/evidence/ops11-portability.json) records the
simulation boundary. The initial setgid failure remains documented alongside
the fix and clean rerun; it is not counted as a passing attempt.

The 2026-09-19 external Kind run passed clean installation, real Keycloak PKCE,
Session/context/audit, verified database TLS with a client certificate, bad
credential/CA/hostname/issuer/audience refusals, runtime restarts and the normal
Helm upgrade/migration rerun. Strict chart/Rust/deployment checks, the exact-role
database suite and existing Compose smoke passed. This one private development
topology does not qualify public HTTPS ingress, cloud, OpenShift, HA or release
publication; OPS-11 remains open for its later slices.

The subsequent starter matrix passed all four PostgreSQL/Keycloak ownership
combinations over HTTPS/private CA: explicit owner, member/viewer/stranger,
cross-workspace refusal, service/MCP access, 400 captured events and 80 context
runs per case, pod recreation, stable upgrade, retained reinstall and unchanged-
token revocation. The [resource report](../../demos/evidence/ops11-starter.json)
records short-run CPU/memory observations, not capacity guarantees. Restricted
OpenShift, real ingress, published pulls and joint restore remain the next
qualification work; issuer migration/linking remains MEM-7.

The 2026-09-12 [interoperability execution plan](../INTEROPERABILITY_PLAN.md)
records ADPT-1/2 and CPR-12/23 repairs, the initial ADPT-4 SDK slice and CPR-39
delivery. Codex CLI 0.152.0 joins Claude Code as a verified lifecycle in the
authoritative registry. Its native Keycloak run covered automatic compaction,
approved Skill reading, MCP recall, outage/recovery and both SDKs on one task:
19 unique events, 17 Capture candidates, explicit end, Knowledge reuse and a
valid audit chain through sequence 858. Disposable public-API grants proved
live deny/allow/revoke/re-authorisation with one unchanged bearer and client.
Eight Codex tests replay the authentic manual/automatic boundaries and retain
bounded delivery and stable task identity. No product code changed for this
qualification; broader CI/database evidence remains dated in the plan.

Under [ADPT-4](ADPT-4.md), clean npm packing now builds its entry point instead
of producing an unusable archive. Installed npm/wheel validation passed nine
tests per SDK on Docker Linux arm64 Node 22.23.2/Python 3.11.16 and macOS arm64
Node 24.18.0/Python 3.14.6, with matching clean-build hashes and no skips.
`make sdk-package-check` also checks exported types and packaged contract
resources; CI invokes it, but the remote job was not run locally.
The 2026-09-19 compatibility increment derives SDK/API versions and the OpenAPI
digest from their existing manifests, exposes them through both public imports
and removes the hard-coded client-header version. Both measured runtime pairs
pass installed metadata/header checks and produce identical archives. Synthetic
package/API version changes fail the existing generator drift gate. CI now
declares both pairs, with gateway acceptance only on the minimum pair; those
remote jobs remain unverified. The [SDK guide](../../sdks/README.md#compatibility-and-release-boundary)
separates the measured matrix from public support commitments.
The owner-authorised Apache-2.0 choice now covers the repository and both SDKs,
with Synveda contributor attribution in root `NOTICE`. Cargo, npm/Python and
generated OpenAPI metadata agree. Existing SDK generation checks the exact
root licence/notice copies; the real npm archive and Python sdist/wheel include
them. All nine installed tests per SDK pass on Docker Linux arm64 and host
macOS arm64, zero skips, with matching reproducible archive hashes. Six OpenAPI
tests and strict workspace Clippy pass; only the API licence metadata changed.
Public distribution still needs verified npm/PyPI publishing access,
provenance custody and the supported server/runtime window.
The [release decision proposal](ADPT-4.md#release-decision-proposal)
now gives concrete choices and three small follow-on batches. At `09ff50a`,
the existing archive suites also pass all 18 tests under pinned offline
emulated Linux amd64, zero skips, with hashes identical to the earlier native
arm64 results. Native amd64 CI remains unverified. Both proposed public package
names returned anonymous HTTP 404; namespace ownership is still unverified.
The published product `v0.2.0` predates the checked contract, so matching version
labels do not establish SDK/server compatibility. No release configuration or
public package was changed.

OPS-8/CPR-39 now includes the compiled Codex hook and its existing shared
Session runtime in the current plugin archive. Eight captured tests pass from
the extracted archive on Node 22/Linux arm64 and Node 24/macOS arm64. The
installer preserves client configuration, replaces the hook on upgrade and
refuses an incomplete runtime before mutation. Native execution from a published
installation, non-text Codex results and other client versions/platforms remain
unqualified. ADPT-9 delivers a verified Copilot CLI 1.0.83 /
gpt-5.6-luna source-build lifecycle on macOS arm64 with the Docker/Keycloak
reference. The clean pair at `c86d5c6` delivered authenticated context at both
native start and resume, retained one task across both SDK workflows, activated
the exact approved Skill and passed MCP recall and foreign-workspace denial.
Eighteen unique events, zero duplicates on public-API replay, 16 Capture
candidates, explicit end, Knowledge reuse and audit verification through
sequence 1179 pass. The last assistant event arrived on a later no-user-prompt
reopen/exit without increased usage counters. All 144 adapter tests (31 Copilot)
pass on macOS Node 24 and pinned offline Docker Node 22, zero skips. Earlier
marker, parser and hook-location failures remain preserved in their fixtures.
CPR-45's confirmed hosts-witness renewal restored Compose/PKCE; current smoke
passes and the earlier deployment checkpoint passed 345 tests without skips.
ADPT-9 also packages the existing runtime through the archive/installer path.
Eight Codex and 23 Copilot tests pass from the extracted archive on host Node
24/macOS arm64 and pinned offline Docker Node 22/Linux arm64 and emulated
x86_64. All 12 installer fixtures pass on host and Docker x86_64, preserving
client configuration and refusing incomplete runtimes before mutation. The
initial Linux arm64 installer run failed ten tests at the deliberately
unsupported-platform boundary; it is not counted as passing. All 347 deployment
tests and static gates pass across completed component runs; the full command's
deadline failure and sandbox socket denial remain explicit in
[the package evidence](../integrations/copilot-cli.md#release-archive-validation).
Native outage/compaction, other result formats/platforms and execution from a
published installation remain unqualified. Pi had no executable in the earlier inventory.
These are independent of the public-release decisions.

The apparent deployment-gate stall was delayed reporting: the unchanged
89-test lifecycle suite passed in 360 seconds, and the interrupted build itself
exited in 2.5 seconds. A test-only event-loop yield now lets results flush between
synchronous fixtures. The full unfiltered `make check-deploy` passes all 342
tests with zero skips plus the Compose render and deployment-convergence checks.
Four focused checks also pass on Node 22/Linux arm64 Docker as an ordinary user.
[CPR-45](CPR-45.md#deterministic-gate-progress-2026-09-12) records the closed
checkpoint and the remaining live acceptance. The two earlier terminated gates
remain incomplete runs, not passes.

FND-1's [CI maintenance decision](../adr/adr-0108-ci-gate-ownership-and-build-caching.md)
assigns deployment checks to one job, removes unused beta/duplicate TypeScript
builds, reuses package downloads and adds scoped Kind image caches. Job names
and distinct acceptance gates remain. Local validation passes the full
351-test deployment gate with zero skips, the final five cache-helper cases,
395 adapter/console tests, 31 installed-hook replay tests and the production
console build. Workflow lint, formatting, demo/dependency and documentation
checks pass. Hosted cache reuse and wall-clock savings remain unmeasured;
the next PR/main runs must compare cold and warm Kind timings with the
29m35s baseline linked in ADR-0108. This does not change readiness claims.

## Phase 0 — Foundation (wk 1)

- [x] FND-1: Workspace scaffold — delivered 2026-07-16; CI gate ownership and build caching: ADR-0108
- [x] FND-2: Dev environment — delivered 2026-07-17
- [x] FND-3: synveda-types + error model — delivered 2026-07-18
- [x] FND-4: Migrations & bitemporal base tables — delivered 2026-07-18
- [x] FND-5: Observability baseline — delivered 2026-07-18
- [x] FND-6: ADRs 0001–0004 — delivered 2026-07-18
- [x] FND-7: Public website and canonical brand assets — delivered 2026-09-20; ADR-0113

## Phase 1 — The spine (wk 2–5)

- [x] TEN-1: Tenant model & resolution — delivered 2026-07-18
- [x] TEN-2: Postgres row-level security as backstop — delivered 2026-07-18
- [x] AUTH-1: OIDC login (code+PKCE) — delivered 2026-07-18
- [x] HIER-1: Hierarchy store — delivered 2026-07-18; ADR-0074
- [x] AUTHZ-1: Cedar PDP embedded — delivered 2026-07-18
- [x] AUTH-2: JIT user provisioning from claims — delivered 2026-07-18
- [x] AUTHZ-2: Policy packs — delivered 2026-07-19
- [x] AUTHZ-3: Roles & role bindings — delivered 2026-07-19; ADR-0074
- [x] HIER-2: Scope chain resolver — delivered 2026-07-19; ADR-0074
- [x] HIER-3: Cedar entity sync — delivered 2026-07-19
- [x] AUTH-3: Service identities — delivered 2026-07-19
- [x] AUD-1: Hash-chained audit log — delivered 2026-07-19
- [x] MEM-1: observe API + PGMQ buffer — delivered 2026-07-19
- [x] MEM-2: Redaction & secret scanning — delivered 2026-07-19
- [x] MEM-3: Extraction pipeline — delivered 2026-07-22
- [x] MEM-4: Transactional embed-or-fail — delivered 2026-07-22
- [x] CTX-1: Hybrid retrieval — delivered 2026-07-23
- [x] CTX-2: Composition engine — delivered 2026-07-23
- [x] CTX-3: inject API — delivered 2026-07-23
- [x] ADPT-1: Claude Code adapter — delivered 2026-07-25
- [x] EVAL-1: Eval harness skeleton — delivered 2026-07-25

## Phase 2 — Governance (wk 6–10)

- [x] FLOW-1: Object store — delivered 2026-07-25; ADR-0030
- [x] FLOW-2: Channels — delivered 2026-07-25; ADR-0031
- [x] FLOW-3: Proposals & approval matrix — delivered 2026-07-25; ADR-0032
- [x] FLOW-4: Auto-promotion rules — delivered 2026-07-25; ADR-0033
- [x] FLOW-5: Cross-scope promotion — delivered 2026-07-25; ADR-0034
- [x] FLOW-6: CLI review flow — delivered 2026-07-25; ADR-0035
- [x] FLOW-7: Rollback & pinning — delivered 2026-07-25; ADR-0036
- [x] AUTHZ-4: Lapses (controlled relaxation) — delivered 2026-07-26; ADR-0037
- [x] AUTHZ-5: ABAC conditions — delivered 2026-07-26; ADR-0038
- [x] MEM-5: Always-on dedup & conflict detection — delivered 2026-07-26; ADR-0039
- [x] MEM-6: Decay, TTL & staleness — delivered 2026-07-26; ADR-0040
- [x] CTX-4: Tiered injection / progressive disclosure — delivered 2026-07-27; ADR-0041
- [x] CTX-5: recall API + MCP tool — delivered 2026-07-27; ADR-0042
- [x] GRPH-1: Multi-graph schema — delivered 2026-07-28; ADR-0043
- [x] GRPH-2: Graph-linking stage — delivered 2026-07-28; ADR-0044
- [x] GRPH-4: AGE performance spike / graph fallback assessment — delivered 2026-07-25; ADR-0029
- [x] AUD-2: Audit query & auditor role surface — delivered 2026-07-28; ADR-0045
- [x] EVAL-2: Extraction quality suite — delivered 2026-07-30; ADR-0046
- [x] EVAL-4: Retrieval & injection quality — delivered 2026-07-31; ADR-0047
- [x] EVAL-5: Security evals — delivered 2026-07-31; ADR-0048
- [x] PRMT-1: Prompt templates as assets — delivered 2026-08-02; ADR-0049
- [x] PRMT-2: Context packs — delivered 2026-08-03; ADR-0050

## Phase 3 — Enterprise (wk 11–16)

- [x] SKIL-1: agentskills.io-compliant model — delivered 2026-08-03; ADR-0051
- [x] SKIL-2: Security scanning gate — delivered 2026-08-03; ADR-0052
- [x] SKIL-3: Skill quality scoring — delivered 2026-08-03; ADR-0053
- [x] SKIL-4: Scope-targeted distribution — delivered 2026-08-03; ADR-0054
- [x] OPS-1: SMB profile — delivered 2026-08-04; ADR-0055
- [x] CNSL-1: Proposals inbox (hero screen) — delivered 2026-08-04; ADR-0056
- [x] ADPT-2: Generic MCP server — delivered 2026-08-05; ADR-0057
- [x] CNSL-2: Hierarchy & policy explorer — delivered 2026-08-05; ADR-0058
- [x] AUTH-4: SCIM 2.0 server — delivered 2026-08-05; ADR-0059
- [x] AUTH-5: Directory sync fallback — delivered 2026-08-07; ADR-0060
- [x] EVAL-3: Public benchmark adapters — delivered 2026-08-09; ADR-0061
- [x] OPS-2: Helm chart / enterprise profile — delivered 2026-08-10; ADR-0062
- [x] TEN-3: Dense-leg retrieval benchmark — delivered 2026-08-10; ADR-0063
- [x] TEN-4: Per-tenant encryption keys — delivered 2026-08-11; ADR-0064
- [x] OPS-8: Release & distribution — delivered 2026-08-11; ADR-0065
- [ ] [OPS-9: Release-shaped beta acceptance](OPS-9.md) — open
- [ ] [OPS-10: Uninstall & cleanup](OPS-10.md) — open
- [ ] [TEN-5: Tenant lifecycle](TEN-5.md) — open
- [ ] [TEN-6: Cross-tenant isolation test harness](TEN-6.md) — open
- [ ] [AUD-3: External immutable audit retention](AUD-3.md) — open
- [ ] [AUD-4: SIEM streaming](AUD-4.md) — open
- [x] GRPH-3: Graph-augmented recall — delivered; ADR-0097
- [ ] [EVAL-6: Load & latency suite](EVAL-6.md) — open
- [ ] [CTX-7: Dense-leg plan stability](CTX-7.md) — open
- [ ] [OPS-3: Residency routing](OPS-3.md) — open
- [ ] [OPS-4: Vector index scale decision](OPS-4.md) — open
- [ ] [ADPT-3: Additional API transport decision](ADPT-3.md) — open
- [ ] [CTX-6: Session compression assist](CTX-6.md) — open
- [ ] [FLOW-8: Git bridge — export](FLOW-8.md) — open

## Phase 4 — Ecosystem

- [ ] [ADPT-4: Python & TS SDKs](ADPT-4.md) — open
- [ ] [ADPT-5: Source-format converters](ADPT-5.md) — open
- [ ] [ADPT-6: LlamaIndex memory adapter](ADPT-6.md) — open
- [ ] [ADPT-7: Semantic Kernel memory connector](ADPT-7.md) — open
- [x] ADPT-8: Observation that survives a session that does not wait — delivered 2026-08-24; ADR-0027
- [x] ADPT-9: GitHub Copilot CLI adapter — delivered 2026-09-19; ADR-0107
- [ ] [PRMT-3: Prompt experiment evidence](PRMT-3.md) — open
- [ ] [SKIL-5: Authentic Skill usage reporting](SKIL-5.md) — open
- [ ] [MEM-7: Identity stitching](MEM-7.md) — open
- [ ] [OPS-5: Backup/restore & DR](OPS-5.md) — open
- [ ] [OPS-6: Upgrade and rollback discipline](OPS-6.md) — open
- [ ] [OPS-7: Gateway horizontal scale](OPS-7.md) — open
- [ ] [OPS-11: Small-team Kubernetes release](OPS-11.md) — open
- [ ] [CNSL-3: Audit temporal and disclosure views](CNSL-3.md) — open
- [x] CNSL-4: Knowledge browser — delivered 2026-08-24; ADR-0082
- [x] CNSL-5: Console theme and everyday usability — delivered 2026-09-20; ADR-0114
- [ ] [AUD-5: Compliance mapping doc](AUD-5.md) — open
- [ ] [AUTHZ-6: Authorisation scale decision](AUTHZ-6.md) — open
- [ ] [AUTHZ-7: Governed admin-plane mutation](AUTHZ-7.md) — open
- [ ] [TEN-7: Tenant storage partition decision](TEN-7.md) — open
- [ ] [EVAL-7: A second public benchmark](EVAL-7.md) — open

## Phase 5 — Context platform redesign

- [x] CPR-1: Implementation baseline & locked decisions — delivered 2026-08-17; ADR-0068
- [x] CPR-2: Fresh schema epoch, startup guard & local reset — delivered 2026-08-17; ADR-0069
- [x] CPR-3: Generic governed scope substrate — delivered 2026-08-17; ADR-0070
- [x] CPR-4: Workspaces, projects & canonical repository identity — delivered 2026-08-17; ADR-0071
- [x] CPR-5: Membership, groups, grants & invitations — delivered 2026-08-18; ADR-0072
- [x] CPR-6: Governed scope anchors — the PDP re-cut — delivered 2026-08-19; ADR-0073
- [x] CPR-7: The hierarchy cutover — one scope tree — delivered 2026-08-20; ADR-0074
- [x] CPR-8: The console product shell & first-run onboarding — delivered 2026-08-21; ADR-0075
- [x] CPR-9: The foundation audit — hardening the scope and access cutover — delivered 2026-08-22
- [x] CPR-10: The session ledger and runtime API — delivered 2026-08-23; ADR-0076
- [x] CPR-11: The session product experience — delivered 2026-08-24; ADR-0077
- [x] CPR-12: Durable Claude session delivery — delivered 2026-08-23; ADR-0078
- [x] CPR-13: The demo corpus re-point — delivered 2026-08-24
- [x] CPR-14: Live Claude Code session acceptance gate — delivered 2026-08-24; ADR-0079
- [x] CPR-15: Versioned Knowledge aggregate and provenance — delivered 2026-08-24; ADR-0080
- [x] CPR-16: Governed Knowledge mutation lifecycle — delivered 2026-08-24; ADR-0081
- [x] CPR-17: Public Knowledge API, search and browser — delivered 2026-08-24; ADR-0082
- [x] CPR-18: Session-based capture batches and reviewable candidates — delivered 2026-08-24; ADR-0083
- [x] CPR-19: New Learnings lightweight review workflow — delivered 2026-08-24
- [x] CPR-20: Explainable Knowledge context planning and scoped query — delivered 2026-08-24; ADR-0084
- [x] CPR-21: Context Inspector and outcome feedback — delivered 2026-08-24
- [x] CPR-22: Core individual and small-team MVP acceptance — delivered 2026-08-24
- [x] CPR-23: Immutable skill versions, bindings and usage — delivered 2026-08-24; ADR-0085
- [x] CPR-24: Skills Library product experience — delivered 2026-08-24
- [x] CPR-25: Trusted MCP server catalogue and project bindings — delivered 2026-08-25; ADR-0086
- [x] CPR-26: MCP Tools catalogue product experience — delivered 2026-08-25
- [x] CPR-27: OKF v0.2 knowledge exchange adapter — delivered 2026-08-25; ADR-0087
- [x] CPR-28: OKF import and export product workflows — delivered 2026-08-25
- [x] CPR-29: Public contract and client convergence — delivered 2026-08-25; ADR-0088
- [x] CPR-30: Governed runtime configuration artifacts — delivered 2026-08-25; ADR-0089
- [x] CPR-31: Governed auto-apply and policy relaxations — delivered 2026-08-25; ADR-0090
- [x] CPR-32: Unified approvals across governed artifacts — delivered 2026-08-25; ADR-0091
- [x] CPR-33: Context-platform audit query and deterministic export — delivered 2026-08-25; ADR-0092
- [x] CPR-34: Directory adapter convergence — delivered 2026-08-25; ADR-0093
- [x] CPR-35: Context-platform key and secret convergence — delivered 2026-08-25; ADR-0094
- [x] CPR-36: One-runtime deployment convergence — delivered 2026-08-25; ADR-0095
- [x] CPR-37: Conflict, supersession and freshness engine — delivered 2026-08-25; ADR-0096
- [x] CPR-38: Bounded graph-augmented retrieval — delivered 2026-08-25; ADR-0097
- [x] CPR-39: Second verified client — delivered 2026-09-12; ADR-0098, ADR-0106
- [x] CPR-40: Context-platform product and trust evaluation — delivered 2026-08-26; ADR-0099
- [x] CPR-41: One-command realistic product demo — delivered 2026-08-26; ADR-0100
- [x] CPR-42: Context-platform security and product-integrity audit — delivered 2026-08-26; ADR-0078
- [x] CPR-43: Final context-platform hard cut — delivered 2026-08-26; ADR-0069
- [x] CPR-44: Production hardening and maintainability cut — delivered 2026-08-26; ADR-0101
- [ ] [CPR-45: Docker-first portable reference deployment](CPR-45.md) — open

CPR-45 follows ADR-0105's direct-acceptance plan. Canonical Compose now owns the
only source and packaged single-host topology: proxy-only public edge, isolated
Synveda/Keycloak databases and roles, optimized Keycloak, separate product
gateway/worker commands, mounted secrets and a private Collector. The
digest-bound reference archive and environment manifest use that same graph.
Retired provider, workflow-scheduler, contributor, installed-profile and host-
gateway paths are deleted; `synveda init` is a side-effect-free refusal.
The root documentation index now points source users to one detailed Compose
guide and labels the unverified packaged-reference workflow separately.

Deterministic gates cover the provider/runtime matrix, a four-principal
governed retry-review browser scenario, restart lifecycle, logical database/key recovery, same-schema image
upgrade, external dependency wiring, bounded local metrics, customer-safe
Operations view and the disabled-by-default Apalis 0.7.4 Skill-validation
transport. PostgreSQL operation/outbox state remains authoritative and the
native worker is the rollback.

On 2026-09-12 the current four-principal revision passed clean-volume
development acceptance on macOS 26.6.2 arm64 with OrbStack Docker Engine 29.4.0
and Compose 5.1.2, at
`d1486929d73dc459ed6dc0bf029c5c825d3a1bb6` plus the working-tree increment,
using only `synveda-development-acceptance-e2e` and synthetic identities. The
full deployment gate, 251 console tests/build, 189 CLI tests/strict Clippy,
77 policy tests, six OpenAPI tests and 18 fresh exact-role access API tests
pass. The real seed replay preserves a browser edit; the viewer's raw
Session-content and approval requests are denied. Capture, distinct review and
apply, Knowledge provenance, exact redacted Context links, two-person Skill
approval and the administration controls pass in the existing browser suite.
All six native service restarts and final live receipt/browser verification
pass. Canonical down/up reused the same product volume and byte-identical
keys/issuer; a fresh browser login reopened the same persisted pages. Safe
desktop/laptop screenshots were inspected. The isolated demo remains running
for inspection. The normal stack is stopped, its volume and existing
keys/issuer are unchanged, and returning to it requires the documented
operator hosts-block handoff. No normal-project reset was performed.
See the [current validation record](CPR-45.md#current-local-demonstration-validation).

The incremental console walkthrough now covers readable Knowledge and Session
evidence, routed review/apply state, Context requests with exact revision links,
and actual Skill binding resolution using the existing generated public client.
Focused console tests, the production build and deterministic contract checks
pass. On 2026-09-11 the connected in-app Browser and public API exercised an
allowed invitation, malformed-email validation, pending and accepted state,
the same viewer denial in UI and API, exact-grant cleanup and final revoked
state without recording the one-time link. The exercise also fixed custom HTTP
origin idempotency generation and safe rendering of structured policy denials.
The administration increment shows governed names beside exact grant scope,
typed Capture and Context proposals, invitation state and evidence-backed
Skill, Tool and Operations status without changing the public API or authority
model. The exact-role `access_api` suite passed all 18 allowed, denied,
isolation and token-safety cases.

The feature remains open for repeat development runs on Linux and Docker
Desktop, reference HTTPS, live recovery/upgrade/Apalis execution, live external
providers, and one published-registry installed-reference run. This evidence
would support controlled single-host use only. It would not establish HA,
host-loss tolerance, DR/RPO/RTO, SaaS readiness, signing or Helm production
readiness; OPS-5 retains S3/WAL-PITR and encrypted off-host recovery work.

## Unscheduled — not listed in the Sequencing section

- [ ] [AUTH-6: Session & token hygiene](AUTH-6.md) — open
