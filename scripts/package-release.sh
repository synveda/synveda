#!/usr/bin/env bash
# Assembles the release profile bundle (OPS-8, ADR-0065 decision 3) — the
# self-contained directory `synveda init` runs from on a machine with no
# checkout. Run by .github/workflows/release.yml, and by the OPS-8 demo,
# which installs from what this produces rather than from the tree.
#
# Usage: scripts/package-release.sh <version> <output-dir>
#
# Produces <output-dir>/synveda-profile-<version>.tar.gz containing:
#
#   docker-compose.yml    deploy/release's, with the version substituted in
#   rauthy/config.toml    copied from deploy/compose — one Rauthy config
#                         exists in this repository and this is it
#   version               the tag, which `synveda init` compares against its
#                         own before it starts anything (decision 5)
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

# The demo seeder (OPS-9, ADR-0066 decision 1). It rides in this bundle
# rather than as an asset of its own because the bundle already carries a
# `version` that `synveda init` compares against the CLI's before it starts
# anything (ADR-0065 decision 5) — so a seeder that has drifted from the
# product it seeds is caught by machinery that already exists.
mkdir -p "$stage/demo"
cp deploy/release/demo/seed.sh "$stage/demo/seed.sh"
cp deploy/release/demo/organisation.txt "$stage/demo/organisation.txt"
chmod +x "$stage/demo/seed.sh"

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
# The seeder is the one thing in this bundle a person executes directly, so a
# bundle that shipped it non-executable would fail in their hands rather than
# here — the OPS-8 pattern of asserting the artefact rather than its presence.
if [ ! -x "$stage/demo/seed.sh" ]; then
  echo "package-release: demo/seed.sh is not executable in the bundle" >&2
  exit 1
fi
if ! sh -n "$stage/demo/seed.sh"; then
  echo "package-release: demo/seed.sh is not valid POSIX shell" >&2
  exit 1
fi

tar -czf "$outdir/synveda-profile-$version.tar.gz" -C "$outdir" "synveda-profile-$version"
echo "packaged $outdir/synveda-profile-$version.tar.gz"
