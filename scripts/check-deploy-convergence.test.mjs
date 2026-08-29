import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

import {
  contributorPostgresBuildFindings,
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
    "usage: synveda-container {gateway|worker|database-preflight|migrate|probe {gateway|worker} {live|ready}}\n",
  );
});

test("the product launcher dispatches every implemented role exactly", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-product-launcher-"));
  const launcher = join(scratch, "synveda-container");
  try {
    const instrumented = readFileSync(PRODUCT_LAUNCHER, "utf8")
      .replace("exec /usr/local/bin/synveda-gateway", "exec /bin/echo gateway")
      .replace("exec /usr/local/bin/synveda-worker", "exec /bin/echo worker")
      .replace("exec /usr/local/bin/synveda db preflight", "exec /bin/echo database-preflight")
      .replace("exec /usr/local/bin/synveda db migrate", "exec /bin/echo migrate")
      .replace("exec /usr/bin/curl \\\n", "exec /bin/echo curl \\\n");
    writeFileSync(launcher, instrumented);

    const cases = [
      [["gateway"], "gateway\n"],
      [["worker"], "worker\n"],
      [["database-preflight"], "database-preflight\n"],
      [["migrate"], "migrate\n"],
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
      dbTest.replace(
        "cargo sqlx migrate run --no-dotenv",
        "cargo run -q -p synveda-cli --bin synveda -- db migrate",
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
      dbTest.replace("sqlx_cli_banner=$(cargo sqlx --version)", "sqlx_cli_banner=unchecked"),
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
  const delayedVersionProbe = dbTest
    .replace("    sqlx_cli_banner=$(cargo sqlx --version)\n", "")
    .replace(
      "    unset sqlx_library_version sqlx_cli_banner\n",
      "    sqlx_cli_banner=$(cargo sqlx --version)\n" +
        "    unset sqlx_library_version sqlx_cli_banner\n",
    );
  assert.ok(
    sqlxPrepareFixtureFindings(delayedVersionProbe).some((finding) =>
      finding.includes("mutates before its version proof"),
    ),
  );
  assert.ok(
    sqlxPrepareFixtureFindings(
      dbTest.replace(
        '        echo "db-test: cargo-sqlx must exactly match the locked sqlx library" >&2\n' +
          "        exit 69",
        '        echo "db-test: cargo-sqlx must exactly match the locked sqlx library" >&2\n' +
          "        :",
      ),
    ).some((finding) => finding.includes("mutates before its version proof")),
  );
  assert.ok(
    sqlxPrepareFixtureFindings(
      dbTest.replace(
        '      && [ "$sqlx_cli_banner" = "sqlx-cli-sqlx $sqlx_library_version" ] || {',
        '      && [ "$sqlx_cli_banner" = "sqlx-cli-sqlx $sqlx_library_version" ] && {',
      ),
    ).some((finding) => finding.includes("mutates before its version proof")),
  );
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
