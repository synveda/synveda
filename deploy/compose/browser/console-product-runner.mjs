import { BrowserContractError } from "./console-login-contract.mjs";
import { runBrowserAcceptance } from "./console-login-runner.mjs";

const ADMIN_PASSWORD_FILE = "/run/secrets/keycloak_demo_admin_password";
const MEMBER_PASSWORD_FILE = "/run/secrets/keycloak_demo_member_password";
const VIEWER_PASSWORD_FILE = "/run/secrets/keycloak_demo_viewer_password";
const FETCH_TIMEOUT = 5_000;
const UUID_V7 =
  /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const COMMIT = /^[0-9a-f]{64}$/;

export const USER_EDITED_DESCRIPTION =
  "Operator-edited CPR-45 acceptance description retained across seed replay.";

function failure(stage) {
  return new BrowserContractError(stage);
}

async function atProductStage(stage, operation) {
  try {
    return await operation();
  } catch (error) {
    if (error instanceof BrowserContractError) throw error;
    const wrapped = failure(stage);
    wrapped.cause = error;
    throw wrapped;
  }
}

function requireUuid(value, stage) {
  if (typeof value !== "string" || !UUID_V7.test(value)) throw failure(stage);
  return value;
}

function requireCommit(value, stage) {
  if (typeof value !== "string" || !COMMIT.test(value)) throw failure(stage);
  return value;
}

function requireValue(value, stage) {
  if (typeof value !== "string" || value.length === 0) throw failure(stage);
  return value;
}

function requireTrue(value, stage) {
  if (value !== true) throw failure(stage);
}

async function visible(locator, timeout) {
  await locator.waitFor({ state: "visible", timeout });
  return locator;
}

async function selectedText(page, label, expected, stage, timeout) {
  const control = page.getByRole("combobox", {
    name: new RegExp(`^${label}$`, "i"),
  });
  await visible(control, timeout);
  const actual = (await control.locator("option:checked").textContent())?.trim();
  if (actual !== expected) throw failure(stage);
}

async function openPage(page, settings, path, heading, timeout, reload = false) {
  await page.goto(`${settings.appOrigin}${path}`, {
    timeout,
    waitUntil: "domcontentloaded",
  });
  await visible(
    page.getByRole("heading", { name: heading, exact: true, level: 1 }),
    timeout,
  );
  if (new URL(page.url()).pathname !== path) throw failure("console-route");
  if (reload) {
    await page.reload({ timeout, waitUntil: "domcontentloaded" });
    await visible(
      page.getByRole("heading", { name: heading, exact: true, level: 1 }),
      timeout,
    );
    if (new URL(page.url()).pathname !== path) throw failure("console-route");
  }
}

async function stateOf(page, timeout) {
  const term = page.locator("dt").filter({ hasText: /^state$/ }).first();
  await visible(term, timeout);
  const value = term.locator("xpath=following-sibling::dd[1]");
  await visible(value, timeout);
  return (await value.textContent())?.trim();
}

async function assertShell(page, timeout) {
  await visible(page.getByRole("navigation", { name: "Product navigation" }), timeout);
  await visible(page.getByRole("button", { name: "Sign out", exact: true }), timeout);
  await selectedText(page, "Workspace", "Northstar Delivery", "console-selection", timeout);
  await selectedText(page, "Project", "Ingestion API", "console-selection", timeout);
}

async function assertKeyboardNavigation(page, timeout) {
  await page.evaluate(() => {
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
  });
  await page.keyboard.press("Tab");
  const focus = await page.evaluate(() => {
    const active = document.activeElement;
    if (!(active instanceof HTMLElement)) return { interactive: false, outlined: false };
    const style = getComputedStyle(active);
    return {
      interactive: active.matches("a, button, input, select, textarea"),
      outlined:
        style.outlineStyle !== "none" && Number.parseFloat(style.outlineWidth || "0") > 0,
    };
  });
  requireTrue(focus.interactive && focus.outlined, "console-keyboard-focus");

  const sessions = page
    .getByRole("navigation", { name: "Product navigation" })
    .getByRole("link", { name: "Sessions", exact: true });
  await sessions.focus();
  await page.keyboard.press("Enter");
  await visible(
    page.getByRole("heading", { name: "Sessions", exact: true, level: 1 }),
    timeout,
  );
}

async function assertViewport(page, width, height, stage) {
  await page.setViewportSize({ width, height });
  const fits = await page.evaluate(() =>
    document.documentElement.scrollWidth <= document.documentElement.clientWidth + 1,
  );
  requireTrue(fits, stage);
}

async function assertSeeded({ page, settings, timeout, receipt, edit }) {
  const sessionId = requireUuid(receipt?.resources?.source_session?.id, "console-seeded");
  const skillChange = requireUuid(
    receipt?.resources?.skill_install?.change_id,
    "console-seeded",
  );

  await atProductStage("console-seeded-shell", () => assertShell(page, timeout));
  await atProductStage("console-seed-duplicates", async () => {
    const workspaceOptions = await page
      .getByRole("combobox", { name: /^workspace$/i })
      .locator("option")
      .allTextContents();
    const projectOptions = await page
      .getByRole("combobox", { name: /^project$/i })
      .locator("option")
      .allTextContents();
    requireTrue(
      workspaceOptions.filter((value) => value.trim() === "Northstar Delivery").length === 1 &&
        projectOptions.filter((value) => value.trim() === "Ingestion API").length === 1,
      "console-seed-duplicates",
    );
  });

  if (edit) {
    await atProductStage("console-keyboard-navigation", () =>
      assertKeyboardNavigation(page, timeout),
    );
    await atProductStage("console-seeded-session", async () => {
      await openPage(
        page,
        settings,
        `/console/sessions/${sessionId}`,
        "Session",
        timeout,
        true,
      );
      await visible(
        page.getByRole("heading", {
          name: /^Synthetic replay: determine ingestion retry behaviour\s+running$/,
          level: 2,
        }),
        timeout,
      );
      await visible(page.getByText("running", { exact: true }).first(), timeout);
      await visible(page.getByRole("heading", { name: "Timeline", exact: true }), timeout);
      await visible(
        page.getByText("No evidence snapshot has been frozen for this Session.", { exact: true }),
        timeout,
      );
      await visible(
        page.getByText(
          "No policy-visible learning has been extracted yet. Pending work will appear here when the worker completes it.",
          { exact: true },
        ),
        timeout,
      );
    });

    await atProductStage("console-seeded-knowledge", async () => {
      await openPage(page, settings, "/console/knowledge", "Knowledge", timeout);
      await visible(page.getByText("Ingestion idempotency baseline", { exact: true }), timeout);
    });

    await atProductStage("console-seeded-skills", async () => {
      await openPage(page, settings, "/console/skills", "Skills", timeout);
      await visible(
        page.getByText(
          "No installed Skill is visible under this policy. A denied aggregate is omitted, so this does not disclose whether one exists elsewhere.",
          { exact: true },
        ),
        timeout,
      );
    });

    await atProductStage("console-seeded-review-page", async () => {
      await openPage(
        page,
        settings,
        `/console/advanced/reviews/${skillChange}`,
        "Review",
        timeout,
        true,
      );
    });
    await atProductStage("console-seeded-review-state", async () => {
      if ((await stateOf(page, timeout)) !== "open") throw failure("console-pending-review");
    });
    const confirmation = page.getByRole("group", { name: "Confirm cancellation" });
    await atProductStage("console-confirmation-open", async () => {
      await page
        .getByRole("button", { name: "Cancel proposal", exact: true })
        .click({ timeout });
      await visible(confirmation, timeout);
    });
    await atProductStage("console-confirmation-dismiss", async () => {
      await page
        .getByRole("button", { name: "Keep proposal", exact: true })
        .click({ timeout });
      await confirmation.waitFor({ state: "hidden", timeout });
    });
  }

  await atProductStage("console-seeded-settings", async () => {
    await openPage(page, settings, "/console/settings", "Settings", timeout);
    const description = page.getByRole("textbox", { name: /^description$/i });
    await visible(description, timeout);
    if (edit) {
      await description.fill(USER_EDITED_DESCRIPTION);
      await page.getByRole("button", { name: "Save", exact: true }).click({ timeout });
      await visible(page.getByText("saved", { exact: true }), timeout);
    }
    if ((await description.inputValue()) !== USER_EDITED_DESCRIPTION) {
      throw failure(edit ? "console-user-edit" : "console-seed-preservation");
    }
  });
}

async function directViewerDenials(page, receipt, proposal, protectedMarker, timeout) {
  const denial = await page.evaluate(
    async ({ commit, eventId, fetchTimeout, id, protectedMarker, sessionId }) => {
      const request = async (path, init) => {
        const controller = new AbortController();
        const deadline = setTimeout(() => controller.abort(), fetchTimeout);
        try {
          const response = await fetch(path, {
            ...init,
            credentials: "same-origin",
            signal: controller.signal,
          });
          const body = await response.text();
          return {
            status: response.status,
            protectedAbsent: !body.includes(protectedMarker),
          };
        } finally {
          clearTimeout(deadline);
        }
      };
      const protectedEvent = await request(
        `/v1/sessions/${encodeURIComponent(sessionId)}/events/${encodeURIComponent(eventId)}`,
        { method: "GET", headers: { accept: "application/json" } },
      );
      const approval = await request(
        `/v1/proposals/${encodeURIComponent(id)}/approve`,
        {
          method: "POST",
          headers: { accept: "application/json", "content-type": "application/json" },
          body: JSON.stringify({ expected_commit: commit, comment: "denial probe" }),
        },
      );
      return { protectedEvent, approval };
    },
    {
      commit: requireCommit(proposal?.commit, "console-viewer-denial"),
      eventId: requireUuid(receipt?.resources?.source_event?.id, "console-viewer-denial"),
      fetchTimeout: Math.min(timeout, FETCH_TIMEOUT),
      id: requireUuid(proposal?.id, "console-viewer-denial"),
      protectedMarker,
      sessionId: requireUuid(
        receipt?.resources?.source_session?.id,
        "console-viewer-denial",
      ),
    },
  );
  requireTrue(
    [403, 404].includes(denial?.protectedEvent?.status) &&
      [403, 404].includes(denial?.approval?.status) &&
      denial.protectedEvent.protectedAbsent === true &&
      denial.approval.protectedAbsent === true,
    "console-viewer-denial",
  );
}

async function assertViewerDenied({ page, settings, timeout, receipt, proposal }) {
  const marker = requireValue(
    receipt?.resources?.source_event?.payload?.text,
    "console-viewer-denial",
  );
  await directViewerDenials(page, receipt, proposal, marker, timeout);
  await openPage(
    page,
    settings,
    `/console/advanced/reviews/${proposal.id}`,
    "Review",
    timeout,
    true,
  );
  // The proposal arrives before its target-scope capability probe. Wait for
  // the settled refusal or read-only explanation, not the first detail row.
  await visible(
    page.locator('main [role="alert"], main .verdict-section').filter({
      hasText: /proposal\.read|which does not include casting a verdict here\./,
    }).first(),
    timeout,
  );
  const view = await page.evaluate(() => {
    const alert = document.querySelector('main [role="alert"]');
    const terms = [...document.querySelectorAll("main dt")];
    const state = terms.find((term) => term.textContent?.trim() === "state")
      ?.nextElementSibling?.textContent?.trim();
    const main = document.querySelector("main")?.innerText ?? "";
    return {
      gated: alert?.textContent?.includes("proposal.read") === true,
      openWithoutVerdict: state === "open" && main.includes("does not include casting a verdict here."),
    };
  });
  requireTrue(
    (view.gated === true || view.openWithoutVerdict === true) &&
      (await page.getByRole("button", { name: "Approve", exact: true }).count()) === 0 &&
      (await page.getByRole("button", { name: "Apply approved change", exact: true }).count()) ===
        0 &&
      (await page.locator(".banner.success").count()) === 0,
    "console-viewer-ui",
  );
}

async function assertReviewerOpen({ page, settings, timeout, proposal }) {
  await openPage(
    page,
    settings,
    `/console/advanced/reviews/${requireUuid(proposal?.id, "console-reviewer")}`,
    "Review",
    timeout,
    true,
  );
  if ((await stateOf(page, timeout)) !== "open") throw failure("console-reviewer");
  const reason = page.getByLabel(/^why /);
  await visible(reason, timeout);
  await visible(page.getByRole("button", { name: "Approve", exact: true }), timeout);
  const reject = page.getByRole("button", { name: "Reject", exact: true });
  requireTrue(await reject.isDisabled(), "console-review-labels");
  await reason.fill("Synthetic CPR-45 browser review inspection");
  requireTrue(!(await reject.isDisabled()), "console-review-labels");
}

async function assertApprovedNotApplied({ page, settings, timeout, receipt, proposal }) {
  await openPage(
    page,
    settings,
    `/console/advanced/reviews/${requireUuid(proposal?.id, "console-approved")}`,
    "Review",
    timeout,
  );
  if ((await stateOf(page, timeout)) !== "approved") throw failure("console-approved");
  await visible(
    page.getByRole("button", { name: "Apply approved change", exact: true }),
    timeout,
  );

  await openPage(page, settings, "/console/knowledge", "Knowledge", timeout);
  await visible(page.getByText("Ingestion idempotency baseline", { exact: true }), timeout);
  requireTrue(
    (await page.getByText("Retry behaviour for ingestion requests", { exact: true }).count()) === 0,
    "console-approved-not-applied",
  );
  requireUuid(receipt?.resources?.learning?.id, "console-approved-not-applied");
}

async function assertNativeValidationAndConfirmation(page, receipt, timeout) {
  const approverSubject = requireValue(receipt?.approver?.subject, "console-administration");
  const viewerSubject = requireValue(receipt?.viewer?.subject, "console-administration");
  const approverGrant = requireUuid(receipt?.resources?.grant_approver?.id, "console-administration");
  const viewerGrant = requireUuid(receipt?.resources?.grant_viewer?.id, "console-administration");
  await visible(
    page
      .locator("tr")
      .filter({ hasText: approverGrant })
      .filter({ hasText: approverSubject })
      .filter({ hasText: "administrator" }),
    timeout,
  );
  const revoke = page
    .locator("tr")
    .filter({ hasText: viewerGrant })
    .filter({ hasText: viewerSubject })
    .filter({ hasText: "viewer" })
    .getByRole("button", { name: "Revoke exact grant", exact: true });
  await visible(revoke, timeout);
  let revocations = 0;
  const countRevocation = (request) => {
    const url = new URL(request.url());
    if (request.method() === "DELETE" && url.pathname.startsWith("/v1/grants/")) revocations += 1;
  };
  page.on("request", countRevocation);
  const dialogPromise = page.waitForEvent("dialog", { timeout });
  const clickPromise = revoke.click({ timeout });
  const dialog = await dialogPromise;
  const validDialog =
    dialog.type() === "confirm" &&
    dialog.message().includes("Revoke only the viewer grant") &&
    dialog.message().includes(viewerSubject);
  await dialog.dismiss();
  await clickPromise;
  page.off("request", countRevocation);
  requireTrue(validDialog && revocations === 0, "console-administration-confirmation");
  await visible(revoke, timeout);

  const email = page.getByLabel("Email label (optional)", { exact: true });
  const role = page.getByRole("combobox", { name: "Exact role to grant", exact: true });
  await visible(email, timeout);
  await visible(role, timeout);
  await email.fill("not-an-email");
  let invitations = 0;
  const countInvitation = (request) => {
    const url = new URL(request.url());
    if (
      request.method() === "POST" &&
      /^\/v1\/workspaces\/[^/]+\/invites$/.test(url.pathname)
    ) invitations += 1;
  };
  page.on("request", countInvitation);
  await page.getByRole("button", { name: "Create pending invitation", exact: true }).click({ timeout });
  page.off("request", countInvitation);
  const invalid = await email.evaluate(
    (input) => input.validity.typeMismatch && !input.checkValidity() && input.validationMessage.length > 0,
  );
  requireTrue(invalid && invitations === 0, "console-administration-validation");
}

async function assertVerified({ page, settings, timeout, status }) {
  const receipt = status?.receipt;
  const resources = receipt?.resources;
  const verification = resources?.verification;
  const learningId = requireUuid(resources?.learning?.id, "console-verified");
  const learningRevision = requireUuid(
    verification?.knowledge_revision_id,
    "console-verified",
  );
  const sourceEvent = requireUuid(resources?.source_event?.id, "console-verified");
  const sessionId = requireUuid(resources?.source_session?.id, "console-verified");
  const contextRun = requireUuid(verification?.context_run_id, "console-verified");
  const skillId = requireUuid(resources?.skill_install?.skill_id, "console-verified");
  const learningChange = requireUuid(resources?.learning?.change_id, "console-verified");
  const sourceCandidate = resources?.capture_candidates?.candidates?.find(
    (candidate) =>
      Array.isArray(candidate?.source_event_ids) &&
      candidate.source_event_ids.includes(sourceEvent),
  );
  requireUuid(sourceCandidate?.id, "console-verified");
  const sourceCandidateCount = resources?.capture_candidates?.candidates?.length;
  requireTrue(
    Number.isSafeInteger(sourceCandidateCount) && sourceCandidateCount > 0,
    "console-verified",
  );
  const verifiedStage = (name, operation) =>
    atProductStage(`console-verified-${name}`, operation);

  await verifiedStage("review", async () => {
    await assertShell(page, timeout);
    await openPage(
      page,
      settings,
      `/console/advanced/reviews/${learningChange}`,
      "Review",
      timeout,
      true,
    );
    if ((await stateOf(page, timeout)) !== "applied") {
      throw failure("console-applied-review");
    }
    requireTrue(
      (await page
        .getByRole("button", { name: "Apply approved change", exact: true })
        .count()) === 0,
      "console-applied-review",
    );
  });

  await verifiedStage("session", async () => {
    await atProductStage("console-verified-session-route", () =>
      openPage(
        page,
        settings,
        `/console/sessions/${sessionId}`,
        "Session",
        timeout,
        true,
      ),
    );
    await atProductStage("console-verified-session-capture", () =>
      visible(page.getByText("completed", { exact: true }).first(), timeout),
    );
    await atProductStage("console-verified-session-learning", () =>
      (async () => {
        const rows = page.locator(".capture-panel ul.capture-rows > li");
        await visible(rows.first(), timeout);
        requireTrue(
          (await rows.count()) === sourceCandidateCount,
          "console-verified-session-learning",
        );
        const row = rows.first();
        await row.getByText("Source event addresses", { exact: true }).click({ timeout });
        await visible(row.getByText(sourceEvent, { exact: true }), timeout);
      })(),
    );
  });

  await verifiedStage("knowledge", async () => {
    await atProductStage("console-verified-knowledge-route", () =>
      openPage(
        page,
        settings,
        `/console/knowledge/${learningId}`,
        "Knowledge item",
        timeout,
        true,
      ),
    );
    await atProductStage("console-verified-knowledge-title", () =>
      visible(
        page.getByRole("heading", {
          name: "Retry behaviour for ingestion requests",
          exact: true,
          level: 2,
        }),
        timeout,
      ),
    );
    await atProductStage("console-verified-knowledge-body", async () => {
      const body = page.locator(".knowledge-detail .knowledge-body").first();
      await visible(body, timeout);
      requireTrue(
        (await body.textContent()) === resources.source_event.payload.text,
        "console-verified-knowledge-body",
      );
    });
    await atProductStage("console-verified-knowledge-source", () =>
      visible(page.getByText(`Event ${sourceEvent}`, { exact: true }), timeout),
    );
  });

  await verifiedStage("context", async () => {
    await atProductStage("console-verified-context-route", () =>
      openPage(
        page,
        settings,
        `/console/context-runs/${contextRun}`,
        "Context Inspector",
        timeout,
        true,
      ),
    );
    await atProductStage("console-verified-context-query", () =>
      visible(
        page.getByRole("heading", {
          name: "Task text not retained",
          exact: true,
          level: 2,
        }),
        timeout,
      ),
    );
    await atProductStage("console-verified-context-redaction", async () => {
      await visible(
        page.getByRole("status").filter({ hasText: /^Redacted trace:/ }).first(),
        timeout,
      );
      await visible(
        page.getByText(
          "The original task is unavailable under this trace-retention mode.",
          { exact: true },
        ),
        timeout,
      );
      requireTrue(
        (await page
          .getByText("Retry behaviour for ingestion requests", { exact: true })
          .count()) === 0,
        "console-verified-context-redaction",
      );
    });
    const selectedHref = `/console/knowledge/${learningId}#revision-${learningRevision}`;
    const selectedLink = page.locator(
      `.context-inspector .context-selection a[href="${selectedHref}"]`,
    );
    await atProductStage("console-verified-context-selection", async () => {
      await visible(selectedLink, timeout);
      requireTrue(
        (await selectedLink.count()) === 1,
        "console-verified-context-selection",
      );
    });
    await atProductStage("console-verified-context-link", async () => {
      requireTrue(
        (await selectedLink.textContent())?.trim() === "Open selected Knowledge revision",
        "console-verified-context-link",
      );
      if ((await selectedLink.getAttribute("href")) !== selectedHref) {
        throw failure("console-verified-context-link");
      }
    });
    await atProductStage("console-verified-context-layout", () =>
      assertViewport(page, 1024, 768, "console-laptop-layout"),
    );
  });

  await verifiedStage("skill", async () => {
    await openPage(page, settings, `/console/skills/${skillId}`, "Skill", timeout, true);
    await visible(
      page.getByRole("heading", {
        name: "ingestion-retry-review",
        exact: true,
        level: 2,
      }),
      timeout,
    );
    await visible(page.getByText("available", { exact: true }).first(), timeout);
  });

  await verifiedStage("operations", async () => {
    await openPage(page, settings, "/console/operations", "Operations", timeout);
    await visible(
      page.getByRole("heading", { name: "Recent durable operations", exact: true }),
      timeout,
    );
    await visible(page.getByRole("heading", { name: "Recent sessions", exact: true }), timeout);
    await visible(
      page.getByRole("heading", { name: "Recent context runs", exact: true }),
      timeout,
    );
  });

  await verifiedStage("people", async () => {
    await atProductStage("console-verified-people-route", () =>
      openPage(page, settings, "/console/people", "People", timeout),
    );
    await atProductStage("console-verified-people-access", () =>
      visible(
        page.getByRole("heading", { name: "Exact access grants", exact: true }),
        timeout,
      ),
    );
    await atProductStage("console-verified-people-controls", () =>
      assertNativeValidationAndConfirmation(page, receipt, timeout),
    );
  });

  await verifiedStage("desktop", async () => {
    await assertViewport(page, 1440, 900, "console-desktop-layout");
    await openPage(page, settings, "/console/", "Home", timeout);
    await assertShell(page, timeout);
    await assertViewport(page, 1440, 900, "console-desktop-layout");
  });
}

function identityFor(checkpoint, receipt) {
  if (checkpoint === "viewer-denial") {
    return {
      admissionStage: "viewer-admission",
      expectedSubject: receipt?.viewer?.subject,
      passwordFile: VIEWER_PASSWORD_FILE,
      requiredRoleKeys: [],
      username: "synveda-demo-viewer",
    };
  }
  if (checkpoint === "reviewer-open") {
    return {
      admissionStage: "reviewer-admission",
      expectedSubject: receipt?.reviewer?.subject,
      passwordFile: MEMBER_PASSWORD_FILE,
      requiredRoleKeys: [],
      username: "synveda-demo-member",
    };
  }
  return {
    admissionStage: "author-admission",
    expectedSubject: receipt?.author?.subject,
    passwordFile: ADMIN_PASSWORD_FILE,
    requiredRoleKeys: ["administrator"],
    username: "synveda-demo-admin",
  };
}

export async function runConsoleProductCheckpoint({
  chromium,
  environment = process.env,
  checkpoint,
  receipt,
  proposal,
  status,
  runSession = runBrowserAcceptance,
  timeout = 60_000,
} = {}) {
  if (
    ![
      "seeded-edit",
      "seeded-preserved",
      "viewer-denial",
      "reviewer-open",
      "approved-not-applied",
      "verified",
    ].includes(checkpoint) ||
    typeof runSession !== "function"
  ) throw failure("console-checkpoint-configuration");
  const effectiveReceipt = checkpoint === "verified" ? status?.receipt : receipt;
  const identity = identityFor(checkpoint, effectiveReceipt);
  requireValue(identity.expectedSubject, "console-checkpoint-configuration");

  return runSession({
    chromium,
    environment,
    ...identity,
    timeout,
    afterLogin: async ({ page, settings }) => {
      return atProductStage(`console-${checkpoint}`, async () => {
        switch (checkpoint) {
          case "seeded-edit":
            return assertSeeded({ page, settings, timeout, receipt, edit: true });
          case "seeded-preserved":
            return assertSeeded({ page, settings, timeout, receipt, edit: false });
          case "viewer-denial":
            return assertViewerDenied({ page, settings, timeout, receipt, proposal });
          case "reviewer-open":
            return assertReviewerOpen({ page, settings, timeout, proposal });
          case "approved-not-applied":
            return assertApprovedNotApplied({ page, settings, timeout, receipt, proposal });
          case "verified":
            return assertVerified({ page, settings, timeout, status });
        }
      });
    },
  });
}
