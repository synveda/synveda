// Offline owned-fixture oracle. Never an input to production retrieval.
export function diagnoseContextFixture(input) {
  if (input?.scope !== "owned_fixture_oracle" || !/^[0-9a-f]{64}$/.test(input.frozen_delivery_hash ?? "")) {
    throw new Error("Diagnosis requires an already frozen owned-fixture delivery");
  }
  const evidence = input.evidence ?? {};
  let stage = "unknown";
  let explanation = "The retained observations do not establish a failure stage.";
  if (evidence.source_published === false) {
    stage = "representation";
    explanation = "The owned source was deliberately left unpublished; the oracle establishes representation absence.";
  } else if (evidence.exact_eligible === true && evidence.query_retrieved === false) {
    stage = "retrieval";
    explanation = "The authorized exact-ID lens found the frozen revision; ordinary query retrieval omitted it.";
  } else if (evidence.query_retrieved === true && evidence.qualifier_preserved === false) {
    stage = "assembly";
    explanation = "The selected proposition lost its required qualifier in the explicitly supplied diagnostic output.";
  } else if (evidence.delivered_exact === true && evidence.evidence_class === "authenticated_runner" && evidence.task_outcome === "failed") {
    stage = "delivery_use";
    explanation = "The exact context was delivered and an authenticated controlled runner reported failure; causal attribution remains unknown.";
  }
  return {
    schema_version: 1,
    scope: "owned_fixture_oracle",
    frozen_delivery_hash: input.frozen_delivery_hash,
    stage,
    explanation,
    causal_attribution: "unknown",
    fault_injected_output: input.fault_injected_output === true,
    observed_retrieval_limits: input.observed_retrieval_limits ?? [],
    production_ranking_input: false,
  };
}
