---
title: "CPR-6: Governed scope anchors — the PDP re-cut"
labels:
  - epic:CPR
  - phase:5
size: L
---

# CPR-6: Governed scope anchors — the PDP re-cut

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** L

## Description

Prompt 6 of the 33-prompt context-platform programme. CPR-3 built the governed
scope tree, CPR-4 put workspaces and projects on it, CPR-5 put people in them —
and every one of those three recorded the same debt in its own ADR: **the
decision point still described the old hierarchy.**

So the product had, at the start of this prompt, a complete membership model
that decided nothing. A workspace `owner` grant was a governed record of
authority; what actually let somebody administer a workspace was a
`role_bindings` row bound at a node of the tree this programme is deleting.

Five pieces:

- **A scope-anchor resolver** (`synveda_store::anchors`) — where a request
  stands, as an *ordered set* rather than one chain.
- **A rewritten Cedar entity model** — seven entities, and a decision that
  names the thing it is about.
- **The rank vocabulary out of the PDP** — `Principal.department` deleted,
  `Scope.kind` re-cut to the five shapes, `standard`'s sharing default
  re-expressed over grants.
- **Personal principal-scope privacy** as a base-layer forbid, with a door.
- **`GET /v1/me` capabilities from real decisions**, per anchor, and the SCIM
  boundary projecting onto the same four access tables.

Decisions in ADR-0073.

## Why this exists

Because a membership model that does not decide anything is a table.

The sharper reason is the one the old model could not express at all.
`hierarchy_closure` gave every caller a **chain**: one path from a placement
node up to the org root, walked by every rule. That encoded two assumptions,
and the governed model breaks both:

1. **A caller stands in one place.** They do not. Somebody can hold their own
   scope, a workspace shared with them, one project inside a *different*
   workspace, and a tenant-wide grant. Four applicable places, none containing
   the others; a chain can represent one.
2. **Authority runs upward.** In a placement model, being *in* a team made you
   a member of the department and the org — membership was containment, read
   from the leaf up. A grant is the other direction: being given a project must
   reach that project's subtree and stop, and "a project-only grant does not
   reach its workspace" was a sentence the old model could not make true.

## Design

### The anchor set

```
AnchorSet (ordered, most specific first)
  ScopeAnchor { scope_id, kind, parent_scope_id, depth,
                source, roles, granted_at, via_groups }
```

Six inputs: the authenticated principal's own scope; the selected project; the
selected workspace; the organisation-unit relationships above either of them;
the tenant root; and every scope a direct or group grant reaches this caller
at, whether or not the selection named it — which is what makes project-only
access work at all.

Ordering is `(source precedence, depth descending, scope id)`. **Depth is a
tie-break for readers, never an authorisation input**: nothing grants more
because an anchor sorted earlier, and no comparison anywhere asks whether one
*kind* outranks another. Two entries for one scope merge into one anchor under
the more specific source, so a set built twice from the same rows is the same
set.

Principal privacy is applied while the set is built
(`synveda_types::access::inherits_into`), in the same SQL shape
`access::members_of` carries it — so a resolver and a member listing cannot
disagree about whose notes are whose.

### Seven Cedar entities

```
Tenant
Scope      in [Tenant, Scope]          { tenant, kind, sealed }
Group      in [Tenant]                 { tenant }
Principal  in [Tenant, Scope, Group]   { tenant, quarantined, own_scope?,
                                         token_scope?, anchors, ambit, private }
Workspace  in [Tenant, Scope]          { tenant, scope }
Project    in [Tenant, Scope]          { tenant, scope, workspace }
ScopeGrant in [Tenant, Scope]          { tenant, scope, role, source }
```

The four new ones are parented to the scope they belong to, so every
containment rule written over scopes reaches them without being restated.

`principal.anchors` is the **downward** direction — `resource in
principal.anchors` is "inside something I hold". Anchors are deliberately *not*
entity parents: an entity parent is the upward direction, and a project grant
that made its holder a member of the workspace above it would be the placement
chain returning under another name.

### What each route now names

| Was | Is |
|---|---|
| `Resource::Tenant` for `WorkspaceRead`/`Update` | `Resource::Workspace(id)` |
| `Resource::Tenant` for `ProjectRead`/`Update` | `Resource::Project(id)` |
| `Resource::Tenant` for `ProjectCreate` | the parent workspace |
| `Resource::Tenant` for `MembershipRead`/`Grant` at a workspace or project | that workspace or project |
| `Resource::Tenant` for revoking a grant | `Resource::Grant(id)` |
| `Resource::Tenant` for updating a group | `Resource::Group(id)` |
| `Resource::Tenant` for the tenant-plane listings and creations | the tenant **root scope**, or the tenant when none exists yet |

Two stay on the tenant plane for reasons rather than for want of a resource:
creating a group has no group to name, and redeeming an invitation must work
for somebody who holds nothing anywhere.

### The vocabulary that left

`Principal.department` is gone. It named the nearest department-kind ancestor
of a placement and was the last rung of the rank ladder inside the PDP;
`standard`'s entire sharing default read it.

What replaces it is `principal.ambit` — the **parent** of every scope the
caller holds, minus the tenant root. It reproduces the rule's shape (a grant at
one team shares the teams beside it) without asking what kind of thing that
parent is, and excluding the root is what keeps `standard` from meaning
`open-collaboration`.

## Acceptance criteria

- An anchor set is ordered most-specific-first, merges one scope into one
  anchor however many ways it became applicable, and orders by structure rather
  than rank — nesting the same tree four levels deeper changes no verdict,
  asserted over every probed action.
- A workspace grant is in force at that workspace's projects **with no row
  written there**, and reaches neither a sibling workspace nor the tenant.
- A **project-only** grant reaches the project and refuses every read, update
  and administration of the workspace above it.
- A grant naming a group reaches its members; membership of a group with no
  grant naming it confers nothing.
- Revoking a grant refuses the very next decision, with nothing invalidated;
  and revoking is itself a decision that names the grant.
- A profile assigned at an organisation unit governs everything beneath it
  however deep, and a grant written there reaches the same subtree.
- **Nobody reaches into somebody else's own scope** — not a tenant-root owner,
  under no pack, at no tier, for content or for membership — while their own
  scope is theirs and a grant written *directly at* it reaches it.
- A foreign tenant's chain, anchor and entity grant nothing, and a chain
  spliced across two tenants launders nothing.
- The capability block for an anchor is the set of decisions it forecasts,
  moves with the grant and the profile and with nothing else, and forecasts
  nothing at all for somebody holding nothing.
- `GET /v1/me` mints the caller's own scope, serves its anchors and names how
  many the bound dropped.

## Out of scope

- **Renaming the packs to `personal`/`team`/`enterprise`** (ADR-0068
  decision 2). A separate cut with its own defaults; this one re-anchors the
  mechanism.
- **The runtime planes.** Observe, inject, recall, prompts, context packs,
  skills, channels, proposals and quarantine still resolve chains from
  `hierarchy_closure` and supply no anchors — which changes no verdict, because
  an anchor's scope is never a node of a hierarchy chain. Prompts 7–18.
- **Deleting the old hierarchy.** It stays until the prompt that removes it;
  what changed is that PDP evaluation no longer *requires* it.
