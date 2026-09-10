import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { PassThrough } from "node:stream";
import { fileURLToPath } from "node:url";
import test from "node:test";

import {
  BrowserContractError,
  validateSettings,
} from "../deploy/compose/browser/console-login-contract.mjs";
import {
  allowedCliRequest,
  validateCliHandoffUrl,
  validateCliLoginUrl,
  validateDemoReceipt,
  validateDemoStatus,
} from "../deploy/compose/browser/product-demo-contract.mjs";
import { runProductAcceptance } from "../deploy/compose/browser/product-demo-runner.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SETTINGS = validateSettings(
  "http://app.synveda.test:8080",
  "http://auth.synveda.test:8080/realms/synveda",
);
const STATE = "s".repeat(43);
const REDIRECT = "http://127.0.0.1:43127/callback";
const RESOURCE_NAMES = [
  "workspace",
  "project",
  "bob_invite",
  "bob_member",
  "bob_principal",
  "first_session",
  "first_events",
  "first_capture",
  "first_candidates",
  "webhook_knowledge",
  "private_knowledge",
  "reuse_session",
  "reuse_context",
  "private_isolation",
  "current_session",
  "current_context",
  "release_skill",
];
const STATUS_NAMES = [
  "workspace",
  "project",
  "first_session",
  "first_capture",
  "reuse_session",
  "current_session",
  "webhook_knowledge",
  "private_knowledge",
  "reuse_context",
  "current_context",
];

const uuid = (value) =>
  `019b53c0-7c00-7${value.toString(16).padStart(3, "0")}-8000-${value
    .toString(16)
    .padStart(12, "0")}`;

function startUrl(overrides = {}, extra = []) {
  const value = new URL(`${SETTINGS.appOrigin}/auth/login`);
  for (const [name, entry] of Object.entries({
    cli_redirect_uri: REDIRECT,
    cli_state: STATE,
    issuer: SETTINGS.issuer,
    ...overrides,
  })) {
    if (entry !== undefined) value.searchParams.append(name, entry);
  }
  for (const [name, entry] of extra) value.searchParams.append(name, entry);
  return value.href;
}

function refuse(operation, stage) {
  assert.throws(operation, (error) => {
    assert.ok(error instanceof BrowserContractError);
    assert.equal(error.stage, stage);
    return true;
  });
}

function receipt() {
  const ids = {
    workspace: uuid(1),
    workspaceScope: uuid(2),
    project: uuid(3),
    projectScope: uuid(4),
    invite: uuid(5),
    grant: uuid(6),
    firstSession: uuid(7),
    firstCapture: uuid(8),
    firstEvent: uuid(9),
    privateEvent: uuid(10),
    webhookCandidate: uuid(11),
    privateCandidate: uuid(12),
    webhookKnowledge: uuid(13),
    webhookRevision: uuid(14),
    privateKnowledge: uuid(15),
    privateRevision: uuid(16),
    reuseSession: uuid(17),
    reuseContext: uuid(18),
    currentSession: uuid(19),
    currentContext: uuid(20),
    releaseSkill: uuid(24),
    releaseSkillVersion: uuid(25),
    releaseSkillValidation: uuid(26),
    releaseSkillTestRun: uuid(27),
  };
  const workspace = {
    id: ids.workspace,
    scope_id: ids.workspaceScope,
    slug: "pulseboard-demo",
    status: "active",
  };
  const project = {
    id: ids.project,
    workspace_id: ids.workspace,
    scope_id: ids.projectScope,
    slug: "delivery-api",
    status: "active",
  };
  const session = (id, principal) => ({
    id,
    workspace_id: ids.workspace,
    project_id: ids.project,
    scope_id: ids.projectScope,
    principal_id: principal,
    client_name: "synveda-demo",
    status: "active",
  });
  const context = (id, sessionId, rendered) => ({
    id,
    session_id: sessionId,
    workspace_id: ids.workspace,
    project_id: ids.project,
    scope_id: ids.projectScope,
    completion_status: "completed",
    selection_count: 1,
    rendered,
  });
  return {
    receipt_version: 1,
    profile: "team",
    state: "active",
    actor_subject: "alice-subject",
    resources: {
      workspace,
      project,
      bob_invite: {
        invite: {
          id: ids.invite,
          scope_id: ids.workspaceScope,
          role: "member",
          status: "pending",
        },
      },
      bob_member: {
        scope_id: ids.workspaceScope,
        grant: {
          id: ids.grant,
          scope_id: ids.workspaceScope,
          subject_kind: "principal",
          principal_id: "bob-subject",
          role: "member",
          source: "invite",
          invite_id: ids.invite,
          directory_managed: false,
        },
      },
      bob_principal: { subject: "bob-subject", credential_profile: "bob" },
      first_session: session(ids.firstSession, "alice-subject"),
      first_events: {
        events: [ids.firstEvent, ids.privateEvent, uuid(21), uuid(22)].map(
          (id) => ({ outcome: "appended", event: { id } }),
        ),
      },
      first_capture: {
        id: ids.firstCapture,
        source_kind: "session",
        session_id: ids.firstSession,
        project_id: ids.project,
        scope_id: ids.projectScope,
        state: "completed",
        event_count: 4,
        candidate_count: 2,
      },
      first_candidates: {
        candidates: [
          {
            id: ids.webhookCandidate,
            batch_id: ids.firstCapture,
            session_id: ids.firstSession,
            proposed_scope_id: ids.projectScope,
            proposed_project_id: ids.project,
            source_event_ids: [ids.firstEvent],
          },
          {
            id: ids.privateCandidate,
            batch_id: ids.firstCapture,
            session_id: ids.firstSession,
            proposed_scope_id: uuid(23),
            proposed_owner_principal_id: "alice-subject",
            knowledge_type: "preference",
            source_event_ids: [ids.privateEvent],
          },
        ],
      },
      webhook_knowledge: {
        id: ids.webhookKnowledge,
        revision_id: ids.webhookRevision,
        candidate_id: ids.webhookCandidate,
        outcome: "applied",
      },
      private_knowledge: {
        id: ids.privateKnowledge,
        revision_id: ids.privateRevision,
        candidate_id: ids.privateCandidate,
        outcome: "applied",
      },
      reuse_session: session(ids.reuseSession, "bob-subject"),
      reuse_context: context(
        ids.reuseContext,
        ids.reuseSession,
        "Webhook deliveries are deduplicated by provider event ID.",
      ),
      private_isolation: {
        session_id: ids.reuseSession,
        private_knowledge_id: ids.privateKnowledge,
        inspected_count: 1,
        private_knowledge_absent: true,
        evidence_kind: "owner_scope",
      },
      current_session: session(ids.currentSession, "bob-subject"),
      current_context: context(
        ids.currentContext,
        ids.currentSession,
        "PulseBoard public requests use the W3C traceparent header.",
      ),
      release_skill: {
        outcome: "applied",
        skill_id: ids.releaseSkill,
        version_id: ids.releaseSkillVersion,
      },
      release_skill_validation: {
        id: ids.releaseSkillValidation,
        kind: "skill_validation",
        operation_version: 1,
        state: "succeeded",
        skill_id: ids.releaseSkill,
        skill_version_id: ids.releaseSkillVersion,
        progress_percent: 100,
        attempts: 1,
        test_run_id: ids.releaseSkillTestRun,
      },
    },
  };
}

function status() {
  const currentReceipt = receipt();
  const resources = currentReceipt.resources;
  const ended = (session) => ({ ...session, status: "ended" });
  const knowledge = (handle, owner) => ({
    id: handle.id,
    ...(owner === undefined
      ? { project_id: resources.project.id }
      : { owner_principal_id: owner }),
    lifecycle_state: "active",
    current_revision: {
      id: handle.revision_id,
      knowledge_item_id: handle.id,
    },
  });
  const detail = (run, selections) => ({
    run: {
      ...run,
      rendered: undefined,
      trace_retention_mode: "redacted",
      selection_count: selections.length,
    },
    candidates: [],
    selections,
    feedback: [],
  });
  const sharedSelection = {
    channel: "current_knowledge",
    knowledge_item_id: resources.webhook_knowledge.id,
    knowledge_revision_id: resources.webhook_knowledge.revision_id,
  };
  return {
    receipt: currentReceipt,
    live: {
      workspace: { status: "visible", value: resources.workspace },
      project: { status: "visible", value: resources.project },
      first_session: {
        status: "visible",
        value: ended(resources.first_session),
      },
      first_capture: {
        status: "visible",
        value: resources.first_capture,
      },
      reuse_session: {
        status: "visible",
        value: ended(resources.reuse_session),
      },
      current_session: {
        status: "visible",
        value: ended(resources.current_session),
      },
      webhook_knowledge: {
        status: "visible",
        value: knowledge(resources.webhook_knowledge, undefined),
      },
      private_knowledge: {
        status: "visible",
        value: knowledge(resources.private_knowledge, "alice-subject"),
      },
      reuse_context: {
        status: "visible",
        value: detail(resources.reuse_context, [sharedSelection]),
      },
      current_context: {
        status: "visible",
        value: detail(resources.current_context, [sharedSelection]),
      },
      release_skill_validation: {
        status: "visible",
        value: { ...resources.release_skill_validation },
      },
    },
  };
}

function pendingPrivateReceipt() {
  const value = receipt();
  delete value.resources.private_knowledge.revision_id;
  value.resources.private_knowledge.change_id = uuid(28);
  value.resources.private_knowledge.outcome = "pending_review";
  value.resources.private_isolation.evidence_kind =
    "pending_review_not_published";
  value.notices = [
    "private quick-test preference remains pending in Advanced Reviews; the demo does not claim it as active Knowledge",
  ];
  return value;
}

function pendingPrivateStatus() {
  const value = status();
  value.receipt = pendingPrivateReceipt();
  value.live.private_knowledge = {
    status: "unavailable",
    reason: "not published",
  };
  return value;
}

function pendingSkillReceipt(value = receipt()) {
  value.resources.release_skill.outcome = "pending_review";
  value.resources.release_skill.change_id = uuid(29);
  delete value.resources.release_skill_validation;
  value.notices ??= [];
  value.notices.push(
    "Release Skill installation is in Advanced > Reviews; no unreviewed version was advertised or pinned",
    "Release Skill validation remains pending until its governed version is applied",
  );
  return value;
}

function governedStatus() {
  const value = pendingPrivateStatus();
  value.receipt = pendingSkillReceipt(value.receipt);
  delete value.live.release_skill_validation;
  return value;
}

function fakeChild() {
  const child = new EventEmitter();
  child.stdout = new PassThrough();
  child.stderr = new PassThrough();
  return child;
}

function successfulSpawn(outputs, calls) {
  return (file, args, options) => {
    const child = fakeChild();
    const output = outputs.shift();
    calls.push({ file, args, options });
    child.kill = () => true;
    queueMicrotask(() => {
      child.emit("exit", 0, null);
      child.stdout.write(JSON.stringify(output));
      child.stdout.end();
      child.stderr.end();
      setImmediate(() => child.emit("close", 0, null));
    });
    return child;
  };
}

test("CLI login and loopback handoff URLs are exact and secret-free", () => {
  const login = validateCliLoginUrl(startUrl(), SETTINGS);
  assert.equal(login.redirect, REDIRECT);
  assert.equal(login.state, STATE);
  const handoff = `${REDIRECT}?code=opaque-one-time-code&state=${STATE}`;
  assert.equal(validateCliHandoffUrl(handoff, login), true);
  assert.equal(allowedCliRequest(handoff, SETTINGS, login), true);
  assert.equal(
    allowedCliRequest(`${SETTINGS.appOrigin}/auth/login`, SETTINGS, login),
    true,
  );

  for (const mutant of [
    startUrl({ cli_redirect_uri: "http://localhost:43127/callback" }),
    startUrl({ cli_redirect_uri: "http://127.0.0.1/callback" }),
    startUrl({ cli_redirect_uri: "http://127.0.0.1:43127/other" }),
    startUrl({ cli_redirect_uri: "https://127.0.0.1:43127/callback" }),
    startUrl({ cli_state: "short" }),
    startUrl({ issuer: "http://other.invalid/realms/synveda" }),
    startUrl({ issuer: undefined }),
    startUrl({}, [["token", "private"]]),
    startUrl().replace("app.synveda.test", "other.synveda.test"),
  ]) refuse(() => validateCliLoginUrl(mutant, SETTINGS), "cli-login-url");

  for (const mutant of [
    `${REDIRECT}?code=opaque-one-time-code&state=wrong`,
    `${REDIRECT}?code=opaque-one-time-code&state=${STATE}&token=private`,
    `${REDIRECT}?state=${STATE}`,
    `http://127.0.0.1:43128/callback?code=opaque&state=${STATE}`,
    `http://user@127.0.0.1:43127/callback?code=opaque&state=${STATE}`,
  ]) refuse(() => validateCliHandoffUrl(mutant, login), "cli-handoff");
  assert.equal(
    allowedCliRequest("http://127.0.0.1:43127/favicon.ico", SETTINGS, login),
    false,
  );
  assert.equal(
    allowedCliRequest("http://127.0.0.1:43128/callback", SETTINGS, login),
    false,
  );
});

test("the product receipt requires the real team, Capture, Knowledge and reuse legs", () => {
  assert.equal(validateDemoReceipt(receipt()), true);
  assert.equal(validateDemoStatus(status()), true);
  assert.equal(validateDemoReceipt(pendingPrivateReceipt()), true);
  assert.equal(validateDemoStatus(pendingPrivateStatus()), true);
  assert.equal(validateDemoReceipt(pendingSkillReceipt()), true);
  assert.equal(validateDemoReceipt(governedStatus().receipt), true);
  assert.equal(validateDemoStatus(governedStatus()), true);
  for (const name of RESOURCE_NAMES) {
    const mutant = receipt();
    delete mutant.resources[name];
    refuse(() => validateDemoReceipt(mutant), "product-demo");
  }
  for (const name of STATUS_NAMES) {
    const mutant = status();
    mutant.live[name].status = "unavailable";
    refuse(() => validateDemoStatus(mutant), "product-status");
  }

  for (const mutate of [
    (value) => { value.resources.bob_member.grant.principal_id = "alice-subject"; },
    (value) => { value.resources.first_capture.state = "running"; },
    (value) => {
      value.resources.webhook_knowledge.candidate_id =
        value.resources.private_knowledge.candidate_id;
    },
    (value) => { value.resources.reuse_context.rendered = "test-fast"; },
    (value) => { value.resources.private_isolation.private_knowledge_absent = false; },
    (value) => { value.resources.private_isolation.evidence_kind = "pending_review_not_published"; },
    (value) => { value.resources.release_skill_validation.state = "dead_lettered"; },
  ]) {
    const mutant = receipt();
    mutate(mutant);
    refuse(() => validateDemoReceipt(mutant), "product-demo");
  }

  for (const mutate of [
    (value) => { value.notices = []; },
    (value) => {
      value.resources.release_skill_validation =
        receipt().resources.release_skill_validation;
    },
    (value) => { delete value.resources.release_skill.change_id; },
  ]) {
    const mutant = pendingSkillReceipt();
    mutate(mutant);
    refuse(() => validateDemoReceipt(mutant), "product-demo");
  }

  for (const mutate of [
    (value) => { value.notices = []; },
    (value) => { value.resources.private_knowledge.revision_id = uuid(29); },
    (value) => { value.resources.private_isolation.evidence_kind = "owner_scope"; },
  ]) {
    const mutant = pendingPrivateReceipt();
    mutate(mutant);
    refuse(() => validateDemoReceipt(mutant), "product-demo");
  }

  for (const mutate of [
    (value) => {
      value.live.webhook_knowledge.value.current_revision.id = uuid(99);
    },
    (value) => {
      value.live.reuse_context.value.selections[0].knowledge_item_id =
        value.receipt.resources.private_knowledge.id;
    },
    (value) => { value.live.first_session.value.status = "active"; },
    (value) => { value.live.release_skill_validation.value.test_run_id = uuid(99); },
  ]) {
    const mutant = status();
    mutate(mutant);
    refuse(() => validateDemoStatus(mutant), "product-status");
  }
});

test("seed uses two real CLI profiles, one existing demo flow and a receipt reopen", async () => {
  const calls = [];
  const passwords = [Buffer.from("a".repeat(64)), Buffer.from("b".repeat(64))];
  let passwordIndex = 0;
  const command = async (args) => {
    calls.push(["command", ...args]);
    return args[1] === "status" ? status() : receipt();
  };
  const login = async ({ profile, username, password }) => {
    calls.push(["login", profile, username]);
    assert.equal(password.length, 64);
  };
  assert.equal(
    await runProductAcceptance({
      chromium: { launch: async () => {} },
      environment: {
        SYNVEDA_BROWSER_APP_URL: SETTINGS.appOrigin,
        SYNVEDA_BROWSER_ISSUER: SETTINGS.issuer,
      },
      readPassword: () => passwords[passwordIndex++],
      login,
      command,
    }),
    true,
  );
  assert.deepEqual(calls, [
    ["login", "alice", "synveda-demo-admin"],
    ["login", "bob", "synveda-demo-member"],
    [
      "command",
      "demo",
      "start",
      "--profile",
      "team",
      "--credentials",
      "alice",
      "--bob-credentials",
      "bob",
      "--json",
    ],
    [
      "command",
      "demo",
      "start",
      "--profile",
      "team",
      "--credentials",
      "alice",
      "--bob-credentials",
      "bob",
      "--json",
    ],
    ["command", "demo", "status", "--credentials", "alice", "--json"],
  ]);
  for (const password of passwords) {
    assert.ok(password.every((value) => value === 0));
  }
});

test("post-restart verification requires the persisted receipt and only Alice's login", async () => {
  const calls = [];
  const adminPassword = Buffer.from("a".repeat(64));
  assert.equal(
    await runProductAcceptance({
      chromium: { launch: async () => {} },
      phase: "verify",
      environment: {
        SYNVEDA_BROWSER_APP_URL: SETTINGS.appOrigin,
        SYNVEDA_BROWSER_ISSUER: SETTINGS.issuer,
      },
      readPassword: (path) => {
        calls.push(["password", path]);
        return adminPassword;
      },
      login: async ({ profile, username, password }) => {
        calls.push(["login", profile, username]);
        password.fill(0);
      },
      command: async (args) => {
        calls.push(["command", ...args]);
        return status();
      },
    }),
    true,
  );
  assert.deepEqual(calls, [
    ["password", "/run/secrets/keycloak_demo_admin_password"],
    ["login", "alice", "synveda-demo-admin"],
    ["command", "demo", "status", "--credentials", "alice", "--json"],
  ]);
  assert.ok(adminPassword.every((value) => value === 0));
});

test("the product runner refuses unknown phases before reading a secret", async () => {
  await assert.rejects(
    runProductAcceptance({
      chromium: { launch: async () => {} },
      phase: "other",
      environment: {
        SYNVEDA_BROWSER_APP_URL: SETTINGS.appOrigin,
        SYNVEDA_BROWSER_ISSUER: SETTINGS.issuer,
      },
      readPassword: () => {
        throw new Error("must not read");
      },
    }),
    (error) => {
      assert.ok(error instanceof BrowserContractError);
      assert.equal(error.stage, "configuration");
      return true;
    },
  );
});

test("a partial password read failure clears the first secret and stops", async () => {
  const adminPassword = Buffer.from("a".repeat(64));
  let reads = 0;
  await assert.rejects(
    runProductAcceptance({
      chromium: { launch: async () => {} },
      environment: {
        SYNVEDA_BROWSER_APP_URL: SETTINGS.appOrigin,
        SYNVEDA_BROWSER_ISSUER: SETTINGS.issuer,
      },
      readPassword: () => {
        reads += 1;
        if (reads === 1) return adminPassword;
        throw new Error("member secret unavailable");
      },
      login: async () => {
        throw new Error("login must not run");
      },
      command: async () => {
        throw new Error("command must not run");
      },
    }),
  );
  assert.ok(adminPassword.every((value) => value === 0));
});

test("the exact CLI is spawned with a closed environment and waits for close", async () => {
  const calls = [];
  const passwords = [Buffer.from("a".repeat(64)), Buffer.from("b".repeat(64))];
  assert.equal(
    await runProductAcceptance({
      chromium: { launch: async () => {} },
      environment: {
        HOME: "/tmp/browser-home",
        HTTP_PROXY: "http://private-proxy.invalid",
        NODE_OPTIONS: "--require=/private/injection.cjs",
        SYNVEDA_BROWSER_APP_URL: SETTINGS.appOrigin,
        SYNVEDA_BROWSER_ISSUER: SETTINGS.issuer,
        SYNVEDA_INSECURE_DEVELOPMENT_HTTP: "true",
        SYNVEDA_TOKEN: "private-bearer",
        XDG_CONFIG_HOME: "/tmp/browser-home/config",
        XDG_STATE_HOME: "/tmp/browser-home/state",
      },
      readPassword: () => passwords.shift(),
      login: async ({ password }) => password.fill(0),
      spawnProcess: successfulSpawn([receipt(), receipt(), status()], calls),
    }),
    true,
  );
  assert.equal(calls.length, 3);
  for (const call of calls) {
    assert.equal(call.file, "/usr/local/bin/synveda");
    assert.deepEqual(Object.keys(call.options.env).sort(), [
      "HOME",
      "LANG",
      "LC_ALL",
      "PATH",
      "SYNVEDA_INSECURE_DEVELOPMENT_HTTP",
      "TZ",
      "XDG_CONFIG_HOME",
      "XDG_STATE_HOME",
    ]);
    assert.doesNotMatch(JSON.stringify(call.options), /private-bearer|private-proxy|injection/);
  }
});

test("CLI timeout escalates to SIGKILL and oversized output fails closed", async () => {
  for (const mode of ["timeout", "oversized"]) {
    const signals = [];
    const spawnProcess = () => {
      const child = fakeChild();
      let closed = false;
      const close = (signal) => {
        if (closed) return;
        closed = true;
        child.stdout.end();
        child.stderr.end();
        queueMicrotask(() => child.emit("close", null, signal));
      };
      child.kill = (signal) => {
        signals.push(signal);
        if (mode === "oversized" || signal === "SIGKILL") close(signal);
        return true;
      };
      if (mode === "oversized") {
        queueMicrotask(() => child.stdout.write(Buffer.alloc(2 * 1024 * 1024 + 1)));
      }
      return child;
    };
    const passwords = [Buffer.from("a".repeat(64)), Buffer.from("b".repeat(64))];
    await assert.rejects(
      runProductAcceptance({
        chromium: { launch: async () => {} },
        environment: {
          SYNVEDA_BROWSER_APP_URL: SETTINGS.appOrigin,
          SYNVEDA_BROWSER_ISSUER: SETTINGS.issuer,
        },
        readPassword: () => passwords.shift(),
        login: async ({ password }) => password.fill(0),
        spawnProcess,
        demoTimeout: 10,
      }),
      (error) => {
        assert.ok(error instanceof BrowserContractError);
        assert.equal(error.stage, "product-demo");
        return true;
      },
    );
    assert.ok(signals.includes("SIGTERM"));
    if (mode === "timeout") assert.ok(signals.includes("SIGKILL"));
  }
});

test("the fixture copies the product CLI and exposes no token-transfer shortcut", () => {
  const dockerfile = readFileSync(
    join(ROOT, "deploy/compose/product/Dockerfile"),
    "utf8",
  );
  const runner = readFileSync(
    join(ROOT, "deploy/compose/browser/product-demo-runner.mjs"),
    "utf8",
  );
  const lifecycle = readFileSync(
    join(ROOT, "deploy/compose/scripts/compose.sh"),
    "utf8",
  );
  assert.match(
    dockerfile,
    /^COPY --from=build \/src\/target\/release\/synveda \/usr\/local\/bin\/synveda$/m,
  );
  assert.match(
    dockerfile,
    /^FROM debian:bookworm-slim@sha256:[0-9a-f]{64} AS runtime$/m,
  );
  for (const forbidden of [
    /SYNVEDA_TOKEN/,
    /password_grant/,
    /grant_type[^\n]*password/,
    /client_secret/,
    /docker\.sock/,
    /\.screenshot\s*\(/,
    /storageState\s*[:(]/,
    /ignoreHTTPSErrors/,
    /--no-sandbox/,
  ]) assert.doesNotMatch(runner, forbidden);
  assert.match(runner, /"login",\s*"--gateway"/);
  assert.match(runner, /"demo",\s*"start"/);
  assert.match(
    lifecycle,
    /run_product_acceptance seed "\$@"[\s\S]*restart_service_name=postgres[\s\S]*rerun_browser_acceptance "\$@"[\s\S]*run_product_acceptance verify "\$@"/,
  );
  assert.doesNotMatch(lifecycle, /product_acceptance_state|product-demo\.mjs start/);
});
