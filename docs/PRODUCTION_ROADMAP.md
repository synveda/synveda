# Production roadmap

Planning proposal, 2026-09-29. This document orders work for a first supported
self-hosted offer, then a measured medium-team and dedicated enterprise offer.
It does not mark any feature delivered or change the [readiness verdict](PRODUCTION_READINESS.md).
[STATUS](backlog/STATUS.md) owns feature state; open briefs own acceptance;
[current ADRs](adr/README.md) own architecture. The owner accepted the first
offer's recovery, release, capacity and policy-pack values on 2026-11-01
(below); the rest of its support contract remains open.

## First offer: one small-team installation

Qualify a published, self-hosted Linux/Compose HTTPS installation first. Plan
for one customer tenant, one exact OIDC issuer, one gateway and one core worker,
with documented maintenance interruption and independently retained recovery
material. Qualify each advertised CPU architecture and client version from
published bytes. Team size (roughly 2–25 people for planning) is not a capacity
promise; the supported concurrent-agent, event, corpus, retention and provider
envelope comes from [Load & latency suite](backlog/EVAL-6.md). A customer needing availability
across host failure needs a separately qualified topology.

The release should complete one real two-person journey: install, sign in,
create a workspace and project, admit a teammate, connect a named supported
client, collect Session evidence, approve a Knowledge candidate, reuse it,
install and bind an immutable Skill, review an update, roll back its binding,
and inspect content-free audit evidence. The present product implements the
main components; [Release-shaped beta acceptance](backlog/OPS-9.md), [Consumer installation and harness setup](backlog/OPS-12.md) and
[Docker-first portable reference deployment](backlog/CPR-45.md) must establish the complete published-artifact
journey and supported installation contract. Tool catalogue evidence is
discovery and approval, not remote tool-execution enforcement.

Small-team production promotion requires the following gates on the **same
release artifacts and declared topology**. An unavailable live environment is
an open gate, not a pass.

| Gate | Feature ownership and required evidence |
| --- | --- |
| Recoverability and key custody | [Backup/restore & DR](backlog/OPS-5.md): encrypted off-host base backup and WAL/PITR, separately protected Synveda/identity/key recovery set, selected-point isolated restore, wrong/missing-key refusal, audit-prefix verification and recurring measured RPO/RTO. Qualify local key custody and rotation for this offer; a cloud KMS is not assumed. |
| Safe upgrades | [Upgrade and rollback discipline](backlog/OPS-6.md): accepted compatibility policy, a genuinely compatible published N-1/N pair, read-only preflight, data/key-preserving upgrade, injected-failure recovery and a measured maintenance window. No reset of customer production data. |
| Access and workload bounds | [Session & token hygiene](backlog/AUTH-6.md): audited session inventory/revocation and measured token/issuer behavior. [Request limits and tenant usage budgets](backlog/OPS-14.md) owns PR-07's request/concurrency limits and tenant usage budgets. Prove overload and revocation bounds on supported issuers. |
| Reliable work and operations | [Capture retry and provider backpressure](backlog/OPS-13.md) owns PR-05's bounded Capture retry/backpressure; [Operational signals and recovery runbooks](backlog/OPS-15.md) owns PR-10's SLIs, alerts, runbooks and support diagnostics. Demonstrate provider failure, database interruption, actionable alerts and operator recovery without lost governed effects. |
| Tenant and security lifecycle | [Tenant lifecycle](backlog/TEN-5.md) and [Cross-tenant isolation test harness](backlog/TEN-6.md): scoped suspension/export/import/erasure semantics and generated cross-tenant route/table checks. Keep backup-retention and legal-hold treatment explicit. |
| Measured capacity and search | [Load & latency suite](backlog/EVAL-6.md): mixed public-API workload, noisy-neighbour/fault tests, 24-hour soak, p50/p95/p99, lag and storage growth. [Dense-leg plan stability](backlog/CTX-7.md) must pass for any dense-search performance promise; a narrower lexical-first contract can be qualified separately. |
| Release and client support | [Release-shaped beta acceptance](backlog/OPS-9.md), [Consumer installation and harness setup](backlog/OPS-12.md) and [Docker-first portable reference deployment](backlog/CPR-45.md): independent HTTPS installation, supported-client use from released archives, verified artifact identity/SBOM and a published support/update/security-response window. [Python & TS SDKs](backlog/ADPT-4.md) publishes its explicitly bounded SDK slice if offered. |

### Owner decisions (2026-11-01)

The owner accepted these values for the first small-team offer. They are the
targets the live recovery, upgrade, alerting and capacity gates must meet. The
v0.4.7 evaluation release makes none of these commitments.

| Decision | Value | Owning brief |
| --- | --- | --- |
| RPO / RTO | 15 minutes / 4 hours | [Backup/restore & DR](backlog/OPS-5.md) |
| Backup destination and window | An S3-compatible bucket in the customer's own account with object lock enabled; 14-day point-in-time window | [Backup/restore & DR](backlog/OPS-5.md) |
| Key custody | Local file-backed KEK and sealed recovery bundle, held offline by a custodian the customer names; yearly KEK replacement; no cloud KMS | [Backup/restore & DR](backlog/OPS-5.md) |
| Drill cadence | `pitr-drill.sh` monthly; a manual isolated restore quarterly | [Backup/restore & DR](backlog/OPS-5.md), [Operational signals and recovery runbooks](backlog/OPS-15.md) |
| Supported releases | The current release and the one before it; N-1 upgrades directly to N | [Upgrade and rollback discipline](backlog/OPS-6.md) |
| Upgrade outage | A planned maintenance window stated in each release's notes | [Upgrade and rollback discipline](backlog/OPS-6.md) |
| Capacity envelope | The proposal in [the capacity envelope](CAPACITY.md), approved only after its 24-hour soak passes | [Load & latency suite](backlog/EVAL-6.md) |
| Small-team policy pack | `regulated-strict` stays the default; first-run onboarding explains the packs and lets the first administrator choose `standard` | [Consumer installation and harness setup](backlog/OPS-12.md) |

Three of these need mechanism work before they can be met. Compose takes base
backups only on demand and never expires them, so the 14-day window needs a
schedule and a verified expiry step. Object lock must be qualified with the
backup tool. Under `regulated-strict` a Configuration change needs four
distinct people (an author, two other administrators approving and a separate
person applying), so the pack choice must happen when the tenant is first
configured; that needs an ADR first.

Still open: the maximum upgrade outage, minimum rehearsal dataset and pre-1.0
reset end date; SLOs, on-call, support tiers and telemetry backend; the
vulnerability response window; erasure, retention and legal-hold rules; and
the concrete bucket, custodian and isolated host for the qualifying drill.
[SECURITY.md](../SECURITY.md) keeps stating that no support window exists until
a release actually ships under these terms.

[ADR-0132](adr/adr-0132-verify-publisher-before-installing-release-code.md) adds
source-installer publisher verification for the release gate: remote installs
require the attested inventory, fixed release workflow/tag and expected commit
before code execution. Clean source `f433eb17` passed full CI and the
nonpublishing release drill on all six native clients and both Linux
architectures. [Independent evidence](../demos/evidence/ops12-source-qualification.json)
verifies all 31 same-run assets and source notice carriage in archives, charts,
plugin packages and images. Native Windows publisher-policy fixtures pass;
real issuer/harness use and signed future publication remain unqualified.
Complete artifact SBOMs, vulnerability and publisher incident policy, PR-13's
support window and third-party notice review remain open under consumer installation and release distribution.
This source checkpoint does not change the production verdict.

[ADR-0133](adr/adr-0133-embed-rust-dependency-inventory-in-release-binaries.md)
now embeds pinned Rust metadata in product/browser images and requires actual
OCI subject/hash and Cargo-content checks during candidate creation and assembly.
[Local native ARM evidence](../demos/evidence/ops12-oci-rust-sbom.json)
contains 351/295 distinct Cargo identities and an ordinary-binary refusal.
Clean hosted qualification on both architectures, native archive SBOMs and
broader dependency coverage remain open under consumer installation and harness setup.

The next native archive source increment now requires per-binary Rust SPDX and
hash-bound reports for six clients and both historical server archives. Local
macOS ARM packaging and client execution pass; its separate clean hosted drill
is still required. Non-Rust inventory, third-party notices and vulnerability/
publisher incident policy follow under consumer installation and harness setup.

The first implementation sequence is: establish the supported deployment and
upgrade contract; drill backup/restore & DR against the accepted recovery
values; prove an upgrade and rollback discipline release pair;
then close access, workload, operations and lifecycle gates before a complete
published-artifact acceptance run. Document owner choices in the relevant open
brief and record any new architecture in an ADR before implementation. The
current [v0.4.7 release](https://github.com/synveda/synveda/releases/tag/v0.4.7)
remains a self-hosted evaluation release while these gates are open.

The first upgrade and rollback discipline implementation candidate follows
[ADR-0121](adr/adr-0121-preserve-v0-4-3-through-forward-migration.md): retain the
exact published v0.4.3 `0001` bytes and apply the session-checkpoint and context-optimisation additions through a
transactional `0002`, with a read-only prior-head check and planned writer
interruption. v0.4.0 has a different `0001` checksum and remains refused. A
source database test or same-source Helm rerun is not a released N-1 upgrade;
the published-artifact, joint recovery and failure drills remain required.
[Small-team Kubernetes release](backlog/OPS-11.md) records earlier release evidence.

The first backup/restore & DR source slice follows
[ADR-0122](adr/adr-0122-provider-owned-object-store-backup-seam.md): optional
CNPG WAL archiving and scheduled base backups reference an operator-owned
Barman Cloud ObjectStore. The S3-compatible/AWS path has a render-tested chart
contract. [ADR-0123](adr/adr-0123-opt-in-compose-physical-backup.md) adds a
source-built opt-in Compose pgBackRest image with S3 WAL archiving and a bounded
base-backup action. Neither has live off-host backup, selected-point restore or
joint custody evidence; the first-offer recovery gate remains open under backup/restore & DR.

## Medium-team deployment

Qualify the existing Helm chart on a named customer-managed Kubernetes platform
when that platform or its controls are needed. External PostgreSQL and OIDC,
ingress, enforcing CNI, CSI, certificates, backup and exact release images each
need live qualification; Kind renders and local fixtures are narrower evidence.
One gateway/worker with planned interruption can remain the initial contract
if measured capacity and the customer availability target permit it. A genuine
availability promise requires [Gateway horizontal scale](backlog/OPS-7.md): durable cross-pod
login/handoff, bounded authority invalidation, safe work claiming and draining,
then pod-loss and rolling-upgrade tests. Add [Authentic Skill usage reporting](backlog/SKIL-5.md) usage
reporting and [Audit temporal and disclosure views](backlog/CNSL-3.md) audit views where teams need them;
preserve unknown execution/outcome states. [Identity stitching](backlog/MEM-7.md) is required
before issuer replacement or multiple issuers share a tenant.

## Dedicated enterprise, then managed service

Start with one named customer region/VPC or on-prem environment, one qualified
enterprise IdP, key provider and audit destination. Prove joint application,
database, identity and key failover/recovery against contracted objectives;
qualify actual network, storage and provider behavior. Add [External immutable audit retention](backlog/AUD-3.md)
external immutable audit custody, [SIEM streaming](backlog/AUD-4.md) durable SIEM delivery,
[Governed admin-plane mutation](backlog/AUTHZ-7.md) review of selected high-impact authority changes,
scoped incident support access and [Compliance mapping doc](backlog/AUD-5.md) owner-reviewed
control/evidence mapping. A dedicated single-region offer does not require
global routing. [Residency routing](backlog/OPS-3.md) starts only with a funded regional
hosting/residency promise that includes model providers, telemetry, backups and
keys. Pooled SaaS additionally needs a separately designed tenant provisioning,
metering, quota, support and billing plane.

After the first recoverable release, product differentiation can build on the
existing immutable Skill and MCP catalogues: verified publisher/source identity,
recurring scan and revocation evidence, impact-aware Skill rollout, and honest
exposure/activation/outcome reporting. These are new scope where the current
briefs do not cover them; assign feature IDs and decisions before implementation.
Keep optional context optimisation ([Session compression assist](backlog/CTX-6.md),
[Governed context optimisation](backlog/CTX-8.md)) behind its live client/model-quality gates. Do not
infer task success or provider savings from local token-count reductions.

## Agent ecosystem review (2026-11-01)

A survey of agent harnesses, SDKs, memory products and governance practice on
2026-11-01 found work this plan did not cover. None of it is a first-offer gate
unless the owner makes it one; each brief starts with the owner decision or
ADR it needs.

| Tier | Work | Why now |
| --- | --- | --- |
| Small team | [Harness-native memory coexistence](backlog/ADPT-10.md) | Claude Code, Codex and Copilot now keep their own unreviewed memory beside Synveda's. |
| Small to medium | [Agent SDK session-store adapters](backlog/ADPT-11.md) and the [Microsoft Agent Framework context provider](backlog/ADPT-7.md), retargeted from Semantic Kernel | Session stores became an SDK integration point; Agent Framework succeeds Semantic Kernel. |
| Medium and enterprise | [Additional API transport](backlog/ADPT-3.md), retargeted from gRPC to a remote MCP endpoint with MCP authorization | Cloud and hosted agents cannot launch a local stdio server, and MCP's enterprise-managed authorization is now stable. |
| Enterprise | [Delegated agent authority](backlog/AUTHZ-8.md) | Agents act with their person's full grants; major IdPs now treat agents as identities with human sponsors. |
| All | [Governed scheduled consolidation](backlog/MEM-10.md) | Offline memory consolidation is common, and mostly unreviewed. |

[A second public benchmark](backlog/EVAL-7.md) now lists licence-checked
candidates, and the [compliance mapping](backlog/AUD-5.md) names external
threat, assurance and regulatory reference points.
