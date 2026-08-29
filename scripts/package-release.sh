#!/usr/bin/env bash
# Assembles the transitional release profile bundle (OPS-8, ADR-0065 decision
# 3). CPR-45 retains this artifact for release/replacement evidence while its
# lifecycle is withdrawn; it is not a turnkey reference profile. The release
# workflow and OPS-8 packaging checks consume the artifact directly.
#
# Usage: scripts/package-release.sh <version> <output-dir>
#
# Produces <output-dir>/synveda-profile-<version>.tar.gz containing:
#
#   docker-compose.yml    deploy/release's, with the version substituted in
#   rauthy/config.toml    copied from deploy/compose — one Rauthy config
#                         exists in this repository and this is it
#   version               the tag paired with the binaries (decision 5)
#
# It writes nothing outside <output-dir>.
set -euo pipefail

cd "$(dirname "$0")/.."

version="${1:?usage: package-release.sh <version> <output-dir>}"
outdir="${2:?usage: package-release.sh <version> <output-dir>}"

# The compose file is the artefact this profile *is*, so a placeholder that
# survived substitution, or a build stanza that crept in from the dev file,
# has to fail here rather than on a tester's laptop.
source_compose="deploy/release/docker-compose.yml"
if ! grep -q "__SYNVEDA_VERSION__" "$source_compose"; then
  echo "package-release: $source_compose has no __SYNVEDA_VERSION__ placeholder" >&2
  echo "  (the packager substitutes it; a file with none is pinned to something)" >&2
  exit 1
fi

stage="$outdir/synveda-profile-$version"
rm -rf "$stage"
mkdir -p "$stage/rauthy"

sed "s/__SYNVEDA_VERSION__/$version/g" "$source_compose" > "$stage/docker-compose.yml"
cp deploy/compose/rauthy/config.toml "$stage/rauthy/config.toml"
printf '%s\n' "$version" > "$stage/version"

# Two assertions about what was produced, both cheap and both about the one
# property that makes this a *released* profile: it pulls, it never builds.
if grep -q "__SYNVEDA_VERSION__" "$stage/docker-compose.yml"; then
  echo "package-release: substitution left a placeholder behind" >&2
  exit 1
fi
if grep -qE '^\s*build:' "$stage/docker-compose.yml"; then
  echo "package-release: the packaged compose file has a build: stanza." >&2
  echo "  A released profile pulls published images; a machine running it has" >&2
  echo "  no source tree to build from (ADR-0065 decision 3)." >&2
  exit 1
fi
# CPR-36's hard cut: the retired ACME seeder called deleted runtime routes.
# The release bundle must not retain it as a known-dead executable or grow a
# compatibility shim. The PulseBoard walkthrough is compiled into the same
# public-API CLI binary; it has no release-bundle seeder or data directory.
if find "$stage" -type f -path '*/demo/*' -print -quit | grep -q .; then
  echo "package-release: a retired demo asset entered the runtime profile" >&2
  exit 1
fi

tar -czf "$outdir/synveda-profile-$version.tar.gz" -C "$outdir" "synveda-profile-$version"
echo "packaged $outdir/synveda-profile-$version.tar.gz"
