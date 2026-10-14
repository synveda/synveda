import { test } from "node:test";
import { strict as assert } from "node:assert";
import { diagnoseContextFixture } from "./health-diagnosis.mjs";
const frozen = { scope: "owned_fixture_oracle", frozen_delivery_hash: "a".repeat(64) };
test("the four failure stages remain distinct and cannot claim causal credit", () => {
  const cases = [
    [{ source_published: false }, "representation"],
    [{ source_published: true, exact_eligible: true, query_retrieved: false }, "retrieval"],
    [{ query_retrieved: true, qualifier_preserved: false }, "assembly"],
    [{ delivered_exact: true, evidence_class: "authenticated_runner", task_outcome: "failed" }, "delivery_use"],
    [{ delivered_exact: true, evidence_class: "client_reported", task_outcome: "failed" }, "unknown"],
  ];
  for (const [evidence, stage] of cases) {
    const result = diagnoseContextFixture({ ...frozen, evidence });
    assert.equal(result.stage, stage);
    assert.equal(result.causal_attribution, "unknown");
    assert.equal(result.production_ranking_input, false);
  }
});
test("an unfrozen snapshot or production claim cannot use the oracle", () => {
  assert.throws(() => diagnoseContextFixture({ scope: "production", frozen_delivery_hash: frozen.frozen_delivery_hash }));
  assert.throws(() => diagnoseContextFixture({ ...frozen, frozen_delivery_hash: "unfrozen" }));
});
