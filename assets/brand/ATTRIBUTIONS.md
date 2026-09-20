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

The build-only renderer @napi-rs/canvas 1.0.9 and fontkit 2.0.4 use MIT licences
([Canvas](https://github.com/Brooooooklyn/canvas/blob/v1.0.9/LICENSE),
[fontkit](https://github.com/foliojs/fontkit/blob/v2.0.4/README.md#license)). Their
code is not shipped in the static site; resolved npm licences remain checked by
the existing repository licence gate.
