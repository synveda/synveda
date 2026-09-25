# Public site maintenance

The site is one static HTML/CSS page. It has no browser JavaScript, remote
fonts, analytics or connection to the product runtime. Product and installation
claims must agree with the [release manifest](../docs/installation.json),
[client registry](../adapters/registry.json) and
[readiness assessment](../docs/PRODUCTION_READINESS.md).

## Build and preview

From the repository root, use Node 22+ and the pinned pnpm version:

```sh
pnpm install --frozen-lockfile --filter @synveda/website
pnpm --filter @synveda/website check
pnpm --filter @synveda/website preview
```

Open [the local preview](http://127.0.0.1:4173/synveda/). Stop it with Ctrl-C.
Rebuild after editing; there is no watcher. `build` copies the explicit public
files and substitutes the site URL and release fields. `check` also verifies
links, anchors, client claims, image sizes and exact brand exports. It needs no
backend or credentials.

Only the files listed in [config.mjs](config.mjs), plus `.nojekyll`, enter the
ignored `website/dist` directory. Keep private state, reference boards and
source-only assets outside that list.

## Brand assets

[synveda-mark.svg](../assets/brand/synveda-mark.svg) is the editable geometry
master. To regenerate its SVG variants, outlined wordmark, avatars, favicons
and social preview:

```sh
pnpm --filter @synveda/website brand
pnpm --filter @synveda/website check
```

The build uses licensed Inter and a pinned WebAssembly renderer so raster
exports are byte-stable across supported build hosts. Review the mark at 16,
32, 64 and 256 px, in light and dark views, and in square and circular crops.
The console uses the same brand assets; after changing them, run its tests and
build and inspect both themes:

```sh
pnpm --filter @synveda/console test
pnpm --filter @synveda/console build
```

See [asset attributions](../assets/brand/ATTRIBUTIONS.md) for licence notices.
The repository social preview is
[social-preview.png](../assets/brand/social-preview.png); the organisation
avatar is [synveda-avatar-512.png](../assets/brand/synveda-avatar-512.png).

## Publish and verify

The Pages workflow checks pull requests and main on native Linux AMD64 and
ARM64. Only main in the canonical repository deploys through the
`github-pages` environment. The project path is
[synveda.github.io/synveda/](https://synveda.github.io/synveda/).

Before publishing, run the site and documentation checks:

```sh
pnpm --filter @synveda/website check
make check-docs check-backlog check-adr-status check-adapters check-deps
```

Inspect the built page at 1440 px and 390 px. Check images, horizontal
overflow, keyboard focus, skip link, mobile menu, section links and all calls
to action. The fictional product screenshot must remain clearly labelled as
sample data. A passing local build does not prove that Pages deployed; check
the workflow and live URL after publication.

For a new repository, select **Settings → Pages → Build and deployment →
GitHub Actions** and allow deployment from main in the `github-pages`
environment. The workflow uses the official Pages actions and the repository
token; it requires no personal access token. Repository social image and
organisation avatar uploads are separate owner actions.
