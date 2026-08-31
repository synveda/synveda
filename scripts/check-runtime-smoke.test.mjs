import assert from "node:assert/strict";
import test from "node:test";

import {
  boundedResponseBody,
  parseComposePs,
  runtimeStateFindings,
} from "../deploy/compose/scripts/check-runtime-smoke.mjs";

const oneShots = [
  "database-bootstrap",
  "database-preflight",
  "issuer-diagnostic",
  "keycloak-database-bootstrap",
  "migrate",
  "tenant-convergence",
];
const longRunning = [
  "gateway",
  "keycloak",
  "keycloak-realm-convergence",
  "otel-collector",
  "postgres",
  "proxy",
  "worker",
];

function healthyRows() {
  return [
    ...oneShots.map((Service) => ({ Service, State: "exited", ExitCode: 0, Health: "" })),
    ...longRunning.map((Service) => ({ Service, State: "running", ExitCode: 0, Health: "healthy" })),
  ];
}

test("Compose ps accepts array and newline-delimited JSON", () => {
  const rows = healthyRows();
  assert.deepEqual(parseComposePs(JSON.stringify(rows)), rows);
  assert.deepEqual(parseComposePs(rows.map((row) => JSON.stringify(row)).join("\n")), rows);
});

test("bundled topology requires every convergence and healthy process", () => {
  const selection = { postgres: "bundled", oidc: "bundled" };
  assert.deepEqual(runtimeStateFindings(healthyRows(), selection), []);

  const failed = healthyRows().map((row) =>
    row.Service === "tenant-convergence" ? { ...row, ExitCode: 1 } : row,
  );
  assert.ok(
    runtimeStateFindings(failed, selection).includes(
      "tenant-convergence convergence did not complete successfully",
    ),
  );
  const degraded = healthyRows().map((row) =>
    row.Service === "worker" ? { ...row, Health: "unhealthy" } : row,
  );
  assert.ok(
    runtimeStateFindings(degraded, selection).includes(
      "worker is not running and healthy",
    ),
  );
});

test("external provider rows reject bundled provider residue", () => {
  const external = healthyRows().filter(
    ({ Service }) =>
      ![
        "database-bootstrap",
        "keycloak-database-bootstrap",
        "keycloak",
        "keycloak-realm-convergence",
        "postgres",
      ].includes(Service),
  );
  assert.deepEqual(
    runtimeStateFindings(external, { postgres: "external", oidc: "external" }),
    [],
  );
  assert.ok(
    runtimeStateFindings(healthyRows(), { postgres: "external", oidc: "external" }).includes(
      "Compose service status set differs from the selected topology",
    ),
  );
});

test("public response bodies are bounded before parsing", async () => {
  assert.equal(
    (await boundedResponseBody(new Response("safe"), 4)).toString("utf8"),
    "safe",
  );
  await assert.rejects(
    boundedResponseBody(new Response("oversized"), 4),
    /bounded response was refused/,
  );
  const streamed = new Response(
    new ReadableStream({
      start(controller) {
        controller.enqueue(new TextEncoder().encode("abc"));
        controller.enqueue(new TextEncoder().encode("def"));
        controller.close();
      },
    }),
  );
  await assert.rejects(
    boundedResponseBody(streamed, 5),
    /bounded response was refused/,
  );
});
