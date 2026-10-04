/**
 * First-run onboarding (CPR-8, ADR-0075 decision 6).
 *
 * Six steps from a deployment with nothing in it to an agent that can reach
 * it. The judgements — what `personal` and `team` seed, which commands a
 * client needs, what the connection check can honestly claim — are in
 * `onboarding.mts`, tested there; this is the screen that drives them.
 *
 * # It is a client of the product's own API and nothing else
 *
 * Every step here is a call anybody could make: `POST /v1/workspaces`,
 * `POST /v1/workspaces/{id}/projects`, `POST /v1/projects/{id}/repositories`,
 * and the generated Configuration create/bind operations. There is no bootstrap path, no
 * privileged route and nothing that writes behind the decision point — an
 * installer or a wizard runs once, before anybody is watching, which makes
 * it the worst place in a product to keep a shortcut (seed §2.2).
 *
 * # And it never fails on a seeding step
 *
 * The workspace and the project are the deliverable. Creating and binding Configuration
 * and creating a group are **seeding**, and a first caller may not be
 * permitted either — so those are attempted, reported in the reader's own
 * words, and never allowed to block the wizard. Silently skipping them is
 * the one thing that would be wrong: somebody would leave believing their
 * workspace was governed the way they picked.
 */

import { useCallback, useEffect, useRef, useState } from "react";

import { ME_KEY } from "./App.js";
import { idempotencyKey, request } from "./client.mjs";
import { invalidate } from "./Query.js";
import { navigate } from "./Router.js";
import { hrefOf } from "./routes.mjs";
import { useApp } from "./Shell.js";
import {
  CHECK_CANNOT,
  CHECK_COVERS,
  CLIENTS,
  STEP_COUNT,
  checkVerdict,
  clientEvidence,
  readProgress,
  allowExploration,
  clientOf,
  connectionSteps,
  initialStep,
  nextStep,
  seedPlan,
  seedSentence,
  slugFrom,
  stepNumber,
  type CheckVerdict,
  type ClientEvidence,
  type SeedOutcome,
  type Shape,
  type Step,
} from "./onboarding.mjs";

export function Onboarding() {
  const { me, selection, chooseWorkspace, chooseProject, reload } = useApp();
  // Resume where the deployment actually is rather than at step one: a
  // person who created a workspace and closed the tab should not be asked
  // to create another. The server's own word decides (`me.onboarding`).
  const progressKey = `synveda.onboarding:${me.tenant.id}:${me.principal.subject}`;
  const storage = () => { try { return typeof window === "undefined" ? null : window.localStorage; } catch { return null; } };
  const [saved] = useState(() => readProgress(storage(), progressKey, selection.projectId));
  const [setupCompleted, setSetupCompleted] = useState(saved?.setupCompleted ?? false);
  const [evidence, setEvidence] = useState<ClientEvidence | null>(null);
  const [step, setStep] = useState<Step>(() =>
    saved?.step ?? initialStep(me.onboarding.state, selection),
  );
  const [shape, setShape] = useState<Shape>("personal");
  const [seed, setSeed] = useState<SeedOutcome | null>(saved?.seed ?? null);
  const [workspaceId, setWorkspaceId] = useState<string | null>(
    selection.workspaceId,
  );
  const [projectId, setProjectId] = useState<string | null>(
    selection.projectId,
  );
  const [reconnecting] = useState(
    me.onboarding.state === "ready" && selection.projectId !== null,
  );
  const [clientId, setClientId] = useState<string>(
    saved?.clientId ?? CLIENTS[0]?.id ?? "claude-code",
  );
  const [verdict, setVerdict] = useState<CheckVerdict | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const heading = useRef<HTMLHeadingElement>(null);
  const previousStep = useRef(step);

  useEffect(() => {
    if (previousStep.current !== step) {
      heading.current?.focus();
      previousStep.current = step;
    }
  }, [step]);

  useEffect(() => {
    try { storage()?.setItem(progressKey, JSON.stringify({ projectId, step, clientId, setupCompleted, seed })); } catch { /* Setup can continue when browser storage is unavailable. */ }
  }, [progressKey, projectId, step, clientId, setupCompleted, seed]);

  const fail = (message: string) => {
    setBusy(false);
    setError(message);
  };

  /** Step 1: the workspace, and the seeding its shape asks for. */
  const createWorkspace = useCallback(
    async (displayName: string, slug: string) => {
      setBusy(true);
      setError(null);
      const created = await request("create_workspace", {
        idempotencyKey: idempotencyKey(),
        body: { display_name: displayName, slug },
      });
      if (created.kind !== "ok") {
        fail(
          created.kind === "unauthenticated"
            ? "Your session has expired."
            : created.message,
        );
        return;
      }
      const workspace = created.body;
      setWorkspaceId(workspace.id);
      chooseWorkspace(workspace.id);

      // Seeding: best-effort, reported either way. See the module note.
      const plan = seedPlan(shape);
      const listed = await request("list_configuration_templates", {});
      const template =
        listed.kind === "ok"
          ? listed.body.templates.find(
              (candidate) => candidate.name === plan.template,
            )
          : undefined;
      if (!template) {
        setSeed({
          kind: "refused",
          what: `Creating the ${plan.template} runtime Configuration`,
          why:
            listed.kind === "unauthenticated"
              ? "your session has expired"
              : listed.kind === "ok"
                ? "the server did not offer that template"
                : listed.message,
        });
      } else {
        const createdConfiguration = await request("create_configuration", {
          idempotencyKey: idempotencyKey(),
          body: {
            governing_scope_id: workspace.scope_id,
            name: `${plan.template}-runtime`,
            source_template: plan.template,
            document: template.document,
          },
        });
        if (createdConfiguration.kind !== "ok") {
          setSeed({
            kind: "refused",
            what: `Creating the ${plan.template} runtime Configuration`,
            why:
              createdConfiguration.kind === "unauthenticated"
                ? "your session has expired"
                : createdConfiguration.message,
          });
        } else if (createdConfiguration.body.outcome === "pending_review") {
          setSeed({
            kind: "pending",
            what: `The ${plan.template} runtime Configuration`,
            changeId: createdConfiguration.body.change_id,
          });
        } else if (
          createdConfiguration.body.outcome === "applied" &&
          createdConfiguration.body.artifact_id
        ) {
          const bound = await request("create_configuration_binding", {
            idempotencyKey: idempotencyKey(),
            body: {
              scope_id: workspace.scope_id,
              artifact_id: createdConfiguration.body.artifact_id,
              enabled: true,
            },
          });
          setSeed(
            bound.kind !== "ok"
              ? {
                  kind: "refused",
                  what: `Binding the ${plan.template} runtime Configuration`,
                  why:
                    bound.kind === "unauthenticated"
                      ? "your session has expired"
                      : bound.message,
                }
              : bound.body.outcome === "pending_review"
                ? {
                    kind: "pending",
                    what: `The ${plan.template} runtime Configuration binding`,
                    changeId: bound.body.change_id,
                  }
                : bound.body.outcome === "applied"
                  ? {
                      kind: "applied",
                      what: `The ${plan.template} runtime Configuration at this workspace`,
                    }
                  : {
                      kind: "refused",
                      what: `Binding the ${plan.template} runtime Configuration`,
                      why: "the governed change was rejected",
                    },
          );
        } else {
          setSeed({
            kind: "refused",
            what: `Creating the ${plan.template} runtime Configuration`,
            why: "the governed change was rejected",
          });
        }
      }
      invalidate(ME_KEY);
      setBusy(false);
      setStep(nextStep("workspace"));
    },
    [shape, chooseWorkspace],
  );

  /** Step 2: the first project. */
  const createProject = useCallback(
    async (displayName: string, slug: string) => {
      const parent = workspaceId;
      if (!parent) {
        fail("No workspace to create this in.");
        return;
      }
      setBusy(true);
      setError(null);
      const created = await request("create_project", {
        path: { workspace_id: parent },
        idempotencyKey: idempotencyKey(),
        body: { display_name: displayName, slug },
      });
      if (created.kind !== "ok") {
        fail(
          created.kind === "unauthenticated"
            ? "Your session has expired."
            : created.message,
        );
        return;
      }
      setProjectId(created.body.id);
      chooseProject(created.body.id);
      invalidate(ME_KEY);
      setBusy(false);
      setStep(nextStep("project"));
    },
    [workspaceId, chooseProject],
  );

  /** Step 3: the repository, which is optional and says so. */
  const attachRepository = useCallback(
    async (remote: string) => {
      if (!projectId) {
        setStep(nextStep("repository"));
        return;
      }
      setBusy(true);
      setError(null);
      const attached = await request("attach_repository", {
        path: { project_id: projectId },
        idempotencyKey: idempotencyKey(),
        body: { remote_uri: remote },
      });
      if (attached.kind !== "ok") {
        fail(
          attached.kind === "unauthenticated"
            ? "Your session has expired."
            : attached.message,
        );
        return;
      }
      setBusy(false);
      setStep(nextStep("repository"));
    },
    [projectId],
  );

  /** Step 6: the check, over what a browser can honestly verify. */
  const runCheck = useCallback(async () => {
    setBusy(true);
    setError(null);
    const id = projectId;
    if (!id) {
      setBusy(false);
      setVerdict(
        checkVerdict({
          projectReadable: false,
          projectWhy: "no project was selected",
          repositoryCount: 0,
        }),
      );
      return;
    }
    setEvidence(null);
    const project = await request("get_project", { path: { project_id: id } });
    const repositories = await request("list_repositories", {
      path: { project_id: id },
    });
    const [sessions, runs] = await Promise.all([
      request("list_sessions", { query: { project_id: id, principal_id: me.principal.subject, limit: "50" } }),
      request("list_context_runs", { query: { project_id: id, principal_id: me.principal.subject, limit: "50" } }),
    ]);
    setEvidence(sessions.kind === "ok" && runs.kind === "ok"
      ? clientEvidence(sessions.body.sessions, runs.body.runs, id, me.principal.subject, clientId)
      : { checked: false, contextRecorded: false, observationsRecorded: false, lines: ["Client evidence could not be checked. Open Sessions and Context, or sign in again if your session expired. Browser project access still does not prove client delivery."] });
    setBusy(false);
    setVerdict(
      checkVerdict({
        projectReadable: project.kind === "ok",
        projectWhy:
          project.kind === "ok"
            ? undefined
            : project.kind === "unauthenticated"
              ? "your session has expired"
              : project.message,
        repositoryCount:
          repositories.kind === "ok"
            ? repositories.body.repositories.length
            : 0,
      }),
    );
  }, [projectId, clientId, me.principal.subject]);

  const finish = useCallback(() => {
    allowExploration(me.tenant.id, me.principal.subject);
    invalidate(ME_KEY);
    reload();
    navigate(hrefOf("home"));
  }, [reload, me.tenant.id, me.principal.subject]);

  if (me.onboarding.state === "blocked") {
    return (
      <div className="onboarding">
        <header className="page-heading">
          <h1>Getting started</h1>
        </header>
        <div className="banner" role="status">
          Ask your administrator for workspace access before connecting an
          agent.
        </div>
      </div>
    );
  }

  return (
    <div className="onboarding">
      <header className="page-heading">
        <h1 ref={heading} tabIndex={-1} aria-describedby="setup-progress">
          {reconnecting ? "Connect an agent" : "Getting started"}
        </h1>
        <p id="setup-progress" className="muted">
          Step {reconnecting ? stepNumber(step) - 3 : stepNumber(step)} of{" "}
          {reconnecting ? 3 : STEP_COUNT}
          {projectId
            ? ` · ${me.projects.find((item) => item.id === projectId)?.display_name ?? "Selected project"}`
            : ""}
        </p>
        <progress
          aria-label="Setup progress"
          value={reconnecting ? stepNumber(step) - 3 : stepNumber(step)}
          max={reconnecting ? 3 : STEP_COUNT}
        />
      </header>

      <p><button type="button" onClick={finish}>Explore the console; connect a client later</button></p>
      <ul aria-label="First-use status">
        <li>Server available: this browser loaded the console.</li>
        <li>Signed in: {me.principal.display_name ?? me.principal.subject}.</li>
        <li>Project accessible: {projectId ? "selected; check below to verify current access" : "select or create a project"}.</li>
        <li>Client setup completed: {setupCompleted ? "confirmed by you; vendor loading remains unverified" : "not confirmed"}.</li>
        <li>Client operation observed: {evidence === null ? "not checked" : !evidence.checked ? "check unavailable" : evidence.contextRecorded || evidence.observationsRecorded ? "authenticated Session evidence recorded; client initiation remains unverified" : "no operation found in the checked page"}.</li>
      </ul>
      {seed ? <div className="banner" role="status">{seedSentence(seed)}</div> : null}
      {error ? (
        <div className="banner error" role="alert">
          {error}
        </div>
      ) : null}

      {step === "workspace" ? (
        <WorkspaceStep
          shape={shape}
          onShape={setShape}
          busy={busy}
          onSubmit={(name, slug) => void createWorkspace(name, slug)}
        />
      ) : null}

      {step === "project" ? (
        <>
          {seed ? <p className="muted">{seedSentence(seed)}</p> : null}
          <NameStep
            title="Your first project"
            blurb="A project keeps the sessions, knowledge and settings for one piece of work together. It usually follows a repository."
            placeholder="Payments"
            busy={busy}
            action="Create project"
            onSubmit={(name, slug) => void createProject(name, slug)}
          />
        </>
      ) : null}

      {step === "repository" ? (
        <RepositoryStep
          busy={busy}
          onAttach={(remote) => void attachRepository(remote)}
          onSkip={() => setStep(nextStep("repository"))}
        />
      ) : null}

      {step === "client" ? (
        <ClientStep
          chosen={clientId}
          onChoose={(id) => { setClientId(id); setSetupCompleted(false); setEvidence(null); }}
          onNext={() => setStep(nextStep("client"))}
        />
      ) : null}

      {step === "instructions" ? (
        <InstructionsStep
          clientId={clientId}
          projectId={projectId}
          onNext={() => { setSetupCompleted(true); setStep(nextStep("instructions")); }}
        />
      ) : null}

      {step === "check" ? (
        <CheckStep
          busy={busy}
          verdict={verdict}
          evidence={evidence}
          onRun={() => void runCheck()}
          onFinish={finish}
        />
      ) : null}
    </div>
  );
}

function WorkspaceStep({
  shape,
  onShape,
  busy,
  onSubmit,
}: {
  shape: Shape;
  onShape: (shape: Shape) => void;
  busy: boolean;
  onSubmit: (displayName: string, slug: string) => void;
}) {
  const plan = seedPlan(shape);
  return (
    <section>
      <h2>Your workspace</h2>
      <p className="muted">
        A workspace brings your projects and the people working on them
        together. Give it a name you will recognise.
      </p>
      <fieldset className="choice">
        <legend>Who is this for?</legend>
        <label>
          <input
            type="radio"
            name="shape"
            checked={shape === "personal"}
            onChange={() => onShape("personal")}
          />
          Just me
        </label>
        <label>
          <input
            type="radio"
            name="shape"
            checked={shape === "team"}
            onChange={() => onShape("team")}
          />
          A team
        </label>
      </fieldset>
      <p className="muted">{plan.summary}</p>
      <p className="muted">
        Synveda will request the <strong>{plan.template}</strong> settings
        template for this workspace. Your permissions and review policy still
        apply. You can change these settings later under Advanced →
        Configuration.
      </p>
      <NameStep
        title=""
        blurb=""
        placeholder="Payments team"
        busy={busy}
        action="Create workspace"
        onSubmit={onSubmit}
      />
    </section>
  );
}

/** A name and a handle. Shared by the workspace and project steps. */
function NameStep({
  title,
  blurb,
  placeholder,
  busy,
  action,
  onSubmit,
}: {
  title: string;
  blurb: string;
  placeholder: string;
  busy: boolean;
  action: string;
  onSubmit: (displayName: string, slug: string) => void;
}) {
  const [displayName, setDisplayName] = useState("");
  const [slug, setSlug] = useState("");
  const [touched, setTouched] = useState(false);
  const handle = touched ? slug : slugFrom(displayName);
  return (
    <section>
      {title ? <h2>{title}</h2> : null}
      {blurb ? <p className="muted">{blurb}</p> : null}
      <form
        className="inline-form"
        onSubmit={(event) => {
          event.preventDefault();
          onSubmit(displayName.trim(), handle.trim());
        }}
      >
        <label>
          <span className="switcher-label">Name</span>
          <input
            required
            value={displayName}
            onChange={(event) => setDisplayName(event.target.value)}
            placeholder={placeholder}
          />
        </label>
        <label>
          <span className="switcher-label">Handle</span>
          <input
            required
            aria-describedby="handle-help"
            value={handle}
            onChange={(event) => {
              setTouched(true);
              setSlug(event.target.value);
            }}
          />
        </label>
        <button
          type="submit"
          disabled={
            busy || displayName.trim().length === 0 || handle.length === 0
          }
        >
          {action}
        </button>
      </form>
      <p id="handle-help" className="field-help muted">
        The handle is a short identifier, filled in from the name. You can edit
        it before creating.
      </p>
    </section>
  );
}

function RepositoryStep({
  busy,
  onAttach,
  onSkip,
}: {
  busy: boolean;
  onAttach: (remote: string) => void;
  onSkip: () => void;
}) {
  const [remote, setRemote] = useState("");
  return (
    <section>
      <h2>What is this project about?</h2>
      <p className="muted">
        Add the repository's Git URL so Synveda can recognise the same project
        across machines. HTTPS and SSH URLs work; a local folder path does not.
        You can also add this later in Settings.
      </p>
      <form
        className="inline-form"
        onSubmit={(event) => {
          event.preventDefault();
          onAttach(remote.trim());
        }}
      >
        <label>
          <span className="switcher-label">Repository URL</span>
          <input
            value={remote}
            onChange={(event) => setRemote(event.target.value)}
            placeholder="https://github.com/acme/payments"
          />
        </label>
        <button type="submit" disabled={busy || remote.trim().length === 0}>
          Add repository
        </button>
        <button type="button" onClick={onSkip} disabled={busy}>
          Skip for now
        </button>
      </form>
    </section>
  );
}

function ClientStep({
  chosen,
  onChoose,
  onNext,
}: {
  chosen: string;
  onChoose: (id: string) => void;
  onNext: () => void;
}) {
  return (
    <section>
      <h2>Which agent client do you use?</h2>
      <ul className="client-choices">
        {CLIENTS.map((client) => (
          <li key={client.id}>
            <label>
              <input
                type="radio"
                name="client"
                checked={chosen === client.id}
                onChange={() => onChoose(client.id)}
              />
              <strong>{client.label}</strong>
              <div className="muted">{client.note}</div>
              <div className="muted">Tested versions: {client.testedVersions.join(", ") || "none"}. {client.limits}</div>
              <div>{client.registration === "manual" ? "Manual configuration available" : "Configuration writer available"}</div>
            </label>
          </li>
        ))}
      </ul>
      <button type="button" onClick={onNext}>
        Show me how
      </button>
    </section>
  );
}

function InstructionsStep({
  clientId,
  projectId,
  onNext,
}: {
  clientId: string;
  projectId: string | null;
  onNext: () => void;
}) {
  const client = clientOf(clientId);
  // This deployment's own origin rather than a placeholder: the console is
  // served from the gateway (ADR-0056 decision 1), so the address in the
  // reader's address bar is the address their CLI needs.
  const origin = window.location.origin;
  return (
    <section>
      <h2>Connect {client.label}</h2>
      <p className="muted">
        Run these commands in a terminal on the machine where you use{" "}
        {client.label}. They require the Synveda CLI on your PATH.
      </p>
      <p><a href="https://github.com/synveda/synveda/blob/main/docs/CONSUMER_CLI.md">Install the Synveda CLI</a> matching this server’s release or source candidate, then add its binary directory to PATH. These setup commands require the current candidate CLI; the published v0.4.3 CLI has an earlier client contract.</p>
      <p>Run project setup at the Git repository root. Observation is initially off. To opt into recording transcript/tool events, explicitly rerun <code>synveda setup --project {projectId} --observation on</code> and follow the trusted-hook instructions. Capture creates proposed learnings for review.</p>
      <ol className="commands">
        {connectionSteps(client, origin, projectId ?? undefined).map((command) => (
          <li key={command}>
            <code className="breakable">{command}</code>
          </li>
        ))}
      </ol>
      <p>Use your client's normal repository, hook and MCP trust prompts. For the dedicated Codex/Copilot invocation, replace <code>YOUR-TASK-KEY</code> with one stable key for this task. Reuse it only for the same task; keep fixed task keys out of shared global configuration. A native conversation ID is not a Synveda Session ID.</p>
      <p>The managed MCP writer records this selected project's arguments in the client’s user configuration. Shared MCP servers stay unbound: pass the Synveda Session ID from host context with each call. Without a host-supplied Session, use the dedicated task route or the manual guide before recall. Optional hooks own observations and use <code>--writes host</code>; follow the guide before combining them with MCP.</p>
      <p>The printed Codex/Copilot route exposes recall only. For other MCP routes, <code>remember</code> is an explicit governed observation tool; call it only for evidence you consent to retain. Project hook consent does not control separate MCP tool calls.</p>
      <p><a href={client.guide}>Manual setup, hooks and platform limits for {client.label}</a> · <a href="https://github.com/synveda/synveda/blob/main/docs/CLIENT_SUPPORT.md">Detailed conformance evidence</a></p>
      {client.id === "other" ? (
        <p className="muted">
          Add your client's settings to{" "}
          <code>~/.config/synveda/mcp-clients.jsonc</code>, or use{" "}
          <code>--print</code> and paste the entry yourself.
        </p>
      ) : null}
      <button type="button" onClick={onNext}>
        I have run these
      </button>
    </section>
  );
}

function CheckStep({
  busy,
  verdict,
  evidence,
  onRun,
  onFinish,
}: {
  busy: boolean;
  verdict: CheckVerdict | null;
  evidence: ClientEvidence | null;
  onRun: () => void;
  onFinish: () => void;
}) {
  return (
    <section>
      <h2>Connection check</h2>
      <p className="muted">What this checks:</p>
      <ul>
        {CHECK_COVERS.map((line) => (
          <li key={line}>{line}</li>
        ))}
      </ul>
      <p className="muted">
        You will still need to check these in your agent client:
      </p>
      <ul className="muted">
        {CHECK_CANNOT.map((line) => (
          <li key={line}>{line}</li>
        ))}
      </ul>
      <button type="button" onClick={onRun} disabled={busy}>
        {busy ? "Checking…" : "Run the check"}
      </button>
      {verdict ? (
        <div
          className={verdict.kind === "pass" ? "banner" : "banner error"}
          role="status"
        >
          <p>
            <strong>
              {verdict.kind === "pass"
                ? "Your project is reachable."
                : "The connection check needs attention."}
            </strong>
          </p>
          <ul>
            {verdict.lines.map((line) => (
              <li key={line}>{line}</li>
            ))}
          </ul>
          {verdict.kind === "fail" ? <p>{verdict.why}</p> : null}
        </div>
      ) : null}
      <ul aria-label="Authenticated client evidence">{evidence?.lines.map((line) => <li key={line}>{line}</li>)}</ul>
      <p>
        <button type="button" onClick={onFinish}>
          Go to Home
        </button>
      </p>
    </section>
  );
}
