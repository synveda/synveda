# ADR-0093: directory facts project once onto shared principals and access

- **Status**: Accepted
- **Date**: 2026-08-25
- **Feature(s)**: CPR-34
- **Deciders**: autonomous context-platform continuation

## Context

ADR-0059 and ADR-0060 built a conformant SCIM resource mirror, one
joiner/leaver reconciler, safe absence inference and durable pull state. CPR-6
then projected SCIM groups into the new Group aggregate, and CPR-7 deleted
group-driven hierarchy placement. The transition stopped halfway: SCIM push
writes both `scim_groups`/`scim_group_members` and
`groups`/`group_members`, scheduled pull updates only the former, membership is
keyed by token subject so a directory-created pre-login identity cannot be a
member, and no supported application command creates the directory-owned
`scope_grants` the new PDP actually consumes.

The external protocol still needs attributes that have no product meaning, so
the directory user resource cannot be collapsed blindly into `identities`.
Groups and membership have no such distinction: the shared aggregates already
carry their product identity, lifecycle, revision and source. Retaining a
second group graph makes removal and access depend on which writer ran.

## Decision

1. **One product projection.** A directory user remains an adapter resource
   linked one-to-one to `identities`; the identity and its principal-shaped
   scope are the product principal. The adapter row retains the source and
   external id needed for SCIM echo, pull reconciliation and first-login
   adoption. Every lookup is qualified by tenant before the external id is
   considered.

2. **A directory group is a Group.** `groups` carries its directory source,
   stable provider resource id and optional protocol `externalId`.
   `group_members` keys the member by stable `IdentityId`, not by a token
   subject that may not exist yet. Effective-access reads join the identity
   and require an active, bound subject before emitting an authority. The
   `scim_groups` and `scim_group_members` tables and their store DTOs are
   deleted without row translation.

3. **Push and pull share one replacement projection.** SCIM group requests
   and directory snapshots call the same store command, which upserts one
   source-owned Group and replaces its identity membership atomically. Pull
   connector output includes stable vendor group ids and memberships. A
   complete pass may archive source groups it did not see; a partial pass may
   only establish presence. Connector/pass state remains the existing durable
   tenant row, and a connector change clears absence evidence before another
   conclusion is drawn.

4. **An access assignment is a `scope_grants` row, not adapter policy.** A
   dedicated public directory command creates or revokes a group-subject grant
   carrying the directory source/resource evidence. Creation requires an
   `Idempotency-Key`; both directions take the same scope/grant-anchored
   `MembershipGrant` Cedar decision and use the same RLS transaction and
   hash-chained `access.granted`/`access.revoked` actions as direct access.
   The ordinary revoke/update paths continue to refuse a directory-owned row;
   the explicit directory command is the only supported owner path.

5. **Removal is resolved, not copied.** An archived Group contributes no
   members, a removed `group_members` row contributes no group anchor, and an
   inactive/departed identity contributes no effective member. Consequently a
   group deletion, membership removal or principal disable withdraws access on
   the next request without fan-out grants, cache repair, role bindings or a
   directory-specific Cedar path. Historical grant and audit evidence remains.

6. **Evidence labels do not improve themselves.** Existing Entra and Okta
   fixtures remain labelled transcribed or captured exactly as their source
   permits. This package may prove protocol and deterministic connector
   behaviour locally; it does not claim live vendor verification without a
   real tenant run.

## Options considered

1. **Keep both group graphs and repair every writer.** Smaller code change,
   but every removal remains a two-table transaction with a permanent drift
   mode. Rejected by the one-domain-model decision.
2. **Collapse every SCIM user attribute into `identities`.** Removes the
   mirror, but pollutes the principal aggregate with protocol-only phone/name
   fields and cannot honestly echo an unbound SCIM resource. Rejected.
3. **Key membership by token subject and wait for another sync after login.**
   Keeps the current API but strands a correctly provisioned user indefinitely
   when the directory sends no later group event. Rejected; identity is the
   stable principal address.
4. **Translate old mirror rows into the new Group graph.** Preserves local
   development data and violates the locked pre-1.0 hard cut. Rejected; reset
   is the supported transition.

## Consequences

- Positive: SCIM and pull have one access result; pre-login membership has a
  stable address; delete/disable/remove all hit the same effective-authority
  query; directory assignments are visible and auditable as ordinary grants.
- Negative / accepted: group-management request bodies now name identity UUIDs
  rather than free-form subjects; a pre-cut database with directory/group rows
  cannot traverse this migration and must be reset as the epoch contract
  already requires.
- Reversal trigger: a supported directory provides authoritative per-user or
  per-group scoped role assignments with a stable revision/cursor → extend the
  adapter input and map them to the same grant command; never add a second
  permission table.

## Compliance notes

- **PDP/RLS:** assignment mutations use `MembershipGrant`; effective access
  remains the existing anchor/Cedar path. All adapter and shared rows are
  tenant-qualified and forced-RLS.
- **Audit:** access mutations reuse the existing chain actions with source ids;
  reconciliation records identifiers/counts, never directory payloads or
  credentials.
- **Secrets:** credential custody is unchanged in this package. No connector
  secret enters a response, trace, fixture, audit payload or frontend state.
