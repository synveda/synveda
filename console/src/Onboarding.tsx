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

import { useCallback, useState } from "react";

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
  clientOf,
  connectionSteps,
  nextStep,
  seedPlan,
  seedSentence,
  slugFrom,
  stepNumber,
  type CheckVerdict,
  type SeedOutcome,
  type Shape,
  type Step,
} from "./onboarding.mjs";

export function Onboarding() {
  const { me, chooseWorkspace, chooseProject, reload } = useApp();
  // Resume where the deployment actually is rather than at step one: a
  // person who created a workspace and closed the tab should not be asked
  // to create another. The server's own word decides (`me.onboarding`).
  const [step, setStep] = useState<Step>(
    me.onboarding.state === "needs_project" ? "project" : "workspace",
  );
  const [shape, setShape] = useState<Shape>("personal");
  const [seed, setSeed] = useState<SeedOutcome | null>(null);
  const [workspaceId, setWorkspaceId] = useState<string | null>(null);
  const [projectId, setProjectId] = useState<string | null>(null);
  const [clientId, setClientId] = useState<string>(CLIENTS[0]?.id ?? "claude-code");
  const [verdict, setVerdict] = useState<CheckVerdict | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

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
        fail(created.kind === "unauthenticated" ? "Your session has expired." : created.message);
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
          ? listed.body.templates.find((candidate) => candidate.name === plan.template)
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
                  why: bound.kind === "unauthenticated" ? "your session has expired" : bound.message,
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
      const parent = workspaceId ?? me.workspaces[0]?.id;
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
        fail(created.kind === "unauthenticated" ? "Your session has expired." : created.message);
        return;
      }
      setProjectId(created.body.id);
      chooseProject(created.body.id);
      invalidate(ME_KEY);
      setBusy(false);
      setStep(nextStep("project"));
    },
    [workspaceId, me.workspaces, chooseProject],
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
          attached.kind === "unauthenticated" ? "Your session has expired." : attached.message,
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
    const id = projectId ?? me.projects[0]?.id;
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
    const project = await request("get_project", { path: { project_id: id } });
    const repositories = await request("list_repositories", { path: { project_id: id } });
    setBusy(false);
    setVerdict(
      checkVerdict({
        projectReadable: project.kind === "ok",
        projectWhy: project.kind === "ok" ? undefined : project.kind === "unauthenticated"
          ? "your session has expired"
          : project.message,
        repositoryCount: repositories.kind === "ok" ? repositories.body.repositories.length : 0,
      }),
    );
  }, [projectId, me.projects]);

  const finish = useCallback(() => {
    invalidate(ME_KEY);
    reload();
    navigate(hrefOf("home"));
  }, [reload]);

  return (
    <div className="onboarding">
      <header className="page-heading">
        <h1>Getting started</h1>
        <p className="muted">
          Step {stepNumber(step)} of {STEP_COUNT}
        </p>
      </header>

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
            blurb="A project is what an agent works on — usually one repository. It gets its own governed scope beneath the workspace."
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
        <ClientStep chosen={clientId} onChoose={setClientId} onNext={() => setStep(nextStep("client"))} />
      ) : null}

      {step === "instructions" ? (
        <InstructionsStep clientId={clientId} onNext={() => setStep(nextStep("instructions"))} />
      ) : null}

      {step === "check" ? (
        <CheckStep busy={busy} verdict={verdict} onRun={() => void runCheck()} onFinish={finish} />
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
        Nobody is asked to declare an organisation. A workspace is the first thing you make, and
        the tenant's root scope is minted underneath it because something needed a parent.
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
        This choice <strong>seeds</strong> the immutable runtime Configuration bound here ({plan.template})
        {plan.invitesMembers ? " and sets you up to invite people" : ""}. It is not an edition:
        nothing records it, nothing branches on it, and a workspace made for one person becomes a
        team's by inviting somebody.
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
            value={displayName}
            onChange={(event) => setDisplayName(event.target.value)}
            placeholder={placeholder}
          />
        </label>
        <label>
          <span className="switcher-label">Handle</span>
          <input
            value={handle}
            onChange={(event) => {
              setTouched(true);
              setSlug(event.target.value);
            }}
          />
        </label>
        <button type="submit" disabled={busy || displayName.trim().length === 0 || handle.length === 0}>
          {action}
        </button>
      </form>
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
        A repository, identified by its <strong>canonical remote</strong>. Paste it in any form
        git accepts — <code>https://host/owner/name</code>, <code>git@host:owner/name.git</code>,{" "}
        <code>ssh://…</code> — and the transport, credential, port and <code>.git</code> collapse
        into one identity. A path on your machine is refused, because it differs per machine and
        changes when you move a directory.
      </p>
      <form
        className="inline-form"
        onSubmit={(event) => {
          event.preventDefault();
          onAttach(remote.trim());
        }}
      >
        <label>
          <span className="switcher-label">Remote</span>
          <input
            value={remote}
            onChange={(event) => setRemote(event.target.value)}
            placeholder="https://github.com/acme/payments"
          />
        </label>
        <button type="submit" disabled={busy || remote.trim().length === 0}>
          Attach
        </button>
        <button type="button" onClick={onSkip} disabled={busy}>
          Skip — no repository
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

function InstructionsStep({ clientId, onNext }: { clientId: string; onNext: () => void }) {
  const client = clientOf(clientId);
  // This deployment's own origin rather than a placeholder: the console is
  // served from the gateway (ADR-0056 decision 1), so the address in the
  // reader's address bar is the address their CLI needs.
  const origin = window.location.origin;
  return (
    <section>
      <h2>Connect {client.label}</h2>
      <p className="muted">
        Run these on the machine that runs {client.label}. The console cannot do it for you — it
        configures an application on your computer, and a browser has no business there.
      </p>
      <ol className="commands">
        {connectionSteps(client, origin).map((command) => (
          <li key={command}>
            <code className="breakable">{command}</code>
          </li>
        ))}
      </ol>
      {client.id === "other" ? (
        <p className="muted">
          A client this release has not heard of is a config file rather than a release: add it to{" "}
          <code>~/.config/synveda/mcp-clients.jsonc</code>, or use <code>--print</code> and paste
          the entry yourself.
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
  onRun,
  onFinish,
}: {
  busy: boolean;
  verdict: CheckVerdict | null;
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
      <p className="muted">What it cannot check, and does not claim to:</p>
      <ul className="muted">
        {CHECK_CANNOT.map((line) => (
          <li key={line}>{line}</li>
        ))}
      </ul>
      <button type="button" onClick={onRun} disabled={busy}>
        {busy ? "Checking…" : "Run the check"}
      </button>
      {verdict ? (
        <div className={verdict.kind === "pass" ? "banner" : "banner error"} role="status">
          <p>
            <strong>{verdict.kind === "pass" ? "Everything answered." : "Something is wrong."}</strong>
          </p>
          <ul>
            {verdict.lines.map((line) => (
              <li key={line}>{line}</li>
            ))}
          </ul>
          {verdict.kind === "fail" ? <p>{verdict.why}</p> : null}
        </div>
      ) : null}
      <p>
        <button type="button" onClick={onFinish}>
          Finish
        </button>
      </p>
    </section>
  );
}
