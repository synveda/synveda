/**
 * What the Sessions pages actually render (CPR-11, ADR-0077).
 *
 * `renderToStaticMarkup` plus `toText`, the convention CNSL-1 established
 * (ADR-0056 decision 7): a browser is where markup is laid out, but *which
 * facts appear in it* is decided by this code and by nothing the DOM
 * contributes. The pure derivations are in `sessions.test.mts`; these are the
 * claims this feature makes to a reader, asserted as text.
 *
 * The cache is primed rather than mocked. `useQuery` reads
 * `cache.read(key)` through `useSyncExternalStore`'s server snapshot, and no
 * effect runs during a static render — so a key seeded with an answer renders
 * that answer, and no request is ever made.
 */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { beforeEach, test } from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import { cache } from "./cache.mjs";
import { Session } from "./Session.js";
import { Sessions } from "./Sessions.js";
import { AppProvider, appContext } from "./Shell.js";
import { reconcile } from "./selection.mjs";
import { toText } from "./text.mjs";
import { listQuery, NO_FILTERS } from "./sessions.mjs";
import type { Outcome } from "./api.mjs";
import type {
  MeView,
  ProjectView,
  SessionView,
  TimelineView,
  WorkspaceView,
} from "./generated/api.js";

const WORKSPACE_SCOPE = "scope-w-1";
const PROJECT_SCOPE = "scope-p-1";
const CLAUDE_TIMELINE = JSON.parse(
  readFileSync(new URL("../src/fixtures/claude-timeline.json", import.meta.url), "utf8"),
) as TimelineView;

function workspace(): WorkspaceView {
  return {
    id: "w-1",
    slug: "payments",
    display_name: "Payments",
    status: "active",
    revision: 1,
    scope_id: WORKSPACE_SCOPE,
    created_at: "2026-08-21T00:00:00Z",
    updated_at: "2026-08-21T00:00:00Z",
  };
}

function project(): ProjectView {
  return {
    id: "p-1",
    workspace_id: "w-1",
    slug: "ledger",
    display_name: "Ledger",
    status: "active",
    revision: 1,
    scope_id: PROJECT_SCOPE,
    created_at: "2026-08-21T00:00:00Z",
    updated_at: "2026-08-21T00:00:00Z",
  };
}

/** A caller with the forecast this test wants, at the project's scope. */
function me(actions: Record<string, boolean> = {}): MeView {
  return {
    principal: { subject: "robin@example.test", display_name: "Robin", quarantined: false },
    tenant: { id: "t-1", slug: "acme", name: "ACME", status: "active" },
    onboarding: { state: "ready", workspace_count: 1, project_count: 1 },
    workspaces: [workspace()],
    projects: [project()],
    capabilities: { actions: { "session.read": true }, role_keys: ["member"] },
    anchors: [
      {
        scope_id: PROJECT_SCOPE,
        kind: "project",
        source: "selected_project",
        direct: true,
        roles: ["member"],
        actions: { "session.read": true, ...actions },
      },
    ],
  };
}

function session(overrides: Partial<SessionView> = {}): SessionView {
  return {
    id: "s-1",
    workspace_id: "w-1",
    project_id: "p-1",
    scope_id: PROJECT_SCOPE,
    principal_id: "robin@example.test",
    client_name: "claude-code",
    status: "active",
    started_at: "2026-08-23T10:00:00Z",
    metadata: {},
    created_at: "2026-08-23T10:00:00Z",
    updated_at: "2026-08-23T10:00:00Z",
    ...overrides,
  };
}

function timeline(overrides: Partial<TimelineView> = {}): TimelineView {
  return {
    session_id: "s-1",
    entries: [],
    event_counts: {},
    truncated: false,
    ...overrides,
  };
}

/** Seeds one cache key with a settled answer. */
async function seed(key: string, outcome: Outcome): Promise<void> {
  await cache.ensure(key, async () => outcome);
}

const ok = (body: unknown): Outcome => ({ kind: "ok", body });

/** The list page, with a project selected. */
function renderList(view: MeView): string {
  const selection = reconcile({ workspaceId: "w-1", projectId: "p-1" }, view);
  const context = appContext(view, selection, () => {});
  return toText(
    renderToStaticMarkup(
      <AppProvider value={context}>
        <Sessions />
      </AppProvider>,
    ),
  );
}

/** The detail page for one run. */
function renderDetail(view: MeView, sessionId = "s-1"): string {
  const selection = reconcile({ workspaceId: "w-1", projectId: "p-1" }, view);
  const context = appContext(view, selection, () => {});
  return toText(
    renderToStaticMarkup(
      <AppProvider value={context}>
        <Session sessionId={sessionId} />
      </AppProvider>,
    ),
  );
}

/** The key the list page reads under, for a first page with no filters. */
function listKey(): string {
  return `sessions/${PROJECT_SCOPE}/${JSON.stringify(listQuery(NO_FILTERS, PROJECT_SCOPE, null))}`;
}

beforeEach(() => {
  cache.clear();
});

test("an active run reads as running, and is a link to its own address", async () => {
  await seed(
    listKey(),
    ok({
      sessions: [session({ task_summary: "Refactor the ledger" })],
      next_cursor: null,
    }),
  );
  const rendered = renderList(me());
  assert.match(rendered, /Refactor the ledger/);
  assert.match(rendered, /running/);
  // The address is the requirement: a run somebody is investigating has to
  // survive a refresh and paste into a ticket.
  assert.ok(
    renderToStaticMarkup(
      <AppProvider value={appContext(me(), reconcile({ workspaceId: "w-1", projectId: "p-1" }, me()), () => {})}>
        <Sessions />
      </AppProvider>,
    ).includes('href="/console/sessions/s-1"'),
    "the row links to the run's own route",
  );
  // A run that is still going is not "incomplete": it is running, and the
  // page must not offer an explanation for a thing that has not happened.
  assert.doesNotMatch(rendered, /No reason was recorded/);
});

test("a completed run shows how it ended, and a failed one shows why", async () => {
  await seed(
    "sessions/one/s-1",
    ok(
      session({
        status: "ended",
        ended_at: "2026-08-23T10:42:00Z",
        task_summary: "Refactor the ledger",
      }),
    ),
  );
  await seed("sessions/timeline/s-1", ok(timeline({ event_counts: { "message.user": 3 } })));
  await seed("projects/p-1/repositories", ok({ repositories: [] }));
  const finished = renderDetail(me());
  assert.match(finished, /finished/);
  assert.match(finished, /2026-08-23 10:42 UTC/, "the end instant is named, not inferred");
  assert.match(finished, /ran for 42m/);
  assert.doesNotMatch(finished, /failed/);

  cache.clear();
  await seed(
    "sessions/one/s-1",
    ok(
      session({
        status: "failed",
        ended_at: "2026-08-23T10:05:00Z",
        end_reason: "the SessionEnd hook timed out",
      }),
    ),
  );
  await seed("sessions/timeline/s-1", ok(timeline()));
  await seed("projects/p-1/repositories", ok({ repositories: [] }));
  const broken = renderDetail(me());
  // The end reason is the whole reason the field exists: the status says a
  // run failed, this says what failed.
  assert.match(broken, /the SessionEnd hook timed out/);
});

test("a run names the repository it was against, not the attachment's id", async () => {
  await seed("sessions/one/s-1", ok(session({ repository_id: "r-1", branch: "main" })));
  await seed("sessions/timeline/s-1", ok(timeline()));
  await seed(
    "projects/p-1/repositories",
    ok({
      repositories: [
        {
          id: "r-1",
          project_id: "p-1",
          canonical_uri: "https://github.com/acme/ledger",
          provider: "github",
          repository_name: "acme/ledger",
          metadata: {},
          created_at: "2026-08-20T09:00:00Z",
          updated_at: "2026-08-20T09:00:00Z",
        },
      ],
    }),
  );
  const rendered = renderDetail(me());
  assert.match(rendered, /https:\/\/github.com\/acme\/ledger/);
  assert.match(rendered, /branch main/);
  assert.doesNotMatch(rendered, /\br-1\b/, "an attachment id is not something a reader recognises");
});

test("a delayed event shows both clocks and says it did not arrive live", async () => {
  await seed("sessions/one/s-1", ok(session()));
  await seed("sessions/timeline/s-1", ok(
    timeline({
      event_counts: { "message.user": 2 },
      entries: [
        {
          kind: "event",
          id: "e-1",
          at: "2026-08-23T10:00:00Z",
          received_at: "2026-08-23T10:00:01Z",
          delayed: false,
          event_type: "message.user",
          sequence: 1,
          summary: "fix the rounding bug",
        },
        {
          kind: "event",
          id: "e-2",
          at: "2026-08-23T10:01:00Z",
          received_at: "2026-08-23T11:31:00Z",
          delayed: true,
          event_type: "message.assistant",
          sequence: 2,
          summary: "here is the patch",
        },
      ],
    }),
  ));
  await seed("projects/p-1/repositories", ok({ repositories: [] }));
  const rendered = renderDetail(me());

  // Both instants, both labelled: the reader this page is for is asking
  // "did that actually happen when it says?".
  assert.match(rendered, /occurred 2026-08-23 10:01 UTC/);
  assert.match(rendered, /received 2026-08-23 11:31 UTC/);
  // And the entry says how far behind it was, without naming a cause: a
  // local spool replay and a wrong clock produce the same two instants.
  assert.match(rendered, /recovered or delayed/);
  assert.match(rendered, /1h 30m later/);
  // The live one carries no badge at all — a marker on every row is a marker
  // nobody reads.
  const lines = rendered.split("\n");
  const liveRow = lines.findIndex((line) => line.includes("fix the rounding bug"));
  assert.ok(liveRow >= 0);
  assert.doesNotMatch(lines[liveRow] as string, /recovered or delayed/);
});

test("a failed delivery is surfaced as a warning, in a banner and in place", async () => {
  await seed("sessions/one/s-1", ok(session({ status: "abandoned", ended_at: "2026-08-23T10:20:00Z" })));
  await seed("sessions/timeline/s-1", ok(
    timeline({
      event_counts: { "adapter.warning": 1, "message.user": 1 },
      entries: [
        {
          kind: "event",
          id: "e-9",
          at: "2026-08-23T10:19:00Z",
          received_at: "2026-08-23T10:19:01Z",
          delayed: false,
          event_type: "adapter.warning",
          sequence: 9,
          summary: "could not deliver 4 events; spooled to disk",
        },
      ],
    }),
  ));
  await seed("projects/p-1/repositories", ok({ repositories: [] }));
  const rendered = renderDetail(me());

  // The banner, because a warning must not be one grey line among two
  // hundred…
  assert.match(rendered, /recorded 1 adapter warning/);
  // …and the entry itself, because the banner does not say which one.
  assert.match(rendered, /could not deliver 4 events/);
  // The run is abandoned, and the page says what that means rather than
  // showing a status word.
  assert.match(rendered, /Nobody closed this run/);
});

test("raw payloads are not on the page, and the expansion is offered only where the forecast allows it", async () => {
  const entries = [
    {
      kind: "event",
      id: "e-1",
      at: "2026-08-23T10:00:00Z",
      received_at: "2026-08-23T10:00:01Z",
      delayed: false,
      event_type: "message.user",
      sequence: 1,
      summary: "fix the rounding bug",
    },
  ];
  await seed("sessions/one/s-1", ok(session()));
  await seed("sessions/timeline/s-1", ok(timeline({ entries, event_counts: { "message.user": 1 } })));
  await seed("projects/p-1/repositories", ok({ repositories: [] }));

  // Without the forecast: no control, and one sentence naming what it takes.
  const plain = renderDetail(me());
  assert.match(plain, /session\.diagnostics/);
  assert.doesNotMatch(plain, /Show raw payload/);

  // With it: the control appears — and still nothing is fetched until it is
  // clicked, which is why no payload text is on the page either way.
  const privileged = renderDetail(me({ "session.diagnostics": true }));
  assert.match(privileged, /Show raw payload/);
});

test("the database-backed Claude replay timeline renders without transcript content", async () => {
  await seed(
    "sessions/one/s-1",
    ok(session({ status: "ended", ended_at: "2026-08-23T21:05:00Z" })),
  );
  await seed("sessions/timeline/s-1", ok(CLAUDE_TIMELINE));
  await seed("projects/p-1/repositories", ok({ repositories: [] }));

  const rendered = renderDetail(me({ "session.diagnostics": true }));
  for (const summary of [
    "message.user (50 characters)",
    "tool.invoked",
    "tool.result",
    "message.assistant (111 characters)",
    "Synveda supplied 0 knowledge items",
  ]) {
    assert.ok(rendered.includes(summary), "missing gateway-produced summary: " + summary);
  }
  assert.ok(
    rendered.indexOf("tool.invoked") < rendered.indexOf("tool.result"),
    "the gateway's server-assigned order is the console's order",
  );
  assert.doesNotMatch(rendered, /Read notes\.txt|full jitter|Write that budget down/);
  assert.match(rendered, /Show raw payload/);
  const markup = renderToStaticMarkup(
    <AppProvider value={appContext(me(), reconcile({ workspaceId: "w-1", projectId: "p-1" }, me()), () => {})}>
      <Session sessionId="s-1" />
    </AppProvider>,
  );
  assert.ok(
    markup.includes('href="/console/context-runs/ID"'),
    "a context delivery opens the exact inspector rather than an unlinked timeline label",
  );
});

test("a run in a project this caller may not read renders the refusal and nothing about the run", async () => {
  await seed("sessions/one/s-2", {
    kind: "forbidden",
    message: "policy denied session.read on session s-2",
  });
  const rendered = renderDetail(me(), "s-2");
  assert.match(rendered, /Your roles do not allow this/);
  assert.match(rendered, /session\.read/);
  // Not one fact about the run, because the console was told none.
  assert.doesNotMatch(rendered, /claude-code/);
  assert.doesNotMatch(rendered, /robin@example.test/, "no principal, no client, no timeline");
  assert.doesNotMatch(rendered, /Timeline/);
});

test("another tenant's run reads exactly like an id nobody ever minted", async () => {
  // The gateway serves a uniform 404 for both (ADR-0012 decision 7), and the
  // console must not invent the distinction the product refuses to make —
  // nor leak a single field of the other tenant's row, because it was served
  // none.
  const message = "not found: session s-9";
  await seed("sessions/one/s-9", { kind: "invalid", message });
  const foreign = renderDetail(me(), "s-9");

  cache.clear();
  await seed("sessions/one/s-8", { kind: "invalid", message: "not found: session s-8" });
  const fictional = renderDetail(me(), "s-8");

  assert.equal(foreign.replace(/s-9/g, "ID"), fictional.replace(/s-8/g, "ID"));
  for (const leak of ["victim", "claude-code", "Refactor", "Timeline", "occurred"]) {
    assert.ok(!foreign.includes(leak), `the page must not carry ${leak}`);
  }
});

test("an empty page says which kind of empty it is", async () => {
  await seed(listKey(), ok({ sessions: [], next_cursor: null }));
  const rendered = renderList(me());
  // Unfiltered: it names the other possibility rather than claiming nothing
  // happened.
  assert.match(rendered, /nobody has run an agent/);
  assert.doesNotMatch(rendered, /Clear them/);
});

test("a page that has more says so, and offers the way to it", async () => {
  await seed(listKey(), ok({ sessions: [session()], next_cursor: "Y3Vyc29y" }));
  const rendered = renderList(me());
  assert.match(rendered, /Load more/);
  // And the honest note about what a page count means when rows are decided
  // one at a time.
  assert.match(rendered, /runs you may not read are skipped/);

  cache.clear();
  await seed(listKey(), ok({ sessions: [session()], next_cursor: null }));
  const last = renderList(me());
  assert.doesNotMatch(last, /Load more/);
  assert.match(last, /every run you can read here/);
});
