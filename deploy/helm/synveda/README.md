# Install Synveda on an existing Kubernetes cluster

Synveda installs a console/API gateway, a separate worker and a bounded
installation Job. Choose who owns PostgreSQL and identity; the chart installs
no cluster operator, ingress controller or StorageClass. A tenant is this
installation's identity boundary; a workspace groups people and projects.

<!-- installation-version: 0.4.4; publication: unreleased -->
**The current published release is v0.4.3.** The preparation, customer presets,
preflight and console changes in this checkout are **source candidates for the
next release**. They are included by chart packaging and tested locally; they
are absent from the immutable v0.4.3 archive. For that release, follow its
[versioned installation guide](https://github.com/synveda/synveda/blob/v0.4.3/deploy/helm/synveda/README.md).
Do not mix an older published chart with commands introduced in this candidate.

The procedure below is the candidate's shipped operator guide. Set `CHART` to
the extracted, reviewed candidate chart and use its matching digest overlay.
The download ceremony also applies to a named published release, using that
release's versioned instructions. Local source tests are not publisher proof.
<!-- chart-package-source-status:end -->

The service is available for controlled self-hosted evaluation. One gateway
and one combined worker use planned downtime for upgrades. No HA, supported
cross-release upgrade window or production-readiness claim is made. See the
[readiness register](../../../docs/PRODUCTION_READINESS.md) and
[qualification record](../../../docs/backlog/OPS-11.md).

## 1. Choose a route and dependency owners

| Route | Recommended audience | What you provide |
| --- | --- | --- |
| **Local evaluation on Kubernetes** | One person trying the product | Authorised namespace, persistent storage, explicit evaluation identities and two loopback ports. PostgreSQL and Keycloak are bundled. |
| **Shared installation on an existing cluster** | A team using real identities | Application HTTPS/DNS, trusted ingress, an initial human administrator and recovery ownership. Bundle either dependency or use existing PostgreSQL and OIDC independently. |
| **Advanced existing-CNPG installation** | Operators already running CloudNativePG | Existing compatible controller/API, storage and its database lifecycle. Identity can be bundled or external. |

These are infrastructure recipes. Product behavior is governed Configuration,
selected after sign-in. Start locally for individual evaluation. Start with
shared HTTPS and bundled dependencies for a team without existing services;
use the [existing-service worksheet](CONFIGURATION.md) when those services
already have owners. Existing-CNPG users follow the same procedure with
`--route cnpg`; the chart never installs its operator.

Customer presets in `examples/` are ordinary Helm values, packaged with the
chart and used by tests. [The preset index](examples/README.md) lists the
advanced combinations. No customer command depends on `ci/` fixtures.

## 2. Check prerequisites with your cluster administrator

All commands below run in a **workstation terminal**, as an ordinary account.
You need Helm (qualified: **4.2.3**), kubectl matching the cluster (qualified:
**1.36.1**), trusted GitHub CLI with attestation verification, curl and tar.
Candidate preparation additionally needs **Node 22+ and OpenSSL** on Linux or
macOS. Docker, Rust and npm installation are unnecessary for preparation.
Windows preparation is unqualified; native Windows client packages are separate.

Choose one context and a namespace the administrator permits you to install
into. Namespace creation is an administrator action if your credentials are
namespaced. Do not use an arbitrary current context for a disposable drill.

```sh
export CONTEXT=your-authorised-context     # replace
export NAMESPACE=synveda-evaluation       # change if administrator supplied another
export RELEASE=synveda                   # change freely; keep the same selectors on retry
kubectl config use-context "$CONTEXT"
kubectl --context "$CONTEXT" -n "$NAMESPACE" auth can-i create deployments
kubectl --context "$CONTEXT" -n "$NAMESPACE" get resourcequota,limitrange
```

Success means the namespace exists and the selected identity can inspect it
and install namespaced workloads/Secrets. Ask the administrator for Helm's
get/list/watch/create/update/patch/delete access to Deployments, Jobs,
Services, ConfigMaps, Secrets, ServiceAccounts and selected StatefulSets/PVCs
and Ingresses. Pod inspection/logs and explicit private verification need
Pods get/list/watch, pods/log get, pods/exec create and pods/portforward create
for local access. CNPG adds its selected
namespaced API; NetworkPolicy/Route access is needed only when enabled.
Cluster-admin is unnecessary. Helm stores its release metadata in Secrets.

For bundled storage select a named StorageClass supporting PostgreSQL fsync,
ReadWriteOnce and the cluster's UID/fsGroup rules. The administrator supplies
its provisioner, binding mode, reclaim policy and expansion support. If you
cannot list StorageClasses, obtain this exact input rather than assuming a
default. A Pending claim with **WaitForFirstConsumer** waits for the database
Pod to be scheduled; it is not alone evidence of a failed provisioner.

Default reservations below exclude cluster, ingress and external-provider
resources. They include a single installation Job; its sequential bootstrap,
preflight and migration containers are not simultaneous reservations.

| Database / identity recipe | Running CPU / memory | During installation CPU / memory | PVC |
| --- | --- | --- | --- |
| Bundled / bundled (local or shared) | 1500m / 2560Mi | 1600m / 2688Mi, 5 Pods | 20Gi selected class |
| Bundled / external | 1000m / 1280Mi | 1100m / 1408Mi, 4 Pods | 20Gi selected class |
| External / bundled | 1250m / 2048Mi | 1350m / 2176Mi, 4 Pods | Database owner's storage |
| External / external | 750m / 768Mi | 850m / 896Mi, 3 Pods | None for the lexical app |
| Existing CNPG / bundled, one DB instance | 2250m / 4096Mi | 2350m / 4224Mi, 5 Pods | 20Gi selected class |
| Existing CNPG / external, one DB instance | 1750m / 2816Mi | 1850m / 2944Mi, 4 Pods | 20Gi selected class |

Memory limits are 2Gi each for gateway, bundled DB and Keycloak, 1Gi for
worker and install Job. Reserve quota for those limits, replacement Pods,
retained claims and Helm's optional verification Pod (100m/128Mi request,
1Gi memory limit). These are requests, not observed use or measured minimums.
[Short-run observations](../../../demos/evidence/ops11-starter.json) exclude
cluster/driver overhead. No setup-time or minimum-cluster claim is published.

For a macOS **local evaluation**, OrbStack Kubernetes can supply the selected
cluster. Start it with `orb start k8s`, set `CONTEXT=orbstack`, and inspect its
actual StorageClasses with `kubectl --context "$CONTEXT" get storageclasses`.
Use a newly authorised namespace and a kubectl client within one minor version
of its API server; OrbStack's bundled client can lag its server. Confirm both
versions with `kubectl --context "$CONTEXT" version -o json` before preparing.
Keep the two loopback forwards below even when OrbStack offers direct Service
access, so the application origin, issuer and callback stay consistent.
This local route needs no ingress controller. Shared HTTPS ingress, other
distributions and human timing trials require their own acceptance.

## 3. Download and verify a named release

The release chart is already packaged with its locked Keycloak dependency,
customer presets and preparation tools. You do not compile Helm, build Rust
or run `helm dependency build` to install a downloaded release. For a reviewed
source checkout, [package the chart with one command](BUILD.md); its images
remain a separate, matching source candidate.

Run this **entire block** from a workstation directory for downloads. The
subshell fails closed: extraction occurs only after publisher and all selected
checksums pass. The directory must be new.
<!-- chart-package-source-identity:start -->
The source commit shown is the
published v0.4.3 source; take any future release's commit from its reviewed
release record. Never copy an expected commit from an unverified downloaded
inventory. GitHub CLI must be installed independently of these assets.
<!-- chart-package-source-identity:end -->

```sh
(
  set -eu
  RELEASE_VERSION=0.4.3
  SOURCE_SHA=2acc66f02625727b2ccdfe223358468bf10eef85
  release_url="https://github.com/synveda/synveda/releases/download/v$RELEASE_VERSION"
  mkdir "synveda-chart-$RELEASE_VERSION"
  cd "synveda-chart-$RELEASE_VERSION"
  for file in "synveda-$RELEASE_VERSION.tgz" "synveda-images-$RELEASE_VERSION.yaml" \
    "synveda-cnpg-image-$RELEASE_VERSION.yaml" SHA256SUMS SHA256SUMS.sigstore.json; do
    curl --fail --location --proto '=https' --proto-redir '=https' \
      --connect-timeout 15 --max-time 120 --output "$file" "$release_url/$file"
  done
  gh attestation verify SHA256SUMS --bundle SHA256SUMS.sigstore.json \
    --repo synveda/synveda \
    --signer-workflow synveda/synveda/.github/workflows/release.yml \
    --source-ref "refs/tags/v$RELEASE_VERSION" --source-digest "$SOURCE_SHA" \
    --cert-oidc-issuer https://token.actions.githubusercontent.com \
    --predicate-type https://slsa.dev/provenance/v1 --deny-self-hosted-runners
  awk -v chart="synveda-$RELEASE_VERSION.tgz" \
    -v images="synveda-images-$RELEASE_VERSION.yaml" \
    -v cnpg="synveda-cnpg-image-$RELEASE_VERSION.yaml" \
    '$2 == chart || $2 == images || $2 == cnpg { seen[$2]++; print }
     END { if (seen[chart] != 1 || seen[images] != 1 || seen[cnpg] != 1) exit 1 }' \
    SHA256SUMS > chart.sha256
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 --check chart.sha256
  else
    sha256sum --check chart.sha256
  fi
  mkdir chart
  tar -xzf "synveda-$RELEASE_VERSION.tgz" -C chart
  cp "synveda-images-$RELEASE_VERSION.yaml" release-images.yaml
)
```

Success ends with matching hashes and an extracted chart. On any failure,
stop and correct download/verifier access; do not execute partially obtained
code. Preserve the inventory, attestation, overlay and expected source with
operator configuration. The archive and public OCI chart are byte-identical.
[Publisher policy](../../../docs/adr/adr-0132-verify-publisher-before-installing-release-code.md)
and [release evidence](../../../docs/CI.md) explain verification limits.

Set paths once, using the actual extracted **candidate** with the tools below:

```sh
export CHART=/absolute/path/to/reviewed-candidate/chart/synveda
export IMAGES=/absolute/path/to/its/matching/release-images.yaml
export PREPARED="$HOME/.synveda-kubernetes-$RELEASE"
export ARCHITECTURE=arm64               # administrator-confirmed amd64 or arm64
export STORAGE_CLASS=your-persistent-class  # replace; do not guess a class name
```

For advanced CNPG, merge the matching verified CNPG reference into the image
overlay before preparation. Both original files must pass the same inventory
verification. Do not concatenate duplicate YAML `postgres` keys. On the
workstation, set `CNPG_IMAGES` to that verified file and run:

```sh
export CNPG_IMAGES=/absolute/path/to/synveda-cnpg-image-VERSION.yaml
export MERGED_IMAGES="$PWD/release-images-cnpg.yaml"
node --input-type=module <<'NODE'
import { readFileSync, writeFileSync } from 'node:fs';
const cnpg=readFileSync(process.env.CNPG_IMAGES,'utf8');
const reference=/^  image: ([^\n]+)$/m.exec(cnpg)?.[1];
if (!reference || !/@sha256:[a-f0-9]{64}["']?$/.test(reference)) throw Error('verified CNPG digest reference missing');
const base=readFileSync(process.env.IMAGES,'utf8');
const merged=base.includes('\npostgres:\n')?base.replace('\npostgres:\n','\npostgres:\n  image: '+reference+'\n'):base+'\npostgres:\n  image: '+reference+'\n';
writeFileSync(process.env.MERGED_IMAGES,merged,{flag:'wx',mode:0o600});
NODE
export IMAGES="$MERGED_IMAGES"
```

Preserve the two verified originals and the derived overlay. If the derived
file already exists, reuse the existing inspected file instead of replacing
it. A matching version string alone does not establish publication.

## 4. Prepare the selected configuration

Preparation writes files only. It prints context, namespace, release, exposure
and dependency owners; it contacts no cluster. Inspect `values.json` afterward.
Keep `state.json` and `secrets.json` private with the original database/identity
recovery set. Repeating the same command preserves tenant UUIDv7, passwords,
CA/private keys and issuer binding; interrupted derived outputs resume from
saved state. Conflicting inputs/files stop with a recovery action.

**Local evaluation** (recommended for one person): choose two unused ports and
explicitly opt into four unique synthetic evaluation identities.

```sh
export APP_PORT=8120 IDENTITY_PORT=8081
node "$CHART/examples/prepare.mjs" --route local \
  --context "$CONTEXT" --namespace "$NAMESPACE" --release "$RELEASE" \
  --storage-class "$STORAGE_CLASS" --storage-size 20Gi \
  --app-port "$APP_PORT" --identity-port "$IDENTITY_PORT" \
  --images "$IMAGES" --output "$PREPARED"
```

**Shared installation**, bundled dependencies (recommended for a team without
existing services): provision both public DNS names and their TLS Secrets
through the certificate owner first. Replace the five environment inputs below.
The proxy CIDRs must be the actual ingress source addresses, never a public /0.

```sh
export APP_URL=https://memory.your-domain.example
export IDENTITY_URL=https://identity.your-domain.example
export INGRESS_CLASS=your-ingress-class
export PROXY_CIDRS=your-ingress-source-cidrs
export ADMIN_USER=your-private-bootstrap-admin
node "$CHART/examples/prepare.mjs" --route shared \
  --context "$CONTEXT" --namespace "$NAMESPACE" --release "$RELEASE" \
  --storage-class "$STORAGE_CLASS" --storage-size 20Gi \
  --app-url "$APP_URL" --identity-url "$IDENTITY_URL" \
  --ingress-class "$INGRESS_CLASS" --proxy-cidrs "$PROXY_CIDRS" \
  --app-tls-secret application-tls --identity-tls-secret identity-tls \
  --admin-user "$ADMIN_USER" --images "$IMAGES" --output "$PREPARED"
```

Success reports protected files and no applied resource. For existing services,
use `--database external` and/or `--identity external` with the
[worksheet and exact inputs](examples/README.md#existing-services).
For advanced CNPG, use `--route cnpg`, otherwise the shared arguments above;
its operator creates the owner credential and CA after installation starts.

`postgres.bundled.size/storageClass` configures bundled PVCs;
`postgres.storage.size/storageClass` configures **CNPG only**. The wrong mode
is refused. An existing bundled claim uses `postgres.bundled.existingClaim`
and matching original data/Secrets, without size/class overrides.
No Helm template generates credentials or reads live Secrets to render.
The [manual equivalent](MANUAL.md) uses ordinary values and protected files.

## 5. Validate and install

Run on the **same workstation** with the saved selectors. Offline checks need
no cluster; preflight reads the selected Kubernetes API within your permissions.
The `secrets` action explicitly creates missing prepared Secrets and refuses
conflicting existing credentials. It never replaces existing Secret values.

```sh
node "$CHART/examples/operator.mjs" offline --prepared "$PREPARED" --architecture "$ARCHITECTURE"
node "$CHART/examples/operator.mjs" secrets --prepared "$PREPARED"
node "$CHART/examples/operator.mjs" preflight --prepared "$PREPARED" --architecture "$ARCHITECTURE"
helm lint "$CHART" --strict -f "$PREPARED/values.json" -f "$PREPARED/release-images.yaml"
```

Run each step only after the previous exits successfully. A FAIL gives the
specific missing permission/input and retry command. Under namespaced RBAC,
preflight may request administrator confirmation of a named StorageClass;
`--confirmed-storage-class "$STORAGE_CLASS"` records that input, not proof of
provisioner health. Preflight checks CPU/memory and storage quota, installation
headroom, and Container/Pod/PVC limits for these recipes. Extra containers or
provider-managed sidecars need administrator reconciliation with the rendered
pods. Registry/pod-connectivity requirements remain explicit. Workstation network access
cannot prove a Pod can reach a provider.

For external PostgreSQL, run `operator.mjs database-probe --prepared
"$PREPARED" --architecture "$ARCHITECTURE"` before the Helm command. This
explicitly creates a bounded temporary Job/ConfigMap, using the existing
three-role authority verifier and no administrator credentials or DDL. Require
its printed `kubectl wait ... Complete` command to succeed. Failure goes to
the DBA handoff before a long Helm wait.

Now install on the workstation after every prerequisite has passed:

```sh
helm upgrade --install "$RELEASE" "$CHART" --kube-context "$CONTEXT" -n "$NAMESPACE" \
  -f "$PREPARED/values.json" -f "$PREPARED/release-images.yaml" \
  --wait --wait-for-jobs --timeout 15m
```

Installation succeeds when Helm reports deployed and the revision-named
installation Job completes. It runs administrator bootstrap (bundled/CNPG),
three-role database authority preflight, migration and tenant admission in
order. The Job is mutating, bounded to 15 minutes; it is not a read-only probe.
A failed init/container identifies the failing stage. Start with:

```sh
node "$CHART/examples/operator.mjs" diagnose --prepared "$PREPARED"
```

Use [the stage recovery table](OPERATIONS.md#installation-stage-recovery).
Correct the prerequisite and rerun the exact Helm command with the same files.
Never regenerate credentials or reset a database as install recovery.

## 6. Open the console and sign in

Local evaluation: run each forward in a **separate workstation terminal**.
Keep both running through browser and CLI authentication.

```sh
kubectl --context "$CONTEXT" -n "$NAMESPACE" port-forward --address 127.0.0.1 "service/$RELEASE" "$APP_PORT:8120"
kubectl --context "$CONTEXT" -n "$NAMESPACE" port-forward --address 127.0.0.1 "service/$RELEASE-keycloak-http" "$IDENTITY_PORT:80"
```

Open `http://localhost:8120/console/`, using your chosen `APP_PORT`.
Retrieve only the selected account password in a **private terminal**:

```sh
node -e 'const fs=require("fs"); const s=JSON.parse(fs.readFileSync(process.env.PREPARED+"/secrets.json")); process.stdout.write(s.items.find(x=>x.metadata.name===process.env.RELEASE+"-evaluation-accounts").stringData.admin+"\n")'
```

Sign in as `synveda-demo-admin` (Avery Author). Shared installations open
`$APP_URL/console/` and follow the exact
[private first-human administration procedure](FIRST_LOGIN.md) before login.
External OIDC uses [the configuration worksheet](CONFIGURATION.md#external-oidc-worksheet).
Emails grant no authority; only the selected verified identity may hold the
one-time bootstrap group. On failure, follow the TLS/OIDC rows in the runbook.

## 7. Complete the first useful workflow

In Getting started, create or select a workspace/project, optionally attach a
repository, then choose a client. You can explore the console before connecting
one. The instructions use the actual gateway and project and offer manual
routes for Codex CLI and Copilot CLI. Install the CLI from the verified
client-only release instructions if it is missing. Review each client's tested
version/platform limits and normal trust prompts. Observation starts off;
recording transcript/tool evidence requires explicit consent.

Server available, signed in, project accessible, setup confirmed and client
operation observed are separate states. Browser access does not verify a
client. Run an actual context/recall operation, then inspect its Session and
Context evidence; follow source links to immutable Knowledge revisions.

[First use and optional fictional example](FIRST_USE.md) follows source
inspection → learning review → explicit apply → Knowledge retrieval. It works
against the Kubernetes gateway without Docker. Sample proposals retain their
normal reviewers; no command automatically approves them. The initial
`deterministic` providers are lexical/synthetic, not semantic or model-backed.

## 8. Verify, retry, stop and uninstall safely

```sh
node "$CHART/examples/operator.mjs" verify --prepared "$PREPARED" --architecture "$ARCHITECTURE"
helm test "$RELEASE" --kube-context "$CONTEXT" -n "$NAMESPACE" --logs --timeout 90s
```

Verification inspects the installation stage and private readiness. `helm test`
creates a temporary bounded Pod using the same product image, without customer
credentials. It needs scheduling/DNS/network/quota headroom and cannot prove
login, client delivery or Capture. Complete the authenticated workflow above.

Retry a failed installation or reapply the same release with the same Helm
command, selectors, values, overlay and original Secrets. Stop local forwards
with Ctrl-C; this does not remove data. Application shutdown retains its
recovery material. A retained uninstall is:

```sh
helm uninstall "$RELEASE" --kube-context "$CONTEXT" -n "$NAMESPACE" --wait
```

The bundled PVC and separately owned Secrets remain. Reinstall with the same
files plus `--set-string "postgres.bundled.existingClaim=$RELEASE-pg-data"`
and `--set-string postgres.bundled.storageClass=` on the Helm command.
CNPG retains its Cluster only with `postgres.retain: true` (the customer preset
sets it). The retained operator-managed database continues running.
External providers retain their own state. Never delete a namespace for normal
uninstall: it can delete all recovery credentials and claims.

Recovery needs both application and identity databases, original issuer and
subject identities, KMS key/reference, TLS/private CA and provider credentials.
Retained PVCs are not backups. [Operations](OPERATIONS.md) covers quiesced,
paired logical recovery into a fresh target. No published general N-1 upgrade
pair is qualified; the source forward path from v0.4.3 requires backup and
planned downtime. **Helm rollback does not reverse database migrations or
identity changes.** Use a compatible roll-forward or the verified joint restore.
