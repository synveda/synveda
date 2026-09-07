#!/bin/sh
# Validates the one plain release version used in archive names, Helm
# metadata, Kubernetes labels and OCI tags. Build metadata is deliberately
# excluded because `+` is not an OCI tag character; callers that accept a Git
# tag strip one optional leading `v` before invoking this command.
set -eu

fail() {
  printf '%s\n' \
    'release-version: expected label-safe MAJOR.MINOR.PATCH with an optional SemVer prerelease (maximum 63 ASCII characters)' >&2
  exit 64
}

[ "$#" -eq 1 ] || fail
version=$1

[ -n "$version" ] && [ "${#version}" -le 63 ] || fail
case "$version" in
  *[!0-9A-Za-z.-]* | *[!0-9A-Za-z]) fail ;;
esac

# Core numbers are capped at 19 digits, which keeps every accepted value below
# the unsigned range Helm parses. A prerelease identifier is either a canonical
# integer or contains at least one ASCII letter/hyphen. This excludes numeric
# leading zeroes while retaining the SemVer identifier alphabet.
LC_ALL=C printf '%s\n' "$version" | LC_ALL=C grep -Eq \
  '^(0|[1-9][0-9]{0,18})\.(0|[1-9][0-9]{0,18})\.(0|[1-9][0-9]{0,18})(-(0|[1-9][0-9]*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*)(\.(0|[1-9][0-9]*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*))*)?$' \
  || fail
