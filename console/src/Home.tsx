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

export function Home() {
  const { me, workspace, project, selection } = useApp();
  const projects = projectsOf(me, selection.workspaceId);
  const blocked = me.onboarding.state === "blocked";

  return (
    <>
      <PageHeading route="home" />
      {blocked ? (
        <div className="banner" role="status">
          <h2>You need access to a workspace</h2>
          <p>
            Ask your administrator to invite you. Your workspaces and projects
            will appear here once you have access.
          </p>
        </div>
      ) : (
        <>
          <section className="project-overview" aria-label="Current selection">
            <div>
              <p className="eyebrow">Workspace</p>
              <h2>{workspace?.display_name ?? "No workspace selected"}</h2>
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
