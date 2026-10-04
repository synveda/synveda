# OPS-11: Small-team Kubernetes release

## Problem and evidence

The published [v0.4.3 release](https://github.com/synveda/synveda/releases/tag/v0.4.3)
includes the Helm chart, digest-bound image overlays and native Linux AMD64/ARM64
Kubernetes reports. The existing chart offers external, bundled or optional
preinstalled-CNPG PostgreSQL independently of bundled Keycloak or external OIDC.
Kind acceptance covers the four operator-free database/identity combinations;
source-built CNPG starter runs cover the other two. Team admission, Session and
Capture, Knowledge and Context, interruption recovery, retained reinstall and
joint logical restore have executable fixtures. See the
[installation guide](../../deploy/helm/synveda/README.md),
[operations runbook](../../deploy/helm/synveda/OPERATIONS.md) and
[CI record](../CI.md) for the current contract and reports.

Two release/platform decisions remain. Routes, assigned-ID workloads and NetworkPolicies render,
but only Kind and structural schema checks have run; no real OpenShift SCC,
router, CNI or CSI target has been qualified. Separately, published v0.4.0 and
v0.4.3 both use schema epoch 3 and baseline revision 3, but v0.4.3 changes the
single `0001_context_platform.sql` migration by adding `source_payload_hash`.
The read-only `synveda db migrate --check` compares the exact SQLx migration
checksum, so this pair cannot be declared a supported in-place upgrade.
OPS-6 / ADR-0121 now carries the exact v0.4.3 baseline plus a source-only
`0002` forward migration; there is still no accepted published upgrade pair.

## Scope

- Qualify a named, disposable OpenShift 4.19 or 4.20 target with external
  PostgreSQL and OIDC first. Record the exact patch, admitted SCC, assigned
  UID/group, storage class, ingress/router, CNI and provider versions. Qualify
  packaged Keycloak and optional CNPG separately before claiming them on that
  target.
- Decide the pre-1.0 upgrade promise. Either implement and test a
  data-preserving forward migration for an explicitly supported published pair,
  or state that OPS-11 ships fresh installations and keep cross-release upgrade
  work in [OPS-6](OPS-6.md). Do not treat same-source Helm reapply as an upgrade.
- Keep installation, portability and release guidance aligned with the published
  artifacts and the selected support boundary.
- Complete the human first-install journey under ADR-0137: repeatable native
  preparation, customer presets, prerequisite checks, private first-human login,
  truthful client setup, governed Knowledge retrieval and safe recovery. Qualify
  the guide on a named self-managed target and with three unfamiliar engineers
  before making platform or setup-time claims.

## Non-goals

No new operator, authentication framework, service mesh, cloud provisioning,
application HA, zero-downtime rollout or automatic old-epoch data conversion.
[OPS-5](OPS-5.md) owns recurring encrypted off-host backup and PITR;
[OPS-7](OPS-7.md) owns multi-replica gateway/worker availability. Optional
model-provider qualification does not block the lexical small-team path.

## Architecture seam

Extend the single [Synveda chart](../../deploy/helm/synveda/values.yaml) under
[ADR-0109](../adr/adr-0109-small-team-kubernetes-release.md) through
[ADR-0112](../adr/adr-0112-small-team-operational-evidence.md). One product
image supplies a gateway/console, private worker and bounded installation Job.
Secrets remain file-mounted; database runtime roles remain separate and
non-owner. External providers retain their own bootstrap, credentials and
recovery. The chart installs no cluster-scoped operator or ingress controller.

The starter retains one database instance and planned application downtime.
OpenShift overlays must pass the existing Cedar, forced-RLS, VedaFlow, audit and
issuer contracts without an SCC exception or authority bypass. The
[portability guide](../../deploy/helm/synveda/PORTABILITY.md) owns the exact
platform inputs; the [runbook](../../deploy/helm/synveda/OPERATIONS.md) owns
backup, restore and rollback procedure.

## Acceptance criteria

- The published chart and image digests remain tied to the tested source.
  Existing native Kind reports cover login, team admission, Session/Capture,
  Knowledge/Context, restart, role separation and isolated logical restore for
  all four operator-free combinations; chart contracts cover the optional CNPG
  combinations.
- A real selected OpenShift target admits gateway, worker and installer under
  its restricted assigned UID with no SCC exception. Fresh install and
  same-source upgrade pass real OIDC login, membership/revocation,
  Session/Capture, Knowledge/Context, restart, audit and retained-data checks.
- The real Route redirects to HTTPS with the expected certificate and renewal;
  metrics, readiness and identity administration stay private. An enforcing CNI
  allows only declared DNS, database, issuer and optional-provider traffic;
  unrelated ingress and undeclared egress fail. Storage survives pod recreation
  and an isolated restore is checked against original keys and subjects.
- Packaged Keycloak, CNPG-generated pods or managed providers are added to the
  supported target matrix only after their own live admission, storage, trust
  and recovery checks. A render or Kind run alone does not grant that claim.
- The upgrade policy is explicit. If a published pair is supported, a restored
  prior-release database passes read-only compatibility, migration, public
  functional checks and rollback-or-restore rehearsal without discarding data.
  Otherwise the release guide clearly limits OPS-11 to fresh installation and
  leaves published cross-release compatibility to OPS-6.

## Required tests

Keep the existing `make ci`, `make db-test`, `make chart-lint`,
`make chart-schema`, `make check-deploy`, `make compose-config`,
`make claude-acceptance` and `make eval-check` gates. Run the
[Helm acceptance fixture](../../demos/ops-2-helm-install.sh) against only an
explicitly selected disposable target; retain content-free results and exact
artifact identities. Test the upgrade candidate with `synveda db migrate
--check` against a restored published database before allowing a write. A
missing cluster or supported release pair is an unmet acceptance item, not a
passing test.

### Fresh-install source checkpoint (2026-10-03)

This uncommitted candidate starts at
`d289fd46b2368eea9d305f06b67aab8a3b94e3be`. Accepted ADR-0137 precedes its
implementation. The reproduced bundled-storage trap previously rendered a
default 10Gi PVC after accepting CNPG-only `postgres.storage.*`. Correct bundled
settings now render the selected 37Gi/class and wrong-mode non-default settings
refuse. Tests consume packaged customer presets for all six ownership recipes.

Node 22+/OpenSSL preparation replaces Docker on the Kubernetes workstation.
Protected tenant/key/password/CA state is committed once before ordinary Helm
and Secret outputs; interrupted outputs resume without rotating credentials.
Offline checks, read-only namespaced API checks, explicit Secret creation and
a temporary external-database authority probe are separate actions. The normal
mutating install order remains bootstrap → three-role preflight → migration →
tenant admission. Readiness tests carry no user credentials; support diagnostics
include only allowlisted status. CPR-8/CPR-39 regenerate registry-backed manual
Codex/Copilot choices and distinguish browser access, setup confirmation and
authenticated Session evidence. The guide covers private first-human admission,
shared Knowledge/review/context and an optional staged loopback sample.

The [source validation record](../../demos/evidence/ops11-first-install-source.json)
records exact gates/tool versions and limits. Controlled API fixtures cover
namespaced RBAC, absent/default storage, missing keys, wrong CA/issuer, Secret
conflicts, registry pull status, interruption/retry, quota/limits and retained
claim rendering. These do not establish live retained-data continuity or login.
Published v0.4.3 remains separate from this candidate.

### OrbStack local acceptance checkpoint (2026-10-04)

The owner selected OrbStack Kubernetes for live acceptance. OrbStack 2.2.3,
Kubernetes `v1.35.6+orb1`, Helm 4.2.3 and a matching kubectl 1.35.6 ran the
packaged bundled/bundled local recipe in a fresh restricted namespace. The
installer used a namespace-only ServiceAccount, a non-default release, two
selected loopback ports and a dedicated Retain StorageClass backed by
`rancher.io/local-path` with WaitForFirstConsumer. The actual PVC bound at
37Gi. A credential-free probe allowed HTTP before a NetworkPolicy and denied
it afterward. No ingress controller was installed or qualified.

Preparation retry preserved private state and credentials. Preflight, the
installation Job, private readiness and `helm test` passed. Real browser PKCE,
workspace/project creation, manual Codex/Copilot discovery, reload recovery,
exploration and logout passed. Ordinary governed Knowledge, native CLI MCP
recall with project/Session binding, exact-revision Context, synthetic repository
provenance and a valid audit chain passed. Retained uninstall/reinstall preserved
the same PVC UID, nine original Secret objects/bytes and original product
addresses; fresh browser authentication and MCP/context checks passed afterward.

`make chart-package` now produces the workspace-version archive without Rust,
Docker or dependency downloads. The package includes its tracked locked
Keycloak dependency, operator tools and customer instructions. Packaging twice
produced identical bytes; incomplete source refuses before producing output.
The [content-free record](../../demos/evidence/ops11-orbstack-first-use.json)
names the source/image identities, checks and limits. These are local source
probes, not qualification of the final 0.4.4 release artifacts.

The owner subsequently approved isolated native Keycloak administration and a
brief stop of this task's identity Pod. Provider maintenance used the owner's
credentials because the ordinary installer could not patch the StatefulSet's
scale subresource. The original Pod returned Ready. A fifth synthetic identity
received no provider bootstrap membership or tenant roles, then accepted an
ordinary audited workspace administrator invitation. The temporary provider
administrator was deleted, its authentication refused, and its Job/Secret and
private administrator files removed.

The [reviewed resume record](../../demos/evidence/ops11-orbstack-reviewed-resume.json)
shows the original Configuration and binding applied by that separate effect
actor after two distinct reviews. Author/reviewer effects and premature binding
effects were denied. Request/change addresses and the receipt stayed intact;
seed resumed without a reset. The existing product runner then passed real
CLI/browser PKCE, safe seed replay preserving console edits, Session capture,
restricted-viewer denial, reviewed Knowledge and Skill publication/binding,
exact-revision/source Context and audit verification. Its native CLI is 0.4.4
source `97c12bc1`; live server images remain the earlier committed source
candidate. It is not final release-artifact qualification.

**Blockers and next action:** all selected checks and `CI Result` passed for
[PR #68](https://github.com/synveda/synveda/pull/68) at `97c12bc1`. This PR run
does not replace full main-push CI: its Docker stage is intentionally unselected.
Three unfamiliar engineer trials still need participants; the owner is deciding
whether 0.4.4 waits for those trials or ships as controlled evaluation with them
explicitly pending. Preserve the existing tenant/receipt and qualify one exact
committed release source; no reset or automatic approval is needed.

Shared HTTPS ingress, private first-human admission/bootstrap removal, external
providers, OpenShift SCC/router and actual vendor/model use remain separate live
checks. Qualify only the named local route from this evidence. The coordinated
0.4.4 source remains unreleased until its exact committed source passes full
main-push CI and the existing nonpublishing Release drill, followed by the
authorised immutable tagged publication.

Three unfamiliar engineer trials also await participants. Give each only the
shipped guide, its own authorised disposable namespace and fresh human identity
on the declared environment/artifact set. Start at route selection, record
prerequisite delays separately, and stop at verified sign-in and the exact
approved Knowledge revision delivered in client context. Count substitutions,
documentation jumps, every error and guide resolution, and maintainer help.
Retained recovery material stays with its trial owner.

| Trial | Sign-in elapsed | Useful client context elapsed | Substitutions / doc jumps | Errors resolved by guide | Maintainer assistance |
| --- | --- | --- | --- | --- | --- |
| Engineer 1 | Not run | Not run | Not measured | Not measured | Not measured |
| Engineer 2 | Not run | Not run | Not measured | Not measured | Not measured |
| Engineer 3 | Not run | Not run | Not measured | Not measured | Not measured |

No five-minute claim, new platform support or production qualification follows
from these source tests. OPS-11 remains open.

## Rollout and rollback

Promote only the exact published chart and immutable images that passed the
selected platform checks. Preserve a jointly verified PostgreSQL, identity and
key recovery set before changing an installation. Helm rollback changes
manifests, not SQL or provider state; restore or roll forward when the old
binary cannot serve the new database. Keep one gateway and worker until OPS-7
qualifies more replicas.

## Dependencies

The blocking external input is an authorised disposable OpenShift project with
its patch, SCC, router, CNI, CSI, TLS and provider inventory. The owner must
also choose whether this milestone promises a cross-release upgrade; v0.4.0 to
v0.4.3 is not a compatible pair under the current migration contract. OPS-6
owns any general N-1 policy. OPS-5 owns production backup custody/PITR, and
OPS-8 owns release publication mechanics; neither requires a new OPS-11 chart.
