#!/usr/bin/env sh
# FND-6 acceptance demo: ADRs 0001-0004 exist, follow the adr-0000 template,
# and are Accepted. On Windows, run via Git Bash.
set -eu

cd "$(dirname "$0")/.."

check_adr() {
  file="docs/adr/$1"
  echo "==> $file"
  test -f "$file" || { echo "MISSING: $file"; exit 1; }
  for section in "## Context" "## Decision" "## Options considered" \
                 "## Consequences" "## Compliance notes"; do
    grep -q "^$section" "$file" || { echo "MISSING SECTION '$section' in $file"; exit 1; }
  done
  grep -q -- '- \*\*Status\*\*: Accepted' "$file" || { echo "NOT ACCEPTED: $file"; exit 1; }
  grep -q -- '- \*\*Feature(s)\*\*:.*FND-6' "$file" || { echo "MISSING FND-6 REF: $file"; exit 1; }
}

check_adr adr-0001-postgres-first-rust-stack.md
check_adr adr-0002-cedar-embedded-pdp.md
check_adr adr-0003-vedaflow-in-postgres.md
check_adr adr-0004-multi-graph-age-schema.md

echo "==> STATUS.md marks FND-6 done"
grep -q -- '- \[x\] \[FND-6' docs/backlog/STATUS.md || { echo "FND-6 not checked off"; exit 1; }

echo ""
echo "FND-6 ADRs: all checks green."
