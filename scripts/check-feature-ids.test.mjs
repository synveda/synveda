import assert from "node:assert/strict";
import test from "node:test";

import { featureIds, findings } from "./check-feature-ids.mjs";

// Made-up IDs that are not in the real tracker, so this file passes the gate.
const ids = featureIds(
  "- [x] ABC-1: Workspace scaffold — delivered\n- [ ] [XYZ-12: Consumer installation](XYZ-12.md) — open\n",
);

test("known feature IDs are found with their line numbers", () => {
  assert.deepEqual([...ids].sort(), ["ABC-1", "XYZ-12"]);
  assert.deepEqual(findings("ok\n// XYZ-12: install\r\nABC-1 and XYZ-12", ids), [
    { line: 2, id: "XYZ-12" },
    { line: 3, id: "ABC-1" },
    { line: 3, id: "XYZ-12" },
  ]);
});

test("tracker links, ADR numbers and unknown look-alikes are not findings", () => {
  const text = "[record](docs/backlog/XYZ-12.md#x) ADR-0155 ticket XYZ-1123 AUTH-401";
  assert.deepEqual(findings(text, ids), []);
  assert.deepEqual(findings("[XYZ-12](backlog/XYZ-12.md)", ids), [{ line: 1, id: "XYZ-12" }]);
});
