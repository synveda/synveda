/** Focused failure-state coverage for the governed relaxation panel (CPR-44). */

import assert from "node:assert/strict";
import { test } from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import type { Outcome } from "./api.mjs";
import { RelaxationPanel } from "./Scopes.js";
import { toText } from "./text.mjs";

const SCOPE_ID = "0199bb11-1111-7111-8111-111111111111";

function render(state: Outcome | { kind: "loading" }): string {
  return toText(renderToStaticMarkup(<RelaxationPanel state={state} scopeId={SCOPE_ID} />));
}

test("a failed relaxation read is not rendered as an empty successful list", () => {
  const text = render({ kind: "forbidden", message: "policy denied this read" });

  assert.match(text, /roles do not allow this/i);
  assert.ok(!text.includes("nothing is relaxed here"), text);
  assert.ok(!text.includes("0 relaxation"), text);
});

test("an empty policy-visible relaxation list keeps the honest success copy", () => {
  const text = render({ kind: "ok", body: { relaxations: [], next_cursor: null } });

  assert.ok(text.includes("nothing is relaxed here"), text);
});

test("a truncated visible page never claims that the scope has no relaxation", () => {
  const text = render({ kind: "ok", body: { relaxations: [], next_cursor: "next" } });

  assert.ok(!text.includes("nothing is relaxed here"), text);
  assert.match(text, /first visible page.*more results are available/i);
});
