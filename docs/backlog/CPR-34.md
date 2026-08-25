---
title: "CPR-34: Directory adapter convergence"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-34: Directory adapter convergence

**Epic:** CPR — Context-platform redesign · **Phase:** 5 · **Size:** XL

## Description

Re-anchor the existing SCIM push and scheduled Entra/Okta pull adapters on the
context platform's principal, shared Group, `group_members` and `scope_grants`
model. Directory state remains boundary metadata; it does not become a second
identity, membership or authorisation runtime.

## Acceptance criteria

- A SCIM or pull user projects onto one tenant-owned identity and principal
  scope. The adapter retains its external id and source ownership, and a later
  login adopts that identity rather than minting another principal.
- A directory group is the shared `groups` aggregate. Its membership is stored
  only in `group_members`, keyed by the stable identity so a pre-login
  principal is representable without inventing a token subject. The old SCIM
  group and membership tables are deleted without translating their rows.
- Both SCIM push and complete/partial pull enumeration call one idempotent
  projection. Pull records stable vendor group ids rather than treating a
  mutable display name as identity; an incomplete pass records presence but
  cannot retire an unseen group or principal.
- Directory access assignments are ordinary group-subject `scope_grants` with
  explicit source evidence. Creation is idempotent, takes the same
  scope-anchored `MembershipGrant` Cedar decision as a direct grant, and every
  create/revoke chains the existing access audit action. Direct group/grant
  mutation routes refuse directory-owned rows by name.
- Removing a membership, disabling a principal, or deleting a group withdraws
  effective authority on the next request through the same anchor resolver,
  Cedar entities and RLS queries used for manually managed access. No stale
  role binding, placement convention or directory-only permission table
  participates.
- Every external-id lookup is tenant-qualified, adapter ownership is retained,
  every tenant table is forced-RLS, and adversarial tests prove that identical
  external ids in different tenants cannot cross-link.
- Sync state remains durable across passes and connector changes. Authentic
  Entra/Okta fixtures retain their captured/transcribed labels; deterministic
  tests are not described as live verification.
- OpenAPI/generated clients, focused SCIM/pull/access/RLS tests, an acceptance
  demo, `make ci` and `make db-test` pass.

## Evidence

Delivered 2026-08-25 from
`3c61e5e0fa35f8e9a0056f1e7d53a19bfe43debc` under accepted ADR-0093 and
migration `0059_directory_adapter_convergence`. SCIM push and scheduled pull
now converge on source-qualified directory users, shared Groups and
identity-keyed membership; stable vendor resource ids survive display-name
changes, while incomplete pull passes make no absence claim. Directory access
assignments are separately `MembershipGrant`-decided, idempotent
group-subject `scope_grants` with source evidence and hash-chained create/revoke
events. Direct group/grant routes refuse source-owned rows. The old
`scim_groups` and `scim_group_members` tables and DTOs are deleted without
translation, and a database carrying affected pre-cut rows is refused with the
documented reset command.

Focused evidence: identity connector fixtures **5/5**; store access **30/30**,
anchors **13/13** and directory sync **8/8**; gateway access **18/18**,
directory sync **9/9**, SCIM **10/10** and anchors **9/9**; OpenAPI **6/6**;
console **212/212**; forced-RLS **83/83**; full offline workspace compilation
and SQLx prepare/check pass. The generated contract grows **165 → 167
operations** and **264 → 266 schemas**; epoch 2 has **57 migration files**,
**694 SQLx descriptions** and **89 forced-RLS tenant tables**. Isolated
`demos/cpr-34-directory-convergence.sh` passes with three shared directory
groups, six chained group transitions, identity-keyed membership and zero old
mirror tables; the complete **82-script** demo drift gate, final `make ci` and
full fresh-database `make db-test` pass. Entra/Okta inputs remain honestly
labelled captured/transcribed fixtures: no live vendor tenant was available and
no live verification is claimed. The feature commit hash is recorded by the
next checkpoint.
