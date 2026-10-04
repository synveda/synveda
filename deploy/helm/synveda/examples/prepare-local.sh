#!/bin/sh
# OPS-11: native preparation contacts no cluster and applies no resources.
set -eu
umask 077
chart=$(CDPATH= cd "$(dirname "$0")/.." && pwd -P)
output=${1:?usage: prepare-local.sh /absolute/private/output-directory [prepare options]}
shift
exec node "$chart/examples/prepare.mjs" --output "$output" --route local "$@"
