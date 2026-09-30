#!/bin/sh
# Packages the existing Helm deployment with the exact release version as
# both chart version and application version. This produces a release asset;
# it does not publish it or claim that its images are pullable.
set -eu

cd "$(dirname "$0")/.."

version="${1:?usage: package-chart.sh <version> <output-dir>}"
outdir="${2:?usage: package-chart.sh <version> <output-dir>}"

# Version validation must precede every derived output path.
sh scripts/release-version.sh "$version"

command -v helm >/dev/null 2>&1 || {
  printf '%s\n' 'package-chart: helm is not on PATH' >&2
  exit 69
}

mkdir -p "$outdir"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT INT TERM
cp -R deploy/helm/synveda "$stage/synveda"
cp LICENSE NOTICE "$stage/synveda/"
helm package "$stage/synveda" \
  --version "$version" \
  --app-version "$version" \
  --destination "$outdir"
