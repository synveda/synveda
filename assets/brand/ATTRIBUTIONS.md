# Brand asset sources

Retrieved and licence-checked 2026-09-20.

| Local file                                                                            | Creator and exact source                                                                                                                           | Licence                                                                                                 | Modifications / attribution                                                                                                                                       |
| ------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| fonts/InterVariable.woff2                                                             | Rasmus Andersson / Inter Project Authors; [Inter 4.1 WOFF2](https://raw.githubusercontent.com/rsms/inter/v4.1/docs/font-files/InterVariable.woff2) | [SIL OFL-1.1](https://raw.githubusercontent.com/rsms/inter/v4.1/LICENSE.txt), included in fonts/OFL.txt | Unmodified, self-hosted. Copyright (c) 2016 The Inter Project Authors. Retain the notice and licence with redistribution.                                         |
| fonts/InterVariable.ttf                                                               | Rasmus Andersson / Inter Project Authors; [Inter 4.1 TTF](https://raw.githubusercontent.com/rsms/inter/v4.1/docs/font-files/InterVariable.ttf)     | Same OFL-1.1                                                                                            | Unmodified build input only; not copied to the website.                                                                                                           |
| synveda-wordmark.svg, synveda-lockup.svg, synveda-lockup-dark.svg, social-preview.png | Synveda, typeset using the above Inter master                                                                                                      | First-party artwork under the repository Apache-2.0 licence; font software retains OFL-1.1              | Outlined glyphs, weight 650 and optical size 32 for the wordmark, single-storey a (cv11), adjusted spacing. Social text also outlined. No client font dependency. |

The layered S, its colour/mono variants, avatars, favicons and website/console line
icons are first-party vector work reconstructed from the owner-supplied
approved design. The boards remain design inputs; neither board, its generated
photography, customer marks nor fictional claims is a public asset. No external
photography or third-party icon set is used. No trademark-clearance claim is made.

The console bundles the same light/dark lockups, favicon and WOFF2 directly
from these sources. Its Vite build includes the font licence as
assets/Inter-OFL.txt; no external font or image request is required.

Build-only tools are pinned in the workspace lockfile:

- CanvasKit 0.42.0 uses [BSD-3-Clause](https://skia.googlesource.com/skia/+/main/LICENSE)
  and supplies the same WebAssembly rasteriser/PNG encoder on every architecture.
- Canvg 4.0.3 uses [MIT](https://github.com/canvg/canvg/blob/v4.0.3/LICENSE);
  xmldom 0.9.12 uses [MIT](https://github.com/xmldom/xmldom/blob/0.9.12/LICENSE).
  They parse the generated SVGs for CanvasKit.
- Fontkit 2.0.4 uses [MIT](https://github.com/foliojs/fontkit/blob/v2.0.4/README.md#license)
  and outlines the licensed Inter font.

Their code and WASM are not shipped in the static site or console; resolved npm
licences remain checked by the existing repository licence gate.
