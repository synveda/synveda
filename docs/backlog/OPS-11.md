# OPS-11: Small-team Kubernetes release

## Problem and evidence

The controlled evaluation [v0.4.4 release](https://github.com/synveda/synveda/releases/tag/v0.4.4)
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
OPS-6 / ADR-0121 preserves the exact v0.4.3 baseline in v0.4.4 and adds
`0002`; ADR-0126 adds `0003`. There is still no accepted published upgrade pair.

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

### Issue #71: Helm parent-chart compatibility plan

[Issue #71](https://github.com/synveda/synveda/issues/71) reports that Helm's
injected `global` value prevents a parent chart from consuming Synveda as a
dependency. This plan was prepared with GPT-6 Astra at max reasoning on
2026-10-11. OPS-11 owns the fix, reusing OPS-2's existing chart contract.

#### Verified cause and contract

The pre-fix 0.4.4 source had root `additionalProperties: false` in
`values.schema.json` without a `global` property. A temporary parent chart
failed on Helm 4.2.3 with `additional properties 'global' not allowed` when
globals were omitted, explicitly empty or populated. The direct render with
`--set-json 'global={}'` failed for the same reason. Helm's
[coalescing implementation](https://github.com/helm/helm/blob/v3.19.0/pkg/chartutil/coalesce.go#L97-L171)
copies the [reserved shared namespace](https://helm.sh/docs/chart_template_guide/subcharts_and_globals/#global-chart-values)
into dependencies, including an empty map when the parent supplies none.
Synveda templates do not consume globals; the unchanged locked Keycloak 7.3.2
schema already permits them. The preceding tests rendered Synveda directly
and missed this dependency boundary.

Admit one optional root `global` object with arbitrary contents and retain
root `additionalProperties: false`. Use `type: object` and explicit
`additionalProperties: true` for that property; add neither a required field
nor a default in `values.yaml`. Synveda settings continue to use their
declared fields under the dependency name or alias. Global contents do not
select application images, replicas, credentials or product behavior.

#### Implementation order

1. Extend the existing source-chart packaging test in
   `scripts/check-release-parity.test.mjs`. Reuse its canonical archive,
   temporary directory, cleanup and reproducibility assertions. Create a
   minimal parent chart with the dynamic workspace dependency version and
   place the actual archive in `charts/`; no dependency download is needed.
   Use the packaged external-database recipe under `synveda`. Confirm the
   parent emits gateway, worker and installation resources.
2. Demonstrate the new parent regression against the unchanged schema.
   Cover automatic globals, explicit `{}` and arbitrary nested values
   containing strings, numbers, booleans, arrays and objects. Strictly lint
   and render with schema validation enabled. Also exercise the packaged
   bundled-identity recipe and confirm its Keycloak workload exists.
   Otherwise identical absent/empty/populated globals must produce identical
   application manifests. Valid globals must still reject
   `synveda.redis.enabled` and `synveda.gateway.replicaCount`.
3. Add the optional permissive `global` property to
   `deploy/helm/synveda/values.schema.json`. Preserve all other schema,
   template, image and dependency contracts.
4. Extend `scripts/check-helm-contract.mjs` with direct empty/populated-object
   success and string, number, boolean and array refusal. Malformed cases
   must target Synveda's own `global`, because Helm may discard a malformed
   parent global before passing it down. Null can be removed during Helm
   coalescing; the contract applies to the final value validated by the
   schema and introduces no custom null handling.
5. Update `deploy/helm/synveda/CONFIGURATION.md` to distinguish Synveda fields
   from the optional parent-owned namespace and show ordinary parent values
   under the dependency name or alias. Keep credentials out of Helm values.
   Record the actual validation result here and link it from STATUS.

This schema correction changes no architecture decision, runtime code,
Cedar, forced RLS, VedaFlow, audit, bootstrap ordering, SQL or generated API
contracts. No ADR, SQLx regeneration, new telemetry/audit event, dependency
update or chart-version bump is needed. Keep released 0.4.3/0.4.4 coordinates
immutable and publish through the normal future release process. Root schema
relaxation, disabled validation and an empty-only global schema do not meet
the intended contract.

#### Runnable acceptance and checkpoint

```sh
# Before the schema edit: the new packaged-parent case must fail on global.
node --test --test-name-pattern='one-command source chart packaging' scripts/check-release-parity.test.mjs

# After the schema edit:
node --test --test-name-pattern='one-command source chart packaging' scripts/check-release-parity.test.mjs
node scripts/check-helm-contract.mjs
make check-fast
make check-deploy chart-lint
git diff --check
```

Acceptance requires the canonically packaged dependency to render and strictly
lint with automatic and populated globals, including enabled Keycloak;
unrelated globals preserve manifests, malformed Synveda globals and unknown
settings refuse, existing deployment checks pass and operator documentation
states the boundary. The existing CI job pins Helm 4.2.3 and runs the affected
gates; no new job is needed. A focused checksum-verified Helm 3 rerun is useful
additional evidence for the reported version, but must be recorded separately
from Helm 4 execution. Rust/database suites and live cluster mutations are
unnecessary for this schema-only change. Optional Kubernetes schema validation
retains its existing tool/network prerequisites.

**Current checkpoint:** current source admits the optional shared object while
retaining the strict root schema and unchanged templates/defaults/dependency.
The canonical packaging test reproduces the automatic-global refusal before
the schema correction and passes afterwards on Helm 4.2.3 with Node 24.18.0.
All six parent combinations render and strictly lint: automatic, empty and
populated globals with external or packaged Keycloak identity. Unrelated
globals preserve exact manifests; unsupported root/nested Synveda fields and
four direct non-object global values refuse. Focused contract checks and
`make check-fast` pass. Astra's read-only review finds no code or test gaps.

The `make check-deploy chart-lint` run passes all deployment prerequisites,
including 89 release/packaging tests, 51 Compose contract tests and 173
entrypoint/lifecycle/recovery tests. Its convergence suite initially stops at
the sandbox's refusal to bind an existing temporary `127.0.0.1` fixture.
With loopback networking enabled, the full 45-case convergence suite passes:

```sh
node --test scripts/check-deploy-convergence.test.mjs
node --test scripts/uninstall.test.mjs
node scripts/check-deploy-convergence.mjs
make chart-lint
make check-docs check-backlog
git diff --check
```

The remaining five uninstall tests, deployment convergence check and complete
chart-lint gate pass, including both strict lint profiles, 23 onboarding tests,
six ownership recipes, backup refusals and portability checks. Final
documentation/backlog and diff checks pass. All required deployment components
therefore pass after the affected suite's environment-only rerun; the full
aggregate was not repeated from the beginning.

**Next action:** review this source correction for a future chart release;
no local implementation or validation blocker remains. Helm 3 and live cluster
execution are not claimed. OPS-11 remains open for its existing platform,
recovery and human-acceptance work; this source correction changes no readiness
or published-release claim.

### Issue #72: PostgreSQL extension compatibility plan

[Issue #72](https://github.com/synveda/synveda/issues/72) identifies the reference
pgvector 0.8.6 pin as a blocker for existing Azure PostgreSQL deployments.
The owner subsequently requests genuine pgvector 0.8.2, 0.8.3, 0.8.4 and
0.8.5 source compatibility, with live Azure qualification excluded from this
implementation.
[Accepted ADR-0140](../adr/adr-0140-finite-pgvector-catalogue-profiles.md) records
that finite admission decision. OPS-11 owns the external-provider boundary;
CPR-45 owns the shared deployment contract.

#### Current admission and evidence

Current source admits exactly `vector` 0.8.2, 0.8.3, 0.8.4, 0.8.5 or 0.8.6
in `public`, `btree_gin` 1.3 in `public` and `plpgsql` 1.0 in `pg_catalog`,
under declared trusted owners.
The [shared fingerprint](../../crates/synveda-store/sql/extension_fingerprint.sql)
joins extension name and exact version to separate reviewed digests. All five
vector profiles contain 237 members, 36 access operators and 54 support
functions; all original traversal bounds remain. No range, suffix, provider
exception, authority bypass or relaxed executable check is admitted.

The upstream `v0.8.2...v0.8.6` SQL comparison adds only empty update scripts.
An independent inventory from genuine upstream 0.8.2 differs only in its
version-bearing metadata; its native catalogue digest is recorded in ADR-0140.
Positive acceptance uses the actual digest-pinned native 0.8.2 library, bitcode
and installation scripts, never a relabelled `pg_extension.extversion`.
The [image inventory](../../deploy/helm/IMAGES.md) records its PostgreSQL licence.
The fixture overlays those inputs on the reference PostgreSQL 17.11 base and
is never a release image or package-provenance claim.

Bundled packages and fresh administrative creation stay pinned to 0.8.6.
An external DBA may preinstall an admitted version; ordinary migration and
retained bootstrap verify it without changing its version. Published `0001`, additive `0002`/`0003`,
SQLx ledger checksums, schema epoch and application catalogue fingerprints
remain immutable. External destructive reset and automatic extension update
or downgrade are outside the supported lifecycle.

The baseline uses cosine HNSW indexes at 16 and 1024 dimensions. No inspected
current query or setting requires a feature introduced in 0.8.6. This is
source compatibility for controlled evaluation, not production or provider
certification: [upstream](https://github.com/pgvector/pgvector/blob/master/CHANGELOG.md)
fixes HNSW vacuum corruption in 0.8.3 and further vacuum/insert errors in 0.8.4.
All admitted versions also have the IVFFlat index-build vulnerability fixed
in 0.8.7 ([upstream report](https://github.com/pgvector/pgvector/issues/1036)).
Runtime roles have no DDL and product indexes use HNSW; those boundaries do not
prove native defects absent. Fingerprints check catalogue definitions and
library references, not loaded binaries or provider backports.

#### Preflight and runtime expectations

The [CLI preflight](../../crates/synveda-cli/src/database_preflight.rs) proves
roles, memberships, target identity and peer isolation. Clean migration proves
extensions before baseline DDL; retained migration and full runtime authority
prove the extension catalogue through application ACL verification. A passed
`db preflight` alone therefore does not establish extension compatibility.
The fixed content-free authority refusals link to
[the complete contract](../DEPLOYMENT_CONTRACT.md#postgresql-compatibility)
without exposing credentials, private paths or existence-sensitive details.
The restart wrapper retains its exact status/stdout/stderr classifier.

Catalogue drift, extra extensions, wrong schema/owner, event triggers, unsafe
roles, peer CONNECT, old epochs and wrong migration checksums still refuse.
Cedar, forced RLS, VedaFlow and content-free audit remain mandatory on the
ordinary tenant and authenticated API paths.

#### Small-team Azure reuse

[The Azure walkthrough](../../deploy/helm/synveda/examples/README.md#reuse-existing-services-on-azure)
uses the maintained external/external Helm recipe or VM/Compose alternative.
An existing AKS deployment needs one gateway/console, one worker and the
bounded installation Job. PostgreSQL, the issuer, storage, network, credentials
and recovery may remain with the team's existing services; no new provider
stack or operator is required.

[Microsoft's current extension table](https://learn.microsoft.com/en-us/azure/postgresql/extensions/concepts-extensions-versions#vector)
lists a version for PostgreSQL 17 that fits the 0.8.2 source profile. That
removes the version blocker only. The DBA must still prove allowed schemas/extensions, trusted
administrator/grantor identities, ordinary-role catalogue and cluster-identity
access, effective peer isolation and verified TLS before product installation.
Do not run dedicated-server revocations against unrelated shared deployments.
AKS, Entra, managed-identity database login and Azure recovery have no live
qualification here. Published v0.4.4 still requires 0.8.6; a source candidate
containing ADR-0140 is needed to exercise the new profile.

#### Runnable acceptance and current checkpoint

```sh
make check-fast check-deps check-ci
cargo fmt --all --check
SQLX_OFFLINE=true cargo clippy -p synveda-store --all-targets -- -D warnings
SYNVEDA_DB_TEST_TASK=sqlx-prepare bash scripts/db-test.sh
for version in 0.8.2 0.8.3 0.8.4 0.8.5; do
  SYNVEDA_DB_TEST_TASK="pgvector-$version" bash scripts/db-test.sh
done
make check-deploy chart-lint
bash scripts/db-test.sh
git diff --check
```

Each closed compatibility task provisions its own genuine extension installation, runs
normal bootstrap twice, preflight and repeated migration, then ordinary-role
Knowledge and authenticated API acceptance. After those tests it removes only
its owned product database so independent scratch-schema tests do not create
forbidden cross-database role dependencies. Those cases cover fresh/repeated
migration, missing extensions, executable drift before DDL and on current
schema, and native indexed cosine results at both dimensions after
insert/update/delete, vacuum and reindex. Basic maintenance checks do not
address the known concurrent-vacuum defects. The Rust CI job runs all four
closed tasks; the unfiltered default wrapper separately exercises 0.8.6 and
all authority/audit/lifecycle cases. Missing services are validation gaps.

Fresh SQLx prepare/check passes. Exactly four generated query replacements
were reviewed for extension fingerprint, runtime version/ACL proof and reset
schema proof; parameter/result descriptions are unchanged. Each genuine 0.8.2,
0.8.3, 0.8.4 and 0.8.5 task passes seven Knowledge store tests, all six API
lifecycle cases and all four schema/native cases, including an actual indexed
vector update before vacuum/reindex at both dimensions. Successful profile
and SQLx wrappers remove their owned resources; the failed timestamp reproduction
was separately removed after immutable receipt, ID and label verification.
The full default 0.8.6 wrapper passes its workspace and serial authority/audit/
lifecycle phases, all 29 live epoch cases and the final readiness refusal.
Its owned containers, volumes and reserved networks are cleaned successfully.

Formatting, strict store/CLI/gateway Clippy, fast/dependency gates and the
aggregate `make check-deploy chart-lint check-ci` pass, including 23 onboarding,
45 convergence and 53 CI tests. Pinned actionlint 1.7.7 also passes. The proxy
inventory includes all 22 guarded image stages and fixed fixture overrides;
canonical context, genuine targets and exact empty proxy arguments remain
mandatory. The reviewed native
case alone may hold an owner URL; mutation tests preserve the ordinary-role
boundary for demos/evaluations and reject dispatch aliases. These are local
macOS ARM64/OrbStack source checks, not new release/provider qualification.
The Rust CI job supplies the repeatable genuine fixtures on Linux AMD64.
The earlier CPR-17 archive/search and FLOW-8 explorer-fixture corrections are
included in [PR #75](https://github.com/synveda/synveda/pull/75); their existing
assertions remain intact.

#### CI timestamp regression

[CI 38070493612](https://github.com/synveda/synveda/actions/runs/38070493612)
failed only its Rust job: a future-conflict fixture reused a nanosecond timestamp
after PostgreSQL persisted microseconds. A forced-nanosecond reproduction fails
before the fixture correction. All six lifecycle cases then pass against the
persisted revision timestamp with explicit microsecond assertions; production
transition equality, Cedar, VedaFlow, forced RLS and audit checks remain intact.
Hosted Linux validation of the profiles and fixture correction is tracked in
[PR #75](https://github.com/synveda/synveda/pull/75), separately from local evidence.

#### Separately gated patched-version and Azure qualification

Qualify 0.8.7 or a subsequently verified patched native release with genuine
pinned AMD64/ARM64 and CNPG inputs, reviewed SQL/native differences and package
provenance. Derive each digest from clean installations and preserve every
catalogue bound. Supporting several external profiles does not require
floating bundled dependencies or automatic native transitions.

Retained-data extension transition needs writer quiescence, joint recovery
material, a deliberate DBA update, candidate migration and verified restart.
Old binaries refuse changed fingerprints; rollback and failed-transition
recovery require independent proof. Include concurrent write/vacuum acceptance
addressing the upstream defects, tenant isolation, governed writes, audit and
retrieval before changing production claims.

Live Azure qualification remains outside this implementation. A selected
real PostgreSQL 17 target must independently prove authority, TLS, migration,
Cedar/forced RLS, governed writes, retrieval, restart and recovery before an
Azure support claim. Coordinate that work with OPS-6, OPS-12, CPR-45 and CTX-7.
**Current next action:** review PR #75's completed finite profiles and hosted
check results. No local source-validation blocker remains. Patched native
artifact/catalogue and retained-data transition qualification remain the
technical follow-up. Live provider proof stays excluded from this implementation.

### Issue #74: KMS key backup warning before installation

[Issue #74](https://github.com/synveda/synveda/issues/74) asks for a prominent
key-loss and backup warning during initial setup. This plan was prepared with
GPT-6 Astra at max reasoning on 2026-10-11. OPS-11 owns the documentation fix;
the existing key and recovery contracts require no new ADR.

#### Verified behavior and wording

`SYNVEDA_KMS_KEY` is the local deployment key-encryption key (KEK), wrapping
deployment and tenant data keys. Losing every recoverable copy makes dependent
encrypted state permanently unrecoverable from ciphertext or database backups
alone. Searchable Knowledge is not application-encrypted; the warning must not
claim that every database row is lost. Replacing a Kubernetes Secret neither
rewraps existing data keys nor re-encrypts payloads. Tenant-DEK rotation under
[ADR-0094](../adr/adr-0094-context-platform-key-and-secret-plane.md) is a separate
operation and does not rotate the deployment KEK.

Automatic preparation generates the key before contacting the cluster and
preserves it in `state.json` and `secrets.json`. The latter carries
`SYNVEDA_KMS_KEY_REF` as `<release>:primary-v1` in the `<release>-kms` Secret,
selected by `kms.existingSecret`. Manual preparation uses a `kms-key` input
file; that filename is not a universal Kubernetes Secret name. Recovery needs
the original key and reference as well as the matching database/identity set.

#### Implementation order

1. Add a prominent warning in the [Helm setup guide](../../deploy/helm/synveda/README.md#4-prepare-the-selected-configuration)
   before preparation commands. Immediately after generation, require an
   encrypted offline backup of the complete private preparation directory,
   including `state.json` and `secrets.json`, before installation or data writes.
2. Move and strengthen the existing [configuration warning](../../deploy/helm/synveda/CONFIGURATION.md#identity-and-secret-formats)
   so it precedes Secret creation. Identify the 64-hex-character KEK and stable
   reference; remove the implied KEK rotation ceremony. Link the existing
   [complete recovery set](../../deploy/helm/synveda/OPERATIONS.md#complete-recovery-set)
   and [backup procedure](../../deploy/helm/synveda/OPERATIONS.md#bounded-write-quiescing-backup).
3. Put the same warning before [manual key generation](../../deploy/helm/synveda/MANUAL.md#generate-private-material-once).
   Preserve `kms-key` immediately and the complete private preparation set,
   including the later `secrets.json` carrying the reference, before applying
   Secrets or installing. Keep all generation/retry commands unchanged.
4. Add a short backup requirement and setup-warning link after the
   [existing-services preparation command](../../deploy/helm/synveda/examples/README.md#existing-services).
   It covers the external, bundled and CNPG variants without duplicating the
   full warning.

Keep the fix to these four Helm guides and the existing OPS-11 inventory. Do
not add backup commands, print keys, change runtime/manifest/generated files,
or imply a KEK migration workflow. The root README's Docker first-start path
generates keys during `up`; a backup-before-`up` instruction there would be
incorrect and is outside this Helm correction.

#### Validation and checkpoint

Review warning placement and key/reference names, then run:

```sh
make check-fast
git diff --check
node --test --test-name-pattern='one-command source chart packaging' scripts/check-release-parity.test.mjs
```

The focused archive test checks reproducible packaging and rewritten guide
identities/links. No new prose-matching tests, Rust checks, database or live
cluster operations are required.

The four guide edits are implemented. Review confirms the warnings precede
key generation or Secret creation/installation and name the actual private
recovery files. On 2026-10-11, `make check-fast`, `git diff --check` and the
focused source-chart packaging test pass with Node 24.18.0 and Helm 4.2.3.
The archive test packages twice and checks reproducibility and guide links.
Next review the documentation correction for a subsequent chart release.
Published v0.4.4 remains unchanged; OPS-11 stays open for its existing platform,
human-installation and recovery qualification. These documentation checks do
not change production-readiness claims.

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

**Blockers and next action:** the controlled evaluation v0.4.4 release is
published from `95139842af512faf2dad711fdff14e471e93a3ba` after exact-source
full main-push CI, the nonpublishing drill and tagged qualification. Its
[release record](../../demos/evidence/ops12-044-controlled-release.json) binds those
results to the published artifacts. The owner authorised release with the three
unfamiliar engineer trials explicitly pending. Recruit those participants and
use only the shipped v0.4.4 guide and matching archive/overlay, preserving the
existing tenant, receipt and private recovery material. No reset or automatic
approval is needed.

Shared HTTPS ingress, private first-human admission/bootstrap removal, external
provider/platform acceptance, OpenShift SCC/router and actual vendor/model use
remain separate live checks. The two native tagged jobs qualify the four named
Kind ownership modes; they do not establish shared production admission.

The packaged guides now name their archive version and reviewed source rather
than retaining the checkout's older publication notes and download pins.
Outside links point to that source; tools, presets and locked dependency bytes
are unchanged. ADR-0137 records this private staging adjustment. It adds no
publisher proof, timing or platform claim.

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
