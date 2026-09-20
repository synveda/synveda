# ADR-0114: Shared console branding and task-focused navigation

- **Status**: Accepted
- **Date**: 2026-09-20
- **Feature(s)**: CNSL-5, FND-7
- **Deciders**: Synveda owner through the console and README usability request

## Context

The public site uses the approved layered S and navy/cobalt/teal palette. The
console still has a text-only identity, small controls and a long navigation
strip on phones. Home gives access internals more weight than useful next steps,
and its connection link restarts workspace creation for existing projects. The
README buries the first-use journey in implementation terminology.

## Decision

Reuse the canonical SVG lockups, favicon and licensed Inter font in the existing
Vite console build. Serve them from the gateway's existing same-origin asset
boundary, retain system-font fallbacks and respect the device's light/dark
preference. Include the font licence in the bundle and its inputs in the console
Docker stage; add no UI library, external asset request or runtime service.

Keep the ADR-0075 route groups and capability forecasts. Use an accessible
collapsible menu on small screens, retain the parent navigation state on detail
pages, and move keyboard focus to the page after navigation. Home leads with the
selected workspace/project and supported tasks; exact access information remains
available in a disclosure. Connecting an existing project starts at client
selection. First-run creation and blocked access still use the server's
onboarding state, and every operation retains its existing public API check.

Use plain task descriptions in headings, onboarding and the root README. Keep
the verified-client summary generated from the registry and link to full setup
and readiness documentation. Do not turn visual checks into deployment or
usability-research claims.

## Options considered

1. **Restyle the existing components (chosen).** Shares assets and improves the
   current journeys without replacing working forms, routing or data clients.
2. **Install a new component system.** Adds dependencies and a wider migration
   without evidence that the existing components need replacing.
3. **Change only the README.** Leaves the public site and console visibly
   inconsistent and the existing-project connection path confusing.

## Consequences

- Positive: one brand master, clearer entry points and usable small-screen
  navigation, with the existing light/dark preference preserved.
- Accepted trade-off: the font adds a self-hosted asset to the console bundle.
  Browser checks cover presentation and interaction; they do not establish
  task success for a representative group of users.
- Reversal trigger: observed user difficulties require deeper workflow changes
  or the flat route structure no longer accommodates supported tasks.

## Compliance notes

No API, authentication, schema, policy, RLS, VedaFlow or audit changes. No
analytics or new browser storage. Preview fixtures contain synthetic data;
real tenant content must not be published as a design screenshot.
