#!/usr/bin/env sh
# CPR-32 acceptance demo: one typed, revision-aware VedaFlow lifecycle for
# every governed context-platform artifact family.
set -eu

. "$(dirname "$0")/lib/current-platform-demo.sh"
demo_start "cpr32" "CPR-32 — unified governed-artifact reviews"

echo "    Exercise typed references, exact-commit verdicts, separation, cancellation and execution across the common review plane."
cargo test -p synveda-gateway \
  --test knowledge_lifecycle \
  --test skills \
  --test tools \
  --test configuration_api \
  --test relaxations \
  --test okf_api \
  -- --nocapture

typed=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from vedaflow_proposals where jsonb_array_length(artifact_references) > 0")
families=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(distinct reference->>'family') from vedaflow_proposals cross join lateral jsonb_array_elements(artifact_references) reference")
commit_bound_reviews=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from vedaflow_proposal_approvals approval join vedaflow_proposals proposal on proposal.tenant_id = approval.tenant_id and proposal.id = approval.proposal_id where approval.commit_hash = proposal.commit_hash")
separated_effects=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from vedaflow_proposals proposal where proposal.state in ('published', 'applied') and proposal.closed_by <> proposal.proposer_id and not exists (select 1 from vedaflow_proposal_approvals approval where approval.tenant_id = proposal.tenant_id and approval.proposal_id = proposal.id and approval.commit_hash = proposal.commit_hash and approval.approver_id = proposal.closed_by)")
content_leaks=$($DEMO_COMPOSE exec -T postgres psql -At -U synveda -d "$DEMO_DATABASE" -c \
  "select count(*) from audit_log where payload::text like '%Provider event IDs are idempotency keys.%'")

if [ "$typed" -lt 6 ] || [ "$families" -lt 6 ] || \
   [ "$commit_bound_reviews" -lt 1 ] || [ "$separated_effects" -lt 1 ] || \
   [ "$content_leaks" -ne 0 ]; then
  echo "CPR-32 state mismatch: typed=$typed families=$families commit_bound_reviews=$commit_bound_reviews separated_effects=$separated_effects content_leaks=$content_leaks" >&2
  exit 1
fi

echo ""
echo "CPR-32 reviews: $typed typed proposals span $families governed families; $commit_bound_reviews verdicts bind exact commits, a regulated effect has a separate actor, stale verdicts fail in acceptance and audit metadata remains content-free."
demo_finish
