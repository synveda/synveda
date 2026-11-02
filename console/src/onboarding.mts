import { GENERATED_AGENT_CLIENTS } from "./generated/adapter-clients.js";
import type { ContextRunView, MeView, SessionView } from "./generated/api.js";
import { profileChoosable } from "./profile.mjs";
import type { Selection } from "./selection.mjs";

/**
 * First-run onboarding: the model behind the wizard (ADR-0075 decision 6,
 * amended by ADR-0162).
 *
 * Seven steps, from a deployment with nothing in it to an agent client that
 * can reach it: the organisation's profile, a workspace, a project, the
 * repository the project is about, which client you use, how to connect it,
 * and a check that the thing you just set up actually answers.
 *
 * # The profile is chosen once, for the organisation, and is nothing else
 *
 * The first step offers the three canonical Configuration templates
 * (`profile.mts`) and binds the chosen one at the tenant root, before anyone
 * is invited; every workspace inherits it. It is offered only when the
 * server says this reader may adopt it now (`me.onboarding.profile`), and
 * skipping it keeps the `regulated-strict` default and leaves the choice open.
 * It does not write an edition anywhere. There is no column recording it, no
 * field on the workspace, and nothing downstream branches on it.
 *
 * ADR-0068 decision 1 is the law here: one domain model, one runtime, and a
 * single person and a regulated bank differ in what their configuration
 * says and never in which code path serves them. A `WorkspaceKind` column
 * would be that branch, arriving through the friendliest possible door.
 *
 * # The adoption is reported, never assumed
 *
 * The workspace and project are the deliverable. Adopting the profile is a
 * governed Configuration change the server may apply, send to review or
 * refuse, so the wizard reports what happened rather than failing: a pending
 * or refused change points at Advanced Reviews or Advanced ▸ Configuration,
 * not a dead end. Silently skipping it would be the unacceptable option —
 * somebody would believe their organisation was governed the way they chose.
 */

/** The steps, in order. The wizard's state is one of these. */
export const STEPS = [
  "profile",
  "workspace",
  "project",
  "repository",
  "client",
  "instructions",
  "check",
  "done",
] as const;

export type Step = (typeof STEPS)[number];

/**
 * The workspace a project would be created in: the selected one when the
 * gateway forecasts project creation there, otherwise the first one where it
 * does, otherwise none. Never an unrelated default selection.
 */
export function projectTarget(
  onboarding: MeView["onboarding"],
  selection: Selection,
): string | null {
  const eligible = onboarding.project_create_workspace_ids;
  if (selection.workspaceId && eligible.includes(selection.workspaceId)) {
    return selection.workspaceId;
  }
  return eligible[0] ?? null;
}

/**
 * Where the wizard starts. Reconnecting a selected project must not create
 * another workspace, and a creation step is only reached where that creation
 * is forecast at its own target (`me.onboarding`).
 */
export function initialStep(onboarding: MeView["onboarding"], selection: Selection): Step {
  if (onboarding.state === "ready" && selection.projectId) return "client";
  // Only first-run setup leads with the profile; Home and Configuration offer
  // it later, so reconnecting an agent is never interrupted.
  if ((onboarding.state === "needs_workspace" || onboarding.state === "needs_project") &&
      profileChoosable(onboarding.profile)) {
    return "profile";
  }
  if (projectTarget(onboarding, selection) !== null &&
      (onboarding.state === "needs_project" || onboarding.state === "ready")) {
    return "project";
  }
  if (onboarding.workspace_create) return "workspace";
  return "client";
}

/** The step after the profile, as [`initialStep`] would choose it. */
export function afterProfile(onboarding: MeView["onboarding"], selection: Selection): Step {
  return initialStep({ ...onboarding, profile: null }, selection);
}

/** The step after this one. `done` is terminal. */
export function nextStep(step: Step): Step {
  const index = STEPS.indexOf(step);
  return STEPS[Math.min(index + 1, STEPS.length - 1)] as Step;
}

/** One-based position, for "step 2 of 7". */
export function stepNumber(step: Step): number {
  return Math.min(STEPS.indexOf(step) + 1, STEPS.length - 1);
}

/** How many steps a person is asked to walk. `done` is not one of them. */
export const STEP_COUNT = STEPS.length - 1;

/** Whether a seeding step landed, was refused, or was never attempted. */
export type SeedOutcome =
  | { kind: "applied"; what: string }
  | { kind: "pending"; what: string; changeId: string }
  | { kind: "refused"; what: string; why: string }
  | { kind: "skipped"; what: string };

/**
 * The sentence a seeding step shows.
 *
 * Names the plane the reader can finish the job on, because a refusal with
 * no next step is a dead end and the next step genuinely exists — an
 * administrator completes the change under Advanced Reviews or chooses
 * under Advanced ▸ Configuration, and the organisation works meanwhile under
 * the profile it already runs.
 */
export function seedSentence(outcome: SeedOutcome): string {
  switch (outcome.kind) {
    case "applied":
      return `${outcome.what} — done.`;
    case "pending":
      return (
        `${outcome.what} is waiting in Advanced Reviews as change ${outcome.changeId}. ` +
        `Your organisation keeps its current profile until that review applies.`
      );
    case "refused":
      return (
        `${outcome.what} was refused: ${outcome.why}. Your organisation keeps its current ` +
        `profile; an administrator can choose one under Advanced ▸ Configuration.`
      );
    case "skipped":
      return (
        `${outcome.what} — not chosen yet. Your organisation runs the regulated-strict default ` +
        `until an administrator chooses under Advanced ▸ Configuration.`
      );
  }
}

/** The agent clients the wizard knows how to talk about. */
export interface AgentClient {
  id: string;
  label: string;
  /** How it is connected: the plugin, or an MCP server entry. */
  via: "plugin" | "mcp";
  /** Evidence level from the product adapter registry. */
  supportLevel:
    | "configured"
    | "captured"
    | "verified"
    | "experimental"
    | "unsupported";
  /** What to say about which surface it gets. */
  note: string;
  registration: "automatic" | "manual";
  /** `synveda adapter install` writes repository hooks and registers MCP. */
  hooks?: boolean;
  testedVersions: readonly string[];
  limits: string;
  guide: string;
}

/**
 * The clients, as data.
 *
 * Seed §2 principle 6 — "the harness is a guest; supporting a new one must
 * never require touching the core" — applies to this list too, which is why
 * the last entry is *any other MCP client* and points at the CLI's own
 * extension path (`~/.config/synveda/mcp-clients.jsonc`) rather than at us.
 * The ids and support evidence are generated from `adapters/registry.json`,
 * the same source the CLI projects into connection configuration. Knowing a
 * config path must never be presented as client verification.
 */
export const CLIENTS: readonly AgentClient[] = [
  ...GENERATED_AGENT_CLIENTS,
  {
    id: "other",
    label: "Another MCP client",
    via: "mcp",
    supportLevel: "unsupported",
    note: "Anything that speaks MCP over stdio. Unknown clients are a config file, not a release.",
    registration: "manual",
    testedVersions: [],
    limits: "No native lifecycle or platform qualification for this client.",
    guide: "https://github.com/synveda/synveda/blob/main/docs/CLIENT_SUPPORT.md",
  },
] as const;

export function clientOf(id: string): AgentClient {
  return (
    CLIENTS.find((client) => client.id === id) ??
    (CLIENTS[CLIENTS.length - 1] as AgentClient)
  );
}

/**
 * The commands to run, for a client and a gateway origin.
 *
 * Real commands, from `synveda --help`, in the order they must be run. The
 * console cannot run any of them — they configure an application on the
 * reader's machine, and a browser has no business there — so what it can do
 * is print them correctly, which means printing this deployment's own
 * origin rather than a placeholder somebody has to substitute.
 */
export function connectionSteps(client: AgentClient, origin: string, projectId?: string): string[] {
  const login = `synveda login --gateway ${origin}`;
  const setup = projectId ? [`synveda setup --project ${projectId} --observation off`] : [];
  if (client.hooks) {
    return [login, ...(projectId ? setup : ["synveda setup --project PROJECT_ID --observation off"]), `synveda adapter install --client ${client.id}`];
  }
  switch (client.via) {
    case "plugin":
      return [login, ...setup, projectId ? "synveda adapter install --client claude-code" : "synveda plugin install"];
    case "mcp":
      return [
        login,
        ...setup,
        projectId
          ? `synveda adapter install --client ${client.id === "other" ? "<your-client>" : client.id} --scope user`
          : client.id === "other"
          ? "synveda mcp install --client <your-client>   # or --print to paste it yourself"
          : `synveda mcp install --client ${client.id}`,
      ];
  }
}

/** Evidence is scoped to this reader and project; client labels are self-reported. */
export interface ClientEvidence { checked: boolean; contextRecorded: boolean; observationsRecorded: boolean; lines: string[] }

export function clientEvidence(sessions: SessionView[], runs: ContextRunView[], projectId: string, subject: string, clientId: string): ClientEvidence {
  const matching = sessions.filter((s) => s.project_id === projectId && s.principal_id === subject && (s.client_name === clientId || (clientId !== "claude-code" && s.client_name === "mcp")));
  const ids = new Set(matching.map((s) => s.id));
  const context = runs.some((r) => ids.has(r.session_id) && r.project_id === projectId && r.completion_status === "completed");
  const observed = matching.some((s) => s.last_observed_at != null);
  return { checked: true, contextRecorded: context, observationsRecorded: observed, lines: [
    context ? "An authenticated context operation is recorded for your selected project's Session." : "No completed client context operation found in the checked page. Run context/recall in the client, then check again or inspect Sessions.",
    observed ? "Authenticated Session observations are recorded. Review their source in Sessions." : "Session observation remains unverified; context-only clients do not capture turns automatically.",
    "Session client names are self-reported; the console can also request context. This evidence does not independently prove client initiation, vendor loading, tool trust or model use.",
  ] };
}

export interface SetupProgress { projectId: string | null; step: Step; clientId: string; setupCompleted: boolean; seed: SeedOutcome | null }

// Exploration affects navigation only, never the server's onboarding authority.
const exploring = new Set<string>();
export function allowExploration(tenant: string, subject: string): void {
  const key = `synveda.explore:${tenant}:${subject}`;
  exploring.add(key);
  try { window.sessionStorage.setItem(key, "true"); } catch { /* This tab still permits exploration. */ }
}
export function canExplore(tenant: string, subject: string): boolean {
  const key = `synveda.explore:${tenant}:${subject}`;
  try { return exploring.has(key) || window.sessionStorage.getItem(key) === "true"; } catch { return exploring.has(key); }
}
export function readProgress(storage: Pick<Storage, "getItem"> | null, key: string, projectId: string | null): SetupProgress | null {
  try {
    const raw = storage?.getItem(key);
    if (!raw || raw.length > 4096) return null;
    const p = JSON.parse(raw) as SetupProgress;
    if (p.projectId !== projectId || !STEPS.includes(p.step) || !CLIENTS.some((c) => c.id === p.clientId) || typeof p.setupCompleted !== "boolean") return null;
    if (p.seed !== null && (!p.seed || typeof p.seed.what !== "string" ||
        !["applied", "pending", "refused", "skipped"].includes(p.seed.kind) ||
        (p.seed.kind === "pending" && typeof p.seed.changeId !== "string") ||
        (p.seed.kind === "refused" && typeof p.seed.why !== "string"))) return null;
    return p;
  } catch { return null; }
}

/**
 * What the connection check actually checks, and what it cannot.
 *
 * Stated as data so the screen can render it beside the verdict. The
 * honesty matters: the browser is confined to this origin by the console's
 * own Content-Security-Policy (`connect-src 'self'`), so it can prove the
 * gateway answers *for this reader* and that the project an agent will name
 * is readable — and it cannot prove anything about a process on the
 * reader's machine. A check that implied otherwise would be the worst kind
 * of green tick.
 */
export const CHECK_COVERS = [
  "the gateway answers this browser, with your session",
  "the project you chose exists and you may read it",
  "the repository you attached is on that project",
] as const;

export const CHECK_CANNOT = [
  "that your agent client is installed, or that it holds a credential",
] as const;

export type CheckVerdict =
  | { kind: "pass"; lines: string[] }
  | { kind: "fail"; lines: string[]; why: string };

/**
 * Turns the three probe results into one verdict.
 *
 * A repository is **not** required to pass. Attaching one is a step of the
 * wizard because a project usually is about a repository, but a project
 * that is not about one is a legitimate project — so a missing repository
 * is reported and does not fail the check, while an *unreadable project* is
 * a failure, because that is the thing an agent is about to name.
 */
export function checkVerdict(probes: {
  projectReadable: boolean;
  projectWhy?: string;
  repositoryCount: number;
}): CheckVerdict {
  const lines = [
    "the gateway answered this browser with your session",
    probes.projectReadable
      ? "the project you chose is readable"
      : "the project you chose could not be read",
    probes.repositoryCount > 0
      ? `${probes.repositoryCount} repository/repositories attached`
      : "no repository attached — that is allowed, and you can add one under Settings",
  ];
  return probes.projectReadable
    ? { kind: "pass", lines }
    : {
        kind: "fail",
        lines,
        why: probes.projectWhy ?? "the gateway did not say why",
      };
}

/**
 * A slug proposed from a display name.
 *
 * The same grammar the gateway enforces (`^[a-z0-9][a-z0-9-]{0,62}$`,
 * ADR-0070/ADR-0071) — a suggestion, because the server is what refuses a
 * bad one, and having to invent a handle before you can name a thing is
 * exactly the friction this wizard exists to remove.
 */
export function slugFrom(displayName: string): string {
  const slug = displayName
    .toLowerCase()
    .normalize("NFKD")
    // NFKD splits `ü` into `u` + a combining diaeresis, and a combining
    // mark left in place becomes a separator two lines down — which turns
    // "Ünicode" into `u-nicode`. Dropping the marks is what makes the
    // decomposition useful rather than harmful.
    .replace(/\p{M}+/gu, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+/, "")
    .replace(/-+$/, "")
    .slice(0, 63);
  return slug.replace(/-+$/, "");
}
