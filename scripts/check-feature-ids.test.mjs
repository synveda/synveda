import assert from "node:assert/strict";
import test from "node:test";

import { featureIds, findings } from "./check-feature-ids.mjs";

const ids = featureIds(
  "- [x] FND-1: Workspace scaffold — delivered\n- [ ] [OPS-12: Consumer installation](OPS-12.md) — open\n",
);

test("known feature IDs are found with their line numbers", () => {
  assert.deepEqual([...ids].sort(), ["FND-1", "OPS-12"]);
  assert.deepEqual(findings("ok\n// OPS-12: install\r\nFND-1 and OPS-12", ids), [
    { line: 2, id: "OPS-12" },
    { line: 3, id: "FND-1" },
    { line: 3, id: "OPS-12" },
  ]);
});

test("tracker links, ADR numbers and unknown look-alikes are not findings", () => {
  const text = "[record](docs/backlog/OPS-12.md#x) ADR-0155 ticket OPS-1123 AUTH-401";
  assert.deepEqual(findings(text, ids), []);
  assert.deepEqual(findings("[OPS-12](backlog/OPS-12.md)", ids), [{ line: 1, id: "OPS-12" }]);
});
