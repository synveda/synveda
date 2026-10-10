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

### Issue #72: PostgreSQL extension compatibility plan

[Issue #72](https://github.com/synveda/synveda/issues/72) reports that the exact
pgvector 0.8.6 prerequisite prevents use of Azure Database for PostgreSQL
Flexible Server. The review below was checked against source and upstream
documentation on 2026-10-10. The compatibility documentation and fixed
diagnostics below are implemented in source; extension admission and live
provider qualification are unchanged.
OPS-11 owns the external-provider boundary and CPR-45 owns the shared
deployment contract. Use those existing feature IDs in implementation commits.
The [Azure reuse instructions](../../deploy/helm/synveda/examples/README.md#reuse-existing-services-on-azure)
document the existing external-services recipes. Validation and the separate
patched-version/provider qualification remain tracked below.

**Recommended resolution:** preserve the current extension checks, explain
their purpose and qualification limits, and improve fixed, content-free
diagnostics. Do not replace equality with `>=0.7.0` or admit Azure's advertised
0.8.2 in this correction. Qualifying a patched pgvector release and a managed
provider are separate, explicit next steps.

#### Evidence and constraints

The current contract admits `vector` 0.8.6 and `btree_gin` 1.3 in `public`,
and `plpgsql` 1.0 in `pg_catalog`, under the declared trusted owners.
[The extension fingerprint](../../crates/synveda-store/sql/extension_fingerprint.sql)
includes `extversion`, member identities, routine definitions, operators and
support functions. Changing a version comparison alone would still fail the
catalogue proof. Matching version strings alone does not prove integrity.

The baseline creates cosine HNSW indexes for Knowledge embeddings at 16 and
1024 dimensions. No inspected current query or setting requires a feature
introduced specifically in pgvector 0.8.6; that observation does not establish
compatibility with older versions. The exact version is today's tested
catalogue contract, not a demonstrated universal functional minimum.

[Microsoft's extension table](https://learn.microsoft.com/en-us/azure/postgresql/extensions/concepts-extensions-versions#vector)
lists pgvector 0.8.2 for PostgreSQL 17. The
[upstream changelog](https://github.com/pgvector/pgvector/blob/master/CHANGELOG.md)
records HNSW vacuum corruption fixed in 0.8.3 and further vacuum/insert fixes
in 0.8.4. Admitting 0.8.2 therefore needs evidence addressing those relevant
defects, including any claimed provider backports. A `>=0.7.0` rule also admits
versions affected by the parallel HNSW build vulnerability fixed in 0.8.2
([upstream report](https://github.com/pgvector/pgvector/issues/959)).

There is a separate dependency follow-up: upstream reports an IVFFlat
index-build overflow affecting 0.8.6 and earlier, fixed in 0.8.7 on
2026-10-01, with possible arbitrary code execution by a user able to create
such an index ([upstream report](https://github.com/pgvector/pgvector/issues/1036)).
Product indexes use HNSW, but that does not dismiss the dependency risk or
other database principals. Describe 0.8.6 as currently admitted and tested;
do not call it a security floor or recommend downgrading a patched server.
The fingerprint checks catalogue definitions and native-library references,
not the loaded pgvector binary. Native patch/backport claims need independent
provenance and behavioral evidence.

#### Correct preflight expectations

[The CLI preflight](../../crates/synveda-cli/src/database_preflight.rs) calls
the migrator/runtime prerequisite verifiers, which reach
`verify_capability_prerequisites_connection` in
[the store role verifier](../../crates/synveda-store/src/runtime_role.rs).
Those prove session, role, membership and database authority; they do not
currently execute the exact extension proof. Clean migration calls
`verify_migration_extension_prerequisites_connection` before SQLx DDL in
[the migration boundary](../../crates/synveda-store/src/lib.rs).
Existing-schema migration and full runtime authority reach extension proof
through `verify_application_acl`.

A passed `synveda db preflight` is therefore not proof that extensions will
pass migration or runtime readiness. Do not label its generic authority error
as a detected pgvector mismatch. Adding an extension-specific precheck would
be an additional behavior change requiring its own acceptance; it is not
needed for the first documentation/diagnostic correction.

#### Compatibility correction

1. The canonical `PostgreSQL compatibility` subsection under PostgreSQL
   ownership in [the deployment contract](../DEPLOYMENT_CONTRACT.md)
   distinguishes immutable bundled image/package pins, mandatory runtime
   admission and named live qualification. It states the exact versions/schemas,
   trusted ownership, catalogue proof, role isolation and cluster-identity
   access. Even lexical-only operation requires vector types/indexes in the
   schema. `btree_gin` and `plpgsql` remain separate requirements.
2. The dated provider table identifies Azure's documented PostgreSQL 17 /
   pgvector 0.8.2 combination as refused by the current contract, with no live
   qualification. Existing reference/Kind reports cover their recorded
   artifacts/environments; they do not certify managed providers. Do not
   infer AWS RDS or Cloud SQL compatibility from branding or advertised
   extension versions. The HNSW defects have the primary sources above,
   without claiming that Azure cannot later offer fixes or another
   version.
3. The external-PostgreSQL sections link to that subsection from the
   [Compose guide](../../deploy/compose/README.md),
   [Helm configuration](../../deploy/helm/synveda/CONFIGURATION.md),
   [Helm installation guide](../../deploy/helm/synveda/README.md),
   [deployment overview](../../deploy/README.md) and
   [portability guide](../../deploy/helm/synveda/PORTABILITY.md).
   Provisioning text and the DBA handoff carry the same boundary. Provider-owned
   extensions, extra extensions, administrator/grantor identities and access to
   `pg_control_system()` can independently prevent admission. A passed role/
   target preflight must still be followed by normal migration/readiness.
   Catalogue edits, fingerprint replacement from production output and
   provider/version bypasses are not supported workarounds.
4. `database_preflight.rs` preserves masking and appends one fixed contract
   reference to its existing authority/writable-target failure. The complete
   message body has only the existing closed setting name varying:

   ```text
   {SETTING} authority or writable-target verification failed; see docs/DEPLOYMENT_CONTRACT.md#postgresql-compatibility for PostgreSQL, extension, role and catalogue requirements
   ```

   This is a hint, not an assertion that preflight inspected extensions.
   Keep connection, timeout and invalid-input failures distinct. Never
   interpolate SQLx/server errors, credentials, URLs, hosts, usernames or
   secret-file paths into this CLI output.
5. Fixed store refusals name the prerequisites in `verify_application_acl`,
   `verify_extension_authority_connection` and `verify_extension_fingerprint`
   in `runtime_role.rs`, and `verify_target_schema` in
   [reset.rs](../../crates/synveda-store/src/reset.rs). The earlier ACL check
   can reject version drift before the dedicated extension helper. They name
   exact prerequisite versions/schemas while retaining ownership/event-trigger
   checks; catalogue mismatch has a separate diagnostic. Messages stay bounded
   to one line and use the same contract reference.
6. Every byte-exact CLI-message consumer uses the new complete message: the post-restart
   classifier and two authority-refusal assertions in
   [db-test.sh](../../scripts/db-test.sh), the classifier markers in
   [deployment convergence](../../scripts/check-deploy-convergence.mjs), and
   [its executable tests](../../scripts/check-deploy-convergence.test.mjs).
   Empty stdout, exact stderr/status matching, three attempts and refusal of
   extra output remain mandatory. Tests reject the old message, missing final
   newline, extra/repeated lines and wrong status. The reviewed fixture digest
   is refreshed only after reviewing its three literal-message changes; the
   mutation and teardown refusal tests retain the same gate.
7. The relevant provider/dependency entries in
   [production readiness](../PRODUCTION_READINESS.md), this brief and
   [STATUS](STATUS.md) track validation and the patched-version/provider
   next actions. The readiness verdict and open feature states remain.
   Existing behavior clarification needs no new brief or ADR.

#### Azure reuse instructions for small teams

Existing deployments are a first-class route: operators can reuse their
PostgreSQL, conformant OIDC, namespace/ingress and optional private collector.
Bundled database/identity images are optional ownership choices. The
[short Azure walkthrough](../../deploy/helm/synveda/examples/README.md#reuse-existing-services-on-azure)
uses the already maintained external/external Helm recipe and is linked from
the Helm installation guide, configuration reference and deployment overview.
It changes no chart modes or workload definitions.

The instructions require this sequence:

1. Perform a read-only PostgreSQL/available-extension inspection through the
   DBA's protected `psql` service connection before preparing resources.
   Stop when Flexible Server offers only 0.8.2. Microsoft controls its
   [available packages](https://learn.microsoft.com/en-us/azure/postgresql/extensions/how-to-create-extensions);
   the [allow list](https://learn.microsoft.com/en-us/azure/postgresql/extensions/how-to-allow-extensions)
   cannot install an unoffered version. Preserve existing allowlist selections.
2. Complete the current DBA/OIDC handoff. Use actual provider administrator/
   grantor identities, three ordinary-role credential Secrets and the issuing
   CA with verified TLS. Do not copy the dedicated-server revocations onto
   an unrelated shared server. Provider extension owners, extra extensions,
   cluster-identity access and effective peer isolation remain independent
   admission prerequisites. Entra ID and managed-identity database login have
   no inferred support from the generic OIDC/external database recipes.
3. On an existing AKS cluster, prepare `--route shared --database external
   --identity external` with the verified chart/product overlay and service
   values. Install one gateway serving the console, one worker and the bounded
   installation Job. No PostgreSQL, Keycloak, CNPG operator or application
   data PVC is created; the workstation needs no Docker. AKS still needs
   real cluster/network/ingress admission evidence.
4. Run offline checks, protected Secret creation, API preflight, Helm lint and
   the temporary ordinary-role database probe. Require its printed wait to
   complete before Helm installation. The probe proves roles/target identity;
   migration proves the complete extension catalogue before DDL.
5. Verify actual browser login, capabilities, private readiness and governed
   Knowledge/context use. Diagnose failures and retry with original prepared
   files/credentials. Record the selected service versions, provider-native
   patch provenance and actual results before any Azure qualification claim.

For a team already operating an Azure VM/container engine, the walkthrough
links the existing release-Compose external/external route. It omits bundled
PostgreSQL/Keycloak but retains the reference proxy, private collector and
lifecycle services. An existing private collector can instead be reused with
the Helm `otel.endpoint` setting under its current transport contract. Neither
recipe creates Azure resources, adds an Azure-specific dependency stack or
requires a new Kubernetes cluster. Apply the same database gate to both.

Documentation acceptance requires that this route is discoverable, its flags
match the packaged native utilities, and its first stop precedes installation
on an incompatible provider. Static documentation checks cannot establish Azure
installation, Entra token/claim behavior, managed identity or cloud recovery.

#### Immediate acceptance and validation

The correction explains why 0.8.6 is exact, identifies Azure 0.8.2 as refused/
unqualified, and supplies actionable content-free diagnostics. It must retain
all version, catalogue, ownership, schema, TLS, role and audit boundaries.
It must accurately distinguish CLI preflight from migration/runtime proof.
String-only changes need no SQLx or OpenAPI regeneration. If SQL changes,
use the documented fresh-database generator and review every query hash.

| Case | Required result |
| --- | --- |
| Valid reference configuration | Existing preflight, migration and runtime success remains. |
| Invalid URL, connection failure or timeout | Distinct fixed error, without credentials or private paths. |
| Inherited peer authority or cross-database CONNECT | Fixed authority error plus contract hint; empty stdout. |
| Extra stdout/stderr or unexpected status | Restart classifier refuses. |
| Extension/catalogue drift | Existing fail-closed behavior remains. |
| Old schema or wrong migration checksum | Existing refusal remains. |

The implementation acceptance commands are:

```sh
make check-fast check-deps
cargo fmt --all --check
SQLX_OFFLINE=true cargo clippy -p synveda-store -p synveda-cli --all-targets -- -D warnings
SQLX_OFFLINE=true cargo test -p synveda-cli --bin synveda database_preflight
SQLX_OFFLINE=true cargo test -p synveda-cli --test database_url_file
node --test scripts/check-deploy-convergence.test.mjs
make check-deploy chart-lint
bash scripts/db-test.sh
git diff --check
```

The wrapper tests actual authority refusals and byte-exact masked output on
disposable ordinary-role fixtures. Missing Docker/services are a validation
gap, not passing database acceptance. Plan/operator-documentation edits require `make check-fast`
and `git diff --check`; report those separately from implementation checks.
Epoch acceptance requires the unfiltered wrapper: it supplies lifecycle
credentials only in its separate privileged/drift phase. A filtered
`--test epoch` invocation omits that phase and cannot establish database acceptance.

#### Source map for later version admission

Review these surfaces together before admitting any new extension profile:

| Source | Contract to preserve |
| --- | --- |
| `crates/synveda-store/src/reset.rs` | `REQUIRED_EXTENSIONS`, pinned installation, schema proof and unit test. |
| `crates/synveda-store/src/runtime_role.rs` | ACL version checks, extension owner/version proof and fingerprint. |
| `crates/synveda-store/sql/extension_fingerprint.sql` | Version-containing digest, member/operator/support counts and bounded traversal. |
| `crates/synveda-store/src/lib.rs` | Pre-DDL proof and exact prior/current migration routes. |
| `deploy/compose/postgres/synveda-extension-contract.sql` | Partial/complete prerequisites, owner/member/routine checks and shared fingerprint. |
| `deploy/compose/postgres/synveda-database-bootstrap` | Available-version checks, existing-extension refusal, pinned creation and post-install proof. |
| `deploy/compose/postgres/Dockerfile` | Exact PGDG package and shared fingerprint/psql adapter. |
| `deploy/helm/postgres/Dockerfile` | Pinned CNPG base, inherited package assertion and shared bootstrap. |
| `scripts/db-test.sh` | Exact-role fixtures and pinned external provisioning. |
| `scripts/check-context-hard-cut.mjs` and tests | Identifier-safe creation loop and automatic update/drop refusal. |
| `scripts/check-compose-contract.test.mjs` | Version/member/fingerprint checks and verification order. |
| `scripts/check-deploy-convergence.mjs` | Package/build and diagnostic contracts. |
| `crates/synveda-store/tests/epoch.rs` | Fresh/reset/retained-schema boundaries; its default-version installer cannot substitute for a genuine version matrix. |
| Deployment guides and `deploy/helm/IMAGES.md` | Prerequisites, provenance and qualification limits. |

Keep published `0001`, additive `0002`/`0003`, SQLx ledger checksums, epoch
markers and application fingerprints unchanged. Extension admission is not a
reason to rewrite migration history.

#### Separately gated patched-version and Azure qualification

Investigate pgvector 0.8.7 or a subsequently verified patched release first.
A finite admitted set with reviewed per-version catalogues is preferable to
unbounded `>=`, but patch-number proximity is insufficient evidence. A permanent
`{0.8.6, 0.8.7}` set retains an affected version; transitional acceptance needs
an explicit security/upgrade decision. Do not invent a minimum such as 0.8.4.

Before changing admission, record an accepted amendment to the applicable
deployment/schema ADR using [the template](../adr/adr-0000-template.md) and
update [the decision index](../adr/README.md). Define the finite versions,
security rationale, catalogue profiles, installation pin and upgrade order.
Obtain genuine pinned AMD64/ARM64 artifacts and an appropriate CNPG image;
review upstream SQL/native changes and package provenance. Derive fingerprints
only from clean real installations and independently review every difference
and member bound. Relabeling `pg_extension.extversion` proves no other binary.

Fresh bundled creation stays pinned; external provisioning stays with the DBA.
Startup must never install, update or downgrade extensions. Old binaries refuse
changed versions/fingerprints, so installed-data transition requires writer
quiescence, joint recovery material, a deliberate extension update, candidate
migration and verified restart. Assess rollback to the old extension/binary
separately. Supporting several external profiles does not require floating
bundled package pins.

Candidate acceptance must cover:

- Genuine admitted versions: fresh/repeated migration, exact v0.4.3 baseline
  through the current chain, retained extension transition/restart and failed
  transition/recovery.
- Cosine results and deterministic ordering at both dimensions; HNSW creation,
  inserts/updates/deletes, vacuum/reindex and concurrent write/vacuum cases
  addressing the upstream fixes.
- Ordinary tenant isolation, Cedar filtering, VedaFlow/audit and existing
  Knowledge/context behavior.
- Missing/unexpected versions or suffixes, wrong schemas/owners, extra
  extensions, event triggers and catalogue tampering.
- Consistent admission in migration, full runtime proof, reset and bootstrap,
  while keeping partial bootstrap distinct from complete runtime requirements.

Coordinate plan/performance evidence with CTX-7, recovery order with OPS-6,
and patched image qualification with CPR-45/OPS-12. Azure remains separate:
a selected real PostgreSQL 17 instance must prove allowed extensions,
administrator/grantor ownership, ordinary-role catalogue/cluster identity
access, verified TLS, migration, Cedar/forced RLS, governed writes, retrieval,
restart and recovery before any provider support claim.

**Current blockers and next action:** the Azure reuse instructions are written;
the canonical compatibility documentation and fixed diagnostics are implemented.
Focused extension acceptance below passes. Full workspace database acceptance
previously stopped on the CPR-17 immediate archive/search assertion in
[Knowledge lifecycle acceptance](../../crates/synveda-gateway/tests/knowledge_lifecycle.rs).
A fresh isolated fixture confirms that the archive applies and current detail
is archived; the immediate search failure is intermittent. The collection was
defaulting transaction-time selection to the application clock and hydrating
historical heads, contrary to ADR-0082's current-state default. The source fix
now selects current heads when `as_known_at` is omitted and retains explicit
historical selection when it is supplied. Listing, lexical/vector candidates,
transition-successor checks and hydration use the same selection. Other
current-state consumers use the current-head selector as well. Cedar decisions,
forced RLS, VedaFlow mutation and content-free audit remain in their existing
paths. No database clock change or root-cause clock-skew claim is made.

Regression coverage checks current versus historical heads after a real archive
in ordinary tenant transactions, including both vector dimensions. Public API
acceptance additionally asserts the applied governance outcome, immediate
listing/search exclusion and an explicit historical first revision. The new
four-leg store regression, all six Knowledge lifecycle cases and all four
Knowledge API unit cases pass on the isolated fixture. Fresh SQLx prepare/check
and strict store/ingest/gateway/CLI Clippy pass. Exactly four regenerated query
hashes were reviewed: listing, lexical search and the 16/1024-dimension semantic
queries; their parameter and result descriptions are unchanged, with no other
cache churn or schema change. The unfiltered `bash scripts/db-test.sh` now
passes, including the formerly failing public archive/search case, serial
authority/audit/lifecycle checks, all 28 live epoch cases with their isolated
lifecycle credentials and the final readiness refusal. The wrapper removed its
owned volumes, networks, image and private state; the retained debug fixture
was separately removed through the same immutable ownership-receipt checks.
The Knowledge archive/search blocker is resolved without relaxing its assertion.

Azure's advertised 0.8.2 does not match admission and lacks evidence for the later HNSW fixes;
no selected live provider target, credentials or complete role/identity/recovery
proof has been supplied. Patched-release artifact/catalogue/transition qualification
is unperformed. The next technical follow-up is to qualify a patched candidate
before expanding version admission. Live Azure qualification is outside this
issue #72 correction; it remains necessary before a provider support claim.
No changed extension
admission or managed-provider acceptance is claimed by this correction.

Current validation passes `make check-fast check-deps`, formatting, strict
Clippy for store/ingest/gateway/CLI, fresh SQLx prepare/check, unfiltered database
acceptance, eight CLI preflight tests, three private-URL process tests and 44
deployment-convergence tests. All constituent deployment checks and
`make chart-lint` pass, including 23 native onboarding tests. The convergence
listener test requires local socket access; the initial sandboxed aggregate
refused that bind and its complete 44-test suite passed with that access.
All 28 epoch cases pass with the wrapper's isolated lifecycle credentials,
including real missing-extension and executable-property drift before baseline
DDL and on retained current data. Four runtime-authority cases and the separate
serial routine/trigger-drift case pass; the report-only catalogue case retains
its dedicated harness requirement.

The live explorer recorder also refreshed three stale FLOW-8 corpus files for
the existing `channel.export` action and `git_export_targets` field. Their exact
gateway verification now passes; the assertions are unchanged. All 263 console
tests pass through the existing TypeScript/Node test script using installed
dependencies. The full workspace database gate now passes as described above.
Live Azure and patched-version qualification have not run; no support
claim follows from these local checks.

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
