import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  realpathSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const SUBPROCESS_TIMEOUT_MS = 20_000;

function installPoisonDocker(directory) {
  writeFileSync(
    join(directory, "docker"),
    '#!/bin/sh\nset -eu\n: > "$0.invoked"\nexit 99\n',
    { mode: 0o700 },
  );
}

function assertPoisonDockerUntouched(directory) {
  assert.equal(existsSync(join(directory, "docker.invoked")), false);
}

function shellFunctionSource(source, name) {
  const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = source.match(new RegExp(`^${escaped}\\(\\) \\{\\n[\\s\\S]*?^\\}\\n`, "m"));
  assert.ok(match, `${name} function is missing`);
  return match[0];
}

import {
  authorityFingerprintFixtureFindings,
  contributorPostgresBuildFindings,
  dbTestNetworkReservationFindings,
  developmentInitdbFindings,
  demoFixtureFindings,
  dockerignoreFindings,
  evalFixtureFindings,
  evalSignalTrapFindings,
  hasRetiredDemoField,
  helmContractFindings,
  initCutoverFindings,
  lifecyclePeerWitnessFindings,
  missingLocalDockerCopySources,
  missingWorkspaceManifestCopies,
  productImageFindings,
  productLauncherFindings,
  postgresImageTargetFindings,
  releasePostgresBuildFindings,
  releaseNoteFindings,
  retiredFindings,
  serviceBlock,
  shellFunctionOrderFindings,
  sqlxPrepareFixtureFindings,
  suppressesCargoBuildFailure,
} from "./check-deploy-convergence.mjs";

const PRODUCT_LAUNCHER = fileURLToPath(
  new URL("../deploy/compose/gateway/synveda-container", import.meta.url),
);
const PRODUCT_DOCKERFILE = fileURLToPath(
  new URL("../deploy/compose/gateway/Dockerfile", import.meta.url),
);
const DOCKERIGNORE = fileURLToPath(new URL("../.dockerignore", import.meta.url));
const INIT_SOURCE = fileURLToPath(
  new URL("../crates/synveda-cli/src/init.rs", import.meta.url),
);
const RELEASE_WORKFLOW = fileURLToPath(
  new URL("../.github/workflows/release.yml", import.meta.url),
);
const DB_TEST = fileURLToPath(new URL("./db-test.sh", import.meta.url));
const DB_TEST_COMPOSE = fileURLToPath(
  new URL("../deploy/compose/compose.db-test.yaml", import.meta.url),
);
const DEMO_HARNESS = fileURLToPath(
  new URL("../demos/lib/current-platform-demo.sh", import.meta.url),
);
const EVAL_LIB = fileURLToPath(new URL("../evals/lib.sh", import.meta.url));
const CI_WORKFLOW = fileURLToPath(new URL("../.github/workflows/ci.yml", import.meta.url));
const EVAL_WORKFLOW = fileURLToPath(new URL("../.github/workflows/eval.yml", import.meta.url));
const EVAL_RUN = fileURLToPath(new URL("../evals/run.sh", import.meta.url));
const EVAL_LONGMEMEVAL_RUN = fileURLToPath(
  new URL("../evals/run-longmemeval.sh", import.meta.url),
);
const CONTRIBUTOR_COMPOSE = fileURLToPath(
  new URL("../deploy/compose/docker-compose.yml", import.meta.url),
);
const POSTGRES_DOCKERFILE = fileURLToPath(
  new URL("../deploy/compose/postgres/Dockerfile", import.meta.url),
);
const DEVELOPMENT_INITDB = fileURLToPath(
  new URL("../deploy/compose/postgres/development-initdb.sql", import.meta.url),
);

const HELM_DATABASE_CONTRACT = `
kind: Cluster
spec:
  bootstrap:
    initdb:
      owner: synveda_migrator
      postInitSQL:
        - revoke connect, temporary on database postgres, template1 from public;
---
kind: ConfigMap
metadata:
  labels:
    app.kubernetes.io/component: database-contract
data:
  roles.json: |-
    {"migrator":"synveda_migrator","gateway":"synveda_gateway","worker":"synveda_worker","administrators":["postgres"],"administrative_memberships":[],"forbidden_databases":["postgres","template1"],"isolated_peer_roles":[]}
---
kind: Deployment
metadata:
  labels:
    app.kubernetes.io/component: gateway
spec:
  template:
    spec:
      containers:
        - name: gateway
          image: example/synveda:sha
          env:
            - name: DATABASE_URL_FILE
              value: /run/secrets/synveda-gateway/database_url
            - name: SYNVEDA_DATABASE_ROLES_FILE
              value: /etc/synveda/database/roles.json
          volumeMounts:
            - name: database-roles
              mountPath: /etc/synveda/database
              readOnly: true
      volumes:
        - secret:
            secretName: synveda-gateway-db
        - name: database-roles
          configMap:
            name: synveda-database-roles
---
kind: Deployment
metadata:
  labels:
    app.kubernetes.io/component: worker
spec:
  template:
    spec:
      containers:
        - name: worker
          image: example/synveda:sha
          env:
            - name: DATABASE_URL_FILE
              value: /run/secrets/synveda-worker/database_url
            - name: SYNVEDA_DATABASE_ROLES_FILE
              value: /etc/synveda/database/roles.json
          volumeMounts:
            - name: database-roles
              mountPath: /etc/synveda/database
              readOnly: true
      volumes:
        - secret:
            secretName: synveda-worker-db
        - name: database-roles
          configMap:
            name: synveda-database-roles
---
kind: Job
metadata:
  labels:
    app.kubernetes.io/component: install
spec:
  activeDeadlineSeconds: 900
  backoffLimit: 3
  ttlSecondsAfterFinished: 3600
  template:
    spec:
      initContainers:
        - name: database-bootstrap
          command: ["/usr/local/bin/synveda-database-bootstrap"]
          args:
            - |
              exec /usr/local/bin/synveda-database-bootstrap synveda
          env:
            - name: BOOTSTRAP
              valueFrom:
                secretKeyRef:
                  name: synveda-pg-superuser
        - name: database-preflight
          args: ["database-preflight"]
          env:
            - name: SYNVEDA_MIGRATOR_DATABASE_URL_FILE
            - name: SYNVEDA_GATEWAY_DATABASE_URL_FILE
            - name: SYNVEDA_WORKER_DATABASE_URL_FILE
            - name: SYNVEDA_DATABASE_ROLES_FILE
              value: /etc/synveda/database/roles.json
          volumeMounts:
            - name: database-roles
              mountPath: /etc/synveda/database
              readOnly: true
        - name: migrate
          args: ["migrate"]
          env:
            - name: DATABASE_URL_FILE
              value: /run/secrets/synveda-migrator/database_url
            - name: SYNVEDA_DATABASE_ROLES_FILE
              value: /etc/synveda/database/roles.json
          volumeMounts:
            - name: database-roles
              mountPath: /etc/synveda/database
              readOnly: true
      containers:
        - name: tenant
          env:
            - name: DATABASE_URL_FILE
              value: /run/secrets/synveda-migrator/database_url
            - name: SYNVEDA_DATABASE_ROLES_FILE
              value: /etc/synveda/database/roles.json
          volumeMounts:
            - name: database-roles
              mountPath: /etc/synveda/database
              readOnly: true
      volumes:
        - name: migrator-database
          secret:
            secretName: synveda-pg-app
        - name: database-roles
          configMap:
            name: synveda-database-roles
`;

test("the Helm database authority matrix fails closed", () => {
  assert.deepEqual(helmContractFindings(HELM_DATABASE_CONTRACT), []);

  for (const [label, rendered, expected] of [
    [
      "owner drift",
      HELM_DATABASE_CONTRACT.replace("owner: synveda_migrator", "owner: synveda_gateway"),
      "CloudNativePG application owner is not synveda_migrator",
    ],
    [
      "application-owner extension hook",
      HELM_DATABASE_CONTRACT.replace(
        "        - revoke connect, temporary on database postgres, template1 from public;",
        "        - revoke connect, temporary on database postgres, template1 from public;\n      postInitApplicationSQL:\n        - create extension if not exists vector;",
      ),
      "CloudNativePG does not close PUBLIC maintenance-database access or still creates extensions as the application owner",
    ],
    [
      "gateway app credential",
      HELM_DATABASE_CONTRACT.replace("secretName: synveda-gateway-db", "secretName: synveda-pg-app"),
      "gateway receives forbidden Secret synveda-pg-app",
    ],
    [
      "role contract drift",
      HELM_DATABASE_CONTRACT.replace('"gateway":"synveda_gateway"', '"gateway":"synveda_migrator"'),
      "database role contract ConfigMap is missing or drifted",
    ],
    [
      "incomplete preflight",
      HELM_DATABASE_CONTRACT.replace("- name: SYNVEDA_WORKER_DATABASE_URL_FILE", "- name: OMITTED"),
      "database preflight is missing SYNVEDA_WORKER_DATABASE_URL_FILE",
    ],
    [
      "migration superuser",
      HELM_DATABASE_CONTRACT.replace(
        'args: ["migrate"]',
        'args: ["migrate"]\n          secretName: synveda-pg-superuser',
      ),
      "migrate receives the PostgreSQL superuser credential",
    ],
    [
      "gateway blocking init",
      HELM_DATABASE_CONTRACT.replace(
        "      containers:\n        - name: gateway",
        "      initContainers:\n        - name: wait-for-schema\n      containers:\n        - name: gateway",
      ),
      "gateway retains a blocking startup init container",
    ],
    [
      "worker retired wait loop",
      HELM_DATABASE_CONTRACT.replace(
        "      containers:\n        - name: worker",
        "      wait-for-schema: retained\n      containers:\n        - name: worker",
      ),
      "worker retains the retired schema-wait loop",
    ],
  ]) {
    assert.ok(helmContractFindings(rendered).includes(expected), label);
  }
});

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
  assert.ok(
    productImageFindings(current.replace("cargo build --locked", "cargo build")).includes(
      "release Cargo builds are not exactly two locked invocations",
    ),
  );
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
        "COPY --from=build /src/target/release/synveda-oidc-diagnostic /usr/local/bin/synveda-oidc-diagnostic\n",
        "",
      ),
    ).includes("final runtime stage omits synveda-oidc-diagnostic"),
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

test("the product image excludes the gateway behavior-test feature", async () => {
  const { productTestSupportFindings } = await import(
    "./check-deploy-convergence.mjs"
  );
  const current = readFileSync(PRODUCT_DOCKERFILE, "utf8");
  const gateway = readFileSync(
    new URL("../crates/synveda-gateway/src/main.rs", import.meta.url),
    "utf8",
  );
  const worker = readFileSync(
    new URL(
      "../crates/synveda-gateway/src/bin/synveda-worker.rs",
      import.meta.url,
    ),
    "utf8",
  );
  assert.deepEqual(productTestSupportFindings(current, gateway, worker), []);
  assert.ok(
    productTestSupportFindings(
      current.replace(
        "COPY crates/synveda-gateway/test-support-enabler/Cargo.toml crates/synveda-gateway/test-support-enabler/\n",
        "",
      ),
      gateway,
      worker,
    ).includes("dependency cache omits the gateway test-support manifest"),
  );
  assert.ok(
    productTestSupportFindings(
      `${current}\nRUN cargo build --release --all-features -p synveda-gateway\n`,
      gateway,
      worker,
    ).includes("product image enables gateway test support"),
  );
  assert.ok(
    productTestSupportFindings(current, gateway.replace("test-support", "test_support"), worker)
      .includes("gateway release binary lacks the test-support refusal"),
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
      current.replace("exec /usr/local/bin/synveda-worker", "# exec /usr/local/bin/synveda-worker"),
    ).includes("worker role does not exec the worker binary"),
  );
  assert.ok(
    productLauncherFindings(
      current.replace(
        'issuer-diagnostic)\n        [ "$#" -eq 1 ] || usage',
        'issuer-diagnostic)\n        [ "$#" -ge 1 ] || usage',
      ),
    ).includes("issuer-diagnostic role does not enforce exact arity"),
  );
  assert.ok(
    productLauncherFindings(
      current.replace("exec /usr/local/bin/synveda tenant converge", "exec /bin/true"),
    ).includes("tenant-converge role does not exec the exact tenant admission command"),
  );
  assert.ok(
    productLauncherFindings(current.replace("            50s \\\n", "            500s \\\n")).includes(
      "issuer-diagnostic role does not enforce the closed 50s execution bound",
    ),
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
    "usage: synveda-container {gateway|worker|issuer-diagnostic|database-preflight|migrate|tenant-converge|probe {gateway|worker} {live|ready}}\n",
  );
});

test("the product launcher dispatches every implemented role exactly", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-product-launcher-"));
  const launcher = join(scratch, "synveda-container");
  try {
    const instrumented = readFileSync(PRODUCT_LAUNCHER, "utf8")
      .replace("exec /usr/local/bin/synveda-gateway", "exec /bin/echo gateway")
      .replace("exec /usr/local/bin/synveda-worker", "exec /bin/echo worker")
      .replace(
        /exec \/usr\/bin\/timeout \\\n\s+--foreground \\\n\s+--signal=TERM \\\n\s+--kill-after=2s \\\n\s+50s \\\n\s+\/usr\/local\/bin\/synveda-oidc-diagnostic/,
        "exec /bin/echo issuer-diagnostic",
      )
      .replace("exec /usr/local/bin/synveda db preflight", "exec /bin/echo database-preflight")
      .replace("exec /usr/local/bin/synveda db migrate", "exec /bin/echo migrate")
      .replace("exec /usr/local/bin/synveda tenant converge", "exec /bin/echo tenant-converge")
      .replace("exec /usr/bin/curl \\\n", "exec /bin/echo curl \\\n");
    writeFileSync(launcher, instrumented);

    const cases = [
      [["gateway"], "gateway\n"],
      [["worker"], "worker\n"],
      [["issuer-diagnostic"], "issuer-diagnostic\n"],
      [["database-preflight"], "database-preflight\n"],
      [["migrate"], "migrate\n"],
      [
        ["tenant-converge"],
        "tenant-converge --id 019b53c0-7c00-7000-8000-000000000045 --slug reference --name Reference Tenant\n",
        {
          SYNVEDA_BOOTSTRAP_TENANT_ID: "019b53c0-7c00-7000-8000-000000000045",
          SYNVEDA_BOOTSTRAP_TENANT_SLUG: "reference",
          SYNVEDA_BOOTSTRAP_TENANT_NAME: "Reference Tenant",
        },
      ],
      [
        ["probe", "gateway", "live"],
        "curl --disable --noproxy * --fail --silent --show-error --connect-timeout 1 --max-time 2 http://127.0.0.1:8120/healthz\n",
      ],
      [
        ["probe", "gateway", "ready"],
        "curl --disable --noproxy * --fail --silent --show-error --connect-timeout 1 --max-time 2 http://127.0.0.1:8120/readyz\n",
      ],
      [
        ["probe", "worker", "live"],
        "curl --disable --noproxy * --fail --silent --show-error --connect-timeout 1 --max-time 2 http://127.0.0.1:8121/healthz\n",
      ],
      [
        ["probe", "worker", "ready"],
        "curl --disable --noproxy * --fail --silent --show-error --connect-timeout 1 --max-time 2 http://127.0.0.1:8121/readyz\n",
      ],
    ];
    for (const [args, expected, extraEnvironment = {}] of cases) {
      const result = spawnSync("sh", [launcher, ...args], {
        encoding: "utf8",
        env: {
          ...process.env,
          SYNVEDA_BOOTSTRAP_TENANT_ID: "",
          SYNVEDA_BOOTSTRAP_TENANT_SLUG: "",
          SYNVEDA_BOOTSTRAP_TENANT_NAME: "",
          ...extraEnvironment,
        },
      });
      assert.equal(result.status, 0, result.stderr);
      assert.equal(result.stdout, expected);
      assert.equal(result.stderr, "");
    }
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("tenant convergence refuses malformed deployment identity before exec", () => {
  for (const extraEnvironment of [
    {},
    {
      SYNVEDA_BOOTSTRAP_TENANT_ID: "019b53c0-7c00-7g00-8000-000000000045",
      SYNVEDA_BOOTSTRAP_TENANT_SLUG: "reference",
      SYNVEDA_BOOTSTRAP_TENANT_NAME: "Reference Tenant",
    },
    {
      SYNVEDA_BOOTSTRAP_TENANT_ID: "019b53c0-7c00-7000-8000-000000000045",
      SYNVEDA_BOOTSTRAP_TENANT_SLUG: "../reference",
      SYNVEDA_BOOTSTRAP_TENANT_NAME: "Reference Tenant",
    },
    {
      SYNVEDA_BOOTSTRAP_TENANT_ID: "019b53c0-7c00-7000-8000-000000000045",
      SYNVEDA_BOOTSTRAP_TENANT_SLUG: "reference",
      SYNVEDA_BOOTSTRAP_TENANT_NAME: "-Reference Tenant",
    },
    {
      SYNVEDA_BOOTSTRAP_TENANT_ID: "019b53c0-7c00-7000-8000-000000000045",
      SYNVEDA_BOOTSTRAP_TENANT_SLUG: "reference",
      SYNVEDA_BOOTSTRAP_TENANT_NAME: "   ",
    },
  ]) {
    const result = spawnSync("sh", [PRODUCT_LAUNCHER, "tenant-converge"], {
      encoding: "utf8",
      env: {
        ...process.env,
        SYNVEDA_BOOTSTRAP_TENANT_ID: "",
        SYNVEDA_BOOTSTRAP_TENANT_SLUG: "",
        SYNVEDA_BOOTSTRAP_TENANT_NAME: "",
        ...extraEnvironment,
      },
    });
    assert.equal(result.status, 78, result.stderr);
    assert.equal(result.stdout, "");
    assert.doesNotMatch(result.stderr, /019b53c0|Reference Tenant/);
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
  assert.ok(
    dockerignoreFindings(current.replace("deploy/compose/runtime\n", "")).includes(
      "build context includes deploy/compose/runtime",
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

test("the withdrawn init entrypoint remains a gate-only boundary", () => {
  const current = readFileSync(INIT_SOURCE, "utf8");
  assert.deepEqual(initCutoverFindings(current), []);
  assert.deepEqual(
    initCutoverFindings(
      current.replace(
        "    reference_cutover_gate()\n}\n\n#[allow(dead_code)]",
        "    let _profile = Profile::discover()?;\n    reference_cutover_gate()\n}\n\n#[allow(dead_code)]",
      ),
    ),
    ["public init entrypoint is not a gate-only cutover refusal"],
  );
  assert.deepEqual(
    initCutoverFindings(current.replace("async fn init_after_cutover", "async fn init_legacy")),
    ["init cutover entrypoint has no isolated dormant implementation"],
  );
});

test("release notes do not advertise an unaccepted turnkey deployment", () => {
  const notes = (body) => `cat > notes.md <<NOTES
${body}
NOTES
`;
  const current = notes(
    "Docker reference deployment acceptance is pending; this is not a turnkey single-host release.",
  );
  assert.deepEqual(releaseNoteFindings(current), []);
  assert.deepEqual(
    releaseNoteFindings(
      notes(`Docker reference deployment acceptance is pending.
synveda init --demo
synveda login
synveda demo start --profile personal`),
    ),
    [
    "retired synveda init --demo command",
      "unaccepted turnkey command synveda init",
      "unaccepted turnkey command synveda login",
      "unaccepted turnkey command synveda demo start",
    ],
  );
  assert.deepEqual(releaseNoteFindings(notes("Artifacts only.")), [
    "Docker reference acceptance notice is missing",
  ]);
});

test("the release PostgreSQL build uses the repository-root context", () => {
  const current = readFileSync(RELEASE_WORKFLOW, "utf8");
  assert.deepEqual(releasePostgresBuildFindings(current), []);
  assert.deepEqual(
    releasePostgresBuildFindings(
      current.replace(
        "          context: .\n          file: deploy/compose/postgres/Dockerfile\n          target: reference\n          platforms:",
        "          context: deploy/compose/postgres\n          platforms:",
      ),
    ),
    [
      "release PostgreSQL build context is not the repository root",
      "release PostgreSQL Dockerfile is not explicit",
      "release PostgreSQL build does not select the reference target",
    ],
  );
});

test("the contributor PostgreSQL build selects one development-only target", () => {
  const compose = readFileSync(CONTRIBUTOR_COMPOSE, "utf8");
  const dockerfile = readFileSync(POSTGRES_DOCKERFILE, "utf8");
  const initdb = readFileSync(DEVELOPMENT_INITDB, "utf8");
  assert.deepEqual(contributorPostgresBuildFindings(compose), []);
  assert.deepEqual(postgresImageTargetFindings(dockerfile), []);
  assert.deepEqual(developmentInitdbFindings(initdb), []);

  assert.deepEqual(
    contributorPostgresBuildFindings(
      compose.replace(
        "    build:\n      context: ../..\n      dockerfile: deploy/compose/postgres/Dockerfile\n      target: development\n",
        "    build: ./postgres\n",
      ),
    ),
    ["contributor PostgreSQL build does not select the repo-root development target"],
  );
  assert.ok(
    postgresImageTargetFindings(
      dockerfile.replace("FROM runtime AS reference", "FROM runtime AS removed-reference"),
    ).length > 0,
  );
  assert.ok(
    postgresImageTargetFindings(
      dockerfile.replace("FROM runtime AS development", "FROM runtime AS reference-leak"),
    ).length > 0,
  );
  assert.ok(
    developmentInitdbFindings(`${initdb}\ncreate role unsafe;\n`).length > 0,
  );
});

test("database evidence helpers are defined before their first call", () => {
  const current = readFileSync(DB_TEST, "utf8");
  const names = ["private_evidence_file", "assert_database_secrets_absent"];
  assert.deepEqual(shellFunctionOrderFindings(current, names), []);
  const call = 'private_evidence_file "$post_copy_stdout"';
  assert.deepEqual(
    shellFunctionOrderFindings(`${call}\n${current}`, names),
    ["private_evidence_file is called before definition"],
  );
});

test("database test owns one collision-resistant external-network quartet", () => {
  const dbTest = readFileSync(DB_TEST, "utf8");
  const compose = readFileSync(DB_TEST_COMPOSE, "utf8");
  assert.deepEqual(dbTestNetworkReservationFindings(dbTest, compose), []);

  for (const [mutatedDbTest, mutatedCompose, expected] of [
    [
      dbTest.replace(
        "  compose run --rm --no-deps database-bootstrap-main",
        '  runner=com""pose\n' +
          "  operation=rm\n" +
          "  : compose run\n" +
          '  "$runner" "$operation" --force --stop postgres-main',
      ),
      compose,
      "differs from the reviewed executable",
    ],
    [
      dbTest.replace(
        "network_ownership_file=$state_dir/network-ownership.tsv",
        '"$docker_bin" network ls --quiet\nnetwork_ownership_file=$state_dir/network-ownership.tsv',
      ),
      compose,
      "observes pre-existing Docker networks",
    ],
    [
      dbTest.replace(
        "network_seed=$(printf '%s\\n' \"$project\" | cksum | awk '{print $1}')",
        "network_seed=0",
      ),
      compose,
      "bounded full-cycle project-derived /28 candidates",
    ],
    [
      dbTest.replace(
        "network_start_slot=$((network_seed % 2048))",
        "network_start_slot=$((network_seed % 512))",
      ),
      compose,
      "bounded full-cycle project-derived /28 candidates",
    ],
    [
      dbTest.replace(
        "network_start_slot=$((network_seed % 2048))",
        "network_start_slot=$((network_seed % 2048))\nnetwork_start_slot=0",
      ),
      compose,
      "bounded full-cycle project-derived /28 candidates",
    ],
    [
      dbTest.replace(
        "network_start_slot=$((network_seed % 2048))",
        "network_start_slot=$((network_seed % 2048))\nreadonly network_start_slot=0",
      ),
      compose,
      "bounded full-cycle project-derived /28 candidates",
    ],
    [
      dbTest.replace(
        "network_start_slot=$((network_seed % 2048))",
        "network_start_slot=$((network_seed % 2048))\ndeclare network_start_slot=0",
      ),
      compose,
      "bounded full-cycle project-derived /28 candidates",
    ],
    [
      dbTest.replace(
        "network_start_slot=$((network_seed % 2048))",
        "network_start_slot=$((network_seed % 2048))\nnetwork_start_slot+=1",
      ),
      compose,
      "bounded full-cycle project-derived /28 candidates",
    ],
    [
      dbTest.replace(
        "network_within=$((network_slot % 4096))",
        "network_within=0",
      ),
      compose,
      "bounded full-cycle project-derived /28 candidates",
    ],
    [
      dbTest.replace(
        "network_fourth=$(((network_within % 16) * 16))",
        "network_fourth=$(((network_within % 16) * 16))\n  network_fourth=0",
      ),
      compose,
      "bounded full-cycle project-derived /28 candidates",
    ],
    [
      dbTest.replace("attempt * 1265", "attempt * 1264"),
      compose,
      "bounded full-cycle project-derived /28 candidates",
    ],
    [
      dbTest.replace(
        "network_reservation_limit=64",
        "network_reservation_limit=64\nnetwork_reservation_limit=8192",
      ),
      compose,
      "bounded full-cycle project-derived /28 candidates",
    ],
    [
      dbTest.replace(
        "\nnetwork_create_is_pool_contention() {",
        "\nnetwork_candidate_subnet() { :; }\n\nnetwork_create_is_pool_contention() {",
      ),
      compose,
      "bounded full-cycle project-derived /28 candidates",
    ],
    [
      dbTest.replace("network_reservation_limit=64", "network_reservation_limit=8192"),
      compose,
      "bounded full-cycle project-derived /28 candidates",
    ],
    [
      dbTest.replace(
        'reserve_test_network 3 "${network_logicals[3]}" "${network_names[3]}" false',
        "# fourth reservation removed",
      ),
      compose,
      "exact four-network topology",
    ],
    [
      dbTest
        .replace(
          "export SYNVEDA_DB_TEST_MAIN_DATA_NETWORK=${network_names[0]}",
          "export SYNVEDA_DB_TEST_MAIN_DATA_NETWORK=${network_names[2]}",
        )
        .replace(
          "export SYNVEDA_DB_TEST_MAIN_HOST_NETWORK=${network_names[2]}",
          "export SYNVEDA_DB_TEST_MAIN_HOST_NETWORK=${network_names[0]}",
        ),
      compose,
      "does not export SYNVEDA_DB_TEST_MAIN_DATA_NETWORK",
    ],
    [
      dbTest.replace(
        "export SYNVEDA_DB_TEST_MAIN_DATA_NETWORK=${network_names[0]}",
        "export SYNVEDA_DB_TEST_MAIN_DATA_NETWORK=${network_names[0]}\n" +
          "declare -x SYNVEDA_DB_TEST_MAIN_DATA_NETWORK=${network_names[2]}",
      ),
      compose,
      "does not export SYNVEDA_DB_TEST_MAIN_DATA_NETWORK",
    ],
    [
      dbTest.replace(
        "export SYNVEDA_DB_TEST_MAIN_DATA_NETWORK=${network_names[0]}",
        "export SYNVEDA_DB_TEST_MAIN_DATA_NETWORK=${network_names[0]}\n" +
          "export SYNVEDA_DB_TEST_MAIN_DATA_NETWORK=${network_names[2]}",
      ),
      compose,
      "does not export SYNVEDA_DB_TEST_MAIN_DATA_NETWORK",
    ],
    [
      dbTest.replace(
        'reserve_test_network 0 "${network_logicals[0]}" "${network_names[0]}" true',
        'reserve_test_network 0 "${network_logicals[0]}" "${network_names[0]}" false',
      ),
      compose,
      "exact four-network topology",
    ],
    [
      dbTest.replace(
        'reserve_test_network 0 "${network_logicals[0]}" "${network_names[0]}" true',
        'reserve_test_network 0 "${network_logicals[0]}" "${network_names[0]}" true || exit 69',
      ),
      compose,
      "exact four-network topology",
    ],
    [
      dbTest.replace(
        "'Error response from daemon: invalid pool request: Pool overlaps with other one on this address space'",
        "'Pool overlaps with other one on this address space'",
      ),
      compose,
      "network creation is not closed",
    ],
    [
      dbTest.replace(
        '[ "$create_status" -eq 1 ] && [ ! -s "$receipt_file" ] && cmp -s --',
        "cmp -s --",
      ),
      compose,
      "network creation is not closed",
    ],
    [
      dbTest.replaceAll(
        '>"$network_receipt_file" 2>"$network_error_file"',
        '2>"$network_error_file"',
      ),
      compose,
      "network creation is not closed",
    ],
    [
      dbTest.replace('[ "${#created_network_id}" -ne 64 ]', '[ "${#created_network_id}" -lt 12 ]'),
      compose,
      "journal and validate immutable network ownership",
    ],
    [
      dbTest.replace(
        "record_network_ownership \\\n      intent",
        "record_network_ownership \\\n      unowned",
      ),
      compose,
      "journal and validate immutable network ownership",
    ],
    [
      dbTest.replace(
        "record_network_ownership \\\n        contended",
        "record_network_ownership \\\n        retried",
      ),
      compose,
      "journal and validate immutable network ownership",
    ],
    [
      dbTest.replace(
        '  record_network_ownership \\\n' +
          '    owned "$logical_name" "$network_name" "$subnet" "$created_network_id" || return 70\n' +
          "  network_subnets[$logical_index]=$subnet",
        "  network_subnets[$logical_index]=$subnet\n" +
          '  record_network_ownership \\\n' +
          '    owned "$logical_name" "$network_name" "$subnet" "$created_network_id" || return 70',
      ),
      compose,
      "journal and validate immutable network ownership",
    ],
    [
      dbTest.replace(
        '[ "$receipt_id" = "${owned_network_ids[$network_index]}" ]',
        '[ "$receipt_id" = uncorrelated ]',
      ),
      compose,
      "journal and validate immutable network ownership",
    ],
    [
      dbTest.replace("cmp -s -- <(printf '0\\n') \"$receipt_status\"", "cmp -s -- /dev/null \"$receipt_status\""),
      compose,
      "journal and validate immutable network ownership",
    ],
    [
      dbTest.replace(
        "trap report_preserved_state EXIT",
        "trap cleanup_successful_fixture EXIT",
      ),
      compose,
      "failure trap can clean",
    ],
    [
      dbTest.replace(
        "network_ownership_file=$state_dir/network-ownership.tsv",
        'cleanup_successful_fixture # early\nnetwork_ownership_file=$state_dir/network-ownership.tsv',
      ),
      compose,
      "exactly two success-only cleanup calls",
    ],
    [
      dbTest.replace(
        '"$docker_bin" network rm "${owned_network_ids[$network_index]}" >/dev/null',
        '"$docker_bin" network rm "${owned_network_ids[$network_index]}" >/dev/null\n' +
          '  "$docker_bin" network rm "${owned_network_ids[0]}" >/dev/null',
      ),
      compose,
      "Docker teardown grammar is not closed",
    ],
    [
      dbTest
        .replace(
          "  compose down --volumes --remove-orphans",
          "  # compose down --volumes --remove-orphans",
        )
        .replace(
          "network_ownership_file=$state_dir/network-ownership.tsv",
          "compose down --volumes --remove-orphans\nnetwork_ownership_file=$state_dir/network-ownership.tsv",
        ),
      compose,
      "cleanup is not fenced",
    ],
    [
      dbTest.replace(
        "validate_owned_network_ledger() {\n  local network_index",
        "validate_owned_network_ledger() {\n  return 0\n  local network_index",
      ),
      compose,
      "cleanup is not fenced",
    ],
    [
      dbTest.replace(
        "expected_owned_network_ledger() {\n  local network_attempt",
        "expected_owned_network_ledger() {\n  return 0\n  local network_attempt",
      ),
      compose,
      "cleanup is not fenced",
    ],
    [
      dbTest.replace(
        "  validate_owned_network_ledger\n  cleanup_started=true",
        "  validate_owned_network_ledger || :\n  cleanup_started=true",
      ),
      compose,
      "cleanup is not fenced",
    ],
    [
      dbTest.replace(
        "  validate_owned_network_ledger\n  cleanup_started=true",
        "  : validate_owned_network_ledger\n  cleanup_started=true",
      ),
      compose,
      "cleanup is not fenced",
    ],
    [
      dbTest.replace(
        "  validate_owned_network_ledger\n  cleanup_started=true",
        "  validate_owned_network_ledger\n" +
          `  owned_network_ids[0]=${"f".repeat(64)}\n` +
          "  cleanup_started=true",
      ),
      compose,
      "cleanup is not fenced",
    ],
    [
      dbTest
        .replace(
          '    "$docker_bin" network rm "${owned_network_ids[$network_index]}" >/dev/null',
          '    # "$docker_bin" network rm "${owned_network_ids[$network_index]}" >/dev/null',
        )
        .replace(
          'reserve_test_network 3 "${network_logicals[3]}" "${network_names[3]}" false',
          'reserve_test_network 3 "${network_logicals[3]}" "${network_names[3]}" false\n' +
            'network_index=0\n' +
            '"$docker_bin" network rm "${owned_network_ids[$network_index]}" >/dev/null',
        ),
      compose,
      "cleanup is not fenced",
    ],
    [
      dbTest.replace(
        "  validate_owned_network_ledger\n  cleanup_started=true\n  compose down --volumes --remove-orphans",
        "  cleanup_started=true\n  compose down --volumes --remove-orphans\n  validate_owned_network_ledger",
      ),
      compose,
      "cleanup is not fenced",
    ],
    [
      dbTest
        .replace('  rm -R -- "$state_dir"', "  # state removed early")
        .replace(
          "  cleanup_started=true",
          '  cleanup_started=true\n  rm -R -- "$state_dir"',
        ),
      compose,
      "cleanup is not fenced",
    ],
    [
      dbTest.replace(
        "network_ownership_file=$state_dir/network-ownership.tsv",
        '"$docker_bin" network prune --force\nnetwork_ownership_file=$state_dir/network-ownership.tsv',
      ),
      compose,
      "Docker teardown grammar is not closed",
    ],
    [
      dbTest.replace(
        "compose config --quiet",
        'compose config --quiet\noperation=rm\ncompose "$operation" --force --stop postgres-main',
      ),
      compose,
      "Docker teardown grammar is not closed",
    ],
    [
      dbTest.replace(
        "network_ownership_file=$state_dir/network-ownership.tsv",
        'export engine=$docker_bin\n"$engine" image prune --all --force\n' +
          "network_ownership_file=$state_dir/network-ownership.tsv",
      ),
      compose,
      "Docker teardown grammar is not closed",
    ],
    [
      dbTest.replace(
        "network_ownership_file=$state_dir/network-ownership.tsv",
        'declare engine=$docker_bin\n"$engine" rmi unrelated\n' +
          "network_ownership_file=$state_dir/network-ownership.tsv",
      ),
      compose,
      "Docker teardown grammar is not closed",
    ],
    [
      dbTest.replace(
        "network_ownership_file=$state_dir/network-ownership.tsv",
        'engine_name=docker_bin\nengine=${!engine_name}\n"$engine" rm unrelated\n' +
          "network_ownership_file=$state_dir/network-ownership.tsv",
      ),
      compose,
      "Docker teardown grammar is not closed",
    ],
    [
      dbTest.replace(
        "network_ownership_file=$state_dir/network-ownership.tsv",
        "docker rm --force unrelated\nnetwork_ownership_file=$state_dir/network-ownership.tsv",
      ),
      compose,
      "Docker teardown grammar is not closed",
    ],
    [
      dbTest.replace(
        "network_ownership_file=$state_dir/network-ownership.tsv",
        "docker image prune --all --force\nnetwork_ownership_file=$state_dir/network-ownership.tsv",
      ),
      compose,
      "Docker teardown grammar is not closed",
    ],
    [
      dbTest.replace(
        "network_ownership_file=$state_dir/network-ownership.tsv",
        'unreviewed_engine network prune --force\nnetwork_ownership_file=$state_dir/network-ownership.tsv',
      ),
      compose,
      "Docker teardown grammar is not closed",
    ],
    [
      dbTest.replace(
        "network_ownership_file=$state_dir/network-ownership.tsv",
        'engine=$docker_bin\nkind=network\noperation=rm\n' +
          '"$engine" "$kind" "$operation" unrelated\n' +
          "network_ownership_file=$state_dir/network-ownership.tsv",
      ),
      compose,
      "Docker teardown grammar is not closed",
    ],
    [
      dbTest.replace(
        "network_ownership_file=$state_dir/network-ownership.tsv",
        'unreviewed_engine system prune --all --force --volumes\n' +
          "network_ownership_file=$state_dir/network-ownership.tsv",
      ),
      compose,
      "Docker teardown grammar is not closed",
    ],
    [
      dbTest.replace(
        "network_ownership_file=$state_dir/network-ownership.tsv",
        'engine=$docker_bin; "$engine" volume rm unrelated\n' +
          "network_ownership_file=$state_dir/network-ownership.tsv",
      ),
      compose,
      "Docker teardown grammar is not closed",
    ],
    [
      dbTest.replace(
        "network_ownership_file=$state_dir/network-ownership.tsv",
        "compose rm --force --stop --volumes postgres-main\n" +
          "network_ownership_file=$state_dir/network-ownership.tsv",
      ),
      compose,
      "Docker teardown grammar is not closed",
    ],
    [
      dbTest
        .replace(
          '    "$docker_bin" image rm "$SYNVEDA_DB_TEST_POSTGRES_IMAGE" >/dev/null',
          '    # "$docker_bin" image rm "$SYNVEDA_DB_TEST_POSTGRES_IMAGE" >/dev/null',
        )
        .replace(
          "network_ownership_file=$state_dir/network-ownership.tsv",
          'if [ "$test_image_owned" = true ]; then\n' +
            '  "$docker_bin" image rm "$SYNVEDA_DB_TEST_POSTGRES_IMAGE" >/dev/null\n' +
            "fi\nnetwork_ownership_file=$state_dir/network-ownership.tsv",
        ),
      compose,
      "Docker teardown grammar is not closed",
    ],
    [
      dbTest.replace(
        "network_ownership_file=$state_dir/network-ownership.tsv",
        '"$docker_bin" compose --project-name "$project" --file "$manifest" down --volumes --remove-orphans\n' +
          "network_ownership_file=$state_dir/network-ownership.tsv",
      ),
      compose,
      "Docker teardown grammar is not closed",
    ],
    [
      dbTest
        .replace(
          "  compose down --volumes --remove-orphans",
          "  # compose down --volumes --remove-orphans",
        )
        .replace(
          "network_ownership_file=$state_dir/network-ownership.tsv",
          "compose \\\n  down --volumes --remove-orphans\n" +
            "network_ownership_file=$state_dir/network-ownership.tsv",
        ),
      compose,
      "Docker teardown grammar is not closed",
    ],
    [
      dbTest.replace(
        "network_ownership_file=$state_dir/network-ownership.tsv",
        "compose down --volumes --remove-orphans\nnetwork_ownership_file=$state_dir/network-ownership.tsv",
      ),
      compose,
      "Docker teardown grammar is not closed",
    ],
    [
      dbTest + '\n"$docker_bin" network create unsafe-extra\n',
      compose,
      "Docker teardown grammar is not closed",
    ],
    [
      dbTest.replace(
        '"$docker_bin" network rm "${owned_network_ids[$network_index]}"',
        '"$docker_bin" network rm "${network_names[$network_index]}"',
      ),
      compose,
      "cleanup is not fenced",
    ],
    [
      dbTest.replace(
        '[ "$status" -eq 0 ] || exit "$status"',
        '# removed fast-path success guard',
      ),
      compose,
      "cleanup is reachable before success",
    ],
    [
      dbTest,
      compose.replace("  main-data:\n    external: true", "  main-data:\n    external: false"),
      "network main-data is not an exact external reservation",
    ],
  ]) {
    assert.ok(
      mutatedDbTest !== dbTest || mutatedCompose !== compose,
      `network-reservation mutant was a no-op: ${expected}`,
    );
    assert.ok(
      dbTestNetworkReservationFindings(mutatedDbTest, mutatedCompose).some((finding) =>
        finding.includes(expected),
      ),
      `network-reservation mutant escaped: ${expected}`,
    );
  }
});

test("database network reservation failures retain their partial ownership ledger", () => {
  for (const [scenario, expectedCreates, expectedLedgerLines] of [
    ["fail-second", 2, 3],
    ["duplicate-second", 2, 3],
    ["malformed-fourth", 4, 7],
  ]) {
    const scratch = mkdtempSync(join(tmpdir(), `synveda-db-network-${scenario}-`));
    const stateRoot = join(scratch, "state");
    const fakeDocker = join(scratch, "configured-engine");
    const log = join(scratch, "docker.log");
    const count = join(scratch, "docker.count");
    mkdirSync(stateRoot, { mode: 0o700 });
    installPoisonDocker(scratch);
    writeFileSync(
      fakeDocker,
      `#!/bin/sh
set -eu
printf '%s\\n' "$*" >> "$FAKE_DOCKER_LOG"
case "$1 $2" in
  "network create") ;;
  *) exit 97 ;;
esac
network_count=0
if [ -f "$FAKE_DOCKER_COUNT" ]; then network_count=$(cat "$FAKE_DOCKER_COUNT"); fi
network_count=$((network_count + 1))
printf '%s\\n' "$network_count" > "$FAKE_DOCKER_COUNT"
if [ "$FAKE_DOCKER_SCENARIO" = fail-second ] && [ "$network_count" -eq 2 ]; then
  exit 55
fi
if [ "$FAKE_DOCKER_SCENARIO" = duplicate-second ] && [ "$network_count" -eq 2 ]; then
  printf '%064d\\n' 1
  exit 0
fi
if [ "$FAKE_DOCKER_SCENARIO" = malformed-fourth ] && [ "$network_count" -eq 4 ]; then
  printf 'not-an-immutable-id\\n'
  exit 0
fi
printf '%064d\\n' "$network_count"
`,
      { mode: 0o700 },
    );
    try {
      const result = spawnSync("bash", [DB_TEST], {
        encoding: "utf8",
        timeout: SUBPROCESS_TIMEOUT_MS,
        env: {
          ...process.env,
          FAKE_DOCKER_COUNT: count,
          FAKE_DOCKER_LOG: log,
          FAKE_DOCKER_SCENARIO: scenario,
          PATH: `${scratch}:${process.env.PATH ?? ""}`,
          SYNVEDA_DOCKER_BIN: fakeDocker,
          TMPDIR: stateRoot,
        },
      });
      assert.equal(result.status, 69, result.stderr);
      const commands = readFileSync(log, "utf8").trim().split("\n");
      assert.equal(commands.length, expectedCreates);
      assert.ok(commands.every((command) => command.startsWith("network create ")));
      assert.ok(commands.every((command) => !/\b(?:ls|inspect|rm)\b/.test(command)));
      const retained = readdirSync(stateRoot);
      assert.equal(retained.length, 1);
      const retainedState = join(stateRoot, retained[0]);
      const ledgerPath = join(retainedState, "network-ownership.tsv");
      assert.equal(statSync(retainedState).mode & 0o777, 0o700);
      assert.equal(statSync(ledgerPath).mode & 0o777, 0o600);
      const ledger = readFileSync(ledgerPath, "utf8")
        .trim()
        .split("\n");
      assert.equal(ledger.length, expectedLedgerLines);
      assert.match(ledger.at(-1), /^intent\t/);
      assert.match(result.stderr, /retained private fixture state and any created resources/);
      assertPoisonDockerUntouched(scratch);
    } finally {
      rmSync(scratch, { recursive: true, force: true });
    }
  }
});

test("database network contention advances each logical lane without ambient observation", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-db-network-contention-"));
  const stateRoot = join(scratch, "state");
  const fakeDocker = join(scratch, "configured-engine");
  const log = join(scratch, "docker.log");
  const count = join(scratch, "docker.count");
  const overlap =
    "Error response from daemon: invalid pool request: " +
    "Pool overlaps with other one on this address space";
  mkdirSync(stateRoot, { mode: 0o700 });
  installPoisonDocker(scratch);
  writeFileSync(
    fakeDocker,
    `#!/bin/sh
set -eu
printf '%s\\n' "$*" >> "$FAKE_DOCKER_LOG"
case "$1 $2" in
  "network create")
    network_count=0
    if [ -f "$FAKE_DOCKER_COUNT" ]; then network_count=$(cat "$FAKE_DOCKER_COUNT"); fi
    network_count=$((network_count + 1))
    printf '%s\\n' "$network_count" > "$FAKE_DOCKER_COUNT"
    if [ $((network_count % 2)) -eq 1 ]; then
      printf '%s\\n' "$FAKE_DOCKER_OVERLAP" >&2
      exit 1
    fi
    printf '%064d\\n' $((network_count / 2))
    ;;
  "compose --project-name") exit 55 ;;
  *) exit 97 ;;
esac
`,
    { mode: 0o700 },
  );
  try {
    const result = spawnSync("bash", [DB_TEST], {
      encoding: "utf8",
      timeout: SUBPROCESS_TIMEOUT_MS,
      env: {
        ...process.env,
        FAKE_DOCKER_COUNT: count,
        FAKE_DOCKER_LOG: log,
        FAKE_DOCKER_OVERLAP: overlap,
        PATH: `${scratch}:${process.env.PATH ?? ""}`,
        SYNVEDA_DOCKER_BIN: fakeDocker,
        TMPDIR: stateRoot,
      },
    });
    assert.equal(result.status, 55, result.stderr);
    const commands = readFileSync(log, "utf8").trim().split("\n");
    const creates = commands.filter((command) => command.startsWith("network create "));
    assert.equal(creates.length, 8);
    assert.ok(creates.every((command) => !/\b(?:ls|inspect|rm)\b/.test(command)));
    const subnets = creates.map((command) => {
      const match = command.match(/--subnet (198\.(?:18|19)\.([0-9]{1,3})\.([0-9]{1,3})\/28)\b/);
      assert.ok(match, command);
      assert.ok(Number(match[2]) <= 255, command);
      assert.ok(Number(match[3]) <= 240, command);
      assert.equal(Number(match[3]) % 16, 0, command);
      return match[1];
    });
    assert.equal(new Set(subnets).size, subnets.length);

    const retained = readdirSync(stateRoot);
    assert.equal(retained.length, 1);
    const retainedState = join(stateRoot, retained[0]);
    const ledger = readFileSync(join(retainedState, "network-ownership.tsv"), "utf8")
      .trim()
      .split("\n")
      .map((line) => line.split("\t"));
    assert.equal(ledger.length, 16);
    for (let logical = 0; logical < 4; logical += 1) {
      const group = ledger.slice(logical * 4, logical * 4 + 4);
      assert.deepEqual(
        group.map((row) => row[0]),
        ["intent", "contended", "intent", "owned"],
      );
      assert.equal(group[0][1], group[3][1]);
      assert.equal(group[0][2], group[3][2]);
      assert.equal(group[0][3], group[1][3]);
      assert.notEqual(group[0][3], group[3][3]);
      assert.equal(group[3][4], `${logical + 1}`.padStart(64, "0"));
    }

    const receipts = join(retainedState, "network-receipts");
    assert.equal(statSync(receipts).mode & 0o777, 0o700);
    assert.equal(readdirSync(receipts).length, 24);
    for (let logical = 0; logical < 4; logical += 1) {
      const contendedStdout = join(receipts, `${logical}-0.stdout`);
      const contendedStderr = join(receipts, `${logical}-0.stderr`);
      const contendedStatus = join(receipts, `${logical}-0.status`);
      const ownedStdout = join(receipts, `${logical}-1.stdout`);
      const ownedStderr = join(receipts, `${logical}-1.stderr`);
      const ownedStatus = join(receipts, `${logical}-1.status`);
      for (const path of [
        contendedStdout,
        contendedStderr,
        contendedStatus,
        ownedStdout,
        ownedStderr,
        ownedStatus,
      ]) {
        assert.equal(statSync(path).mode & 0o777, 0o600);
      }
      assert.equal(readFileSync(contendedStdout, "utf8"), "");
      assert.equal(readFileSync(contendedStderr, "utf8"), `${overlap}\n`);
      assert.equal(readFileSync(contendedStatus, "utf8"), "1\n");
      assert.equal(
        readFileSync(ownedStdout, "utf8"),
        `${`${logical + 1}`.padStart(64, "0")}\n`,
      );
      assert.equal(readFileSync(ownedStderr, "utf8"), "");
      assert.equal(readFileSync(ownedStatus, "utf8"), "0\n");
    }
    assertPoisonDockerUntouched(scratch);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("database network contention classifier rejects ambiguous Docker failures", () => {
  for (const scenario of [
    "nonempty-stdout",
    "extra-stderr",
    "different-error",
    "wrong-status",
  ]) {
    const scratch = mkdtempSync(join(tmpdir(), `synveda-db-network-${scenario}-`));
    const stateRoot = join(scratch, "state");
    const fakeDocker = join(scratch, "configured-engine");
    const log = join(scratch, "docker.log");
    const overlap =
      "Error response from daemon: invalid pool request: " +
      "Pool overlaps with other one on this address space";
    mkdirSync(stateRoot, { mode: 0o700 });
    installPoisonDocker(scratch);
    writeFileSync(
      fakeDocker,
      `#!/bin/sh
set -eu
printf '%s\\n' "$*" >> "$FAKE_DOCKER_LOG"
case "$1 $2" in
  "network create")
    case "$FAKE_DOCKER_SCENARIO" in
      nonempty-stdout)
        printf '%064d\\n' 1
        printf '%s\\n' "$FAKE_DOCKER_OVERLAP" >&2
        ;;
      extra-stderr)
        printf '%s\\n%s\\n' "$FAKE_DOCKER_OVERLAP" extra >&2
        ;;
      different-error) printf '%s\\n' 'Error response from daemon: unavailable' >&2 ;;
      wrong-status)
        printf '%s\\n' "$FAKE_DOCKER_OVERLAP" >&2
        exit 55
        ;;
    esac
    exit 1
    ;;
  *) exit 97 ;;
esac
`,
      { mode: 0o700 },
    );
    try {
      const result = spawnSync("bash", [DB_TEST], {
        encoding: "utf8",
        timeout: SUBPROCESS_TIMEOUT_MS,
        env: {
          ...process.env,
          FAKE_DOCKER_LOG: log,
          FAKE_DOCKER_OVERLAP: overlap,
          FAKE_DOCKER_SCENARIO: scenario,
          PATH: `${scratch}:${process.env.PATH ?? ""}`,
          SYNVEDA_DOCKER_BIN: fakeDocker,
          TMPDIR: stateRoot,
        },
      });
      assert.equal(result.status, 69, result.stderr);
      const commands = readFileSync(log, "utf8").trim().split("\n");
      assert.equal(commands.length, 1);
      assert.ok(commands[0].startsWith("network create "));
      assert.ok(!/\b(?:ls|inspect|rm)\b/.test(commands[0]));
      const retainedState = join(stateRoot, readdirSync(stateRoot)[0]);
      const ledger = readFileSync(join(retainedState, "network-ownership.tsv"), "utf8")
        .trim()
        .split("\n");
      assert.equal(ledger.length, 1);
      assert.match(ledger[0], /^intent\t/);
      const receipt = readFileSync(
        join(retainedState, "network-receipts", "0-0.stdout"),
        "utf8",
      );
      assert.equal(receipt.length > 0, scenario === "nonempty-stdout");
      assertPoisonDockerUntouched(scratch);
    } finally {
      rmSync(scratch, { recursive: true, force: true });
    }
  }
});

test("database network contention exhausts its bounded lane without teardown", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-db-network-exhausted-"));
  const stateRoot = join(scratch, "state");
  const fakeDocker = join(scratch, "configured-engine");
  const log = join(scratch, "docker.log");
  const overlap =
    "Error response from daemon: invalid pool request: " +
    "Pool overlaps with other one on this address space";
  mkdirSync(stateRoot, { mode: 0o700 });
  installPoisonDocker(scratch);
  writeFileSync(
    fakeDocker,
    `#!/bin/sh
set -eu
printf '%s\\n' "$*" >> "$FAKE_DOCKER_LOG"
case "$1 $2" in
  "network create") printf '%s\\n' "$FAKE_DOCKER_OVERLAP" >&2; exit 1 ;;
  *) exit 97 ;;
esac
`,
    { mode: 0o700 },
  );
  try {
    const result = spawnSync("bash", [DB_TEST], {
      encoding: "utf8",
      timeout: SUBPROCESS_TIMEOUT_MS,
      env: {
        ...process.env,
        FAKE_DOCKER_LOG: log,
        FAKE_DOCKER_OVERLAP: overlap,
        PATH: `${scratch}:${process.env.PATH ?? ""}`,
        SYNVEDA_DOCKER_BIN: fakeDocker,
        TMPDIR: stateRoot,
      },
    });
    assert.equal(result.status, 69, result.stderr);
    assert.match(result.stderr, /no uncontended \/28 is available/);
    const commands = readFileSync(log, "utf8").trim().split("\n");
    assert.equal(commands.length, 64);
    assert.ok(commands.every((command) => command.startsWith("network create ")));
    assert.ok(commands.every((command) => !/\b(?:ls|inspect|rm)\b/.test(command)));
    const subnets = commands.map((command) => command.match(/--subnet ([^ ]+)/)?.[1]);
    assert.ok(subnets.every(Boolean));
    assert.equal(new Set(subnets).size, 64);
    const retainedState = join(stateRoot, readdirSync(stateRoot)[0]);
    const ledger = readFileSync(join(retainedState, "network-ownership.tsv"), "utf8")
      .trim()
      .split("\n");
    assert.equal(ledger.length, 128);
    assert.ok(ledger.every((line, index) => line.startsWith(index % 2 ? "contended\t" : "intent\t")));
    assert.equal(readdirSync(join(retainedState, "network-receipts")).length, 192);
    assertPoisonDockerUntouched(scratch);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("concurrent database fixtures reserve and clean only their own disjoint networks", async () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-db-network-concurrent-"));
  const binDir = join(scratch, "bin");
  const barrierDir = join(scratch, "barrier");
  const evidenceDir = join(scratch, "evidence");
  const stateA = join(scratch, "state-a");
  const stateB = join(scratch, "state-b");
  const barrier = join(binDir, "cleanup-barrier");
  const fakeDocker = join(binDir, "configured-engine");
  const fakeCksum = join(binDir, "cksum");
  const harness = join(scratch, "db-network-concurrent.sh");
  const daemonState = join(scratch, "daemon.tsv");
  const daemonCount = join(scratch, "daemon.count");
  const daemonEvents = join(scratch, "daemon-events.tsv");
  const daemonLock = join(scratch, "daemon.lock");
  const sentinelId = "f".repeat(64);
  const sentinelState =
    `203.0.113.0/28\t${sentinelId}\tsentinel-project\tsentinel\t` +
    "sentinel-network\tsentinel\n";
  mkdirSync(binDir, { mode: 0o700 });
  installPoisonDocker(binDir);
  mkdirSync(barrierDir, { mode: 0o700 });
  mkdirSync(evidenceDir, { mode: 0o700 });
  mkdirSync(stateA, { mode: 0o700 });
  mkdirSync(stateB, { mode: 0o700 });
  writeFileSync(daemonState, sentinelState, { mode: 0o600 });
  const source = readFileSync(DB_TEST, "utf8");
  const end = source.indexOf("\nprivate_evidence_file() {");
  assert.ok(end > 0);
  const prefix = source.slice(0, end);
  const repositoryCd = 'cd "$(dirname "$0")/.."';
  assert.ok(prefix.includes(repositoryCd));
  writeFileSync(
    harness,
    `${prefix.replace(repositoryCd, 'cd "$DB_TEST_REPO_ROOT"')}\n` +
      '"$FAKE_CLEANUP_BARRIER"\n' +
      'cp "$network_ownership_file" "$FAKE_EVIDENCE_DIR/$FAKE_CALLER.tsv"\n' +
      'chmod 600 "$FAKE_EVIDENCE_DIR/$FAKE_CALLER.tsv"\n' +
      "cleanup_successful_fixture\n",
    { mode: 0o700 },
  );
  writeFileSync(
    barrier,
    `#!/bin/sh
set -eu
: > "$FAKE_BARRIER_DIR/ready-$FAKE_CALLER"
barrier_attempt=0
while [ ! -f "$FAKE_BARRIER_DIR/ready-A" ] || [ ! -f "$FAKE_BARRIER_DIR/ready-B" ]; do
  barrier_attempt=$((barrier_attempt + 1))
  if [ "$barrier_attempt" -ge 1000 ]; then exit 98; fi
  sleep 0.01
done
`,
    { mode: 0o700 },
  );
  writeFileSync(
    fakeCksum,
    "#!/bin/sh\nset -eu\ncat >/dev/null\nprintf '0 0\\n'\n",
    { mode: 0o700 },
  );
  writeFileSync(
    fakeDocker,
    `#!/bin/sh
set -eu
lock_held=false
release_lock() {
  if [ "$lock_held" = true ]; then
    rmdir "$FAKE_DAEMON_LOCK" 2>/dev/null || :
    lock_held=false
  fi
}
trap release_lock EXIT
trap 'release_lock; exit 130' HUP INT TERM
lock_attempt=0
while ! mkdir "$FAKE_DAEMON_LOCK" 2>/dev/null; do
  lock_attempt=$((lock_attempt + 1))
  if [ "$lock_attempt" -ge 1000 ]; then exit 98; fi
  sleep 0.01
done
lock_held=true
case "$1 $2" in
  "network create")
    subnet=
    project=
    logical=
    network_name=
    expect_subnet=false
    for argument do
      if [ "$expect_subnet" = true ]; then
        subnet=$argument
        expect_subnet=false
      else
        case "$argument" in
          --subnet) expect_subnet=true ;;
          com.synveda.project=*) project=\${argument#com.synveda.project=} ;;
          com.synveda.network=*) logical=\${argument#com.synveda.network=} ;;
        esac
      fi
      network_name=$argument
    done
    if [ -f "$FAKE_DAEMON_STATE" ] && awk -F '\\t' -v subnet="$subnet" '
      $1 == subnet { found = 1 }
      END { exit found ? 0 : 1 }
    ' "$FAKE_DAEMON_STATE"; then
      printf 'contended\\t%s\\t%s\\t%s\\t%s\\t%s\\t-\\n' \
        "$FAKE_CALLER" "$project" "$logical" "$network_name" "$subnet" \
        >> "$FAKE_DAEMON_EVENTS"
      release_lock
      printf '%s\\n' "$FAKE_DOCKER_OVERLAP" >&2
      exit 1
    fi
    network_count=0
    if [ -f "$FAKE_DAEMON_COUNT" ]; then network_count=$(cat "$FAKE_DAEMON_COUNT"); fi
    network_count=$((network_count + 1))
    printf '%s\\n' "$network_count" > "$FAKE_DAEMON_COUNT"
    network_id=$(printf '%064d' "$network_count")
    printf '%s\\t%s\\t%s\\t%s\\t%s\\t%s\\n' \
      "$subnet" "$network_id" "$project" "$logical" "$network_name" "$FAKE_CALLER" \
      >> "$FAKE_DAEMON_STATE"
    printf 'created\\t%s\\t%s\\t%s\\t%s\\t%s\\t%s\\n' \
      "$FAKE_CALLER" "$project" "$logical" "$network_name" "$subnet" "$network_id" \
      >> "$FAKE_DAEMON_EVENTS"
    release_lock
    printf '%s\\n' "$network_id"
    ;;
  "compose --project-name")
    printf 'compose\\t%s\\t%s\\n' "$FAKE_CALLER" "$*" >> "$FAKE_DAEMON_EVENTS"
    release_lock
    exit 0
    ;;
  "network rm")
    network_id=$3
    network_owner=$(awk -F '\\t' -v network_id="$network_id" '
      $2 == network_id { print $6; exit }
    ' "$FAKE_DAEMON_STATE")
    if [ -z "$network_owner" ]; then
      printf 'unexpected\\t%s\\tmissing-network\\t%s\\n' \
        "$FAKE_CALLER" "$network_id" >> "$FAKE_DAEMON_EVENTS"
      exit 96
    fi
    if [ "$network_owner" != "$FAKE_CALLER" ]; then
      printf 'foreign\\t%s\\t%s\\t%s\\n' \
        "$FAKE_CALLER" "$network_owner" "$network_id" >> "$FAKE_DAEMON_EVENTS"
      exit 96
    fi
    state_temp=$FAKE_DAEMON_STATE.$FAKE_CALLER.$$
    awk -F '\\t' -v network_id="$network_id" '$2 != network_id' \
      "$FAKE_DAEMON_STATE" > "$state_temp"
    mv "$state_temp" "$FAKE_DAEMON_STATE"
    printf 'removed\\t%s\\t%s\\t%s\\n' \
      "$FAKE_CALLER" "$network_owner" "$network_id" >> "$FAKE_DAEMON_EVENTS"
    release_lock
    exit 0
    ;;
  *)
    printf 'unexpected\\t%s\\t%s\\n' "$FAKE_CALLER" "$*" >> "$FAKE_DAEMON_EVENTS"
    release_lock
    exit 97
    ;;
esac
`,
    { mode: 0o700 },
  );

  const overlap =
    "Error response from daemon: invalid pool request: " +
    "Pool overlaps with other one on this address space";
  const run = (caller, stateRoot) =>
    new Promise((resolve) => {
      const child = spawn("bash", [harness], {
        detached: true,
        env: {
          ...process.env,
          DB_TEST_REPO_ROOT: fileURLToPath(new URL("..", import.meta.url)),
          FAKE_BARRIER_DIR: barrierDir,
          FAKE_CALLER: caller,
          FAKE_CLEANUP_BARRIER: barrier,
          FAKE_DAEMON_COUNT: daemonCount,
          FAKE_DAEMON_EVENTS: daemonEvents,
          FAKE_DAEMON_LOCK: daemonLock,
          FAKE_DAEMON_STATE: daemonState,
          FAKE_DOCKER_OVERLAP: overlap,
          FAKE_EVIDENCE_DIR: evidenceDir,
          PATH: `${binDir}:${process.env.PATH ?? ""}`,
          SYNVEDA_DB_TEST_POSTGRES_IMAGE: "fake-postgres:owned-elsewhere",
          SYNVEDA_DOCKER_BIN: fakeDocker,
          TMPDIR: stateRoot,
        },
        stdio: ["ignore", "pipe", "pipe"],
      });
      let stdout = "";
      let stderr = "";
      let finished = false;
      let timeoutError = null;
      const finish = (result) => {
        if (finished) return;
        finished = true;
        clearTimeout(timer);
        resolve(result);
      };
      const timer = setTimeout(() => {
        timeoutError = new Error(`concurrent allocator ${caller} timed out`);
        if (child.pid !== undefined) {
          try {
            process.kill(-child.pid, "SIGKILL");
          } catch (error) {
            if (error.code !== "ESRCH") timeoutError = error;
          }
        }
      }, SUBPROCESS_TIMEOUT_MS);
      child.stdout.setEncoding("utf8");
      child.stderr.setEncoding("utf8");
      child.stdout.on("data", (chunk) => {
        stdout += chunk;
      });
      child.stderr.on("data", (chunk) => {
        stderr += chunk;
      });
      child.on("error", (error) =>
        finish({ error, signal: null, status: null, stderr, stdout }),
      );
      child.on("close", (status, signal) =>
        finish({ error: timeoutError, signal, status, stderr, stdout }),
      );
    });

  try {
    const results = await Promise.all([run("A", stateA), run("B", stateB)]);
    for (const result of results) {
      assert.equal(result.error, null, result.error?.message);
      assert.equal(result.signal, null);
      assert.equal(result.status, 0, result.stderr);
    }
    const events = readFileSync(daemonEvents, "utf8")
      .trim()
      .split("\n")
      .map((line) => line.split("\t"));
    const created = events.filter((event) => event[0] === "created");
    const contended = events.filter((event) => event[0] === "contended");
    const compose = events.filter((event) => event[0] === "compose");
    const removed = events.filter((event) => event[0] === "removed");
    assert.equal(created.length, 8);
    assert.equal(contended.length, 4);
    assert.equal(compose.length, 2);
    assert.equal(removed.length, 8);
    assert.equal(new Set(created.map((event) => event[5])).size, 8);
    assert.equal(new Set(created.map((event) => event[6])).size, 8);
    assert.equal(new Set(created.map((event) => event[1])).size, 2);
    assert.deepEqual(
      [...new Set(compose.map((event) => event[1]))].sort(),
      ["A", "B"],
    );
    assert.ok(events.every((event) => !["unexpected", "foreign"].includes(event[0])));
    assert.deepEqual(readdirSync(stateA), []);
    assert.deepEqual(readdirSync(stateB), []);
    assert.deepEqual(readdirSync(evidenceDir).sort(), ["A.tsv", "B.tsv"]);
    assert.ok(existsSync(join(barrierDir, "ready-A")));
    assert.ok(existsSync(join(barrierDir, "ready-B")));

    const allOwnedIds = new Set();
    for (const caller of ["A", "B"]) {
      const evidence = join(evidenceDir, `${caller}.tsv`);
      assert.equal(statSync(evidence).mode & 0o777, 0o600);
      const ledger = readFileSync(evidence, "utf8")
        .trim()
        .split("\n")
        .map((line) => line.split("\t"));
      const owned = ledger.filter((row) => row[0] === "owned");
      assert.equal(owned.length, 4);
      const callerCreates = created.filter((event) => event[1] === caller);
      assert.equal(callerCreates.length, 4);
      const projects = new Set(callerCreates.map((event) => event[2]));
      assert.equal(projects.size, 1);
      const [project] = projects;
      assert.ok(owned.every((row) => row[2].startsWith(`${project}-`)));
      for (const row of owned) {
        assert.ok(
          created.some(
            (event) =>
              event[1] === caller &&
              event[2] === project &&
              event[3] === row[1] &&
              event[4] === row[2] &&
              event[5] === row[3] &&
              event[6] === row[4],
          ),
        );
        assert.ok(!allOwnedIds.has(row[4]));
        allOwnedIds.add(row[4]);
      }
    }
    assert.equal(allOwnedIds.size, 8);
    const createdById = new Map(created.map((event) => [event[6], event]));
    for (const event of removed) {
      assert.equal(event[1], event[2]);
      assert.equal(createdById.get(event[3])?.[1], event[1]);
      assert.notEqual(event[3], sentinelId);
    }
    assert.deepEqual(
      removed.map((event) => event[3]).sort(),
      [...allOwnedIds].sort(),
    );
    assert.equal(readFileSync(daemonState, "utf8"), sentinelState);
    assertPoisonDockerUntouched(binDir);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("database cleanup removes only receipt-correlated IDs in reverse creation order", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-db-network-cleanup-"));
  const stateRoot = join(scratch, "state");
  const fakeDocker = join(scratch, "configured-engine");
  const harness = join(scratch, "db-network-cleanup.sh");
  const log = join(scratch, "docker.log");
  const count = join(scratch, "docker.count");
  mkdirSync(stateRoot, { mode: 0o700 });
  installPoisonDocker(scratch);
  const source = readFileSync(DB_TEST, "utf8");
  const end = source.indexOf("\nprivate_evidence_file() {");
  assert.ok(end > 0);
  const prefix = source.slice(0, end);
  const repositoryCd = 'cd "$(dirname "$0")/.."';
  assert.ok(prefix.includes(repositoryCd));
  writeFileSync(
    harness,
    `${prefix.replace(repositoryCd, 'cd "$DB_TEST_REPO_ROOT"')}\ncleanup_successful_fixture\n`,
    { mode: 0o700 },
  );
  writeFileSync(
    fakeDocker,
    `#!/bin/sh
set -eu
printf '%s\\n' "$*" >> "$FAKE_DOCKER_LOG"
case "$1 $2" in
  "network create")
    network_count=0
    if [ -f "$FAKE_DOCKER_COUNT" ]; then network_count=$(cat "$FAKE_DOCKER_COUNT"); fi
    network_count=$((network_count + 1))
    printf '%s\\n' "$network_count" > "$FAKE_DOCKER_COUNT"
    printf '%064d\\n' "$network_count"
    ;;
  "compose --project-name") exit 0 ;;
  "network rm") exit 0 ;;
  *) exit 97 ;;
esac
`,
    { mode: 0o700 },
  );
  try {
    const result = spawnSync("bash", [harness], {
      encoding: "utf8",
      timeout: SUBPROCESS_TIMEOUT_MS,
      env: {
        ...process.env,
        DB_TEST_REPO_ROOT: fileURLToPath(new URL("..", import.meta.url)),
        FAKE_DOCKER_COUNT: count,
        FAKE_DOCKER_LOG: log,
        PATH: `${scratch}:${process.env.PATH ?? ""}`,
        SYNVEDA_DB_TEST_POSTGRES_IMAGE: "fake-postgres:owned-elsewhere",
        SYNVEDA_DOCKER_BIN: fakeDocker,
        TMPDIR: stateRoot,
      },
    });
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(readdirSync(stateRoot), []);
    const commands = readFileSync(log, "utf8").trim().split("\n");
    const creates = commands.filter((command) => command.startsWith("network create "));
    const removals = commands.filter((command) => command.startsWith("network rm "));
    const compose = commands.filter((command) => command.startsWith("compose --project-name "));
    assert.equal(creates.length, 4);
    assert.deepEqual(removals, [
      `network rm ${"4".padStart(64, "0")}`,
      `network rm ${"3".padStart(64, "0")}`,
      `network rm ${"2".padStart(64, "0")}`,
      `network rm ${"1".padStart(64, "0")}`,
    ]);
    assert.equal(compose.length, 1);
    assert.match(compose[0], / down --volumes --remove-orphans$/);
    assert.ok(commands.every((command) => !/network (?:ls|inspect|prune)\b/.test(command)));
    assertPoisonDockerUntouched(scratch);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("database cleanup refuses receipt drift before any teardown", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-db-network-receipt-drift-"));
  const stateRoot = join(scratch, "state");
  const fakeDocker = join(scratch, "configured-engine");
  const harness = join(scratch, "db-network-receipt-drift.sh");
  const log = join(scratch, "docker.log");
  const count = join(scratch, "docker.count");
  mkdirSync(stateRoot, { mode: 0o700 });
  installPoisonDocker(scratch);
  const source = readFileSync(DB_TEST, "utf8");
  const end = source.indexOf("\nprivate_evidence_file() {");
  assert.ok(end > 0);
  const prefix = source.slice(0, end);
  const repositoryCd = 'cd "$(dirname "$0")/.."';
  assert.ok(prefix.includes(repositoryCd));
  writeFileSync(
    harness,
    `${prefix.replace(repositoryCd, 'cd "$DB_TEST_REPO_ROOT"')}\n` +
      `printf '%064d\\n' 9 > "\${owned_network_receipt_files[0]}"\n` +
      "cleanup_successful_fixture\n",
    { mode: 0o700 },
  );
  writeFileSync(
    fakeDocker,
    `#!/bin/sh
set -eu
printf '%s\\n' "$*" >> "$FAKE_DOCKER_LOG"
case "$1 $2" in
  "network create")
    network_count=0
    if [ -f "$FAKE_DOCKER_COUNT" ]; then network_count=$(cat "$FAKE_DOCKER_COUNT"); fi
    network_count=$((network_count + 1))
    printf '%s\\n' "$network_count" > "$FAKE_DOCKER_COUNT"
    printf '%064d\\n' "$network_count"
    ;;
  *) exit 97 ;;
esac
`,
    { mode: 0o700 },
  );
  try {
    const result = spawnSync("bash", [harness], {
      encoding: "utf8",
      timeout: SUBPROCESS_TIMEOUT_MS,
      env: {
        ...process.env,
        DB_TEST_REPO_ROOT: fileURLToPath(new URL("..", import.meta.url)),
        FAKE_DOCKER_COUNT: count,
        FAKE_DOCKER_LOG: log,
        PATH: `${scratch}:${process.env.PATH ?? ""}`,
        SYNVEDA_DB_TEST_POSTGRES_IMAGE: "fake-postgres:owned-elsewhere",
        SYNVEDA_DOCKER_BIN: fakeDocker,
        TMPDIR: stateRoot,
      },
    });
    assert.equal(result.status, 1, result.stderr);
    assert.match(result.stderr, /refusing cleanup after network receipt drift/);
    assert.match(result.stderr, /retained private fixture state and any created resources/);
    const commands = readFileSync(log, "utf8").trim().split("\n");
    assert.equal(commands.length, 4);
    assert.ok(commands.every((command) => command.startsWith("network create ")));
    assert.ok(commands.every((command) => !/\b(?:rm|ls|inspect|prune)\b/.test(command)));
    assert.equal(readdirSync(stateRoot).length, 1);
    assertPoisonDockerUntouched(scratch);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("database fixtures canonicalize a symlinked platform temp root before private state", () => {
  const scratch = realpathSync(mkdtempSync(join(tmpdir(), "synveda-db-physical-tmp-")));
  const physicalRoot = join(scratch, "physical-temp");
  const linkedRoot = join(scratch, "platform-temp-link");
  const fakeDocker = join(scratch, "configured-engine");
  mkdirSync(physicalRoot, { mode: 0o700 });
  installPoisonDocker(scratch);
  symlinkSync(physicalRoot, linkedRoot, "dir");
  writeFileSync(
    fakeDocker,
    `#!/bin/sh
set -eu
case "$1 $2" in
  "network create")
    for argument do network_name=$argument; done
    case "$network_name" in
      *-main-data) printf '%064d\n' 1 ;;
      *-lifecycle-data) printf '%064d\n' 2 ;;
      *-main-host) printf '%064d\n' 3 ;;
      *-lifecycle-host) printf '%064d\n' 4 ;;
      *) exit 96 ;;
    esac
    ;;
  "compose --project-name") exit 55 ;;
  *) exit 97 ;;
esac
`,
    { mode: 0o700 },
  );
  try {
    const result = spawnSync("bash", [DB_TEST], {
      encoding: "utf8",
      timeout: SUBPROCESS_TIMEOUT_MS,
      env: {
        ...process.env,
        PATH: `${scratch}:${process.env.PATH ?? ""}`,
        SYNVEDA_DOCKER_BIN: fakeDocker,
        TMPDIR: linkedRoot,
      },
    });
    assert.equal(result.status, 55, result.stderr);
    assert.ok(result.stderr.includes(`${physicalRoot}/synveda-db-test.`), result.stderr);
    assert.ok(!result.stderr.includes(`${linkedRoot}/synveda-db-test.`), result.stderr);
    const retained = readdirSync(physicalRoot);
    assert.equal(retained.length, 1);
    const retainedState = join(physicalRoot, retained[0]);
    assert.equal(statSync(retainedState).mode & 0o777, 0o700);
    const stateToken = retained[0].split(".").at(-1).toLowerCase();
    const generatorProject = `synveda-development-acceptance-${stateToken}`;
    assert.ok(
      existsSync(
        join(
          retainedState,
          "generator",
          generatorProject,
          "secrets",
          ".synveda-private-directory",
        ),
      ),
    );
    assert.ok(
      existsSync(
        join(
          retainedState,
          "generator",
          generatorProject,
          "database-authority",
          ".synveda-private-directory",
        ),
      ),
    );
    assertPoisonDockerUntouched(scratch);
    assert.ok(
      existsSync(
        join(
          retainedState,
          "generator",
          generatorProject,
          "keycloak-public-gate",
          ".synveda-private-directory",
        ),
      ),
    );
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("database fixtures reject an unsafe physical temp root before mutation", () => {
  const scratch = realpathSync(mkdtempSync(join(tmpdir(), "synveda-db-unsafe-tmp-")));
  const fakeDocker = join(scratch, "configured-engine");
  installPoisonDocker(scratch);
  writeFileSync(fakeDocker, "#!/bin/sh\nexit 97\n", { mode: 0o700 });
  const spacedRoot = join(scratch, "physical temp");
  mkdirSync(spacedRoot, { mode: 0o700 });

  try {
    for (const [name, target] of [
      ["root-link", "/"],
      ["space-link", spacedRoot],
    ]) {
      const linkedRoot = join(scratch, name);
      symlinkSync(target, linkedRoot, "dir");
      const result = spawnSync("bash", [DB_TEST], {
        encoding: "utf8",
        timeout: SUBPROCESS_TIMEOUT_MS,
        env: {
          ...process.env,
          PATH: `${scratch}:${process.env.PATH ?? ""}`,
          SYNVEDA_DOCKER_BIN: fakeDocker,
          TMPDIR: linkedRoot,
        },
      });
      assert.equal(result.status, 70, result.stderr);
      assert.match(result.stderr, /physical temporary root has an unsafe shape/);
      assert.equal(readdirSync(spacedRoot).length, 0);
    }
    assertPoisonDockerUntouched(scratch);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("evaluation uses the bounded exact-role fixture and proxy-free loopback", () => {
  const dbTest = readFileSync(DB_TEST, "utf8");
  const evalLib = readFileSync(EVAL_LIB, "utf8");
  const ci = readFileSync(CI_WORKFLOW, "utf8");
  const nightly = readFileSync(EVAL_WORKFLOW, "utf8");
  assert.deepEqual(evalFixtureFindings(dbTest, evalLib, ci, nightly), []);
  assert.ok(
    evalFixtureFindings(
      dbTest.replace(
        "compose run --rm --no-deps keycloak-database-bootstrap-main",
        "# missing Keycloak convergence",
      ),
      evalLib,
      ci,
      nightly,
    ).includes("fast fixture bootstrap order is not Synveda-Keycloak-Synveda"),
  );
  assert.ok(
    evalFixtureFindings(dbTest, evalLib.replaceAll("/readyz", "/healthz"), ci, nightly).includes(
      "evaluation readiness is not attested to each launched child",
    ),
  );
  assert.ok(
    evalFixtureFindings(
      dbTest,
      evalLib.replace("curl --disable", "curl"),
      ci,
      nightly,
    ).includes("evaluation HTTP is not one ambient-free bounded curl boundary"),
  );
  assert.ok(
    evalFixtureFindings(
      dbTest,
      evalLib.replace('kill -KILL "$eval_stop_target"', 'kill -TERM "$eval_stop_target"'),
      ci,
      nightly,
    ).includes("evaluation process and log cleanup is not bounded and best-effort"),
  );
  assert.ok(
    evalFixtureFindings(
      dbTest,
      evalLib.replace(
        "unset SYNVEDA_DB_TEST_UID SYNVEDA_DB_TEST_GID SYNVEDA_DB_TEST_SECRETS_DIR",
        "unset SYNVEDA_DB_TEST_UID SYNVEDA_DB_TEST_GID",
      ),
      ci,
      nightly,
    ).includes("evaluation retains fixture control input SYNVEDA_DB_TEST_SECRETS_DIR"),
  );
  assert.ok(
    evalFixtureFindings(
      dbTest,
      evalLib.replace("NO_PROXY=127.0.0.1,localhost", "NO_PROXY=$NO_PROXY"),
      ci,
      nightly,
    ).includes("evaluation bearer traffic is not pinned to proxy-free loopback"),
  );
  assert.ok(
    evalFixtureFindings(
      dbTest,
      evalLib,
      `${ci}\n      - name: Start Postgres\n        run: docker compose up postgres\n`,
      nightly,
    ).includes("CI evaluation still starts the legacy PostgreSQL service"),
  );
});

test("shared demos execute through the fresh exact-role fixture", () => {
  const dbTest = readFileSync(DB_TEST, "utf8");
  const harness = readFileSync(DEMO_HARNESS, "utf8");
  const ci = readFileSync(CI_WORKFLOW, "utf8");
  assert.deepEqual(demoFixtureFindings(dbTest, harness, ci), []);

  for (const mutated of [
    dbTest.replace(
      "  demo|product-evaluation|evaluation|longmemeval-evaluation) fast_fixture=true ;;",
      "  product-evaluation|evaluation|longmemeval-evaluation) fast_fixture=true ;;",
    ),
    dbTest.replaceAll(
      "SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE=$main_migrator_file",
      "SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE=$main_gateway_file",
    ),
    dbTest.replace(
      "          SQLX_OFFLINE=true \\\n",
      "",
    ),
    dbTest.replace(
      'scripts/cargo-with-database-url-file sh "$demo_script" "$@"',
      'sh "$demo_script" "$@"',
    ),
  ]) {
    assert.notDeepEqual(demoFixtureFindings(mutated, harness, ci), []);
  }
  assert.notDeepEqual(
    demoFixtureFindings(
      dbTest,
      harness.replace("SYNVEDA_DB_TEST_TASK=demo", "SYNVEDA_DB_TEST_TASK=workspace"),
      ci,
    ),
    [],
  );
  assert.notDeepEqual(
    demoFixtureFindings(
      dbTest,
      `${harness}\npostgres://synveda:synveda-dev@localhost:5432/synveda\n`,
      ci,
    ),
    [],
  );
});

test("SQLx metadata uses a fresh private exact-migrator fixture", () => {
  const dbTest = readFileSync(DB_TEST, "utf8");
  const mutateSqlxBranch = (mutator) => {
    const start = dbTest.indexOf('  if [ "$db_test_task" = sqlx-prepare ]; then\n');
    const end = dbTest.indexOf(
      '  if [ "$db_test_task" != authority-fingerprints ]; then\n',
      start,
    );
    assert.ok(start >= 0 && end > start, "SQLx prepare branch is missing");
    return `${dbTest.slice(0, start)}${mutator(dbTest.slice(start, end))}${dbTest.slice(end)}`;
  };
  assert.deepEqual(sqlxPrepareFixtureFindings(dbTest), []);
  assert.ok(
    sqlxPrepareFixtureFindings(
      dbTest.replace(
        "cargo sqlx prepare --check --no-dotenv --workspace -- --all-targets",
        "cargo sqlx prepare --no-dotenv --workspace -- --all-targets",
      ),
    ).some((finding) => finding.includes("direct migration, prepare, check")),
  );
  assert.ok(
    sqlxPrepareFixtureFindings(
      mutateSqlxBranch((branch) =>
        branch.replace(
          "cargo sqlx migrate run --no-dotenv",
          "cargo run -q -p synveda-cli --bin synveda -- db migrate",
        ),
      ),
    ).some((finding) => finding.includes("direct migration, prepare, check")),
  );
  assert.ok(
    sqlxPrepareFixtureFindings(
      dbTest.replaceAll(
        "SYNVEDA_CARGO_DATABASE_URL_FILE=$main_migrator_file",
        "SYNVEDA_CARGO_DATABASE_URL_FILE=$main_owner_file",
      ),
    ).some((finding) => finding.includes("non-migrator")),
  );
  assert.ok(
    sqlxPrepareFixtureFindings(dbTest.replaceAll(" -u SQLX_OFFLINE", "")).includes(
      "SQLx migrate/prepare/check do not share the private migrator-file boundary",
    ),
  );
  assert.ok(
    sqlxPrepareFixtureFindings(
      mutateSqlxBranch((branch) =>
        branch.replace(
          "sqlx_cli_banner=$(cargo sqlx --version)",
          "sqlx_cli_banner=unchecked",
        ),
      ),
    ).includes("cargo-sqlx is not proved equal to the locked sqlx library"),
  );
  assert.ok(
    sqlxPrepareFixtureFindings(
      dbTest.replace(
        '  if [ "$db_test_task" = sqlx-prepare ]; then\n',
        '  if [ "$db_test_task" = sqlx-prepare ]; then\n' +
          "    SQLX_OFFLINE=true cargo run -q -p synveda-cli --bin synveda -- db preflight\n",
      ),
    ).some((finding) => finding.includes("extra Cargo invocation")),
  );
  const delayedVersionProbe = mutateSqlxBranch((branch) =>
    branch
      .replace("    sqlx_cli_banner=$(cargo sqlx --version)\n", "")
      .replace(
        "    unset sqlx_library_version sqlx_cli_banner\n",
        "    sqlx_cli_banner=$(cargo sqlx --version)\n" +
          "    unset sqlx_library_version sqlx_cli_banner\n",
      ),
  );
  assert.ok(
    sqlxPrepareFixtureFindings(delayedVersionProbe).some((finding) =>
      finding.includes("mutates before its version proof"),
    ),
  );
  assert.ok(
    sqlxPrepareFixtureFindings(
      mutateSqlxBranch((branch) =>
        branch.replace(
          '        echo "db-test: cargo-sqlx must exactly match the locked sqlx library" >&2\n' +
            "        exit 69",
          '        echo "db-test: cargo-sqlx must exactly match the locked sqlx library" >&2\n' +
            "        :",
        ),
      ),
    ).some((finding) => finding.includes("mutates before its version proof")),
  );
  assert.ok(
    sqlxPrepareFixtureFindings(
      mutateSqlxBranch((branch) =>
        branch.replace(
          '      && [ "$sqlx_cli_banner" = "sqlx-cli-sqlx $sqlx_library_version" ] || {',
          '      && [ "$sqlx_cli_banner" = "sqlx-cli-sqlx $sqlx_library_version" ] && {',
        ),
      ),
    ).some((finding) => finding.includes("mutates before its version proof")),
  );
});

test("authority fingerprints use one isolated report-only catalogue snapshot", () => {
  const dbTest = readFileSync(DB_TEST, "utf8");
  const runtimeRole = readFileSync(
    fileURLToPath(
      new URL("../crates/synveda-store/src/runtime_role.rs", import.meta.url),
    ),
    "utf8",
  );
  assert.deepEqual(authorityFingerprintFixtureFindings(dbTest, runtimeRole), []);

  for (const mutated of [
    dbTest.replace(
      "  authority-fingerprints|sqlx-prepare) fast_fixture=true ;;",
      "  sqlx-prepare) fast_fixture=true ;;",
    ),
    dbTest.replace(
      'if [ "$db_test_task" = authority-fingerprints ] && [ "$#" -ne 0 ]; then',
      'if [ "$db_test_task" = sqlx-prepare ] && [ "$#" -ne 0 ]; then',
    ),
    dbTest.replace(
      "      SYNVEDA_REPORT_AUTHORITY_FINGERPRINTS=1 \\\n",
      "      SYNVEDA_REPORT_AUTHORITY_FINGERPRINTS=2 \\\n",
    ),
    dbTest.replace(
      "      SYNVEDA_TEST_DATABASE_URL_FILE=$main_gateway_file \\\n",
      "      SYNVEDA_TEST_DATABASE_URL_FILE=$main_worker_file \\\n",
    ),
    dbTest.replace(
      "runtime_role::tests::report_live_catalog_fingerprints",
      "runtime_role::tests::live_catalog_fingerprints_match_the_revision_constants",
    ),
    dbTest.replace(
      '  if [ "$db_test_task" != authority-fingerprints ]; then',
      '  if [ "$db_test_task" = authority-fingerprints ]; then',
    ),
    dbTest.replace(
      '  if [ "$db_test_task" = authority-fingerprints ]; then\n',
      '  if [ "$db_test_task" = authority-fingerprints ]; then\n' +
        "    cargo check -p synveda-store\n",
    ),
    dbTest.replace(
      '  if [ "$db_test_task" = authority-fingerprints ]; then\n',
      '  if [ "$db_test_task" = authority-fingerprints ]; then\n' +
        "    cleanup_successful_fixture\n    exit 0\n",
    ),
  ]) {
    assert.notDeepEqual(authorityFingerprintFixtureFindings(mutated, runtimeRole), []);
  }

  for (const mutated of [
    runtimeRole.replace(
      '#[ignore = "run only through the isolated authority-fingerprint fixture"]',
      "",
    ),
    runtimeRole.replace(
      "        configure_authority_snapshot_connection(&mut authority)\n",
      "        verify_session_safety_connection(&mut authority)\n",
    ),
    runtimeRole.replace(
      '            Ok("1"),\n' +
        '            "the authority fingerprint reporter requires its exact harness gate"',
      '            Ok("yes"),\n' +
        '            "the authority fingerprint reporter requires its exact harness gate"',
    ),
    runtimeRole.replace(
      '        let roles = live_test_roles().expect("the exact database role contract is required");',
      '        let roles = DatabaseRoles::parse_json("{}").expect("unchecked roles");',
    ),
    runtimeRole.replace(
      '            "authority-fingerprints baseline_revision={} application_acl={application_acl} routine_catalog={routine_catalog} trigger_catalog={trigger_catalog} forced_rls={forced_rls}",',
      '            "authority-fingerprints baseline_revision={}",',
    ),
  ]) {
    assert.notDeepEqual(authorityFingerprintFixtureFindings(dbTest, mutated), []);
  }
});

test("the lifecycle wrong-cluster witness is genuine and leaves no peer state", () => {
  const dbTest = readFileSync(DB_TEST, "utf8");
  const compose = readFileSync(DB_TEST_COMPOSE, "utf8");
  assert.deepEqual(lifecyclePeerWitnessFindings(dbTest, compose), []);
  assert.ok(
    lifecyclePeerWitnessFindings(
      dbTest,
      compose.replace(
        '      SYNVEDA_DATABASE_REQUIRE_KEYCLOAK_PASSWORD: "true"',
        '      # removed complete credential-set requirement',
      ),
    ).some((finding) => finding.includes("Synveda bootstrap credential set")),
  );
  assert.ok(
    lifecyclePeerWitnessFindings(
      dbTest.replace(
        "database-bootstrap: database credentials must be pairwise distinct",
        "database-bootstrap: removed collision refusal",
      ),
      compose,
    ).some((finding) => finding.includes("credential-collision evidence")),
  );
  const mutateLifecycle = (mutator) => {
    const start = dbTest.indexOf(
      "# A wrong-cluster preflight negative control needs a genuine",
    );
    const end = dbTest.indexOf("# The lifecycle cluster must not already contain", start);
    assert.ok(start >= 0 && end > start, "lifecycle witness fixture is missing");
    return `${dbTest.slice(0, start)}${mutator(dbTest.slice(start, end))}${dbTest.slice(end)}`;
  };

  assert.ok(
    lifecyclePeerWitnessFindings(
      dbTest,
      compose.replaceAll(
        "source: ${SYNVEDA_DB_TEST_ROLES_FILE:?set SYNVEDA_DB_TEST_ROLES_FILE}",
        "source: ${SYNVEDA_DB_TEST_LIFECYCLE_ROLES_FILE:?set SYNVEDA_DB_TEST_LIFECYCLE_ROLES_FILE}",
      ),
    ).some((finding) => finding.includes("lifecycle Keycloak witness service")),
  );
  assert.ok(
    lifecyclePeerWitnessFindings(
      dbTest.replace(
        "compose run --rm --no-deps keycloak-database-bootstrap-lifecycle",
        "# removed lifecycle Keycloak convergence",
      ),
      compose,
    ).some((finding) => finding.includes("converged, retained and catalog-restored")),
  );
  assert.ok(
    lifecyclePeerWitnessFindings(
      mutateLifecycle((branch) =>
        branch.replace("drop role keycloak;", "-- retained role keycloak"),
      ),
      compose,
    ).some((finding) => finding.includes("converged, retained and catalog-restored")),
  );
  assert.ok(
    lifecyclePeerWitnessFindings(
      mutateLifecycle((branch) =>
        branch.replace("or grantor.rolname = 'keycloak'", "or false"),
      ),
      compose,
    ).some((finding) => finding.includes("converged, retained and catalog-restored")),
  );
  assert.ok(
    lifecyclePeerWitnessFindings(
      mutateLifecycle((branch) => {
        const proofStart = branch.indexOf("select 1 / case when not exists (");
        const proofEndMarker = ") then 1 else 0 end;\n";
        const proofEnd = branch.indexOf(proofEndMarker, proofStart) + proofEndMarker.length;
        assert.ok(proofStart >= 0 && proofEnd >= proofEndMarker.length);
        const proof = branch.slice(proofStart, proofEnd);
        return `${branch.slice(0, proofStart)}${branch
          .slice(proofEnd)
          .replace(
            "drop database keycloak with (force);",
            `drop database keycloak with (force);\n${proof}`,
          )}`;
      }),
      compose,
    ).some((finding) => finding.includes("converged, retained and catalog-restored")),
  );
  assert.ok(
    lifecyclePeerWitnessFindings(
      dbTest.replace(
        '[ "$lifecycle_peer_before" = "$lifecycle_peer_after" ] || {',
        'if [ -n "$lifecycle_peer_after" ]; then',
      ),
      compose,
    ).some((finding) => finding.includes("converged, retained and catalog-restored")),
  );
  assert.ok(
    lifecyclePeerWitnessFindings(
      dbTest.replaceAll(
        "main_port=$(published_port postgres-main)",
        "# retained stale published port",
      ),
      compose,
    ).some((finding) => finding.includes("refresh every dynamic-port database URL")),
  );
  for (const [before, after] of [
    [
      "\nwait_for_main_database_authority\n",
      "\nfalse && wait_for_main_database_authority\n",
    ],
    [
      '\nexpect_main_preflight_refusal stale-after-restart "$main_witness_file" "$peer_mismatch_error"\n',
      '\nfalse && expect_main_preflight_refusal stale-after-restart "$main_witness_file" "$peer_mismatch_error"\n',
    ],
  ]) {
    assert.ok(
      lifecyclePeerWitnessFindings(dbTest.replace(before, after), compose).some((finding) =>
        finding.includes("refresh every dynamic-port database URL"),
      ),
    );
  }
  for (const [before, after] of [
    ["wait_for_main_database_authority\n", "# removed authority readiness\n"],
    ['while [ "$attempt" -le 3 ]', 'while [ "$attempt" -le 4 ]'],
    ["-u SYNVEDA_DATABASE_PEER_WITNESS_FILE", "# inherited peer witness"],
    ["database target preflight complete", "unchecked readiness"],
    [
      "sleep 2\n    attempt=$((attempt + 1))",
      "sleep 20\n    attempt=$((attempt + 1))",
    ],
    [
      "sleep 2\n    attempt=$((attempt + 1))",
      "sleep 2\n    attempt=$((attempt - 1))",
    ],
  ]) {
    assert.ok(
      lifecyclePeerWitnessFindings(dbTest.replace(before, after), compose).some((finding) =>
        finding.includes("post-restart") || finding.includes("refresh every dynamic-port"),
      ),
    );
  }
  assert.ok(
    lifecyclePeerWitnessFindings(
      dbTest,
      compose.replace(
        "  secrets:\n    - source: postgres_owner_password",
        "  secrets:\n    - source: external_provider_password\n      target: external_provider_password\n    - source: postgres_owner_password",
      ),
    ).some((finding) => finding.includes("main PostgreSQL inherits")),
  );
  assert.ok(
    lifecyclePeerWitnessFindings(
      dbTest,
      compose.replace(
        "      - source: external_provider_password\n        target: external_provider_password",
        "      # removed external-provider lifecycle mount",
      ),
    ).some((finding) => finding.includes("two lifecycle consumers")),
  );
  assert.ok(
    lifecyclePeerWitnessFindings(
      dbTest.replace(
        "external_provider_password=$(openssl rand -hex 32)",
        "external_provider_password=$(<\"$secret_dir/keycloak_admin_password\")",
      ),
      compose,
    ).some((finding) => finding.includes("generated independently")),
  );
  for (const marker of [
    "secret_dir=$state_dir/generator/$generator_project/secrets",
    'mkdir -p "$state_dir/generator/$generator_project"',
    "SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-$state_token",
    "SYNVEDA_DATABASE_AUTHORITY_DIR=$state_dir/generator/$generator_project/database-authority",
    "SYNVEDA_KEYCLOAK_PUBLIC_GATE_DIR=$state_dir/generator/$generator_project/keycloak-public-gate",
  ]) {
    assert.ok(
      lifecyclePeerWitnessFindings(dbTest.replace(marker, "REMOVED_GENERATOR_MARKER"), compose).some(
        (finding) => finding.includes("secret-generator state is not fixture-local"),
      ),
      `fixture-local generator marker escaped: ${marker}`,
    );
  }
});

test("post-restart database readiness classifier is closed and byte-exact", () => {
  const dbTest = readFileSync(DB_TEST, "utf8");
  const classifier = shellFunctionSource(
    dbTest,
    "classify_main_database_authority_preflight",
  );
  const scratch = mkdtempSync(join(tmpdir(), "synveda-preflight-classifier-"));
  const harness = join(scratch, "classify.sh");
  const stdoutFile = join(scratch, "stdout");
  const stderrFile = join(scratch, "stderr");
  writeFileSync(
    harness,
    `#!/usr/bin/env bash\nset -u\n${classifier}\nclassify_main_database_authority_preflight "$@" || exit $?\n`,
    { mode: 0o700 },
  );

  const classify = (status, stdout, stderr) => {
    writeFileSync(stdoutFile, stdout);
    writeFileSync(stderrFile, stderr);
    return spawnSync("bash", [harness, String(status), stdoutFile, stderrFile], {
      encoding: "utf8",
      timeout: SUBPROCESS_TIMEOUT_MS,
    }).status;
  };

  try {
    assert.equal(classify(0, "", "database target preflight complete\n"), 0);
    for (const error of [
      "synveda: SYNVEDA_MIGRATOR_DATABASE_URL_FILE connection failed",
      "synveda: SYNVEDA_MIGRATOR_DATABASE_URL_FILE preflight timed out",
      "synveda: SYNVEDA_MIGRATOR_DATABASE_URL_FILE authority or writable-target verification failed",
    ]) {
      assert.equal(classify(1, "", `${error}\n`), 75, error);
    }
    assert.equal(classify(0, "unexpected\n", "database target preflight complete\n"), 1);
    assert.equal(classify(1, "", "database target preflight complete\n"), 1);
    assert.equal(
      classify(
        1,
        "",
        "synveda: SYNVEDA_MIGRATOR_DATABASE_URL_FILE connection failed\nextra\n",
      ),
      1,
    );
    assert.equal(classify(1, "", "synveda: unknown failure\n"), 1);
    assert.equal(
      classify(101, "", "synveda: SYNVEDA_MIGRATOR_DATABASE_URL_FILE connection failed\n"),
      1,
    );
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("post-restart database readiness wait is bounded and fail-closed", () => {
  const dbTest = readFileSync(DB_TEST, "utf8");
  const classifier = shellFunctionSource(
    dbTest,
    "classify_main_database_authority_preflight",
  );
  const wait = shellFunctionSource(dbTest, "wait_for_main_database_authority");
  const scratch = mkdtempSync(join(tmpdir(), "synveda-preflight-wait-"));
  const harness = join(scratch, "wait.sh");
  const attemptsFile = join(scratch, "attempts");
  writeFileSync(
    harness,
    `#!/usr/bin/env bash
set -u
state_dir=$1
scenario=$2
attempt_file=$state_dir/attempts
private_evidence_file() { : > "$1"; }
assert_database_secrets_absent() { :; }
sleep() { :; }
run_main_database_authority_preflight() {
  local count=0
  if [ -s "$attempt_file" ]; then count=$(<"$attempt_file"); fi
  count=$((count + 1))
  printf '%s\n' "$count" > "$attempt_file"
  case "$scenario" in
    ready)
      printf '%s\n' 'database target preflight complete' >&2
      return 0
      ;;
    retry-ready)
      if [ "$count" -eq 3 ]; then
        printf '%s\n' 'database target preflight complete' >&2
        return 0
      fi
      ;;
    retry-exhausted)
      ;;
    invalid)
      printf '%s\n' 'synveda: unknown failure' >&2
      return 1
      ;;
    *)
      return 97
      ;;
  esac
  printf '%s\n' 'synveda: SYNVEDA_MIGRATOR_DATABASE_URL_FILE connection failed' >&2
  return 1
}
${classifier}${wait}wait_for_main_database_authority
`,
    { mode: 0o700 },
  );

  const run = (scenario) => {
    rmSync(attemptsFile, { force: true });
    const result = spawnSync("bash", [harness, scratch, scenario], {
      encoding: "utf8",
      timeout: SUBPROCESS_TIMEOUT_MS,
    });
    return {
      ...result,
      attempts: Number(readFileSync(attemptsFile, "utf8").trim()),
    };
  };

  try {
    const ready = run("ready");
    assert.equal(ready.status, 0, ready.stderr);
    assert.equal(ready.stdout, "");
    assert.equal(ready.stderr, "");
    assert.equal(ready.attempts, 1);

    const recovered = run("retry-ready");
    assert.equal(recovered.status, 0, recovered.stderr);
    assert.equal(recovered.stdout, "");
    assert.equal(recovered.stderr, "");
    assert.equal(recovered.attempts, 3);

    const exhausted = run("retry-exhausted");
    assert.equal(exhausted.status, 1);
    assert.equal(exhausted.stdout, "");
    assert.equal(
      exhausted.stderr,
      "db-test: post-restart database authority readiness did not converge\n",
    );
    assert.equal(exhausted.attempts, 3);

    const invalid = run("invalid");
    assert.equal(invalid.status, 1);
    assert.equal(invalid.stdout, "");
    assert.equal(
      invalid.stderr,
      "db-test: post-restart database authority readiness returned an invalid response\n",
    );
    assert.equal(invalid.attempts, 1);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("evaluation signal traps preserve status and clean exactly once", () => {
  const run = readFileSync(EVAL_RUN, "utf8");
  const longmemeval = readFileSync(EVAL_LONGMEMEVAL_RUN, "utf8");
  assert.deepEqual(evalSignalTrapFindings(run, longmemeval), []);
  assert.ok(
    evalSignalTrapFindings(
      run.replace("trap 'eval_finish 143' TERM", "trap eval_down TERM"),
      longmemeval,
    ).some((finding) => finding.includes("evaluation lacks trap 'eval_finish 143' TERM")),
  );

  const scratch = mkdtempSync(join(tmpdir(), "synveda-eval-signal-"));
  const state = join(scratch, "synveda-eval.signal");
  const result = spawnSync(
    "bash",
    [
      "-c",
      `set -eu
. "$1"
mkdir "$2"
EVAL_STATE=$2
EVAL_STATE_PARENT=$3
EVAL_STATE_OWNED=1
trap 'eval_finish $?' EXIT
trap 'eval_finish 130' INT
trap 'eval_finish 143' TERM
kill -TERM $$
exit 91
`,
      "bash",
      EVAL_LIB,
      state,
      scratch,
    ],
    { encoding: "utf8" },
  );
  try {
    assert.equal(result.status, 143, result.stderr);
    assert.equal(result.signal, null);
    assert.equal(readFileSync(EVAL_LIB, "utf8").includes("trap - EXIT INT TERM"), true);
    assert.equal(existsSync(state), false);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("evaluation preflight never trusts inherited cleanup targets", () => {
  const result = spawnSync(
    "bash",
    [
      "-c",
      `set -eu
sentinel=$(mktemp -d)
touch "$sentinel/keep"
sleep 30 &
victim=$!
trap 'kill "$victim" 2>/dev/null || true; wait "$victim" 2>/dev/null || true; rm -R -- "$sentinel"' EXIT
EVAL_STATE=$sentinel
EVAL_STATE_PARENT=/
EVAL_STATE_OWNED=1
EVAL_PID=$victim
EVAL_WORKER_PID=$victim
EVAL_SEED_PID=$victim
. "$1"
if eval_up >/dev/null 2>&1; then exit 91; fi
eval_down
kill -0 "$victim"
test -f "$sentinel/keep"
test -z "$EVAL_STATE"
test -z "$EVAL_PID"
`,
      "bash",
      EVAL_LIB,
    ],
    { encoding: "utf8" },
  );
  assert.equal(result.status, 0, result.stderr);
});

test("evaluation HTTP, child readiness and scratch cleanup fail closed", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-eval-runtime-"));
  const bin = join(scratch, "bin");
  mkdirSync(bin, { mode: 0o700 });
  writeFileSync(
    join(bin, "curl"),
    `#!/bin/sh
printf '%s\n' "$@" > "$EVAL_CURL_ARGS"
cat > "$EVAL_CURL_STDIN"
printf ok
`,
    { mode: 0o700 },
  );
  writeFileSync(
    join(bin, "cp"),
    `#!/bin/sh
: > "$EVAL_CP_CALLED"
exit 1
`,
    { mode: 0o700 },
  );
  const result = spawnSync(
    "bash",
    [
      "-c",
      `set -eu
. "$1"
PATH=$2:$PATH
export PATH
EVAL_CURL_ARGS=$3/curl.args
EVAL_CURL_STDIN=$3/curl.stdin
EVAL_CP_CALLED=$3/cp.called
export EVAL_CURL_ARGS EVAL_CURL_STDIN EVAL_CP_CALLED
listener=
victim=
cleanup_runtime() {
  test -z "$listener" || kill -KILL "$listener" 2>/dev/null || true
  test -z "$victim" || kill -KILL "$victim" 2>/dev/null || true
}
trap cleanup_runtime EXIT

for fixture_name in \
  SYNVEDA_DB_TEST_MAIN_DATA_NETWORK SYNVEDA_DB_TEST_LIFECYCLE_DATA_NETWORK \
  SYNVEDA_DB_TEST_MAIN_HOST_NETWORK SYNVEDA_DB_TEST_LIFECYCLE_HOST_NETWORK \
  SYNVEDA_DB_TEST_MAIN_DATA_SUBNET SYNVEDA_DB_TEST_LIFECYCLE_DATA_SUBNET \
  SYNVEDA_DB_TEST_MAIN_HOST_SUBNET SYNVEDA_DB_TEST_LIFECYCLE_HOST_SUBNET \
  SYNVEDA_DB_TEST_ROLES_FILE SYNVEDA_DB_TEST_LIFECYCLE_ROLES_FILE \
  SYNVEDA_DB_TEST_EXTERNAL_ROLES_FILE SYNVEDA_DB_TEST_MAIN_AUTHORITY_DIR \
  SYNVEDA_DB_TEST_LIFECYCLE_AUTHORITY_DIR SYNVEDA_DB_TEST_UID \
  SYNVEDA_DB_TEST_GID SYNVEDA_DB_TEST_SECRETS_DIR \
  SYNVEDA_DB_TEST_POSTGRES_IMAGE SYNVEDA_DB_TEST_TASK; do
  export "$fixture_name=fixture-control-sentinel"
done
eval_clear_db_test_environment
if env | grep '^SYNVEDA_DB_TEST_' >/dev/null; then exit 90; fi

token=header.payload_signature
test "$(eval_bearer_curl "$token" -fsS http://127.0.0.1:8150/test)" = ok
test "$(sed -n '1p' "$EVAL_CURL_ARGS")" = --disable
if grep -F "$token" "$EVAL_CURL_ARGS" >/dev/null; then exit 91; fi
grep -F "Authorization: Bearer $token" "$EVAL_CURL_STDIN" >/dev/null

ready=$3/term-ready
sh -c 'trap "" TERM; : > "$1"; while :; do sleep 1; done' sh "$ready" &
victim=$!
tries=0
while [ ! -f "$ready" ] && [ "$tries" -lt 50 ]; do
  tries=$((tries + 1))
  sleep 0.02
done
test -f "$ready"
eval_stop_pids_with_grace 2 "$victim"
if kill -0 "$victim" 2>/dev/null; then exit 92; fi
victim=

port_file=$3/listener.port
python3 -c 'import socket,sys,time
s=socket.socket(socket.AF_INET,socket.SOCK_STREAM)
s.bind(("127.0.0.1",0)); s.listen()
open(sys.argv[1],"w",encoding="ascii").write(str(s.getsockname()[1]))
time.sleep(30)' "$port_file" &
listener=$!
tries=0
while [ ! -s "$port_file" ] && [ "$tries" -lt 50 ]; do
  tries=$((tries + 1))
  sleep 0.02
done
test -s "$port_file"
listener_port=$(cat "$port_file")
if eval_port_free "http://127.0.0.1:$listener_port" >/dev/null 2>&1; then exit 93; fi
kill "$listener" 2>/dev/null || true
wait "$listener" 2>/dev/null || true
listener=

dead_log=$3/dead.log
: > "$dead_log"
sh -c 'exit 0' &
dead=$!
wait "$dead" || true
if eval_wait_gateway http://127.0.0.1:8150 "$dead" "$dead_log" >/dev/null 2>&1; then
  exit 94
fi

report_dir=$3/report
mkdir "$report_dir"
state=$3/synveda-eval.no-logs
mkdir "$state"
EVAL_STATE=$state
EVAL_STATE_PARENT=$3
EVAL_STATE_OWNED=1
EVAL_REPORT=$report_dir/report.json
eval_down
test ! -e "$state"

state=$3/synveda-eval.copy-failure
mkdir "$state"
printf diagnostic > "$state/gateway.log"
EVAL_STATE=$state
EVAL_STATE_PARENT=$3
EVAL_STATE_OWNED=1
eval_down
test -f "$EVAL_CP_CALLED"
test ! -e "$state"

state=$3/synveda-eval.replaced
EVAL_STATE=$state
EVAL_STATE_PARENT=$3
EVAL_STATE_OWNED=1
if eval_down >/dev/null 2>&1; then exit 95; fi
test "$EVAL_STATE" = "$state"
`,
      "bash",
      EVAL_LIB,
      bin,
      scratch,
    ],
    { encoding: "utf8", timeout: 10000 },
  );
  try {
    assert.equal(result.status, 0, result.stderr);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("a scratch deletion failure makes a passing evaluation fail", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-eval-rm-failure-"));
  const bin = join(scratch, "bin");
  const state = join(scratch, "synveda-eval.rm-failure");
  mkdirSync(bin, { mode: 0o700 });
  mkdirSync(state, { mode: 0o700 });
  writeFileSync(join(state, "kms.key"), "secret material\n", { mode: 0o600 });
  writeFileSync(
    join(bin, "rm"),
    `#!/bin/sh
exit 73
`,
    { mode: 0o700 },
  );
  const result = spawnSync(
    "bash",
    [
      "-c",
      `set -eu
. "$1"
PATH=$2:$PATH
export PATH
EVAL_STATE=$3
EVAL_STATE_PARENT=$4
EVAL_STATE_OWNED=1
eval_finish 0
`,
      "bash",
      EVAL_LIB,
      bin,
      state,
      scratch,
    ],
    { encoding: "utf8" },
  );
  try {
    assert.equal(result.status, 1, result.stderr);
    assert.equal(existsSync(join(state, "kms.key")), true);
    assert.match(result.stderr, /failed to remove owned scratch state/);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("evaluation accepts only three distinct canonical loopback URLs", () => {
  const result = spawnSync(
    "bash",
    [
      "-c",
      `set -eu
. "$1"
test "$(eval_loopback_port http://127.0.0.1:8150 gateway)" = 8150
for bad in \
  http://localhost:8150 \
  http://0.0.0.0:8150 \
  http://user@127.0.0.1:8150 \
  http://127.0.0.1:0 \
  http://127.0.0.1:08150 \
  http://127.0.0.1:65536 \
  http://127.0.0.1:8150/path \
  'http://127.0.0.1:8150?query' \
  'http://127.0.0.1:8150#fragment'; do
  if eval_loopback_port "$bad" hostile >/dev/null 2>&1; then exit 92; fi
done
EVAL_GATEWAY_URL=http://127.0.0.1:8150
EVAL_SEED_URL=http://127.0.0.1:8150
EVAL_WORKER_URL=http://127.0.0.1:8152
if eval_up >/dev/null 2>duplicate.err; then exit 93; fi
grep -F 'ports must be distinct' duplicate.err >/dev/null
rm -f duplicate.err
`,
      "bash",
      EVAL_LIB,
    ],
    { encoding: "utf8" },
  );
  assert.equal(result.status, 0, result.stderr);
});
