#!/usr/bin/env sh
# Retained foundation decisions and the current graph
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
  grep -q -- "- \\*\\*Feature(s)\\*\\*:.*$feature" "$file" || { echo "MISSING '$feature' REF: $file"; exit 1; }
}

check_adr adr-0001-postgres-first-rust-stack.md 'Foundational architecture decisions'
check_adr adr-0002-cedar-embedded-pdp.md 'Foundational architecture decisions'
check_adr adr-0003-vedaflow-in-postgres.md 'Foundational architecture decisions'
check_adr adr-0097-bounded-knowledge-graph-retrieval.md 'Bounded graph-augmented retrieval'

echo "==> STATUS.md marks the foundational decisions delivered"
grep -q -- ': Foundational architecture decisions — delivered' docs/backlog/STATUS.md || { echo "foundational decisions not checked off"; exit 1; }

echo ""
echo "ADRs: all checks green."
