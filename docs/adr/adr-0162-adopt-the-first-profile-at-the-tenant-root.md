# ADR-0162: The first administrator adopts the tenant's profile at the root during first-run onboarding

- **Status**: Proposed
- **Date**: 2026-11-01
- **Feature(s)**: Consumer installation and harness setup
- **Deciders**: Owner decisions of 2026-11-01: `regulated-strict` stays the
  default; first-run onboarding explains the packs and lets the first
  administrator choose another; onboarding offers all three canonical
  templates; and the choice stays available until it is made. Drafted for
  owner review.

## Context

On 2026-11-01 the owner decided that `regulated-strict` stays the default
policy pack and that first-run onboarding lets the first administrator choose
`standard` for a small team.

ADR-0089 decision 2 makes the tenant-root Configuration binding the tenant
default. With no binding, the built-in enterprise document applies, and it
selects `regulated-strict`. A Configuration change is decided under the
configuration it inherits, so it cannot authorise itself. Under
`regulated-strict` that change needs two administrators other than the author,
and a further person to apply it (`crates/synveda-policy/src/approvals.rs`):
four distinct people.

ADR-0100 decision 2 already breaks the resulting deadlock for a fresh tenant.
A root administrator may auto-apply the tenant's first Configuration artifact,
if its document is exactly one canonical template, and that artifact's first
binding at its governing scope. Both remain typed, audited VedaFlow changes,
and the tenant root row serialises the check
(`initial_profile_adoption` in `crates/synveda-gateway/src/configuration.rs`).

First-run onboarding (ADR-0075 decision 6) uses that rule at the wrong scope.
It creates and binds the chosen template at the new **workspace**
(`console/src/Onboarding.tsx`), and `synveda demo` does the same
(`crates/synveda-cli/src/demo.rs`). The workspace then runs the chosen pack,
but the tenant root stays unbound. Tenant-resource decisions
(`tenant_policy_pack` in `crates/synveda-store/src/configuration.rs`), every
later workspace and every Configuration change decided under the root still
use `regulated-strict`. The one-time rule has been spent. This is why the
v0.4.5 sample's own Configuration waited for two reviewers and a separate
applier.

The rule's closing condition has a second problem. It closes once any
Configuration artifact or binding exists anywhere in the tenant, so a profile
at one workspace closes the tenant-wide choice before anyone has made it.
Every tenant onboarded by today's wizard is in that state.

The packs also differ in who can change Configuration afterwards. Under
`standard`, a Configuration change needs one administrator who is not its
author. Under `open-collaboration`, an administrator may approve their own
change at shared scopes, and changes at a workspace or project need no
approval.

## Decision

1. **First-run onboarding adopts the profile at the tenant root.** The console
   creates the chosen canonical template with the tenant root as its governing
   scope and binds it there. This is the wizard's first step, before anyone
   is invited; the team shape's invitations already come afterwards, from
   People. The workspace step no longer creates or binds a profile;
   workspaces inherit from the root.
2. **Onboarding offers all three canonical templates, or skipping.** The step
   explains each pack and who can approve a later Configuration change under
   it:
   - `enterprise` (`regulated-strict`): two administrators other than the
     author, and a separate person to apply it;
   - `team` (`standard`): one administrator other than the author, so a team
     with one administrator cannot change Configuration until it has two;
   - `personal` (`open-collaboration`): an administrator alone.

   Choosing a template binds it at the root. Skipping binds nothing: the
   tenant runs the `regulated-strict` fail-safe, which is the default, and the
   choice stays available to every root administrator. Before the
   administrator confirms `enterprise`, onboarding says that leaving it later
   needs that four-person review. Nothing records an edition; the binding is
   ordinary governed Configuration (ADR-0075 decision 6 and option 4 stand).
3. **The adoption rule moves to the root and stays open until the root's
   profile is chosen (amends ADR-0100 decision 2).** A Configuration change
   applies without approvals only when all of these hold when it is opened
   and again when its effect runs:
   1. the caller's live decision includes `administrator` granted at the
      tenant root;
   2. no binding exists at the tenant root, and no Configuration artifact is
      governed there other than, for a bind, the artifact being bound;
   3. a create's document is byte-identical to one canonical template, with
      truthful template provenance, and a bind points that root-governed
      artifact at the root.

   Who else holds grants does not matter. Compared with ADR-0100, the rule is
   looser in one direction: profiles at workspaces no longer close it. It is
   tighter in another: a profile at a workspace always follows the live
   approval matrix, even when it is the tenant's first.
4. **Serialisation.** The existing tenant root row lock serialises adoptions
   and root bindings. When two root administrators race, one adoption wins;
   the other change finds the root taken and follows the live approval
   matrix.
5. **It stays a governed change.** Adoption opens a typed Configuration
   proposal, runs live `ConfigurationWrite` and `ProposalOpen` decisions,
   re-authorises the effect, stores immutable objects and chains audit. The
   opened and applied events carry `initial_profile_adoption`, the source
   template, the selected pack, the tenant-root target and the document
   digest. From audit alone, a reviewer can tell who chose which pack and
   when.
6. **The server says whether the choice applies.** The `/v1/me` onboarding
   view gains a closed field: whether this caller may adopt the tenant's
   first profile now, and which canonical template the root carries once one
   is bound. The console offers the choice, during onboarding or later, only
   when the server says so; otherwise it states the tenant's effective
   profile and that changes go through review.
7. **No break-glass Configuration path.** Once the root is bound, its profile
   changes through review under the bound pack.

## Options considered

1. **Keep workspace adoption and document the review path.** Rejected: a team
   of two or three can never leave `regulated-strict`, which defeats the owner
   decision.
2. **Make `standard` the fail-safe.** Rejected: it contradicts the owner's
   default and the seed's conservative fail-safe.
3. **Let the operator choose at install time** (an environment variable, Helm
   value or migrator command). Rejected for the first offer: it moves the
   choice away from the first administrator. It also needs either a
   Configuration effect outside VedaFlow or a new operator VedaFlow actor.
   Reconsider for unattended installs.
4. **Add a break-glass root binding for stuck tenants.** Rejected: it would be
   the first effective Configuration mutation outside VedaFlow (ADR-0089
   decision 5). Under decision 3, any tenant whose root is unbound recovers
   without it.
5. **Keep "any artifact or binding exists" as the closing condition.**
   Rejected: a profile at a workspace, including every v0.4.x onboarding,
   closes the tenant's choice before it is made.
6. **Close the choice once anyone other than the first administrator holds a
   grant.** Rejected. Tenants that invited people before
   choosing, tenants whose grants come from a directory and populated v0.4.x
   tenants would stay on `regulated-strict` behind a four-person review,
   which is the stall the owner decision removes. What it guards against is
   one root administrator making a populated tenant's first choice. That is
   the act this rule exists for; audit names who made it, and a tenant that
   wants the fail-safe to hold can bind `enterprise`.
7. **Require a second root administrator's approval when one exists.**
   Rejected: it adds an approval requirement outside the pack matrices, and
   binding `enterprise` already gives a tenant that protection.
8. **Restrict adoption to the holder of the bootstrap administrator grant**
   (`tenant_administrator_bootstraps`). Rejected: it ties the choice to one
   person. A tenant whose first administrator has left, or that was seeded
   without the IdP administrator group, could never choose.

## Consequences

- Positive:
  - A two-person team can choose `standard` at first run, and the whole
    tenant follows it, including tenant-resource decisions and later
    workspaces.
  - The choice stays available until it is made. Tenants that skipped it,
    invited people first, take grants from a directory or onboarded under
    v0.4.x can still adopt at the root.
  - Every choice is audited: who chose which template and pack, and when.
- Negative / accepted trade-offs:
  - While the root is unbound, any one root administrator can make the
    tenant's first choice without review, including `personal`
    (`open-collaboration`), in a tenant that already has members. Audit
    records it. A tenant that wants the fail-safe to hold against a single
    administrator binds `enterprise` at the root, which closes the choice.
  - Once a template is bound at the root, leaving it needs review under that
    pack. Leaving `enterprise` needs four people.
  - Under `standard`, a team with one administrator cannot change
    Configuration until it has two. Under `open-collaboration`, one
    administrator can change it alone. Onboarding must say this before the
    choice.
  - Implementation touches:
    - the console onboarding flow, and the place it offers the choice later;
    - the `/v1/me` onboarding view, with regenerated OpenAPI and console
      client;
    - the gateway rule;
    - the first-profile tests in `crates/synveda-gateway/tests/configuration_api.rs`;
    - `synveda demo`, which requires its workspace profile to auto-apply and
      must instead adopt at the root while the choice is open, or otherwise
      reuse the root's profile;
    - the demo fixture expectations in ADR-0100's 2026-10-04 amendment.
- Reversal triggers:
  - A supported customer tenant bound `enterprise` and cannot assemble a
    four-person review to leave it: revisit an operator-side selection that
    VedaFlow still records.
  - A customer reports one administrator making the first choice against the
    rest of the tenant's wishes: revisit option 7.
  - A request to choose at install time for unattended deployments: revisit
    option 3.

## Compliance notes

- **PDP:** no new action. Adoption needs live `ConfigurationWrite` and
  `ProposalOpen` at the tenant root, and the effect is re-authorised.
- **RLS:** no new table. Every check runs in the tenant-bound transaction
  under the existing root row lock.
- **VedaFlow and audit:** every adoption is a typed, hashed proposal with
  content-free opened and applied events that name the template, pack,
  target and digest.
- **Configuration:** the change selects one of the existing canonical
  documents; it adds no setting and no way to edit a document without review.
- **On acceptance:** ADR-0100 decision 2 and ADR-0075 decision 6 gain
  amendment pointers to this record, and the index records both.
