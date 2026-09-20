# ADR-0113: A static public site and one canonical brand geometry

- **Status**: Accepted; amended once
- **Date**: 2026-09-20
- **Feature(s)**: FND-7
- **Deciders**: Synveda owner through the approved design implementation brief

## Context

The approved boards in design/reference define the layered S, lowercase
wordmark and restrained navy/cobalt/teal visual language. The repository has
no public site, Pages configuration or vector brand master. The public API,
adapter registry and readiness register govern claims; concept-board content
does not. GitHub's read-only repository API reports public synveda/synveda,
default branch main and no Pages site on 2026-09-20.

## Decision

Build semantic HTML/CSS in website with a small Node build and no browser
JavaScript or product-runtime dependency. Use the existing pnpm workspace and
lockfile for build-only SVG rendering/font outlining. Self-host licensed Inter.
One editable SVG mark supplies every variant and raster export; outline the
wordmark so GitHub rendering needs no installed font.

Copy only an explicit public-file allowlist into website/dist. Derive canonical
metadata from the checked project URL, or GitHub's configure-pages base_url
when deploying. Relative assets must work at /synveda/. Keep the approved
boards, task inputs, tooling and private demo state outside the public output.

A separate Pages workflow checks every pull request without deployment
credentials. Only main in the canonical repository may configure, upload and
deploy through the official Pages actions and github-pages environment.
Existing CI jobs and required checks remain unchanged. Enabling Pages, changing
organisation identity and uploading the repository social image are manual
owner actions, not part of implementation authority.

## Options considered

1. **Static HTML/CSS and a bounded copy build (chosen).** Matches this single
   page and adds no application framework, analytics or runtime service.
2. **Reuse the React console bundle.** Rejected: it couples a public site to
   authenticated product code and its build without a user need.
3. **A documentation platform or hosted site service.** Rejected: maintained
   repository documentation already owns the deeper content; Pages is requested.

## Consequences

- Positive: small auditable public output, reproducible brand exports and
  shared assets across README, website and GitHub uploads.
- Accepted trade-off: homepage prose is curated; checks pin its integration
  labels to the registry and validate repository links, assets and metadata.
- Reversal trigger: substantive multi-page documentation needs navigation or
  search that cannot be maintained in the current small build.

## Compliance notes

No product API, authentication, Cedar, RLS, VedaFlow, audit or database change.
The site contains public product descriptions, not tenant data. No tracking,
cookies, third-party photography, customer claims or production-readiness
promotion. Website generation needs no product credential or running backend.

## Amendment: portable raster generation (2026-09-20)

The first AMD64 Pages build reproduced a native Skia rasterisation difference:
the same SVG inputs generated different PNG edge pixels on AMD64 and ARM64.
Earlier macOS/Linux checks both used ARM64 and did not cover that boundary.

Use one pinned CanvasKit WebAssembly renderer, with Canvg parsing the generated
SVGs and xmldom providing its build-only DOM. These dependencies use the existing
permissive build-tool licence policy. Fontkit still outlines the same licensed
font, and the canonical SVG geometry remains unchanged. No renderer or WASM is
copied to the public site or console.

Retain exact byte comparison for every derived export. Verify the complete site
on both native Linux runner architectures before deployment; do not accept
pixel tolerances or skip the stale-asset gate on a different host architecture.
