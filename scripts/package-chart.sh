#!/bin/sh
# Packages the existing Helm deployment with the exact release version as
# both chart version and application version. This produces a release asset;
# it does not publish it or claim that its images are pullable.
set -eu

cd "$(dirname "$0")/.."

if [ "$#" -gt 2 ]; then
  printf '%s\n' 'usage: package-chart.sh [version] [output-dir]' >&2
  exit 64
fi
version="${1:-$(awk -F '"' '/^\[workspace.package\]$/ { selected=1; next } selected && /^version = / { print $2; exit }' Cargo.toml)}"
outdir="${2:-${SYNVEDA_CHART_OUTPUT:-target/helm}}"

# Version validation must precede every derived output path.
sh scripts/release-version.sh "$version"

command -v helm >/dev/null 2>&1 || {
  printf '%s\n' 'package-chart: install Helm, then retry make chart-package' >&2
  exit 69
}

# OPS-12: the locked dependency travels with source and release archives.
# Packaging must not fetch a different chart or update its lock on a retry.
test -f deploy/helm/synveda/charts/keycloakx-7.3.2.tgz || {
  printf '%s\n' 'package-chart: bundled Keycloak chart is missing; restore the matching source checkout or verified chart archive, then retry. No dependency update is required.' >&2
  exit 66
}
source_sha="${SYNVEDA_BUILD_SOURCE_SHA:-}"
if [ -z "$source_sha" ]; then
  source_sha=$(git rev-parse --verify HEAD 2>/dev/null) || {
    printf '%s\n' 'package-chart: select an exact reviewed source commit before packaging' >&2
    exit 64
  }
fi
case "$source_sha" in ''|*[!a-f0-9]*)
  printf '%s\n' 'package-chart: select an exact reviewed source commit before packaging' >&2
  exit 64;;
esac
test "${#source_sha}" -eq 40 || {
  printf '%s\n' 'package-chart: select an exact reviewed source commit before packaging' >&2
  exit 64
}
case "$outdir" in /*) ;; *) outdir="$PWD/$outdir" ;; esac
mkdir -p "$outdir"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT INT TERM
cp -R deploy/helm/synveda "$stage/synveda"
cp LICENSE NOTICE "$stage/synveda/"
for guide in "$stage/synveda/"*.md; do
  main=0
  test "$(basename "$guide")" != README.md || main=1
  awk -v main="$main" -v version="$version" -v source="$source_sha" \
    -f scripts/package-chart-guide.awk "$guide" > "$stage/guide.md"
  mv "$stage/guide.md" "$guide"
done
awk -v examples=1 -v version="$version" -v source="$source_sha" \
  -f scripts/package-chart-guide.awk "$stage/synveda/examples/README.md" > "$stage/guide.md"
mv "$stage/guide.md" "$stage/synveda/examples/README.md"
# OPS-12: Helm preserves file mtimes. The private copies need a fixed epoch
# so packaging the same source does not depend on the copy's wall-clock second.
find "$stage/synveda" -type f -exec env TZ=UTC touch -t 197001010000.00 {} +
helm package "$stage/synveda" \
  --version "$version" \
  --app-version "$version" \
  --destination "$outdir"
printf '%s\n' "Source chart: $outdir/synveda-$version.tgz" \
  'Includes the locked dependency, customer presets and preparation tools. No Rust build, Docker or dependency download is required.' \
  'Use matching reviewed images; packaging alone does not establish publication or live acceptance.'
