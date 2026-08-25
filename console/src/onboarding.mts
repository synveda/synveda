/**
 * First-run onboarding: the model behind the wizard (CPR-8, ADR-0075
 * decision 6).
 *
 * Six steps, from a deployment with nothing in it to an agent client that
 * can reach it: a workspace, a project, the repository the project is
 * about, which client you use, how to connect it, and a check that the
 * thing you just set up actually answers.
 *
 * # `personal` and `team` are a seeding choice, and nothing else
 *
 * This is the decision the whole feature turns on, so it is stated where
 * the code for it is. Choosing "just me" or "a team" **seeds** two things —
 * the immutable Configuration template bound to the new workspace, and whether a
 * group and invitations are set up beside it — and then it is over. It does
 * not write an edition anywhere. There is no column recording it, no field
 * on the workspace, nothing downstream branches on it, and a workspace
 * created as personal becomes a team workspace by inviting somebody and
 * (if you want) binding a different Configuration version.
 *
 * ADR-0068 decision 1 is the law here: one domain model, one runtime, and a
 * single person and a regulated bank differ in what their configuration
 * says and never in which code path serves them. A `WorkspaceKind` column
 * would be that branch, arriving through the friendliest possible door.
 *
 * # The seeding is best-effort, and says so
 *
 * A first user may not be forecast to publish Configuration or manage a
 * group — those are `configuration.write` and `group.manage`, and a brand-new
 * tenant's first caller holds an `owner` grant on what they just created
 * and nothing else. So each seeding step reports what happened rather than
 * failing onboarding: the workspace and project are the deliverable, and a
 * pending or refused Configuration change points at Advanced ▸ Configuration,
 * not a dead end. Silently skipping it would be the unacceptable option —
 * somebody would believe their workspace was governed the way they chose.
 */

/** The steps, in order. The wizard's state is one of these. */
export const STEPS = [
  "workspace",
  "project",
  "repository",
  "client",
  "instructions",
  "check",
  "done",
] as const;

export type Step = (typeof STEPS)[number];

/** The step after this one. `done` is terminal. */
export function nextStep(step: Step): Step {
  const index = STEPS.indexOf(step);
  return STEPS[Math.min(index + 1, STEPS.length - 1)] as Step;
}

/** One-based position, for "step 2 of 6". */
export function stepNumber(step: Step): number {
  return Math.min(STEPS.indexOf(step) + 1, STEPS.length - 1);
}

/** How many steps a person is asked to walk. `done` is not one of them. */
export const STEP_COUNT = STEPS.length - 1;

/** What the first screen asks. */
export type Shape = "personal" | "team";

/**
 * What a shape seeds.
 *
 * The template names are part of the generated Configuration contract.
 */
export interface SeedPlan {
  /** The complete Configuration template to copy and bind. */
  template: "personal" | "team";
  /** Whether to offer a group and invitations beside it. */
  invitesMembers: boolean;
  /** What the screen tells somebody they are choosing. */
  summary: string;
}

export function seedPlan(shape: Shape): SeedPlan {
  switch (shape) {
    case "personal":
      return {
        template: "personal",
        invitesMembers: false,
        summary:
          "Your own workspace. Everything you capture is available to you immediately, " +
          "with no review step in the way.",
      };
    case "team":
      return {
        template: "team",
        invitesMembers: true,
        summary:
          "A shared workspace. What gets published is reviewed first, and you can invite " +
          "people and manage them under People.",
      };
  }
}

/** Whether a seeding step landed, was refused, or was never attempted. */
export type SeedOutcome =
  | { kind: "applied"; what: string }
  | { kind: "pending"; what: string; changeId: string }
  | { kind: "refused"; what: string; why: string }
  | { kind: "skipped"; what: string };

/**
 * The sentence a refused seeding step shows.
 *
 * Names the plane the reader can finish the job on, because a refusal with
 * no next step is a dead end and the next step genuinely exists — an
 * administrator completes the change under Advanced Reviews or adjusts the
 * binding under Advanced ▸ Configuration, and the
 * workspace works meanwhile under whatever it inherits.
 */
export function seedSentence(outcome: SeedOutcome): string {
  switch (outcome.kind) {
    case "applied":
      return `${outcome.what} — done.`;
    case "pending":
      return (
        `${outcome.what} is waiting in Advanced Reviews as change ${outcome.changeId}. ` +
        `Your workspace works under its inherited Configuration until that review applies.`
      );
    case "refused":
      return (
        `${outcome.what} was refused: ${outcome.why}. Your workspace works; it is running ` +
        `its inherited Configuration until an administrator finishes it under Advanced ▸ Configuration.`
      );
    case "skipped":
      return `${outcome.what} — not needed for this choice.`;
  }
}

/** The agent clients the wizard knows how to talk about. */
export interface AgentClient {
  id: string;
  label: string;
  /** How it is connected: the plugin, or an MCP server entry. */
  via: "plugin" | "mcp";
  /** What to say about which surface it gets. */
  note: string;
}

/**
 * The clients, as data.
 *
 * Seed §2 principle 6 — "the harness is a guest; supporting a new one must
 * never require touching the core" — applies to this list too, which is why
 * the last entry is *any other MCP client* and points at the CLI's own
 * extension path (`~/.config/synveda/mcp-clients.jsonc`) rather than at us.
 * The ids match `crates/synveda-cli/src/mcp/clients.jsonc`, because they are
 * pasted into a command that reads that file.
 */
export const CLIENTS: readonly AgentClient[] = [
  {
    id: "claude-code",
    label: "Claude Code",
    via: "plugin",
    note: "Session-start injection and turn observation, through the plugin.",
  },
  {
    id: "cursor",
    label: "Cursor",
    via: "mcp",
    note: "Two tools — recall and remember — over stdio.",
  },
  {
    id: "claude-desktop",
    label: "Claude Desktop",
    via: "mcp",
    note: "Two tools — recall and remember — over stdio.",
  },
  { id: "zed", label: "Zed", via: "mcp", note: "Two tools — recall and remember — over stdio." },
  {
    id: "other",
    label: "Another MCP client",
    via: "mcp",
    note: "Anything that speaks MCP over stdio. Unknown clients are a config file, not a release.",
  },
] as const;

export function clientOf(id: string): AgentClient {
  return CLIENTS.find((client) => client.id === id) ?? (CLIENTS[CLIENTS.length - 1] as AgentClient);
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
export function connectionSteps(client: AgentClient, origin: string): string[] {
  const login = `synveda login --gateway ${origin}`;
  switch (client.via) {
    case "plugin":
      return [login, "synveda plugin install"];
    case "mcp":
      return [
        login,
        client.id === "other"
          ? "synveda mcp install --client <your-client>   # or --print to paste it yourself"
          : `synveda mcp install --client ${client.id}`,
      ];
  }
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
