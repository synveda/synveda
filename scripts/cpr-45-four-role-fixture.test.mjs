import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  runFourRoleFixture,
  runPartialStartFailureFixture,
  validateFixtureGraph,
} from "./fixtures/cpr-45-four-role/harness.mjs";
import {
  EVIDENCE_CLASS,
  MAX_REQUEST_LIFETIME_MILLISECONDS,
  MAX_SOCKET_PATH_BYTES,
  assertPathInRoot,
  remainingDeadlineMilliseconds,
} from "./fixtures/cpr-45-four-role/protocol.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const FIXTURE_ROOT = join(ROOT, "scripts/fixtures/cpr-45-four-role");

function graphSample() {
  const admissionSha256 = "a".repeat(64);
  const hostagentEdgeSha256 = "b".repeat(64);
  const usernetEdgeSha256 = "c".repeat(64);
  const sshEdgeSha256 = "d".repeat(64);
  const operationId = "9".repeat(64);
  const fixtureToolchain = {
    node_sha256: "5".repeat(64),
    protocol_sha256: "6".repeat(64),
    role_sha256: "7".repeat(64),
  };
  const makeIdentity = (role, depth, pid, launchEdgeSha256, identitySha256) => ({
    argv_sha256: "8".repeat(64),
    cwd_identity_sha256: "8".repeat(64),
    depth,
    endpoint_identity: {
      dev: "1",
      ino: String(1_000 + pid),
      mode: 0o600,
      nlink: 1,
      uid: process.getuid(),
    },
    environment_sha256: "8".repeat(64),
    evidence_class: EVIDENCE_CLASS,
    identity_scope: "trusted-fixture-nonce-not-os-start-identity",
    identity_sha256: identitySha256,
    launch_edge_sha256: launchEdgeSha256,
    launch_nonce_sha256: "8".repeat(64),
    operation_id: operationId,
    pid,
    ppid_at_start: pid - 1,
    process_nonce_sha256: "8".repeat(64),
    role,
    schema: "synveda.fixture.cpr45-four-role-identity.v1",
    toolchain: fixtureToolchain,
    uid: process.getuid(),
  });
  const identities = [
    makeIdentity("outer", 0, 101, admissionSha256, "1".repeat(64)),
    makeIdentity("hostagent", 1, 102, hostagentEdgeSha256, "2".repeat(64)),
    makeIdentity("usernet", 2, 103, usernetEdgeSha256, "3".repeat(64)),
    makeIdentity(
      "ssh-controlmaster",
      2,
      104,
      sshEdgeSha256,
      "4".repeat(64),
    ),
  ];
  const makeEdgeBody = ({
    childRole,
    depth,
    parentIdentitySha256,
    parentRole,
    priorEdgeSha256,
    schema,
    sequence,
  }) => ({
    argv_sha256: "8".repeat(64),
    child_bootstrap_sha256: "8".repeat(64),
    child_launch_nonce_sha256: "8".repeat(64),
    child_role: childRole,
    child_role_key_sha256: "8".repeat(64),
    cwd_identity_sha256: "8".repeat(64),
    depth,
    edge_key_sha256: "8".repeat(64),
    environment_sha256: "8".repeat(64),
    evidence_class: EVIDENCE_CLASS,
    fence_key_sha256: "8".repeat(64),
    operation_id: operationId,
    parent_identity_sha256: parentIdentitySha256,
    parent_role: parentRole,
    prior_edge_sha256: priorEdgeSha256,
    schema,
    sequence,
    toolchain: fixtureToolchain,
  });
  return {
    admission: {
      body: makeEdgeBody({
        childRole: "outer",
        depth: 0,
        parentIdentitySha256: "0".repeat(64),
        parentRole: "fixture-supervisor",
        priorEdgeSha256: null,
        schema: "synveda.fixture.cpr45-four-role-admission.v1",
        sequence: 0,
      }),
      sha256: admissionSha256,
    },
    edges: [
      {
        body: makeEdgeBody({
          childRole: "hostagent",
          depth: 1,
          parentIdentitySha256: identities[0].identity_sha256,
          parentRole: "outer",
          priorEdgeSha256: admissionSha256,
          schema: "synveda.fixture.cpr45-four-role-launch-edge.v1",
          sequence: 1,
        }),
        sha256: hostagentEdgeSha256,
      },
      {
        body: makeEdgeBody({
          childRole: "usernet",
          depth: 2,
          parentIdentitySha256: identities[1].identity_sha256,
          parentRole: "hostagent",
          priorEdgeSha256: hostagentEdgeSha256,
          schema: "synveda.fixture.cpr45-four-role-launch-edge.v1",
          sequence: 2,
        }),
        sha256: usernetEdgeSha256,
      },
      {
        body: makeEdgeBody({
          childRole: "ssh-controlmaster",
          depth: 2,
          parentIdentitySha256: identities[1].identity_sha256,
          parentRole: "hostagent",
          priorEdgeSha256: usernetEdgeSha256,
          schema: "synveda.fixture.cpr45-four-role-launch-edge.v1",
          sequence: 3,
        }),
        sha256: sshEdgeSha256,
      },
    ],
    identities,
  };
}

test("the standalone fixture proves four-role detach, quiescence and cooperative shutdown", async () => {
  const result = await runFourRoleFixture();
  assert.deepEqual(result, {
    causal_graph: {
      causal_graph_sha256: result.causal_graph.causal_graph_sha256,
      edge_count: 3,
      launch_edge_head_sha256:
        result.causal_graph.launch_edge_head_sha256,
      maximum_depth: 2,
      node_count: 4,
    },
    cooperative_shutdown: true,
    detached_hostagent_reparent_observed: true,
    evidence_class: EVIDENCE_CLASS,
    lifecycle_authority: false,
    live_provider_evidence: false,
    operation_id: result.operation_id,
    provider_effect_evidence: false,
    refusal_checks: 6,
    stable_samples: {
      before_fence_roles: 4,
      detached_roles: 3,
      quiesced_roles: 4,
    },
    state_integration: false,
  });
  assert.match(result.operation_id, /^[0-9a-f]{64}$/u);
  assert.match(result.causal_graph.causal_graph_sha256, /^[0-9a-f]{64}$/u);
  assert.match(result.causal_graph.launch_edge_head_sha256, /^[0-9a-f]{64}$/u);
});

test("subtree deadlines admit bounded delayed recursive startup", async () => {
  const result = await runFourRoleFixture({
    exerciseRefusals: false,
    startupDelayMilliseconds: 75,
  });
  assert.equal(result.cooperative_shutdown, true);
  assert.equal(result.refusal_checks, 0);
  assert.equal(result.state_integration, false);
});

test("a partial hostagent start cleans only recursively tracked child handles", async () => {
  assert.deepEqual(await runPartialStartFailureFixture(), {
    cleanup_verified: true,
    evidence_class: EVIDENCE_CLASS,
    failure_point: "hostagent-after-first-child-ready",
    frontier_authenticated: true,
    success_evidence: false,
  });
});

test("the graph validator accepts only the closed four-role structural graph", () => {
  const valid = graphSample();
  const graph = validateFixtureGraph(valid);
  assert.deepEqual(graph, {
    causal_graph_sha256: graph.causal_graph_sha256,
    edge_count: 3,
    launch_edge_head_sha256: "d".repeat(64),
    maximum_depth: 2,
    node_count: 4,
  });

  const mutations = [
    (value) => value.identities.push({ ...value.identities[3], pid: 105 }),
    (value) => {
      value.identities[3].role = "usernet";
    },
    (value) => value.edges.pop(),
    (value) => value.edges.reverse(),
    (value) => {
      value.edges[0].body.parent_role = "hostagent";
    },
    (value) => {
      value.edges[1].body.prior_edge_sha256 = "f".repeat(64);
    },
    (value) => {
      value.edges[2].body.child_role = "outer";
    },
    (value) => {
      value.edges[0].body.padding = "x".repeat(33 * 1024);
    },
  ];
  for (const mutate of mutations) {
    const invalid = structuredClone(valid);
    mutate(invalid);
    assert.throws(() => validateFixtureGraph(invalid), /fixture/u);
  }
});

test("socket bounds count UTF-8 bytes rather than characters", () => {
  const root = "/tmp";
  const prefix = `${root}/`;
  const exact = `${prefix}${"a".repeat(MAX_SOCKET_PATH_BYTES - prefix.length)}`;
  assert.equal(Buffer.byteLength(exact, "utf8"), MAX_SOCKET_PATH_BYTES);
  assert.doesNotThrow(() => assertPathInRoot(exact, root, true));
  assert.throws(
    () => assertPathInRoot(`${exact}a`, root, true),
    /byte bound/u,
  );
  assert.throws(
    () =>
      assertPathInRoot(
        `${prefix}${"a".repeat(MAX_SOCKET_PATH_BYTES - prefix.length - 1)}é`,
        root,
        true,
      ),
    /byte bound/u,
  );
});

test("recursive roles cannot reset or extend the signed request deadline", () => {
  const deadline = Date.now() + 1_000;
  const remaining = remainingDeadlineMilliseconds(deadline);
  assert.ok(remaining > 0 && remaining <= 1_000);
  assert.throws(
    () => remainingDeadlineMilliseconds(Date.now() - 1),
    /deadline/u,
  );
  assert.throws(
    () =>
      remainingDeadlineMilliseconds(
        Date.now() + MAX_REQUEST_LIFETIME_MILLISECONDS + 1,
      ),
    /deadline/u,
  );

  const roleSource = readFileSync(join(FIXTURE_ROOT, "role.mjs"), "utf8");
  assert.doesNotMatch(roleSource, /Date\.now\(/u);
  assert.match(roleSource, /deadline_epoch_milliseconds/u);
  assert.match(roleSource, /remainingDeadlineMilliseconds/u);
  assert.match(roleSource, /body\.pid !== child\.pid/u);
});

test("the four-role fixture remains structurally outside deployment authority", () => {
  const fixturePaths = [
    join(FIXTURE_ROOT, "protocol.mjs"),
    join(FIXTURE_ROOT, "role.mjs"),
    join(FIXTURE_ROOT, "harness.mjs"),
  ];
  const fixtureSource = fixturePaths
    .map((path) => readFileSync(path, "utf8"))
    .join("\n");
  assert.doesNotMatch(
    fixtureSource,
    /deploy\/compose|controlled-background|clean-engine-state|clean-engine-receipts|provider-process-contract|provider-adapter-registry|live-provider|mutation-journal|\.synveda-clean-engine-provider-reservation|COLIMA_LIVE_|\/usr\/bin\/ssh|\bDocker\b|\bColima\b|\bLima\b/u,
  );
  assert.doesNotMatch(fixtureSource, /shell:\s*true|execFile\(|\bexec\(/u);
  assert.doesNotMatch(fixtureSource, /\b(?:import\s*\(|require\s*\()/u);
  assert.doesNotMatch(
    fixtureSource,
    /stderr\.write\([^)]*(edge_key|fence_key|launch_nonce|role_key)/u,
  );
  assert.match(
    readFileSync(join(FIXTURE_ROOT, "role.mjs"), "utf8"),
    /process\.stderr\.write\(`four-role fixture \$\{role\} failed\\n`\)/u,
  );
  const spawnCalls = [...fixtureSource.matchAll(/\bspawn\s*\(/gu)];
  const nodeSpawnCalls = [
    ...fixtureSource.matchAll(/\bspawn\s*\(\s*process\.execPath/gu),
  ];
  assert.equal(spawnCalls.length, 2);
  assert.equal(nodeSpawnCalls.length, spawnCalls.length);
  assert.match(fixtureSource, /shell:\s*false/u);

  for (const path of fixturePaths) {
    const source = readFileSync(path, "utf8");
    for (const match of source.matchAll(/from\s+["']([^"']+)["']/gu)) {
      assert.ok(
        match[1].startsWith("node:") || match[1] === "./protocol.mjs",
        `${path} imported ${match[1]}`,
      );
    }
  }
});
