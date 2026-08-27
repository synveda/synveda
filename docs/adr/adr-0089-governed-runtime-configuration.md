# ADR-0089: runtime configuration is immutable content selected by governed scope bindings

- **Status**: Accepted
- **Date**: 2026-08-25
- **Feature(s)**: CPR-30
- **Deciders**: autonomous context-platform continuation

## Context

ADR-0068 locked one binary, schema and decision path for personal, team and
enterprise use, with policy/configuration profiles rather than edition
conditionals. The current implementation still reaches that goal indirectly:
mutable `policy_pack_defaults` and `policy_pack_assignments` choose a Cedar
pack, while capture, composition, retention and advertisement settings are
fields on the pack row. Direct policy-assignment routes alter those rows
without VedaFlow, and a pack name is simultaneously a policy identity, a
runtime settings document and a deployment-profile label.

That shape cannot identify the exact settings a capture batch or context run
used, compare two settings revisions, roll a project back without rewriting
history, or govern a settings change with the same proposal machinery as
Knowledge, Skills and Tools. It also tempts clients to branch on a profile
name instead of evaluating explicit configuration. The context-platform hard
cut permits deleting that mutable assignment plane; it forbids translating or
dual-writing its rows.

## Decision

1. **Configuration has a stable aggregate and immutable versions.** A
   `ConfigurationArtifact` owns ordered `ConfigurationVersion`s. Each version
   contains one complete, validated document and its canonical BLAKE3 digest.
   Publishing new content mints a version and advances the aggregate's current
   pointer; a version is never updated or deleted.

2. **A revisioned scope binding is the only runtime selector.** A
   `ConfigurationBinding` belongs to one governed scope, is enabled or
   disabled, and either follows the artifact's current version or pins an
   exact version. Resolution walks the resource chain nearest-first. The
   tenant-shaped root is the ordinary tenant default; no separate default
   table or fallback row exists. With no binding, the immutable built-in
   enterprise document is the fail-safe.

3. **Profile templates are data, never branches.** `personal`, `team` and
   `enterprise` are canonical built-in documents exposed by the public API.
   They select the corresponding existing Cedar policy semantics and spell out
   capture, context, freshness, advertisement and provider settings. Creating
   from a template copies that complete document into an ordinary governed
   immutable version. Runtime code reads document fields; it never tests a
   profile/edition name.

4. **The document is closed and complete at this boundary.** It carries the
   policy-pack selector; explicit/session-end capture rules and candidate
   bounds; context token budget, knowledge/candidate channels and trace
   retention; type-aware freshness defaults; Skill and Tool advertisement
   switches; and a closed list of allowed external-provider families. Bounds
   are validated before a proposal opens and again when its effect runs. A
   configuration can narrow access or delivery but cannot grant an action;
   Cedar remains the only authorisation decision point.

5. **Every effective mutation is a typed VedaFlow change.** Create, publish,
   bind, enable/disable, pin/unpin and rollback open an
   `AssetKind::Configuration` proposal with `ProposalEffect::Apply`. The typed
   projection binds the complete command to a content-free manifest hash.
   Application repeats ownership, `ConfigurationWrite`, `ProposalOpen`, live
   approval, payload, current-version and binding-revision checks. A permissive
   matrix may apply the change immediately; it still creates and executes the
   VedaFlow change.

6. **Runtime reads retain exact configuration evidence.** Session-end and
   explicit capture resolve configuration before freezing evidence. Context
   planning resolves it before retrieval and records the exact version/digest
   alongside retrieval versions. Background capture loads the immutable
   version named by the batch, so a later binding change cannot change an
   already-frozen operation. Skill and Tool advertisement and remote-provider
   use resolve the current binding on each request.

7. **The old assignment authority is deleted.** Migration 0055 drops
   `policy_pack_defaults` and `policy_pack_assignments` without translating
   rows. The direct default/scope-policy mutation routes and console controls
   leave with them. Request-time `PolicyAssignment` values remain only as an
   in-memory projection of resolved configuration for the embedded Cedar PDP;
   no caller can persist one directly.

8. **The public API exposes templates, artifacts, versions, comparison,
   effective resolution and bindings.** Collections are cursor-paginated;
   retryable creation requires `Idempotency-Key`; version publication requires
   the expected current version; binding changes require the exact revision.
   Generated console and CLI clients use those operations only.

## Options considered

1. **Immutable documents and scope bindings (chosen).** Gives every runtime
   decision an exact address while preserving Cedar as the one authority.
2. **Keep settings on policy packs and put only assignment through VedaFlow.**
   Smaller, but a policy source update and a runtime-settings update remain one
   mutable noun and exact runtime evidence is still unavailable. Rejected.
3. **Materialise effective settings per project.** Fast reads, but every
   ancestor update requires a fan-out and creates a second current answer.
   Rejected; nearest-first resolution is indexed and bounded by scope depth.
4. **Persist three profile rows per tenant.** Makes templates tenant data that
   can drift before anybody chose them and requires bootstrap mutation outside
   VedaFlow. Rejected; templates are immutable sources, copied only by a
   governed create.

## Consequences

- Positive: configuration is reviewable, comparable, rollbackable and cited by
  runtime work; one explicit document drives every deployment profile; a policy
  pack no longer doubles as an ad-hoc settings record.
- Negative / accepted: fresh tenants run the conservative built-in enterprise
  document until they deliberately create and bind another; following-current
  bindings change when a new governed version publishes, while pinned bindings
  deliberately do not.
- Reversal trigger: bounded nearest-first resolution enters the measured
  context SLO → add a digest-keyed cache invalidated by binding/version
  commits, never a mutable effective-settings authority.

## Compliance notes

- **PDP/VedaFlow:** reads use `ConfigurationRead`; every mutation uses
  `ConfigurationWrite` plus `ProposalOpen`, repeated by the apply effect.
  Binding replacement is decided under inherited configuration so it cannot
  authorise itself.
- **RLS:** every new table is tenant-bound, uses composite tenant foreign keys,
  enabled and forced RLS, and joins the completeness/adversarial gates.
- **Audit:** opened/applied/rejected configuration changes and effective reads
  record ids, revisions, digests, profile provenance and decision context, not
  the provider credentials or document prose.
- **Secrets:** the document contains provider family names and secret-reference
  policy only; it accepts and returns no credential value.
