# Production roadmap

Planning proposal, 2026-09-29. This document orders work for a first supported
self-hosted offer, then a measured medium-team and dedicated enterprise offer.
It does not mark any feature delivered or change the [readiness verdict](PRODUCTION_READINESS.md).
[STATUS](backlog/STATUS.md) owns feature state; open briefs own acceptance;
[current ADRs](adr/README.md) own architecture. The first release target below
is a working assumption until the deployment owner selects its support contract.

## First offer: one small-team installation

Qualify a published, self-hosted Linux/Compose HTTPS installation first. Plan
for one customer tenant, one exact OIDC issuer, one gateway and one core worker,
with documented maintenance interruption and independently retained recovery
material. Qualify each advertised CPU architecture and client version from
published bytes. Team size (roughly 2–25 people for planning) is not a capacity
promise; the supported concurrent-agent, event, corpus, retention and provider
envelope comes from [EVAL-6](backlog/EVAL-6.md). A customer needing availability
across host failure needs a separately qualified topology.

The release should complete one real two-person journey: install, sign in,
create a workspace and project, admit a teammate, connect a named supported
client, collect Session evidence, approve a Knowledge candidate, reuse it,
install and bind an immutable Skill, review an update, roll back its binding,
and inspect content-free audit evidence. The present product implements the
main components; [OPS-9](backlog/OPS-9.md), [OPS-12](backlog/OPS-12.md) and
[CPR-45](backlog/CPR-45.md) must establish the complete published-artifact
journey and supported installation contract. Tool catalogue evidence is
discovery and approval, not remote tool-execution enforcement.

Small-team production promotion requires the following gates on the **same
release artifacts and declared topology**. An unavailable live environment is
an open gate, not a pass.

| Gate | Feature ownership and required evidence |
| --- | --- |
| Recoverability and key custody | [OPS-5](backlog/OPS-5.md): encrypted off-host base backup and WAL/PITR, separately protected Synveda/identity/key recovery set, selected-point isolated restore, wrong/missing-key refusal, audit-prefix verification and recurring measured RPO/RTO. Qualify local key custody and rotation for this offer; a cloud KMS is not assumed. |
| Safe upgrades | [OPS-6](backlog/OPS-6.md): accepted compatibility policy, a genuinely compatible published N-1/N pair, read-only preflight, data/key-preserving upgrade, injected-failure recovery and a measured maintenance window. No reset of customer production data. |
| Access and workload bounds | [AUTH-6](backlog/AUTH-6.md): audited session inventory/revocation and measured token/issuer behavior. [OPS-14](backlog/OPS-14.md) owns PR-07's request/concurrency limits and tenant usage budgets. Prove overload and revocation bounds on supported issuers. |
| Reliable work and operations | [OPS-13](backlog/OPS-13.md) owns PR-05's bounded Capture retry/backpressure; [OPS-15](backlog/OPS-15.md) owns PR-10's SLIs, alerts, runbooks and support diagnostics. Demonstrate provider failure, database interruption, actionable alerts and operator recovery without lost governed effects. |
| Tenant and security lifecycle | [TEN-5](backlog/TEN-5.md) and [TEN-6](backlog/TEN-6.md): scoped suspension/export/import/erasure semantics and generated cross-tenant route/table checks. Keep backup-retention and legal-hold treatment explicit. |
| Measured capacity and search | [EVAL-6](backlog/EVAL-6.md): mixed public-API workload, noisy-neighbour/fault tests, 24-hour soak, p50/p95/p99, lag and storage growth. [CTX-7](backlog/CTX-7.md) must pass for any dense-search performance promise; a narrower lexical-first contract can be qualified separately. |
| Release and client support | [OPS-9](backlog/OPS-9.md), [OPS-12](backlog/OPS-12.md) and [CPR-45](backlog/CPR-45.md): independent HTTPS installation, supported-client use from released archives, verified artifact identity/SBOM and a published support/update/security-response window. [ADPT-4](backlog/ADPT-4.md) publishes its explicitly bounded SDK slice if offered. |

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
support window and third-party notice review remain open under OPS-12/OPS-8.
This source checkpoint does not change the production verdict.

[Proposed ADR-0133](adr/adr-0133-embed-rust-dependency-inventory-in-release-binaries.md)
selects pinned embedded Rust metadata and SBOM content gates. The isolated
macOS ARM CLI probe retained 295 packages through stripping; release build
implementation and native/image qualification remain open under OPS-12.

The first implementation sequence is: establish the supported deployment and
upgrade contract; choose backup destination, retention, encryption/key custody
and recovery objectives; implement and drill OPS-5; prove an OPS-6 release pair;
then close access, workload, operations and lifecycle gates before a complete
published-artifact acceptance run. Document owner choices in the relevant open
brief and record any new architecture in an ADR before implementation. The
current [v0.4.3 release](https://github.com/synveda/synveda/releases/tag/v0.4.3)
remains a self-hosted evaluation release while these gates are open.

The first OPS-6 implementation candidate follows
[ADR-0121](adr/adr-0121-preserve-v0-4-3-through-forward-migration.md): retain the
exact published v0.4.3 `0001` bytes and apply CTX-6/CTX-8 additions through a
transactional `0002`, with a read-only prior-head check and planned writer
interruption. v0.4.0 has a different `0001` checksum and remains refused. A
source database test or same-source Helm rerun is not a released N-1 upgrade;
the published-artifact, joint recovery and failure drills remain required.
[OPS-11](backlog/OPS-11.md) records earlier release evidence.

The first OPS-5 source slice follows
[ADR-0122](adr/adr-0122-provider-owned-object-store-backup-seam.md): optional
CNPG WAL archiving and scheduled base backups reference an operator-owned
Barman Cloud ObjectStore. The S3-compatible/AWS path has a render-tested chart
contract. [ADR-0123](adr/adr-0123-opt-in-compose-physical-backup.md) adds a
source-built opt-in Compose pgBackRest image with S3 WAL archiving and a bounded
base-backup action. Neither has live off-host backup, selected-point restore or
joint custody evidence; the first-offer recovery gate remains open under OPS-5.

## Medium-team deployment

Qualify the existing Helm chart on a named customer-managed Kubernetes platform
when that platform or its controls are needed. External PostgreSQL and OIDC,
ingress, enforcing CNI, CSI, certificates, backup and exact release images each
need live qualification; Kind renders and local fixtures are narrower evidence.
One gateway/worker with planned interruption can remain the initial contract
if measured capacity and the customer availability target permit it. A genuine
availability promise requires [OPS-7](backlog/OPS-7.md): durable cross-pod
login/handoff, bounded authority invalidation, safe work claiming and draining,
then pod-loss and rolling-upgrade tests. Add [SKIL-5](backlog/SKIL-5.md) usage
reporting and [CNSL-3](backlog/CNSL-3.md) audit views where teams need them;
preserve unknown execution/outcome states. [MEM-7](backlog/MEM-7.md) is required
before issuer replacement or multiple issuers share a tenant.

## Dedicated enterprise, then managed service

Start with one named customer region/VPC or on-prem environment, one qualified
enterprise IdP, key provider and audit destination. Prove joint application,
database, identity and key failover/recovery against contracted objectives;
qualify actual network, storage and provider behavior. Add [AUD-3](backlog/AUD-3.md)
external immutable audit custody, [AUD-4](backlog/AUD-4.md) durable SIEM delivery,
[AUTHZ-7](backlog/AUTHZ-7.md) review of selected high-impact authority changes,
scoped incident support access and [AUD-5](backlog/AUD-5.md) owner-reviewed
control/evidence mapping. A dedicated single-region offer does not require
global routing. [OPS-3](backlog/OPS-3.md) starts only with a funded regional
hosting/residency promise that includes model providers, telemetry, backups and
keys. Pooled SaaS additionally needs a separately designed tenant provisioning,
metering, quota, support and billing plane.

After the first recoverable release, product differentiation can build on the
existing immutable Skill and MCP catalogues: verified publisher/source identity,
recurring scan and revocation evidence, impact-aware Skill rollout, and honest
exposure/activation/outcome reporting. These are new scope where the current
briefs do not cover them; assign feature IDs and decisions before implementation.
Keep optional context optimisation ([CTX-6](backlog/CTX-6.md),
[CTX-8](backlog/CTX-8.md)) behind its live client/model-quality gates. Do not
infer task success or provider savings from local token-count reductions.
