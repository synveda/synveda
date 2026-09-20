# Public site and brand maintenance (FND-7)

The site is semantic HTML/CSS with no browser JavaScript, framework, remote
fonts, analytics or product-runtime dependency. [ADR-0113](../docs/adr/adr-0113-static-public-site-and-brand.md)
records the boundary. Product claims come from the checkout, especially the
[adapter registry](../adapters/registry.json) and [readiness register](../docs/PRODUCTION_READINESS.md).
The approved boards remain in design/reference and never enter the public build.

## Build and preview

From the repository root, use Node 22+ and the pinned pnpm 11.13.1:

```sh
pnpm install --frozen-lockfile --filter @synveda/website
pnpm --filter @synveda/website check
pnpm --filter @synveda/website preview
```

Open [the local project-path preview](http://127.0.0.1:4173/synveda/).
Stop the preview with Ctrl-C. Rebuild after editing; there is no watcher.
`pnpm --filter @synveda/website build` only copies and substitutes metadata;
`check` also tests stale-export refusal and verifies reproducible brand exports, links/anchors, client claims,
image sizes and the public allowlist. No backend or credentials are required.

Only files enumerated in config.mjs plus the generated .nojekyll marker enter
the ignored website/dist output. Keep this allowlist narrow. Do not copy the
repository, reference boards, task text, local runtime or font master into it.

## Brand assets

[synveda-mark.svg](../assets/brand/synveda-mark.svg) is the only geometry master.
Edit its paths, then regenerate the colour/mono variants, outlined wordmark and
lockups, avatars, favicons and social preview:

```sh
pnpm --filter @synveda/website brand
pnpm --filter @synveda/website check
```

The build-only fontkit library outlines the Inter 4.1 TTF. Canvg/xmldom parse
the SVGs for pinned CanvasKit WebAssembly, which rasterises and encodes PNGs
with the same binary on every host. No system fonts or GPU are used. Small
PNGs render at their export dimensions. The wordmark uses weight 650, optical size 32, cv11
single-storey a and tightened spacing. Dark variants lighten only the central
navy plane for visibility. There is no raster embedded in any SVG.

Review the mark at 16, 32, 64 and 256 px, on light/dark backgrounds and in
monochrome; also check square/circular avatar crops. Keep safe avatar padding.
The design boards are the visual authority, not production exports.

The console imports these same lockups, favicon and WOFF2 in its Vite build
([ADR-0114](../docs/adr/adr-0114-console-brand-and-navigation.md)). After changing
shared assets, also run `pnpm --filter @synveda/console test` and
`pnpm --filter @synveda/console build`, then inspect its light/dark views.

- [512 px avatar](../assets/brand/synveda-avatar-512.png) and
  [256 px avatar](../assets/brand/synveda-avatar-256.png): symbol only on navy,
  suitable for account/organisation upload and circular cropping.
- [Social preview](../assets/brand/social-preview.png): opaque 1280 × 640 PNG,
  separate from the avatar, under GitHub's 1 MB limit.
- [README light lockup](../assets/brand/synveda-lockup.svg) and
  [dark lockup](../assets/brand/synveda-lockup-dark.svg): outlined type, no
  client-side font requirement. Root README uses a theme-aware picture element.
- [Font and asset attributions](../assets/brand/ATTRIBUTIONS.md): exact sources,
  licence notices and modifications. The unmodified WOFF2 is self-hosted;
  its OFL notice is included in the public artifact. No external photos/icons.

## Pages deployment and owner steps

The public `synveda/synveda` repository uses `main` and has no checked-in CNAME.
After the initial implementation, the owner enabled Pages using GitHub Actions;
read-only verification on 2026-09-20 confirmed that configuration and the URL
[synveda.github.io/synveda](https://synveda.github.io/synveda/).
Check the latest Pages workflow's deployment result before claiming a live site.

The [Pages workflow](../.github/workflows/pages.yml) checks every PR and main
push, covering site sources, brand assets, lockfiles, documentation and the
workflow itself without path-filtered required checks. The full check runs on
native Linux AMD64 and ARM64; deployment waits for both, with only AMD64 uploading
the one public artifact. PRs receive no deploy permissions. Only main in the canonical repository can upload the explicit
public output and deploy through the github-pages environment. Manual dispatch
from another branch also cannot deploy. Existing CI is unchanged.

For a new repository or a configuration change:

1. In **synveda/synveda → Settings → Pages → Build and deployment → Source**,
   choose **GitHub Actions**. The workflow deliberately does not enable Pages
   via API or add a custom domain.
2. Review the **github-pages** environment's deployment protection to permit
   only `main`, and ensure repository/organisation Actions policy allows the
   official actions plus pnpm/action-setup.
3. Run **Actions → Pages → Run workflow**, selecting `main`, or use a later
   main push. Inspect the build and deployment jobs, then visit the returned
   deployment URL and check the project-path assets. A successful local build
   does not prove a live deployment.
4. Optionally set the repository's About website to the verified live URL.
   If adopting an approved custom domain later, configure Pages/DNS first;
   configure-pages supplies its base_url to the build. Set SITE_URL to that
   same absolute HTTPS URL for local production checks. Never invent a CNAME.

Official references checked 2026-09-20:
[custom Pages workflows](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages),
[publishing source](https://docs.github.com/en/pages/getting-started-with-github-pages/configuring-a-publishing-source-for-your-github-pages-site),
[setup-node](https://github.com/actions/setup-node) and
[checkout](https://github.com/actions/checkout). The new workflow uses official
checkout/setup-node v7, configure-pages v5, upload-pages-artifact v4 and
deploy-pages v4. It uses the standard GITHUB_TOKEN/OIDC boundary, no PAT.

## Manual GitHub images

These instructions do not authorise or perform identity changes:

- Repository **Settings → General → Social preview → Edit → Upload an image**:
  select social-preview.png. GitHub recommends 1280 × 640 and a file under
  1 MB; see [repository social previews](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/customizing-your-repositorys-social-media-preview).
- For the **Synveda organisation**, an owner can open **Your organisations →
  Synveda → Settings → profile picture → Upload new picture** and select the
  512 px avatar. See [organisation profile settings](https://docs.github.com/en/organizations/collaborating-with-groups-in-organizations/customizing-your-organizations-profile).
  This changes the organisation identity. A repository has a social-preview
  image; there is no separate repository-avatar setting.

## Validation

Before publishing, run the focused site check and repository documentation
gates, then inspect the production build at its real project base path:

```sh
pnpm --filter @synveda/website check
make check-docs check-backlog check-adr-status check-adapters check-deps
```

Check desktop (1440 px) and mobile (390 px), keyboard focus/skip link and the
native details menu, CTAs/fragment links, console/network errors, README themes,
SVG small sizes/crops and the exact public output. Use an accessibility audit
when available. Recheck claims against current repository evidence before
changing copy. Application/demo validation is separate: use the current
[Compose instructions](../deploy/compose/README.md), preserve existing project
selectors and data, and report missing services rather than claiming a pass.

### Initial implementation validation — 2026-09-20

- Production /synveda/ build and checks passed on macOS arm64 Node 24.18.0
  and pinned Docker Linux arm64 Node 22.23.2, using the frozen pnpm lockfile.
  All 11 derived assets regenerated byte-for-byte on both platforms. The final
  public artifact contains 12 files, approximately 480 kB, with no reference
  boards, private state, browser scripts or font TTF master.
- Headless Brave through the existing Playwright dependency passed image,
  overflow, fragment navigation, skip-link and menu keyboard checks at 1440,
  768, 390 and 320 px. Desktop/mobile screenshots were visually compared with
  the boards. axe-core 4.11.0 reported zero WCAG 2 A/AA and 2.1 AA violations;
  its incomplete contrast items were decorative arrows/separators, reviewed
  manually. This is a focused audit, not a certification or Lighthouse score.
- The canonical mark was inspected at 16/32/64/256 px, in mono and on
  light/dark backgrounds; square/circle crops, social PNG and locally rendered
  README theme selection were checked. GitHub's hosted rendering was not
  changed or observed for this uncommitted implementation.
- actionlint 1.7.11, Prettier 3.6.2, SVG XML validation, git diff whitespace,
  cargo formatting, documentation/backlog/ADR checks, adapter conformance,
  crate dependency direction and the unchanged npm licence gate passed.
- The already-running synveda-development-acceptance-interop demo passed the
  canonical smoke command with its existing demo profile and 10.231.46.0/24
  pool. Hostnames, service/job state, public health/console/OIDC and private
  endpoint refusal passed. No fresh install, browser authentication, reset,
  full backend CI or production deployment was performed. The existing safe
  seeded-console image remains linked in the root README; the site uses
  architecture visuals rather than publishing live demo Session content.

This initial validation did not modify Pages/account settings or upload images.
Its two local platforms both used ARM64; it did not establish AMD64 asset
reproducibility or live GitHub deployment.

### Portable export correction — 2026-09-20

The first AMD64 Pages check failed because native Skia produced different edge
pixels from the same SVG. A pinned WebAssembly renderer now replaces the native
addon. Every SVG, the 512 px avatar and social preview are unchanged; the smaller
PNGs are regenerated directly at their final dimensions. Exact byte checks remain
mandatory on both CI architectures. Regression tests verify that checking rejects
a modified PNG without rewriting it and that a canonical edit reaches every PNG.

Both tests and all 11 exact export checks pass on macOS ARM64 Node 24.18.0,
Linux ARM64 Node 22.23.2 and emulated Linux AMD64 Node 22.23.2. Fresh frozen-lockfile
Linux installations also pass the complete site check and unchanged npm licence
gate. The public allowlist remains 12 files with no WASM or renderer code.
Documentation, workflow lint and formatting pass. The hosted Pages run provides
the separate native AMD64/ARM64 and deployment result.
