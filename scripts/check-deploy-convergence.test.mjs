import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

import {
  dockerignoreFindings,
  hasRetiredDemoField,
  missingLocalDockerCopySources,
  missingWorkspaceManifestCopies,
  productImageFindings,
  productLauncherFindings,
  releaseNoteFindings,
  retiredFindings,
  serviceBlock,
  suppressesCargoBuildFailure,
} from "./check-deploy-convergence.mjs";

const PRODUCT_LAUNCHER = fileURLToPath(
  new URL("../deploy/compose/gateway/synveda-container", import.meta.url),
);
const PRODUCT_DOCKERFILE = fileURLToPath(
  new URL("../deploy/compose/gateway/Dockerfile", import.meta.url),
);
const DOCKERIGNORE = fileURLToPath(new URL("../.dockerignore", import.meta.url));

test("a gateway image cannot copy a deleted workspace manifest", () => {
  const dockerfile = `
COPY package.json pnpm-lock.yaml ./
COPY sdks/typescript/package.json sdks/typescript/
COPY --from=build /src/target/release/synveda /usr/local/bin/synveda
`;
  const present = new Set(["package.json", "pnpm-lock.yaml"]);
  assert.deepEqual(
    missingLocalDockerCopySources(dockerfile, (path) => present.has(path)),
    ["sdks/typescript/package.json"],
  );
});

test("the image cache stage names every crate and fails closed", () => {
  const dockerfile = `
COPY crates/alpha/Cargo.toml crates/alpha/
RUN cargo build --release
`;
  assert.deepEqual(
    missingWorkspaceManifestCopies(dockerfile, [
      "crates/alpha/Cargo.toml",
      "crates/beta/Cargo.toml",
    ]),
    ["crates/beta/Cargo.toml"],
  );
  assert.equal(suppressesCargoBuildFailure(dockerfile), false);
  assert.equal(suppressesCargoBuildFailure("RUN cargo build --release || true\n"), true);
});

test("the product image is role-neutral and non-root", () => {
  const current = readFileSync(PRODUCT_DOCKERFILE, "utf8");
  assert.deepEqual(productImageFindings(current), []);
  assert.deepEqual(productImageFindings(current.replace("65532:65532", "root")), [
    "final runtime user is not an explicit non-zero UID:GID",
  ]);
  assert.ok(
    productImageFindings(current.replace("65532:65532", "0:65532")).includes(
      "final runtime user is not an explicit non-zero UID:GID",
    ),
  );
  assert.deepEqual(productImageFindings(`${current}HEALTHCHECK CMD gateway-only\n`), [
    "image hard-codes a role-specific healthcheck",
  ]);
  assert.ok(
    productImageFindings(`${current}\nFROM scratch\nUSER 65532:65532\n`).includes(
      "final stage is not the named runtime stage",
    ),
  );
  assert.ok(
    productImageFindings(
      current.replace(
        "COPY --from=build /src/target/release/synveda-gateway /usr/local/bin/synveda-gateway\n",
        "",
      ),
    ).includes("final runtime stage omits synveda-gateway"),
  );
  assert.ok(
    productImageFindings(
      current.replace(
        "COPY --from=build /src/target/release/synveda /usr/local/bin/synveda",
        "# COPY --from=build /src/target/release/synveda /usr/local/bin/synveda",
      ),
    ).includes("final runtime stage omits synveda"),
  );
  assert.ok(
    productImageFindings(
      current.replace(
        'ENTRYPOINT ["/usr/local/bin/synveda-container"]',
        '# ENTRYPOINT ["/usr/local/bin/synveda-container"]',
      ),
    ).includes("role-neutral entrypoint is missing"),
  );
  assert.ok(
    productImageFindings(current.replace('CMD ["gateway"]', '# CMD ["gateway"]')).includes(
      "default gateway role is missing",
    ),
  );
});

test("the product launcher execs closed roles without deployment branching", () => {
  const current = readFileSync(PRODUCT_LAUNCHER, "utf8");
  assert.deepEqual(productLauncherFindings(current), []);
  assert.deepEqual(productLauncherFindings(current.replace("exec /usr/bin/curl", "curl")), [
    "probe permits curl configuration loading",
  ]);
  assert.deepEqual(productLauncherFindings(current.replace("--disable ", "")), [
    "probe permits curl configuration loading",
  ]);
  assert.deepEqual(productLauncherFindings(current.replace("--noproxy '*' ", "")), [
    "probe permits inherited proxy routing",
  ]);
  assert.ok(
    productLauncherFindings(`${current}\neval "$role"\n`).includes("launcher evaluates input"),
  );
  assert.ok(
    productLauncherFindings(`${current}\nif compose; then gateway; fi\n`).includes(
      "launcher branches on deployment shape",
    ),
  );
  assert.ok(
    productLauncherFindings(current.replace("ready) path=readyz", "ready) path=healthz")).includes(
      "ready probe does not select /readyz",
    ),
  );
  assert.ok(
    productLauncherFindings(current.replace('"$#" -eq 3', '"$#" -ge 1')).includes(
      "probe role does not enforce exact arity",
    ),
  );
  assert.ok(
    productLauncherFindings(
      current.replace("exec /usr/local/bin/synveda-gateway", "# exec /usr/local/bin/synveda-gateway"),
    ).includes("gateway role does not exec the gateway binary"),
  );
  assert.ok(
    productLauncherFindings(
      current.replace(
        "\n    *)\n        usage",
        "\n        shell) exec /bin/sh ;;\n    *)\n        usage",
      ),
    ).includes("launcher case vocabulary is not closed and ordered"),
  );
  assert.ok(
    productLauncherFindings(
      current.replace(
        "\n    *)\n        usage",
        '\n        "shell") exec /bin/sh ;;\n    *)\n        usage',
      ),
    ).includes("launcher case vocabulary is not closed and ordered"),
  );
});

test("the product launcher rejects an unknown role without interpretation", () => {
  const result = spawnSync("sh", [PRODUCT_LAUNCHER, "unavailable"], { encoding: "utf8" });
  assert.equal(result.status, 64);
  assert.equal(result.stdout, "");
  assert.equal(
    result.stderr,
    "usage: synveda-container {gateway|migrate|probe gateway {live|ready}}\n",
  );
});

test("the product launcher dispatches every implemented role exactly", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-product-launcher-"));
  const launcher = join(scratch, "synveda-container");
  try {
    const instrumented = readFileSync(PRODUCT_LAUNCHER, "utf8")
      .replace("exec /usr/local/bin/synveda-gateway", "exec /bin/echo gateway")
      .replace("exec /usr/local/bin/synveda db migrate", "exec /bin/echo migrate")
      .replace("exec /usr/bin/curl \\\n", "exec /bin/echo curl \\\n");
    writeFileSync(launcher, instrumented);

    const cases = [
      [["gateway"], "gateway\n"],
      [["migrate"], "migrate\n"],
      [
        ["probe", "gateway", "live"],
        "curl --disable --noproxy * --fail --silent --show-error --connect-timeout 1 --max-time 2 http://127.0.0.1:8120/healthz\n",
      ],
      [
        ["probe", "gateway", "ready"],
        "curl --disable --noproxy * --fail --silent --show-error --connect-timeout 1 --max-time 2 http://127.0.0.1:8120/readyz\n",
      ],
    ];
    for (const [args, expected] of cases) {
      const result = spawnSync("sh", [launcher, ...args], { encoding: "utf8" });
      assert.equal(result.status, 0, result.stderr);
      assert.equal(result.stdout, expected);
      assert.equal(result.stderr, "");
    }
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("the Docker build context excludes local state and credentials", () => {
  const current = readFileSync(DOCKERIGNORE, "utf8");
  assert.deepEqual(dockerignoreFindings(current), []);
  assert.deepEqual(dockerignoreFindings(current.replace(".codex\n", "")), [
    "build context includes .codex",
  ]);
  assert.ok(
    dockerignoreFindings(current.replace(/^\.env\.\*\n/m, "")).includes(
      "build context includes .env.*",
    ),
  );
  assert.ok(
    dockerignoreFindings(current.replace("deploy/compose/backups\n", "")).includes(
      "build context includes deploy/compose/backups",
    ),
  );
  assert.deepEqual(dockerignoreFindings(`${current}\n!**/.env\n`).slice(-1), [
    "unreviewed build-context re-inclusion !**/.env",
  ]);
});

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
