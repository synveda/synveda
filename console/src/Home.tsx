/** Home uses only the authorised workspace and access facts already in /v1/me. */
import { Link } from "./Router.js";
import { NavIcon } from "./NavIcon.js";
import { hrefOf, type RouteId } from "./routes.mjs";
import { PageHeading, useApp } from "./Shell.js";
import { projectsOf } from "./selection.mjs";

const tasks: { route: RouteId; title: string; description: string }[] = [
  {
    route: "sessions",
    title: "Follow your agent sessions",
    description: "See what ran, what happened and what was captured.",
  },
  {
    route: "learnings",
    title: "Review new learnings",
    description: "Check suggestions from sessions and decide what to keep.",
  },
  {
    route: "knowledge",
    title: "Browse shared knowledge",
    description:
      "Find decisions, conventions and procedures your agents can use.",
  },
  {
    route: "context",
    title: "Try a context request",
    description: "See which information Synveda selects for a task and why.",
  },
];

/**
 * What a signed-in person without usable shared access sees: why, what to
 * do, and the account it applies to. Signing in proves who somebody is; it
 * is not a workspace grant, so this never offers a creation the gateway would
 * refuse (`me.onboarding`).
 */
export function AccessRequired() {
  const { me, reload } = useApp();
  const state = me.onboarding.state;
  const who = me.principal.display_name ?? me.principal.subject;
  return (
    <div className="banner" role="status">
      {state === "blocked" ? (
        <>
          <h2>Your account cannot use Synveda yet</h2>
          <p>
            This account is not provisioned for {me.tenant.name}, so every request is refused.
            Ask an administrator to check your identity-provider account and its admission.
          </p>
        </>
      ) : me.onboarding.access_reason === "no_usable_project" ? (
        <>
          <h2>No project you can use yet</h2>
          <p>
            You can see {me.workspaces.map((workspace) => workspace.display_name).join(", ")}, but
            it has no project you can use and your role does not allow creating one. Ask a
            workspace administrator to create a project or give you access to one.
          </p>
        </>
      ) : (
        <>
          <h2>You need access to a workspace or project</h2>
          <p>
            You are signed in, but nothing has been shared with this account yet. If someone sent
            you an invitation link, open it in this browser to review and accept it. Otherwise ask
            an administrator to invite you or add you to a workspace or project.
          </p>
          <p className="muted">
            Your private space still works where policy allows it: Knowledge and Skills marked
            “Private to me” belong to you alone.
          </p>
        </>
      )}
      <p className="muted">
        Signed in as <strong>{who}</strong>
        {who !== me.principal.subject ? ` (${me.principal.subject})` : ""} in {me.tenant.name}.
        After an administrator changes your access, refresh it here — signing in again is not
        needed.
      </p>
      {state === "blocked" ? null : (
        <p>
          <button type="button" onClick={reload}>
            Refresh access
          </button>
        </p>
      )}
    </div>
  );
}

export function Home() {
  const { me, workspace, project, selection } = useApp();
  const projects = projectsOf(me, selection.workspaceId);
  const unusable =
    me.onboarding.state === "blocked" || me.onboarding.state === "access_required";

  return (
    <>
      <PageHeading route="home" />
      {unusable ? (
        <AccessRequired />
      ) : (
        <>
          <section className="project-overview" aria-label="Current selection">
            <div>
              <p className="eyebrow">Workspace</p>
              <h2>
                {workspace?.display_name ??
                  (project ? "Shared with you" : "No workspace selected")}
              </h2>
              {!workspace && project ? (
                <p className="muted">
                  This project was shared with you on its own; its workspace is not.
                </p>
              ) : null}
              {workspace ? (
                <p className="muted">
                  {projects.length} project{projects.length === 1 ? "" : "s"}{" "}
                  available to you
                </p>
              ) : null}
            </div>
            <div>
              <p className="eyebrow">Project</p>
              <h2>{project?.display_name ?? "Choose a project"}</h2>
              <p className="muted">
                {project
                  ? "Use the selectors above to switch where you work."
                  : "Create a project in Settings to organise your agent work."}
              </p>
            </div>
          </section>

          <div className="home-columns">
            <section aria-labelledby="home-tasks">
              <h2 id="home-tasks">Continue your work</h2>
              <ul className="task-list">
                {tasks.map((task) => (
                  <li key={task.route}>
                    <Link href={hrefOf(task.route)} className="task-link">
                      <span className="task-icon">
                        <NavIcon route={task.route} />
                      </span>
                      <span>
                        <strong>{task.title}</strong>
                        <span className="muted task-description">
                          {task.description}
                        </span>
                      </span>
                      <span className="task-arrow" aria-hidden="true">
                        →
                      </span>
                    </Link>
                  </li>
                ))}
              </ul>
            </section>

            <section className="setup-panel" aria-labelledby="home-setup">
              <p className="eyebrow">Project setup</p>
              <h2 id="home-setup">Bring your agent along</h2>
              <p className="muted">
                Choose your client and follow the connection instructions for
                this installation.
              </p>
              <Link href={hrefOf("welcome")} className="button primary">
                Connect an agent
              </Link>
              <ul className="setup-links">
                <li>
                  <Link href={hrefOf("settings")}>
                    Manage projects and repositories{" "}
                    <span aria-hidden="true">→</span>
                  </Link>
                </li>
                <li>
                  <Link href={hrefOf("people")}>
                    Invite your team <span aria-hidden="true">→</span>
                  </Link>
                </li>
                <li>
                  <Link href={hrefOf("skills")}>
                    Explore reusable skills <span aria-hidden="true">→</span>
                  </Link>
                </li>
              </ul>
            </section>
          </div>
        </>
      )}

      <details className="access-details">
        <summary>Your access</summary>
        <p className="muted">
          Signed in as{" "}
          <strong>{me.principal.display_name ?? me.principal.subject}</strong>{" "}
          in {me.tenant.name}.
          {me.capabilities.role_keys.length > 0
            ? ` Tenant roles: ${me.capabilities.role_keys.join(", ")}.`
            : " You have no tenant-wide role. Workspace and project access is listed below."}
        </p>
        <ul className="anchors">
          {me.anchors.map((anchor) => (
            <li key={anchor.scope_id}>
              <strong>{anchor.kind}</strong>{" "}
              <span className={`tag ${anchor.direct ? "direct" : "inherited"}`}>
                {anchor.source.replace(/_/g, " ")}
              </span>{" "}
              <span className="muted">
                {anchor.roles.length > 0 ? anchor.roles.join(", ") : "no role"}{" "}
                · {anchor.direct ? "granted here" : "inherited"}
              </span>
            </li>
          ))}
        </ul>
        {me.anchors_not_answered ? (
          <p className="muted">
            Access details for {me.anchors_not_answered} additional scope(s)
            were not included in this response.
          </p>
        ) : null}
      </details>
    </>
  );
}
