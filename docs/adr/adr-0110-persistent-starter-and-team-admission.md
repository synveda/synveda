# ADR-0110: Persistent starter and explicit team admission

- **Status**: Accepted
- **Date**: 2026-09-19
- **Feature(s)**: OPS-11; MEM-7 admission constraint
- **Deciders**: Owner's persistent-starter request; repository implementation

## Context

ADR-0109 already selects optional, preinstalled CloudNativePG and one ordinary
application chart. The external PostgreSQL/OIDC installation works. Small teams
need durable identity storage and a documented route from an operator-selected
owner to revocable workspace membership, without another lifecycle manager.

## Decision

Keep `deploy/helm/synveda`. Add the optional codecentric `keycloakx` 7.3.2
dependency, locked and vendored unchanged, running the existing optimized
Keycloak 26.7.2 image with upstream production startup. Kubernetes Secret
references replace Compose's file adapter at this upstream container boundary.
The chart's redundant `KC_HTTP_RELATIVE_PATH` is removed by `env -u` before
native startup; the existing optimized image's root path remains authoritative.
Both chart path settings are constrained to that root contract. The application
keeps its existing file-only credential inputs. Database and
identity selection remain independent. No chart installs an operator.

The persistent preset uses one existing-CNPG-managed PostgreSQL 17 instance.
Packaged Keycloak shares that server using database/owner `keycloak`; Synveda
keeps its migrator, capability, gateway and worker roles. Reuse the existing
Compose database bootstrap for both databases. With external PostgreSQL, both
databases are independently provisioned by its administrator, with verified TLS.
Never infer Keycloak credentials from the application database URL.

Dependencies belong to the same Helm release. Their versions are explicit,
independent of the application version. Retain the CNPG Cluster across Helm
uninstall so its operator-owned PVCs and database credentials survive. A database
or identity upgrade is an explicit maintenance operation after a joint backup;
a provider selector is not a data migration. One instance and planned downtime
are deliberate. StorageClass, capacity and recovery remain operator inputs.

Public HTTPS terminates at an existing ingress/proxy; only the application
paths and the `synveda` realm/resources are forwarded. Keycloak management and
master-realm administration remain private. No team users, passwords or sample
content are installed. Configure the realm using existing Keycloak tools and
the existing Synveda audience/group mapping contract.

Until MEM-7 supplies a federated identity migration, enforce disjoint issuer
tenant bindings at OIDC admission: multiple static issuers must name distinct
tenants, and a claim-bound issuer cannot coexist with another issuer. Within
that boundary a tenant identifies one exact issuer and grants name its stable
`sub`. Replacing that issuer in place is unsupported; provision a fresh tenant.
Do not enable directory email correspondence in the starter. Email is display
data, never team admission. The operator assigns only the selected initial
owner to `synveda-admins`; the existing durable bootstrap marker closes that
door after its first use. Later admission, role changes and removal use the
existing People/grant/invitation APIs. Headless clients use existing scoped,
short-lived IdP service credentials and registration removal.

The existing Configuration screen offers an explicit organisation target only
when the tenant-root PDP capability forecast permits configuration writes.
It reuses the same generated create/bind APIs and VedaFlow checks; the default
workspace/project/private selection and the role vocabulary remain unchanged.

## Options considered

1. **Existing CNPG plus maintained upstream Keycloak chart** — selected;
   preserves database ownership/authority checks and upstream lifecycle tools.
2. **Another PostgreSQL chart/operator or identity controller** — adds ownership
   and upgrade machinery despite an existing supported integration.
3. **A wrapper duplicating application resources** — unnecessary. The retained
   Cluster and separately pinned dependency images express the lifecycle here.
4. **Full identity/schema re-keying in this packaging slice** — would change
   existing principals and the single baseline. Enforce the narrower issuer
   boundary now; federated linking and provider replacement remain MEM-7.

## Consequences

- Fresh installations have an explicit persistent option; external services and
  Compose retain their contracts. No application HA or release-readiness claim.
- The starter needs a preinstalled CNPG operator and provisioned storage.
  Uninstall intentionally leaves running database resources and Secrets;
  retained storage still requires tested, separate backups and key custody.
- Existing overlapping multi-issuer configurations fail closed with an
  actionable error. Separate issuers serving distinct static tenants continue
  to work. Provider migration requires a separately accepted identity decision.
- Revisit release separation if independent provider ownership is required by
  a concrete installation; do not introduce duplicated application templates.

## Compliance notes

Cedar, forced RLS, ordinary tenant transactions, VedaFlow and audit remain the
authority boundaries. Membership revocation is checked on subsequent governed
requests in the single gateway; already delivered content cannot be recalled.
IdP logout/disable alone is bounded by access-token expiry. Tests must measure
the distinct grant/service-registration and token-expiry behaviours.
