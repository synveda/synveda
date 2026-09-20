#!/usr/bin/env sh
# FND-6 acceptance: retained foundation decisions and the current graph
# replacement follow the ADR template and are Accepted.
set -eu

cd "$(dirname "$0")/.."

check_adr() {
  file="docs/adr/$1"
  feature=$2
  echo "==> $file"
  test -f "$file" || { echo "MISSING: $file"; exit 1; }
  for section in "## Context" "## Decision" "## Options considered" \
                 "## Consequences" "## Compliance notes"; do
    grep -q "^$section" "$file" || { echo "MISSING SECTION '$section' in $file"; exit 1; }
  done
  grep -q -- '- \*\*Status\*\*: Accepted' "$file" || { echo "NOT ACCEPTED: $file"; exit 1; }
  grep -q -- "- \\*\\*Feature(s)\\*\\*:.*$feature" "$file" || { echo "MISSING $feature REF: $file"; exit 1; }
}

check_adr adr-0001-postgres-first-rust-stack.md FND-6
check_adr adr-0002-cedar-embedded-pdp.md FND-6
check_adr adr-0003-vedaflow-in-postgres.md FND-6
check_adr adr-0097-bounded-knowledge-graph-retrieval.md CPR-38

echo "==> STATUS.md marks FND-6 done"
grep -q -- '- \[x\] FND-6:' docs/backlog/STATUS.md || { echo "FND-6 not checked off"; exit 1; }

echo ""
echo "FND-6 ADRs: all checks green."
