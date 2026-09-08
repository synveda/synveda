#!/usr/bin/env sh
# CPR-45: fresh browser login, bounded service restarts and a second login on
# one exact canonical Compose project. The successful stack remains available
# for inspection and the separately confirmed backup/reset gates.
set -eu

exec "$(dirname "$0")/../deploy/compose/scripts/compose.sh" acceptance
