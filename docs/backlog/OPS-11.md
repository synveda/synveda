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

Two decisions remain. Routes, assigned-ID workloads and NetworkPolicies render,
but only Kind and structural schema checks have run; no real OpenShift SCC,
router, CNI or CSI target has been qualified. Separately, published v0.4.0 and
v0.4.3 both use schema epoch 3 and baseline revision 3, but v0.4.3 changes the
single `0001_context_platform.sql` migration by adding `source_payload_hash`.
The read-only `synveda db migrate --check` compares the exact SQLx migration
checksum, so this pair cannot be declared a supported in-place upgrade. There
is no versioned forward migration or accepted published upgrade pair.

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
