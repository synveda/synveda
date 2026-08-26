import assert from "node:assert/strict";
import test from "node:test";

import {
  hasRetiredDemoField,
  releaseNoteFindings,
  retiredFindings,
  serviceBlock,
} from "./check-deploy-convergence.mjs";

test("extracts only the requested compose service", () => {
  const compose = `
services:
  postgres:
    image: postgres
  gateway:
    image: synveda/gateway
    environment:
      DATABASE_URL: postgres://synveda_gateway:redacted@postgres/synveda
  jaeger:
    image: jaeger
volumes:
  data:
`;
  const gateway = serviceBlock(compose, "gateway");
  assert.match(gateway, /synveda_gateway/);
  assert.doesNotMatch(gateway, /jaeger/);
});

test("retired routes in executable text fail while comments do not", () => {
  assert.deepEqual(retiredFindings("# /v1/observe was removed\npath: /v1/sessions\n"), []);
  assert.deepEqual(retiredFindings("path: /v1/observe\n"), ["/v1/observe"]);
});

test("retired release demo assets are recognised", () => {
  assert.deepEqual(retiredFindings("copy: demo/seed.sh\n"), ["demo/seed.sh"]);
});

test("the removed init demo field is recognised without matching a negative test", () => {
  assert.equal(hasRetiredDemoField("  demo: bool,\n"), true);
  assert.equal(hasRetiredDemoField('assert!(error.contains("--demo"));\n'), false);
});

test("release notes advertise only the current init and demo commands", () => {
  const notes = (commands) => `cat > notes.md <<NOTES
Install:

\`\`\`sh
${commands}
\`\`\`
NOTES
`;
  const current = notes(`
synveda init --slug pulseboard --name PulseBoard --embedder tei
synveda login
synveda demo start --profile personal
`);
  assert.deepEqual(releaseNoteFindings(current), []);
  assert.deepEqual(releaseNoteFindings(notes("synveda init --demo")), [
    "retired synveda init --demo command",
    "synveda init --slug pulseboard --name PulseBoard --embedder tei is missing",
    "synveda login is missing",
    "synveda demo start --profile personal is missing",
  ]);
  assert.deepEqual(
    releaseNoteFindings(
      notes(`synveda init --slug pulseboard --name PulseBoard --embedder tei
synveda demo start --profile personal`),
    ),
    ["synveda login is missing"],
  );
});
