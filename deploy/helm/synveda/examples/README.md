# Deploy to Kubernetes

These examples target the **unpublished v0.4.2 chart candidate** and its
matching digest overlay. Public artifact downloads are unavailable until a
complete release is verified. For current source checks, start with the
[chart setup](../README.md#namespace-installer-and-artifact-acquisition),
which set `CHART` and create `release-images.yaml`. The dependency is
vendored at Keycloak chart 7.3.2; installation does not fetch another chart.

## Loopback evaluation

This creates a single persistent PostgreSQL instance and Keycloak using a
separate database/owner on that server. There is no CNPG/Keycloak operator,
ingress controller, cert-manager or cluster-wide role requirement. You need a
namespace you may administer, a working default storage class, Docker for the
short preparation utility, kubectl, and Helm. The tested CLI is Helm 4.2.3;
Helm 3 lifecycle behavior has not been requalified for this recipe.

Inspect `kubectl config current-context` first. Use a disposable cluster for
an evaluation, or an explicitly authorized namespace. These commands do not
create or change a cluster or storage class.
Namespace creation is an administrator step: if your namespaced credentials
cannot create namespaces, ask the cluster owner to provide `synveda-evaluation`
and omit that command. The application install needs no cluster-admin role.

```sh
: "${CHART:?download and verify the published chart first}"
sh "$CHART/examples/prepare-local.sh" "$HOME/.synveda-kubernetes"
kubectl config current-context
kubectl create namespace synveda-evaluation
kubectl -n synveda-evaluation apply -f "$HOME/.synveda-kubernetes/secrets.json"
helm upgrade --install synveda "$CHART" -n synveda-evaluation \
  -f "$HOME/.synveda-kubernetes/values.json" \
  -f release-images.yaml \
  --wait --wait-for-jobs --timeout 15m
```

The verified overlay pins the published product, bundled PostgreSQL and
Keycloak bytes. Keep that same overlay for subsequent operations. The OCI chart
is also available at `oci://ghcr.io/synveda/charts/synveda`, version `0.4.2`;
`helm pull` is not required when using the verified archive above.

Preparation writes mode-0600 private files once. It creates unique database
passwords, a local database TLS CA/certificate, the original KMS key, fixed
issuer/tenant binding and four explicitly labelled evaluation accounts. Keep
these files with the PVC recovery material. Rerunning preparation preserves
them; Helm never generates replacement credentials.

Start the two forwards in separate terminals:

```sh
kubectl -n synveda-evaluation port-forward --address 127.0.0.1 service/synveda 8120:8120
kubectl -n synveda-evaluation port-forward --address 127.0.0.1 service/synveda-keycloak-http 8081:80
```

Open **http://localhost:8120/console/**. Retrieve the first account password
explicitly in a private terminal (do not paste the result in logs or issues):

```sh
kubectl -n synveda-evaluation get secret synveda-evaluation-accounts \
  -o jsonpath='{.data.admin}' | base64 --decode
```

On macOS, use `base64 -D`. Sign in as **synveda-demo-admin** (Avery Author).
Create your workspace using Getting started. The four accounts are synthetic
fixtures for a local evaluation; this does not create sample product content.
For the single-command governed sample, use the
[Docker walkthrough](../../../compose/PREBUILT.md#first-workspace-and-sample).

The canonical issuer stays `http://localhost:8081/realms/synveda`; the gateway
uses the explicitly configured private discovery address. Issuer, audience,
PKCE and endpoint-origin checks remain enabled. Ports are loopback only, and
plaintext is an explicit local-evaluation choice. Remote users require HTTPS
and reviewed external issuer/trust configuration. Console Sign out revokes
Synveda access; provider SSO remains separate. Use private browser sessions
when evaluating different identities.

## Dependency ownership

Start with exactly one database example, then optionally add the bundled
Keycloak example. Replace `.example.test` endpoints and Secret names with your
reviewed inputs before installation:

| Application storage | Authentication | Values files |
|---|---|---|
| Bundled | Bundled Keycloak | `../ci/bundled-values.yaml`, `bundled-keycloak.yaml` |
| Existing | Existing OIDC | `../ci/external-values.yaml` |
| Existing | Bundled Keycloak | `../ci/external-values.yaml`, `external-database-keycloak.yaml` |
| Bundled | Existing OIDC | `../ci/bundled-values.yaml` |

The executable local preparation produces the first combination with exact
localhost settings. The reviewed HTTPS examples require supplied endpoints,
certificates and existing Secrets; they never provision an existing realm or
modify unrelated database roles. A bundled Keycloak always needs its own durable
`keycloak` database/owner and credentials, even with an external application DB.
CNPG users may instead select `../starter-values.yaml` after their operator
administrator has installed the documented compatible CRD/controller.

See [configuration and existing services](../CONFIGURATION.md) for privileges,
redirects, CA/hostname verification, custom Secret keys, resources, scheduling
and OpenShift configuration. Generic Kubernetes restricted-admission and
arbitrary-UID tests are not OpenShift certification.

## Lifecycle

Use the same release, values and original Secrets for reapply. Wait for the
revision-named install Job; it serializes migrations with the existing database
advisory lock. Check `kubectl -n synveda-evaluation get pods,jobs,pvc` and the
non-secret logs. A failed Job is a failed installation even if a Service exists.
Correct its prerequisite and reapply; never replace keys or reset a useful DB
to force migration through.

The single-instance PostgreSQL and Keycloak StatefulSets use **OnDelete**.
After changing their image, deliberately restart their pods in a maintenance
window after backup; Helm does not silently roll the provider. Gateway/worker
use Recreate and have no multi-replica/HA claim.

`helm uninstall synveda -n synveda-evaluation --wait` removes workloads but
retains the bundled database PVC and operator-created Secrets. Reinstall with
`postgres.bundled.existingClaim: synveda-pg-data` and the **same** Secret files.
The claim contains both databases; losing the KMS or identity material makes
retention incomplete. Deleting a namespace also deletes its Secrets and may
delete claims: it is not an ordinary application uninstall.

Follow [operations and recovery](../OPERATIONS.md) for quiesced paired database
dumps and separately protected Secrets. Application rollback does not undo
schema changes. Epochs before 3 have no supported data upgrade. Test restores
into fresh namespaces and storage, never over an existing deployment.
