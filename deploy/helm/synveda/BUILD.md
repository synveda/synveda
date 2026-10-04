# Package the chart from a reviewed source checkout

Published releases already contain `synveda-VERSION.tgz`, including the locked
Keycloak chart, customer presets and native preparation tools. Follow the
[installation guide](README.md#3-download-and-verify-a-named-release) to verify
and use those bytes. Helm renders templates; there is no compilation step for
an operator installing a release.

For a source checkout, run this command from the repository root:

```sh
make chart-package
```

Success prints the complete archive path under `target/helm/`. The existing
packager takes its version from `Cargo.toml`, preserves the locked dependency
and includes the repository licence notices. Git, Make and Helm are sufficient;
no Rust build, Docker, npm installation or dependency download runs. If Helm
is missing, install it and repeat the command. If the bundled dependency is
missing, restore the matching checkout; do not update its lock to work around
an incomplete source tree.

To choose another output directory without changing the release version:

```sh
SYNVEDA_CHART_OUTPUT="$PWD/target/chart-review" make chart-package
```

Repeating either command packages identical source into identical archive
bytes, including guides with the archive version and checkout's exact commit.
Links outside the chart point to that source revision; local guides and tools
remain usable after extraction. The original checkout is not edited. A source
export without Git metadata must supply its independently reviewed commit as
`SYNVEDA_BUILD_SOURCE_SHA`; an unknown source refuses packaging.

Packaging does not create an image overlay or make product images
available. Source acceptance requires images built from the same reviewed
runtime/console source and an explicitly labelled candidate overlay. A version
equal to an older release does not give source-built bytes its publisher proof.
The [source development guide](../../../docs/DEVELOPMENT.md) and
[release process](../../../docs/RELEASING.md) own image builds and publication.
