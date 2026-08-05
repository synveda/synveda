---
title: "AUTH-4: SCIM 2.0 server"
labels:
  - epic:AUTH
  - phase:3
size: L
---

# AUTH-4: SCIM 2.0 server

**Epic:** AUTH — Authentication & identity (functional requirement) · **Phase:** 3 · **Size:** L

## Description

Users+Groups endpoints; joiner/mover/leaver; leaver seals personal scope (retention-held, unreadable by default).

## Acceptance criteria

SCIM conformance tests; mover's memories re-scope per policy.

Read against ADR-0059 (2026-08-05), which is where the two lines above turn
into checkable claims. This is the feature nine earlier ADRs deferred to by
name — ADR-0013's first-login-final placement, migration 0007's missing
update/delete, ADR-0015's explicit-only revocation, ADR-0016/0017's
out-of-process writer, ADR-0055's headless install — so most of the criteria
are about which of those debts are actually discharged and which are not.

- **Conformance is two suites, and each says what it can stand behind.** The
  protocol suite is built from the RFCs' own shapes: `/ServiceProviderConfig`
  advertising exactly what the routes enforce (from the same constants, so the
  two cannot drift), RFC 7644 §3.12's error envelope with its **string**
  `status`, `501 invalidFilter` for a filter outside the implemented subset
  rather than a wrong empty list, and `DELETE` answering `204`-then-`404`
  while sealing rather than deleting. The vendor corpus labels every fixture
  by provenance — replayed from a live tenant, or transcribed from the
  vendor's published table — in ADPT-2's idiom, so "spec-compliant" never
  means more than what was exercised.
- **A mover's memories re-scope per policy, shown as a contrast.** The same
  directory event, the same hierarchy, two packs, two outcomes: out of
  `regulated-strict` the material stays where it was written and that scope is
  sealed; out of `standard` it follows. A demonstration of one of them would
  be a behaviour rather than a policy. And the other half of the same rule: a
  move that resolves the *same* effective pack at both ends asks nothing, so
  changing team inside a department is friction-free.
- **The seal is three layers and no more.** The token stops working at the
  enforcement seam (not at AUTH-6's revocation list, and not at the IdP's
  convenience); the scope is unreadable under a base-layer forbid no pack can
  drop; and the retention sweep stops enumerating it, which is the
  "retention-held" half and the only one that changes an existing loop. There
  is deliberately **no fourth layer**: this feature adds no reader for a
  departed person's private material, and that deferral goes to the export
  plane with a trigger rather than arriving as a side effect of somebody
  resigning.
- **One person never becomes two identities.** The correspondence rule from
  both ends — a SCIM create for somebody who already logged in adopts their
  JIT identity, and a login for somebody the directory created binds to the
  identity waiting for it. The failure this prevents is invisible until it has
  cost somebody their memory: two identities, two personal scopes, half the
  material in each, and nothing anywhere that looks wrong.
- **The seal does not lift.** A rehire is a new identity and a new personal
  scope, in both of the shapes a rehire arrives in, and the sealed one stays
  sealed. A hold the directory can release is not a hold.
- **Losing every group is quarantine, not departure**, and the difference is
  reversible-versus-not.
- **The credential is confined.** A provisioning token reaches no `/v1` route
  and a `/v1` bearer reaches no SCIM route; issuing one is PDP-gated at the
  tenant; revocation binds on the next request; and a credential names its own
  tenant, so a cross-tenant request is absent rather than denied.

## Design

ADR-0059, amended twice while it was built:

1. **The credential names its tenant in the token.** Decision 13 said there
   was no tenant-selecting parameter on the wire; there is one, inside the
   credential, and the alternative was a credential table holding tenant data
   with no tenant policy over it (migration 0036's header).
2. **A hop with quarantine at either end never seals.** Not in the ADR at all.
   Quarantine is not a placement — it is where somebody waits for a mapping to
   be fixed, and where every joiner sits for the moment between being created
   and being put in a group. Without the rule, a tenant whose org root ran a
   different pack from its departments would have sealed every new hire's
   scope seconds after creating it.

Two findings the suite produced rather than the design:

- **A mover under a sealing pack needs a former self.** Sealing derives from
  the identity that owns a node, which works for a leaver and does not work
  for somebody who keeps going at a new scope. The answer is an identity row
  with no subject and the departed status — the same thing decision 12 already
  said a rehire leaves behind, one lifecycle event earlier.
- **`personal_slug`'s uniqueness suffix was not unique.** It took the first
  eight hex characters of a UUIDv7, which are a millisecond clock — identical
  for everything minted in the same ~65-second window. AUTH-2 never hit it;
  AUTH-4 did, by giving one person a second personal scope milliseconds after
  their first.

## Deferrals

- **A reader for sealed material** — the export plane, with its own ADR and
  approval matrix (ADR-0059 decision 8).
- **ADR-0055's headless `init` is not closed by this.** Issuing the credential
  is PDP-gated at `org-admin`, so it presupposes the identity a headless
  install does not have. AUTH-4 gives that deferral a joiner path; it does not
  give it a bootstrap.
- **A two-request move can lose the source department's say.** When a
  directory removes a group before adding the next, the person passes through
  quarantine and the second hop's source pack is quarantine's rather than the
  department's. Bounded by amendment 2 (neither hop seals), and it means a
  cross-regime move done in that order carries material rather than sealing
  it. The trigger is a tenant that assigns record horizons at a department:
  at that point the ordering stops being cosmetic.
