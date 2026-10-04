# Feature inventory

151 features in this index. This file owns feature identity and delivered/open
state. Delivered names identify historical slices; current behavior comes from
code, generated contracts and accepted decisions. Only open features retain
implementation briefs.

114 delivered; 37 open. CI checks the counts, IDs and open-brief contract.

The current [v0.4.3 release](https://github.com/synveda/synveda/releases/tag/v0.4.3)
is available for self-hosted evaluation. [Production readiness](../PRODUCTION_READINESS.md)
tracks deployment and recovery gaps; the generated [client support matrix](../CLIENT_SUPPORT.md)
tracks tested agent versions. The release history and implementation diaries
remain in Git and the linked release evidence.

## Phase 0 — Foundation

- [x] FND-1: Workspace scaffold — delivered
- [x] FND-2: Dev environment — delivered
- [x] FND-3: synveda-types + error model — delivered
- [x] FND-4: Migrations & bitemporal base tables — delivered
- [x] FND-5: Observability baseline — delivered
- [x] FND-6: Foundational architecture decisions — delivered
- [x] FND-7: Public website and canonical brand assets — delivered

## Phase 1 — Core runtime

- [x] TEN-1: Tenant model & resolution — delivered
- [x] TEN-2: Postgres row-level security as backstop — delivered
- [x] AUTH-1: OIDC login (code+PKCE) — delivered
- [x] HIER-1: Hierarchy store — delivered
- [x] AUTHZ-1: Cedar PDP embedded — delivered
- [x] AUTH-2: JIT user provisioning from claims — delivered
- [x] AUTHZ-2: Policy packs — delivered
- [x] AUTHZ-3: Roles & role bindings — delivered
- [x] HIER-2: Scope chain resolver — delivered
- [x] HIER-3: Cedar entity sync — delivered
- [x] AUTH-3: Service identities — delivered
- [x] AUD-1: Hash-chained audit log — delivered
- [x] MEM-1: observe API + PGMQ buffer — delivered
- [x] MEM-2: Redaction & secret scanning — delivered
- [x] MEM-3: Extraction pipeline — delivered
- [x] MEM-4: Transactional embed-or-fail — delivered
- [x] CTX-1: Hybrid retrieval — delivered
- [x] CTX-2: Composition engine — delivered
- [x] CTX-3: inject API — delivered
- [x] ADPT-1: Claude Code adapter — delivered
- [x] EVAL-1: Eval harness skeleton — delivered

## Phase 2 — Governance

- [x] FLOW-1: Object store — delivered
- [x] FLOW-2: Channels — delivered
- [x] FLOW-3: Proposals & approval matrix — delivered
- [x] FLOW-4: Auto-promotion rules — delivered
- [x] FLOW-5: Cross-scope promotion — delivered
- [x] FLOW-6: CLI review flow — delivered
- [x] FLOW-7: Rollback & pinning — delivered
- [x] AUTHZ-4: Lapses (controlled relaxation) — delivered
- [x] AUTHZ-5: ABAC conditions — delivered
- [x] MEM-5: Always-on dedup & conflict detection — delivered
- [x] MEM-6: Decay, TTL & staleness — delivered
- [x] CTX-4: Tiered injection / progressive disclosure — delivered
- [x] CTX-5: recall API + MCP tool — delivered
- [x] GRPH-1: Multi-graph schema — delivered
- [x] GRPH-2: Graph-linking stage — delivered
- [x] GRPH-4: AGE performance spike / graph fallback assessment — delivered
- [x] AUD-2: Audit query & auditor role surface — delivered
- [x] EVAL-2: Extraction quality suite — delivered
- [x] EVAL-4: Retrieval & injection quality — delivered
- [x] EVAL-5: Security evals — delivered
- [x] PRMT-1: Prompt templates as assets — delivered
- [x] PRMT-2: Context packs — delivered

## Phase 3 — Deployment foundations

- [x] SKIL-1: agentskills.io-compliant model — delivered
- [x] SKIL-2: Security scanning gate — delivered
- [x] SKIL-3: Skill quality scoring — delivered
- [x] SKIL-4: Scope-targeted distribution — delivered
- [x] OPS-1: SMB profile — delivered
- [x] CNSL-1: Proposals inbox (hero screen) — delivered
- [x] ADPT-2: Generic MCP server — delivered
- [x] CNSL-2: Hierarchy & policy explorer — delivered
- [x] AUTH-4: SCIM 2.0 server — delivered
- [x] AUTH-5: Directory sync fallback — delivered
- [x] EVAL-3: Public benchmark adapters — delivered
- [x] OPS-2: Helm chart / enterprise profile — delivered
- [x] TEN-3: Dense-leg retrieval benchmark — delivered
- [x] TEN-4: Per-tenant encryption keys — delivered
- [x] OPS-8: Release & distribution — delivered
- [ ] [OPS-9: Release-shaped beta acceptance](OPS-9.md) — open
- [ ] [OPS-10: Uninstall & cleanup](OPS-10.md) — open
- [ ] [TEN-5: Tenant lifecycle](TEN-5.md) — open
- [ ] [TEN-6: Cross-tenant isolation test harness](TEN-6.md) — open
- [ ] [AUD-3: External immutable audit retention](AUD-3.md) — open
- [ ] [AUD-4: SIEM streaming](AUD-4.md) — open
- [x] GRPH-3: Graph-augmented recall — delivered
- [ ] [EVAL-6: Load & latency suite](EVAL-6.md) — open
- [ ] [CTX-7: Dense-leg plan stability](CTX-7.md) — open
- [ ] [OPS-3: Residency routing](OPS-3.md) — open
- [ ] [OPS-4: Vector index scale decision](OPS-4.md) — open
- [ ] [ADPT-3: Additional API transport decision](ADPT-3.md) — open
- [ ] [CTX-6: Session compression assist](CTX-6.md) — open
- [ ] [CTX-8: Governed context optimisation](CTX-8.md) — open

- [ ] [FLOW-8: Git bridge — export](FLOW-8.md) — open

CTX-6 now has an exact-role tested deterministic Claude checkpoint/restart path, a passing source demo, direct capture-candidate and scoped retention probes, and a passing create/read/use/capture policy matrix including revocation and foreign-tenant comparisons. Four predeclared synthetic task probes pass in the fresh-database 23-scenario product suite: 12/12 Session facts and four exact Knowledge bodies retained, provenance intact, local assisted preview p95 below the 500 ms guard. A separate combined-mode gate preserves checkpoint source attribution and exact required Knowledge in both off and conservative modes, omitting optional restart text under a tight shared budget. An opt-in export now prepares the actual paired synthetic ContextRun blocks against a fixed JSON-answer rubric without calling a model, and a capped no-tools runner passes a fake-client end-to-end test. This does not measure model task success or provider usage. A live proprietary-client run remains open. CTX-8's conservative path is implemented and its optional learned backend has a no-go promotion decision. An isolated hostname handoff and resolver check passed without resetting retained interop state, and the original mapping was restored. The pinned product source image built on 2026-09-29 after official Cargo index/archive access recovered. Fresh CTX-6/CTX-8 Compose/browser acceptance and held-out model quality remain open. Next rerun paired deployment checks, then the bounded factual/model and live Claude probes when credentials and spend permission are available. The open briefs hold the exact evidence and limits.

## Phase 4 — Clients and operations

- [ ] [ADPT-4: Python & TS SDKs](ADPT-4.md) — open
- [ ] [ADPT-5: Source-format converters](ADPT-5.md) — open
- [ ] [ADPT-6: LlamaIndex memory adapter](ADPT-6.md) — open
- [ ] [ADPT-7: Semantic Kernel memory connector](ADPT-7.md) — open
- [x] ADPT-8: Observation that survives a session that does not wait — delivered
- [x] ADPT-9: GitHub Copilot CLI adapter — delivered
- [ ] [PRMT-3: Prompt experiment evidence](PRMT-3.md) — open
- [ ] [SKIL-5: Authentic Skill usage reporting](SKIL-5.md) — open
- [ ] [MEM-7: Identity stitching](MEM-7.md) — open
- [ ] [OPS-5: Backup/restore & DR](OPS-5.md) — open

OPS-5 / ADR-0122 has an opt-in source Helm Barman Cloud binding for CNPG WAL
and scheduled physical backups, plus an S3-compatible ObjectStore example and
static chart refusal tests. ADR-0123 adds an opt-in Compose pgBackRest image,
private S3 configuration check and bounded base-backup action. No live
encrypted off-host backup, matching identity/key recovery, recurring drill or
owned RPO/RTO has passed. A 2026-09-29 isolated local S3/TLS drill passed
encrypted full backup, WAL archiving, two fresh-volume point-in-time restores
before/after a committed write and wrong-passphrase refusal. This is database
mechanics evidence only. Next qualify both source candidates with an
owner-selected off-host bucket and independent joint application restore; the
OPS-5 brief owns the exact drill and remaining owner decisions.
The deployment owner has no operator-owned test bucket yet, so that live gate
is waiting on a destination and credentials.

- [ ] [OPS-6: Upgrade and rollback discipline](OPS-6.md) — open

OPS-6 / ADR-0121 now keeps the published v0.4.3 `0001` bytes and adds a
transactional `0002` for CTX-6/CTX-8; OPS-7 appends `0003` for sealed one-time
login state. Byte pins, exact-prefix guards and the full disposable exact-role
database suite pass at `0003`, including all 26 epoch cases. A 2026-09-30
isolated released-byte v0.4.3-to-source rehearsal passed linked logical
database/identity/key backup, old-writer quiescence, read-only old-head
preflight, `0002`/`0003`, old-binary refusal, source gateway/worker/browser
acceptance and wrong-key refusal. A parked login survived a gateway restart
and was consumed once. A distinct rollback restore passed the published
binary, original key and browser checks; every field of the original 52 audit
rows matched the migrated copy by SHA-256. One Knowledge item and five
proposals remained, but the published sample's cross-version rerun conflicted
on its fixed Configuration idempotency key. The CPR-45 client now validates and
reuses recorded Configuration and Sessions; a rebuilt browser image resumed
the v0.4.3 receipt against the migrated gateway and passed `sample` at
`learning_pending`. A second isolated v0.4.3 full-seed fixture passed review,
distinct Skill approvals, binding, context and audit; it upgraded to `0003`
and the new CLI verified the same governed addresses. The candidate browser
fixture now uses the configured loopback origin, and the gateway preserves
released context-run idempotency digests when new options are omitted. A
distinct released-binary restore passed browser/API checks. Ordinary-role
tenant transactions found matching Knowledge/Skill/proposal/context counts,
and audit rows 1–384 matched by serialized-row checksum and chain hash.
An isolated head-`0003` candidate bundle then recovered an unexpired sealed
pending login from a joint database/identity/key set into an empty project.
The restored gateway opened and consumed it once on an IdP denial, refused
replay, and still passed fresh browser login/API/logout. The candidate mixed
local product/browser and published support images; it is not a release pair.
Exact-role client cancellation and PostgreSQL backend termination during later
`0002` DDL each proved transactional rollback, unchanged released
ledger/marker/tenant and retry to `0003`; all 26 serial epoch cases passed on
the isolated fixture. No published N, off-host PITR,
production-shaped outage/lock, successful in-flight code exchange or broader
failure matrix is qualified. OPS-6 stays open for those gates. The published
v0.4.3 verifier correctly refuses the advanced
schema, and v0.4.0 remains incompatible.

- [ ] [OPS-7: Gateway horizontal scale](OPS-7.md) — open
- [ ] [OPS-13: Capture retry and provider backpressure](OPS-13.md) — open
- [ ] [OPS-14: Request limits and tenant usage budgets](OPS-14.md) — open
- [ ] [OPS-15: Operational signals and recovery runbooks](OPS-15.md) — open

OPS-13 owns PR-05's persisted Capture retry schedule, terminal inspection,
queue-age evidence and provider-wide concurrency bound. Existing five-attempt
fencing and the three-pod Capture-only drill protect durable candidates, but a
failed nonterminal batch returns directly to `pending` and may retry on the
next claim. The provider quota, retry-age objective and manual-retry policy
need owner choices and an ADR before new state is implemented.

OPS-14 owns PR-07's request-frequency/concurrency and tenant-usage bounds.
The existing 429 taxonomy does not enforce a budget. The first slice needs
owner-selected operation classes and one-gateway limits; cluster-wide fairness
requires a chosen distributed limiter store and OPS-7 cross-pod evidence.

OPS-15 owns PR-10's SLIs, alerts, release markers, tenant-safe diagnostics and
exercised runbooks. Raw Prometheus/OTLP signals and an optional private
Collector do not establish an on-call or SLO contract. Backend, retention,
thresholds and response ownership remain to be selected before paging.

The 2026-09-30 Rust review found no enforced tenant/pack envelope for the
serial all-tenant policy sweep within its five-second deadline. Next measure
that sweep at a declared maximum and preserve fail-closed Cedar behavior
while isolating failures. Active-tenant and per-tenant stored-pack counts are
now exported without tenant or pack labels beside sweep duration/outcome; no
supported work limit is yet enforced. A manual exact-role 32 × 4 local probe
passed the five-second sweep deadline: cold 0.353 seconds, slowest unchanged
0.026 seconds across five passes, and all-pack revision 0.324 seconds. It had
no request traffic, cross-pod propagation or fault injection and does not
qualify a production envelope or isolate a failed tenant. Proposed ADR-0131
maps the request, core-worker and Apalis tenant-lease checks needed for safe
failure isolation; the global ADR-0127/0128 lease remains authoritative. The
source gateway now retries deployment-key provisioning and unwrap checks
within five-second attempts, keeps readiness closed until both key and policy
convergence succeed, and emits content-free key-ready/attempt metrics. An
exact-role process test proves timeout, retry
recovery and wrong-KEK refusal. Live OIDC/KMS faults and cross-pod key rotation
remain unqualified; a disabled KMS still permits bearer-only readiness.

OPS-7 has a first single-process drain slice under ADR-0124: SIGTERM withdraws
gateway readiness and new request admission before HTTP stops, preserves the
authority sentinel until admitted work drains, and exits inside the configured
bound in an isolated exact-role subprocess test. ADR-0125 then closes the
startup/recovery window between database authority and first stored Cedar
policy-pack convergence by binding readiness and requests to one authority
generation. The chart still pins one gateway and one combined worker by
default, with Recreate updates. Durable
login state is now a source candidate under ADR-0126: additive `0003` stores
sealed, hash-selected one-time rows with database TTL and atomic consumption;
isolated mock-IdP tests complete OIDC login and CLI handoff across independent
gateway instances, and 25 exact-role epoch tests pass. ADR-0127 adds a
provisional 30-second gateway policy lease: a complete successful bounded
sweep renews it, while failed/time-out sweeps do not; expiry withdraws HTTP
admission/readiness and cancels in-flight work. Source route and isolated
two-engine compilation tests pass. ADR-0128 extends the provisional lease to
core and optional Apalis workers; synthetic tests cover core readiness,
cancellation and bounded join. A one-node Kind Helm drill passed baseline
database failover and direct three-pod JSON/CLI login handoff with replay
refusal. Two audited test-pack revisions compiled on all three pods with
maximum database-timestamp-to-metric lags of 1.065 and 4.945 seconds at
light load. A public governed Configuration and project-scoped review grant
then selected a test pack: all three direct pod requests changed from permitted
`ScopeUpdate` to revision-specific 403 after a restrictive edit, with a
maximum database-timestamp-to-denial lag of 5.259 seconds; repeat denials
held, the binding was disabled and the grant revoked. The chart returned to
one gateway. A separate audited, test-only invalid Cedar pack left database
authority ready while all three gateways recorded failed refreshes; 486 paired
authenticated/readiness samples saw all close by 28.996 seconds and recover by
45.747 seconds from baseline after pack removal. This is one-node light-load
evidence, not a production traffic bound. A further live claimed-Capture drill
held the extractor, expired the worker's policy-only lease and saw readiness
withdrawn and the first provider call cancelled after 28 seconds. After pack
repair and fenced lease reclaim, the second call produced one durable candidate
in two attempts, with no candidate before the retry completed. This remains
one-node/one-worker evidence, not multi-worker ownership or exactly-once
provider effects. A second Kind probe temporarily ran two core workers,
deleted the one owning a blocked Capture claim, and observed the other remain
ready while the cancelled claim was reclaimed by a different pod. Two attempts
still produced one candidate after the second provider call completed. This
qualifies one Capture owner-pod loss only; Knowledge indexing can duplicate
external embedding calls and directory pull still lacks a cross-worker pass
owner. ADR-0129 therefore retains one combined worker and adds a closed
capture-only runtime profile. The source profile runs the same authority and
policy gates without maintenance tasks or embedder/KMS configuration; focused
subprocess acceptance passes. The source Helm chart now keeps the combined
worker singleton and accepts zero to two additional capture-only pods, counting
each database pool in the bundled/CNPG budget. A source-image, one-node Kind
run proved three distinct Capture claims across those pods. Deleting one
capture-only owner cancelled its provider call; two surviving calls completed,
a different pod reclaimed the expired claim, and all three batches held one
candidate each after four provider calls. A separate provider 503 retried once
and completed with one candidate. The harness restores the default chart
values. This is Capture-only evidence, without a provider-wide quota,
sustained traffic or cross-node/rolling-upgrade proof. The drill also
exposed a CNPG chart bootstrap defect: PUBLIC
CONNECT had been revoked on `postgres` without restoring it for CNPG's reserved
`streaming_replica` role. A retained standby stayed unready after promotion;
the disposable cluster recovered after a narrow operator grant. The source
chart now grants only that reserved role in the required administrator Job,
after CNPG creates it. A fresh chart-rendered two-instance cluster passed
bootstrap and replica restart with PUBLIC and product maintenance access still
closed; contract checks and an operator repair note cover retained clusters.
The retained release then upgraded to the fixed source image and completed its
administrator bootstrap with two ready CNPG instances. A crash-style second
primary loss promoted the other instance and recovered 2/2 readiness; the
three-gateway login, policy-reload and restrictive-decision probe passed after
promotion, then exact saved Helm values restored the published image baseline.
A normal pod delete had first waited under CNPG's 30-minute termination grace
with failover pending, so this does not qualify planned switchover or timed
recovery. Other worker-family ownership, in-flight process/load, key rotation, multi-node loss
and rolling acceptance remain open in the brief.
Scope, grant, identity and Configuration decisions already use fresh
request-time database rows and exact-shape Cedar fragments; a generic
cross-process entity invalidation bus is not presumed necessary.

- [ ] [OPS-11: Small-team Kubernetes release](OPS-11.md) — open
- [ ] [OPS-12: Consumer installation and harness setup](OPS-12.md) — open

The 2026-10-03 OPS-11/OPS-12/OPS-8 candidate under Accepted ADR-0137 adds native
repeatable Kubernetes preparation, packaged customer recipes, mode-specific
storage refusal, prerequisite checks, private human admission, readiness tests
and allowlisted diagnostics. CPR-8/CPR-39 regenerate registry-backed manual
Codex/Copilot choices and distinguish browser access, setup confirmation and
authenticated Session evidence. The guide carries shared and loopback first-use
review paths. [Local validation](../../demos/evidence/ops11-first-install-source.json)
is separate from immutable v0.4.3 and hosted release proof. The owner's
2026-10-04 OrbStack target passed packaged local installation with namespace-only
RBAC, dedicated 37Gi storage, real browser authentication, native CLI MCP
Knowledge/context/source delivery and retained uninstall/reinstall. The
[OrbStack record](../../demos/evidence/ops11-orbstack-first-use.json) labels its
source/image bytes and excludes shared ingress, real vendor loading and human
timings. `make chart-package` removes manual source packaging steps. ADR-0100's
accepted amendment makes a pending sample Configuration resumable and exposes
effect-actor separation. That live completion awaits a separate operator;
automatic approval review rejected temporary provider administration without
explicit permission. Three unfamiliar engineer participants remain unavailable.
The [current checkpoint](OPS-11.md#orbstack-local-acceptance-checkpoint-2026-10-04)
owns the exact next action. Both features remain open; the coordinated 0.4.4
candidate still needs exact-source hosted CI/Release qualification and authorised
publication. Setup time, shared platform support and production readiness remain
unclaimed.

OPS-12 / ADR-0132 requires source installers to verify the signed checksum
inventory with a trusted GitHub CLI, fixed publisher/workflow/tag/runner policy
and expected source commit before remote code execution. Real macOS verification
of v0.4.3 rejects wrong tag/source and corrupted inventory; wrong-source
reinstall retains the installed manifest. Native Windows verifier-policy and
pre-execution refusal fixtures now pass on both hosted architectures. These
fixtures do not qualify Windows real issuer login or signed future publication.

Clean source `f433eb1719bc58b5f36b0516bedf71e886d23185` passed all 23 jobs in
[full CI](https://github.com/synveda/synveda/actions/runs/36765529532) and the
same-source [nonpublishing Release](https://github.com/synveda/synveda/actions/runs/36765540733)
(fifteen jobs passed, final publication skipped). All six native clients and
both native Linux full Compose/Helm drills passed, including four ownership
modes, three-row migration reruns, outage/reinstall recovery and restricted
local browser login/logout. First-party notice bytes/hashes pass in native
archives, charts, plugin packages and all six Synveda image targets.
[Independent evidence](../../demos/evidence/ops12-source-qualification.json)
verifies the original assembled checksum inventory, all 31 payloads and every
same-run native/deployment report; no earlier run supplies evidence. Dry-run
registry-copy and anonymous public-pull steps were skipped. OPS-11/OPS-12 and
the production verdict remain open; no tag or release was published.

Published v0.4.3 product SBOM inspection found no identified Cargo, Cedar or
SQLx packages on either Linux architecture. Rust dependency coverage is the
next release-security gate. Complete third-party notice/SBOM review,
vulnerability and publisher incident policy, OS signing, real issuer/harness
acceptance and support-window ownership also remain open. The
[OPS-12 brief](OPS-12.md) owns the exact evidence and next action.

[ADR-0133](../adr/adr-0133-embed-rust-dependency-inventory-in-release-binaries.md)
now embeds pinned Rust metadata in product/browser images and requires actual
OCI subject/hash and Cargo-content checks during candidate creation and assembly.
[Local native ARM evidence](../../demos/evidence/ops12-oci-rust-sbom.json)
contains 351/295 distinct Cargo identities and an ordinary-binary refusal.
Clean OCI source `3beeb1cb` passed all 23 CI jobs and 15 nonpublishing Release
jobs, with final publication skipped. [Retained verification](../../demos/evidence/ops12-oci-source-qualification.json)
binds the original 31-payload inventory, six native client reports, both
Rust-bearing image report sets and complete Compose/Helm evidence to that source
and run. Native archive SBOM qualification and broader dependency coverage
remain open under OPS-12.

The following native archive source slice extends the same locked metadata
contract to all six client and both server archives. Every final Rust executable
needs its own hash-bound SPDX document; all 20 report/document sidecars are
required in the release checksum and publisher inventory. Local macOS ARM client
and stripped server checks pass, including private client execution, while the
clean-source gate refuses the local dirty reports. Full hosted qualification of
this new source remains the next action. Non-Rust coverage and release-security
policy remain open; [OPS-12](OPS-12.md) owns the exact boundary.

Native source qualification remains open. The `c535d4f4` runs were cancelled
after a regression test reproduced a Windows CRLF lockfile-reader defect.
Correction `668a3f96` passed native Linux archive checks, but its Windows x64
SBOM check refused pinned Syft's actual `\synveda.exe` file spelling after all
11 packaged-client checks passed. The validator now requires that exact Windows
root-relative name, while retaining the single-file SHA-256 and Cargo checks;
foreign, nested and drive-qualified paths refuse. All 27 CI-tooling tests pass.
The `aea91758` CI again failed on Cargo registry HTTP/2 transfers before
CNPG/native tests; its native runs were cancelled after the transport correction.
Native jobs and the product builder now select Cargo's HTTP/1.1 transport,
preserving TLS and locked dependency checks. Failed qualification source
`1edb6255` is bound to [full CI](https://github.com/synveda/synveda/actions/runs/36798188473)
and an independent [nonpublishing Release](https://github.com/synveda/synveda/actions/runs/36798194065).
Both Windows x64 jobs passed 11 packaged-client checks, then refused an SPDX
file entry without the required SHA-256 checksum. The correction selects all
metadata within the single-file source and requires exactly its pinned synthetic
root plus the expected executable's actual SHA-256 digest. Missing executable
digests, placeholders, foreign hashes and extra entries refuse. The
[local macOS ARM probe](../../demos/evidence/ops12-native-file-digests-probe.json)
passes all three unchanged stripped members and refuses dirty producer evidence;
eleven archive tests cover all six root contracts. Both Windows ARM jobs
subsequently refused the same omitted checksum. The failed CI completed; its
nonpublishing Release was cancelled after the correction was committed. Source
`02023908` started
[full CI](https://github.com/synveda/synveda/actions/runs/36802763712) and
[nonpublishing Release](https://github.com/synveda/synveda/actions/runs/36802769936).
Both trigger SHAs were independently checked. All 29 CI-tooling and 77
release-parity tests, fast checks, pinned actionlint, Rust formatting and diff
checks pass locally. Its Windows x64 CI job passed native Rust/private-state
tests and packaged-client execution, then refused the actual empty synthetic
root filename. Corrected fixtures reproduce that refusal before the fix and
reject the wrongly assumed backslash root. The root now requires the exact
empty name on all targets, retaining the executable's Windows spelling and
actual SHA-256 checks. Latest hosted native source `ef6d61db` is bound to
[full CI](https://github.com/synveda/synveda/actions/runs/36805252052) and
[nonpublishing Release](https://github.com/synveda/synveda/actions/runs/36805260636);
both trigger SHAs were independently checked. The obsolete Release is cancelled
and CI cancellation was requested. All 29 CI-tooling and 77 release-parity tests,
fast checks, Rust formatting, pinned actionlint and diff checks pass for the
correction. Actual retained macOS archive/SPDX bytes still verify, with dirty
source refusal. Both Windows x64 jobs again refused the missing executable
SHA-256 despite full file-metadata selection; this is an unresolved scanner
defect, not a transient download failure. Independent review reproduced a
separate assembly gap with coherently changed report/SPDX digests. Assembly now
rehashes the actual unique regular executable and verifies its platform and
size before SPDX; real ZIP fixtures and Linux libarchive provisioning cover
Windows archives. All 30 CI-tooling / 77 release-parity tests, fast checks,
formatting, actionlint and diff checks pass. Downloaded same-run Linux ARM bytes
pass the stricter check, but no full native source is qualified. Both failed
`ef6d61db` runs are cancelled. [Retained blocker evidence](../../demos/evidence/ops12-native-windows-scanner-blocker.json)
identifies Syft's native Windows path lookup defect: upstream fix 5341 merged
as `6ac7afb4`, while latest released 1.52.0 still has pinned 1.51.0's faulty
resolver bytes. A fixed scanner must pass native path/digest probes before a
new clean-source qualification. The owner's subsequent `continue` resumes the
recommended independent inventory/notices work under ADR-0134 while retaining
the native scanner blocker. Latest upstream release remains 1.52.0; no custom
scanner build is selected. No new qualification source/run is
selected, and the temporary validation branch still points at failed
`ef6d61db`; do not resume its runs. Independently verify all 51 same-run
payloads, eight archive reports and twelve SPDX documents before retaining
proof. The open brief owns the exact failures and next action.
Superseded reports cannot qualify the new source; the preceding `3beeb1cb` OCI
qualification is retained separately. The independent non-Rust slice follows
below; its earlier mechanism probe remains separately retained in [OPS-12](OPS-12.md).

ADR-0134 now pins the maintained build-only SBOM tool and requires independent
rendered React/React DOM/scheduler identities, full package/helper/font notices
and actual console archive source/file hashes. [Local candidate evidence](../../demos/evidence/ops12-console-inventory-candidate.json)
records identical 12-file outputs on macOS ARM and native Linux ARM with unchanged
JavaScript. Eight focused refusal tests, 38 CI-tooling, 77 release-parity and
258 console tests, deployment checks and strict chart lint pass; no clean hosted
qualification follows from the dirty candidate. The follow-up ADR-0134 reader now
checks actual stopped product-image console bytes and binds their entire output
inventory to the archive. [Actual Linux ARM evidence](../../demos/evidence/ops12-console-image-candidate.json)
matches all 12 files to the independent macOS ARM archive; stale and coherently
changed image-content controls refuse. All 38 CI-tooling, 81 release-parity and
258 console tests, deployment/chart checks, formatting, pinned actionlint and
fast/diff checks pass.
The [Node mechanism probe](../../demos/evidence/ops12-node-runtime-probe.json)
checks all six pinned client upstream distributions, both product Linux
distributions and the preceding task-owned product ARM image. Actual product
Node/licence bytes match official upstream content. Syft identifies no embedded
library versions and no Node identity from the original Windows filenames.
Native Node metadata exposes reported dependencies; complete upstream licence
carriage still omits a separate nbytes MIT notice. This is local mechanism
evidence, not six native executions or new source qualification.
ADR-0135 selects reuse of Node's built-in metadata and existing manifests/reports,
with exact executable/licence pins and supplementary upstream notices.
The client source increment now implements six exact executable/licence pins,
supplementary nbytes/SQLite notices and 21 reported dependencies in the existing
manifest/native report. Assembly reads actual archive bytes independently.
[Actual macOS ARM evidence](../../demos/evidence/ops12-client-node-inventory-candidate.json)
passes all nine native archive checks; coherently changed runtime/notice controls
refuse. Six-target TAR/ZIP fixtures pass; this dirty dev-profile candidate is not
Cargo SBOM or clean hosted qualification.
The product follow-up now pins both reviewed Linux Node 22 distributions,
carries supplementary notices and checks actual stopped-image bytes before
isolated native metadata execution. Existing local and both-registry image
reports retain that inventory; assembly independently refuses substitutions.
[Actual Linux ARM evidence](../../demos/evidence/ops12-product-node-inventory-candidate.json)
passes all three complete notice checks, 21 reported dependencies and same-image
console validation. Altered executable and truncated-notice images refuse
before Node starts; owned controls and inspection containers were removed.
This dirty source candidate is separate from clean hosted qualification.
The [native package/notice probe](../../demos/evidence/ops12-native-package-notice-probe.json)
now uses existing Syft catalogers on that retained image: 106 installed Debian
packages, 119 complete copyright/common-licence files and four native-library
files match independent stopped-image reads. All installed package notices map
to verified bytes; SPDX file hashes match. Six static private-Node probes expose
macOS/Linux host-library imports, but PE import lists remain unavailable.
This selects no new production mechanism and qualifies no new source or target.
Next finish the pinned Node/V8 source and notice review, then record the narrow
choice for installed-package/raw-notice checks in existing image/SBOM reports.
Official source downloads and the branch push currently fail with TLS connection
timeouts/reset; retry after GitHub access recovers. Then continue plugin/chart
and other-image coverage before vulnerability and publisher incident policy.
The [pinned source review](../../demos/evidence/ops12-node-source-notice-review.json)
now verifies both official source archives and identifies missing V8 helper
notices. The existing pins/readers carry four additional complete notices in
both Node versions, plus Highway in private Node 24 clients. Installed LLVM
reads both original Windows import lists; this is static metadata only.
[Actual local carriage](../../demos/evidence/ops12-node-helper-notices-candidate.json)
passes seven product notices, eight client notices, same-image console checks
and all nine macOS client checks. Coherently changed/truncated new notices refuse;
owned controls are removed. All 54 focused, 44 CI-tooling and 84 release-parity
checks pass, together with deployment/chart, formatting and fast/diff checks.
No new scanner, report format, release sidecar or clean hosted qualification is
introduced. Ncrypto/fast_float provenance and conditional V8 selection remain
open; GitHub source reads and branch push remain transport-blocked. Next resolve
those exact upstream gaps and independently record the narrow installed-package
and raw-notice report choice before implementation, then plugin/chart/other-image
coverage and security policies. The saved automation remains paused. The native
Windows scanner blocker and OPS-12's open state remain unchanged; its brief owns
the exact next action.

ADR-0136 now extends the existing product SPDX and image reports with installed
Debian identities, complete notice hashes and native-library ownership. Independent
stopped-image reads corroborate the database, actual bytes and native ELF target;
assembly requires matching local and both-registry evidence inside the unchanged
51-payload plan. [Actual ARM evidence](../../demos/evidence/ops12-product-package-inventory-candidate.json)
binds a new clean `7d1ab12e` product artifact to the dirty reader candidate:
106 packages, 119 full notices, four native libraries and the database pass.
Two real changed-image controls refuse and are removed. This is local candidate
evidence, not clean hosted qualification. A chart packaging defect exposed by
the unchanged reproducibility gate is fixed by stable private staging timestamps;
notice bytes remain intact. All 44 focused, 53 CI-tooling and 85 release-parity
tests pass, with fast, deployment/chart, Rust formatting and diff checks.
Next resolve the pinned ncrypto/fast_float provenance and remaining Node/native
coverage, then plugin/chart/other-image inventory and notices before security
policies. GitHub source reads and the feature push remain transport-blocked;
retry when access returns. The saved automation stays paused. The Windows
scanner, operator bucket and real issuer/harness blockers remain separate;
the OPS-12 brief owns the exact continuation checkpoint.

The [plugin/chart inventory probe](../../demos/evidence/ops12-plugin-chart-inventory-probe.json)
now identifies unused test/mock/driver outputs in Claude's broad copy. ADR-0065
amendment 14 closes that payload to the existing runtime module list; native
clients reuse it without a separate filter, and Darwin tar metadata is excluded.
All 73 retained plugin files preserve original runtime/notice bytes. Three actual
controls refuse; 35 archive replay tests, 21 client-package tests and nine actual
macOS ARM client checks pass. All 28 original locked chart members and its retained
complete licence match packaged bytes. Explicit Syft installed-package cataloging
finds four first-party npm occurrences; its initial empty scans and missing
Claude/Helm component identities cannot establish complete coverage. Next record
the narrow metadata/notice inventory choice before implementing existing-artifact
source/hash gates, resolve pinned Node notice provenance when GitHub access
recovers, then complete other-image coverage before security policies. No clean
hosted or six-target qualification follows; all external blockers and the paused
automation remain unchanged.

Completed slices are committed as `4ff8e6ac` and `fe3e1e4f`. The owner
explicitly authorized the pending feature-branch commits and checkpoint with
`push it`, resolving the earlier automatic approval rejection. After the initial
HTTPS/TLS failures and refused SSH authentication, the requested HTTPS retry
succeeded: all seven pending commits through
`e665d486c85a39e951b302858cbe23372a1a1c52` reached `synveda/synveda` on
`codex/synveda-production-roadmap`. An independent `git ls-remote` read confirmed
the exact remote/local HEAD match. The push blocker is resolved; no new product
or hosted qualification follows. Retry the exact pinned upstream notice reads
as needed rather than assuming their earlier TLS failures persist. The
implementation order above and paused automation remain current; the OPS-12
brief owns the exact continuation checkpoint.

[PR #67](https://github.com/synveda/synveda/pull/67) now reviews the branch.
Its initial CI `36908968804` packaging job failed a ZIP fixture because Linux
`bsdtar` was absent. The job now installs the existing ADR-0133
`libarchive-tools` prerequisite; 53 CI-tooling and 21 client-package tests and
pinned actionlint pass. Corrected source `854bc77f` passed that job in
[CI 36919629883](https://github.com/synveda/synveda/actions/runs/36919629883);
only both native Windows SBOM jobs failed the missing executable SHA-256 gate.
The official immutable Syft 1.54.0 release now contains upstream fix 5341.
ADR-0133 records the reviewed native pin upgrade; all six archives, extracted
executable hashes/sizes and platform headers were independently checked.
Both Windows probes in
[diagnostic CI 37117271010](https://github.com/synveda/synveda/actions/runs/37117271010)
passed the actual SHA-256 binding and reported the exact basename `synveda.exe`.
[Retained resolver evidence](../../demos/evidence/ops12-native-syft-release-probe.json)
binds those observations to the reviewed tool bytes and source. That diagnostic
run was stopped after both probes; it is not product qualification. The
validator now requires the basename on every target; the former Windows spelling
refuses. The regression failed against the former check before the fix, and the
temporary script/workflow step is removed. Implementation source
`92a1e26ad43fa93fd0983a0d12c4a3c42c014f94` passes all 53 CI-tooling tests,
fast/dependency gates, formatting, pinned actionlint and diff checks. Its initial
PR run `37118099996` is superseded by this documentation checkpoint; assess the
latest PR checks before continuation. Complete same-source native/archive and
nonpublishing Release qualification remains separate and pending; do not resume
the obsolete `ef6d61db` runs. Next select one clean corrected source and record
fresh full CI/nonpublishing Release IDs before retaining the original 51-payload
proof. No new full qualification pair is selected. The documented inventory
order follows that qualification.
Production qualification and the operator/issuer blockers remain open; the
automation remains paused.

- [ ] [CNSL-3: Audit temporal and disclosure views](CNSL-3.md) — open
- [x] CNSL-4: Knowledge browser — delivered
- [x] CNSL-5: Console theme and everyday usability — delivered
- [ ] [AUD-5: Compliance mapping doc](AUD-5.md) — open
- [ ] [AUTHZ-6: Authorisation scale decision](AUTHZ-6.md) — open
- [ ] [AUTHZ-7: Governed admin-plane mutation](AUTHZ-7.md) — open
- [ ] [TEN-7: Tenant storage partition decision](TEN-7.md) — open
- [ ] [EVAL-7: A second public benchmark](EVAL-7.md) — open

## Phase 5 — Context platform

- [x] CPR-1: Implementation baseline & locked decisions — delivered
- [x] CPR-2: Fresh schema epoch, startup guard & local reset — delivered
- [x] CPR-3: Generic governed scope substrate — delivered
- [x] CPR-4: Workspaces, projects & canonical repository identity — delivered
- [x] CPR-5: Membership, groups, grants & invitations — delivered
- [x] CPR-6: Governed scope anchors — the PDP re-cut — delivered
- [x] CPR-7: The hierarchy cutover — one scope tree — delivered
- [x] CPR-8: The console product shell & first-run onboarding — delivered
- [x] CPR-9: The foundation audit — hardening the scope and access cutover — delivered
- [x] CPR-10: The session ledger and runtime API — delivered
- [x] CPR-11: The session product experience — delivered
- [x] CPR-12: Durable Claude session delivery — delivered
- [x] CPR-13: The demo corpus re-point — delivered
- [x] CPR-14: Live Claude Code session acceptance gate — delivered
- [x] CPR-15: Versioned Knowledge aggregate and provenance — delivered
- [x] CPR-16: Governed Knowledge mutation lifecycle — delivered
- [x] CPR-17: Public Knowledge API, search and browser — delivered
- [x] CPR-18: Session-based capture batches and reviewable candidates — delivered
- [x] CPR-19: New Learnings lightweight review workflow — delivered
- [x] CPR-20: Explainable Knowledge context planning and scoped query — delivered
- [x] CPR-21: Context Inspector and outcome feedback — delivered
- [x] CPR-22: Core individual and small-team MVP acceptance — delivered
- [x] CPR-23: Immutable skill versions, bindings and usage — delivered
- [x] CPR-24: Skills Library product experience — delivered
- [x] CPR-25: Trusted MCP server catalogue and project bindings — delivered
- [x] CPR-26: MCP Tools catalogue product experience — delivered
- [x] CPR-27: OKF v0.2 knowledge exchange adapter — delivered
- [x] CPR-28: OKF import and export product workflows — delivered
- [x] CPR-29: Public contract and client convergence — delivered
- [x] CPR-30: Governed runtime configuration artifacts — delivered
- [x] CPR-31: Governed auto-apply and policy relaxations — delivered
- [x] CPR-32: Unified approvals across governed artifacts — delivered
- [x] CPR-33: Context-platform audit query and deterministic export — delivered
- [x] CPR-34: Directory adapter convergence — delivered
- [x] CPR-35: Context-platform key and secret convergence — delivered
- [x] CPR-36: One-runtime deployment convergence — delivered
- [x] CPR-37: Conflict, supersession and freshness engine — delivered
- [x] CPR-38: Bounded graph-augmented retrieval — delivered
- [x] CPR-39: Second verified client — delivered
- [x] CPR-40: Context-platform product and trust evaluation — delivered
- [x] CPR-41: One-command realistic product demo — delivered
- [x] CPR-42: Context-platform security and product-integrity audit — delivered
- [x] CPR-43: Final context-platform hard cut — delivered
- [x] CPR-44: Production hardening and maintainability cut — delivered
- [ ] [CPR-45: Docker-first portable reference deployment](CPR-45.md) — open

## Unscheduled — not listed in the Sequencing section

- [ ] [AUTH-6: Session & token hygiene](AUTH-6.md) — open

ADR-0130 now accepts a tenant-scoped, credential-free console-session index
beside the pre-tenant custody row, self-only inventory/revoke, and distinct
per-token `jti` and session-family `sid` revocation for a candidate bundled-
Keycloak profile. Its five-minute access, eight-hour session and rotating
refresh contract still needs live proof; external issuers have no revocation
claim. The combined maintenance worker now purges expired
credential rows in bounded, skip-locked batches with sweep and row counters.
Each minute it now drains up to 16 batches of 256 rows within a ten-second
deadline, stopping early on a partial batch. Indexed oldest-expired age and
budget-hit counters expose lag and saturation; a 4,097-row exact-role fixture
proves two bounded passes. The arrival and retention envelopes remain unproved;
next measure age and budget hits under representative load.
The OIDC verifier now carries bounded optional `jti`/`sid`, exact issuer and
token times after signature verification; a mock-signed fixture covers these
values and tampering but is not live Keycloak evidence. No AUTH-6 revocation
bound or inventory is qualified. Next prove the bundled issuer's signed
`jti`/stable `sid` and refresh replay behavior, then implement
the additive index, self-session API and fail-closed request-time lookup before
enforcement and cross-replica acceptance. The local 2026-09-30 preflight has
no running Synveda Keycloak and the development hostname block belongs to the
retained `synveda-development-acceptance-interop` project; use a separate
supported host or a deliberate operator handoff for that live proof.
