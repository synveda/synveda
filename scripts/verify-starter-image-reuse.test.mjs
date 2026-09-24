import assert from "node:assert/strict";
import test from "node:test";
import { verifyStarterImageReuse } from "./verify-starter-image-reuse.mjs";

const tags = ["synveda/product:ops11", "synveda/postgres:ops11", "synveda/keycloak:ops11"];
const report = () => ({
  cases: [{ selection: "external-external" }],
  images: tags.map((tag, index) => ({ tag, localId: `sha256:${String(index + 1).repeat(64)}` })),
});

test("external acceptance reuses only the starter's exact source-built image IDs", () => {
  const evidence = report();
  assert.deepEqual(verifyStarterImageReuse(evidence,
    (tag) => evidence.images.find((entry) => entry.tag === tag).localId), tags);
  assert.throws(() => verifyStarterImageReuse(evidence, () => `sha256:${"f".repeat(64)}`),
    /starter image changed/);
  assert.throws(() => verifyStarterImageReuse({ ...evidence, cases: [] }, () => ""),
    /starter evidence is missing/);
  assert.throws(() => verifyStarterImageReuse({ ...evidence, images: evidence.images.slice(1) }, () => ""),
    /starter image evidence is invalid/);
  assert.throws(() => verifyStarterImageReuse({ ...evidence,
    images: [...evidence.images, evidence.images[0]] }, () => ""),
    /starter image evidence is invalid/);
});
