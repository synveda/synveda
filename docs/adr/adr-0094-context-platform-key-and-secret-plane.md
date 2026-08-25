# ADR-0094: secret identity is stable while ciphertext and keys rotate

- **Status**: Accepted
- **Date**: 2026-08-25
- **Feature(s)**: CPR-35
- **Deciders**: autonomous context-platform continuation

## Context

ADR-0064 introduced one deployment data key, per-tenant data-key generations,
sealed console tokens, a mutable name-keyed `tenant_secrets` table and a sealed
tenant export. That boundary remains sound: searchable Knowledge cannot be
application-encrypted without deleting the lexical/vector query model, so live
content relies on the substrate's encrypted volume while recoverable secrets
and data leaving the deployment use envelope encryption.

The context-platform cut exposed three incomplete joins. A directory credential
is replaced in place under a string name, so there is no stable reference or
content-free history of rotation/revocation. Immutable Tool versions may carry
an opaque reference, but an internal reference is neither tenant/scope checked
nor rechecked when client configuration is generated. Finally, the sealed
tenant export still serialises the deleted Record aggregate rather than
Knowledge revisions and provenance. Rotating a tenant DEK also leaves every
database-held secret on the retired generation until an unrelated rewrite.

## Decision

1. **Keep the two-scope key ring.** The deployment key still seals data needed
   before tenant resolution; a tenant's versioned DEK still seals tenant-bound
   payloads. The existing `Kms` interface remains the provider boundary. This
   package adds no customer-managed-key, cloud-KMS or HSM implementation and
   makes no such support claim.

2. **A local secret is a stable aggregate.** A UUIDv7 `TenantSecretId`, tenant,
   governing scope, closed purpose kind, credential-free label/provider and
   stable `synveda-secret://<uuid>` reference survive value rotation. The
   current envelope, logical value revision and key generation may change;
   revocation destroys the envelope but retains content-free identity and
   timestamps. Old name-keyed rows are refused and the table is rebuilt
   without translation under the pre-1.0 reset contract.

3. **One AAD shape serves every stored tenant secret.** All local values use
   the tenant scope, the closed `tenant.secret` purpose and the stable secret
   UUID as row key. Directory, Tool-server, model-provider and import/export
   are metadata kinds, not different ciphers. A ciphertext transplanted to
   another tenant or reference therefore fails before plaintext is returned.

4. **Internal Tool references are live authority inputs.** Tool descriptors
   may continue to retain external opaque references for a trusted local
   adapter. A reference using Synveda's scheme must parse canonically and name
   an active `tool_server` secret in the same tenant and governing scope when a
   version is staged, approved and rendered. A missing, revoked, wrong-kind or
   cross-tenant reference fails closed with one non-oracular error. Rotating
   the secret value does not mutate or re-digest the immutable Tool version;
   removing a binding still removes it from generated configuration.

5. **Directory custody uses the same aggregate.** The well-known directory
   label resolves one stable root-scope secret. Setting it rotates that value;
   clearing it revokes rather than deleting identity. A revoked or corrupt
   stored credential suppresses deployment fallback, while a tenant that has
   never configured a stored credential may retain the explicitly documented
   deployment fallback.

6. **DEK rotation schedules durable re-encryption.** A tenant rotation creates
   a durable job naming the old and new key generations. Active local-secret
   envelopes are opened with the generation in their headers and resealed
   under the new current key without changing secret ids or logical value
   revisions. Job state and counts are durable and retryable; retired keys
   remain available for old external export archives and any other payload not
   owned by this job.

7. **The sealed export speaks the context-platform model.** A new archive
   magic and format contain stable Knowledge heads, immutable head history,
   revisions, normalised sources, revision-source links, relations and the
   hash-chained audit log. It contains no Record compatibility section and the
   reader does not accept the old archive magic. Knowledge plaintext exists
   only inside the archive's tenant-bound envelope; its clear header contains
   identifiers, generations and counts only. Re-import and tenant deletion
   remain TEN-5 work.

8. **Secret consumers stay at their honest boundaries.** The extractor's
   deployment model credential and Helm/Compose secret injection remain
   deployment configuration because no tenant provider-selection path exists
   yet. OKF v0.2 remains network- and credential-free. The generic tenant
   secret command can custody future scoped provider values, but no dormant
   consumer or support claim is invented.

## Options considered

1. **Put credentials directly in immutable Tool versions.** Rejected: every
   rotation would either mutate history or publish a new version containing a
   secret, and normal APIs would necessarily serialise it.
2. **Keep string names as references.** Rejected: a rename silently retargets
   immutable metadata, and tenant/scope confusion remains indistinguishable
   from an intended lookup.
3. **Destroy a tenant DEK during rotation.** Rejected: old exports and other
   envelopes become unreadable before their owners can be re-encrypted.
4. **Application-encrypt all searchable Knowledge rows.** Rejected under the
   existing threat model because PostgreSQL/Tantivy/pgvector cannot search
   ciphertext; this would delete rather than strengthen the retrieval plane.
5. **Translate old secret rows or read both archive formats.** Rejected by the
   locked hard cut. A reset is the supported pre-1.0 transition.

## Consequences

- Positive: immutable artifacts point at stable identifiers; value and DEK
  rotation do not rewrite their history; stale/cross-tenant references fail
  closed; the only tenant export now preserves Knowledge history and remains
  unreadable without that tenant's key.
- Negative / accepted: databases carrying an old name-keyed secret must reset;
  local-secret rotation is an operator act rather than a public product API;
  retired DEKs remain until the later tenant-erasure workflow can prove that
  no owned envelope needs them.
- Reversal trigger: a supported runtime selects a tenant-scoped external model
  or import provider -> resolve its stable reference through the same boundary
  and add provider-specific delivery outside the gateway process; never expose
  plaintext through the application contract.

## Compliance notes

- **PDP/RLS:** local secret custody remains the documented operator
  break-glass boundary. Runtime Tool use has already passed Tool/project PDP;
  internal-reference resolution adds tenant/scope forced-RLS checks and grants
  no authority by itself.
- **Audit:** provision/rotate/revoke and re-encryption completion carry only
  ids, kinds, revisions, generations and counts. No envelope, path content or
  credential enters the chain.
- **Secrets:** ordinary APIs, generated clients, traces and `Debug` output
  expose descriptors/reference presence at most. Store types redact envelope
  bytes, and errors never echo candidate plaintext.
