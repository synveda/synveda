/**
 * Home (CPR-8): where you are, what you have, and what to do next.
 *
 * It fetches nothing. Everything on it is already in `/v1/me` — the
 * workspaces, the projects, the onboarding state and the caller's anchors
 * with a real policy decision at each (ADR-0073 decision 8) — and a landing
 * page that re-asked for facts the shell is already holding would be four
 * more round trips before the first pixel, which is exactly what `/v1/me`
 * exists to remove.
 */

import { Link } from "./Router.js";
import { hrefOf } from "./routes.mjs";
import { PageHeading, useApp } from "./Shell.js";
import { projectsOf } from "./selection.mjs";

export function Home() {
  const { me, workspace, project, selection } = useApp();
  const projects = projectsOf(me, selection.workspaceId);

  return (
    <>
      <PageHeading route="home" />

      <section className="cards">
        <article className="card">
          <h2>Workspace</h2>
          {workspace ? (
            <>
              <p className="card-value">{workspace.display_name}</p>
              <p className="muted">
                {workspace.slug} · {projects.length} project{projects.length === 1 ? "" : "s"} ·{" "}
                {workspace.status}
              </p>
            </>
          ) : (
            <p className="muted">Nothing selected.</p>
          )}
        </article>

        <article className="card">
          <h2>Project</h2>
          {project ? (
            <>
              <p className="card-value">{project.display_name}</p>
              <p className="muted">
                {project.slug} · revision {project.revision} · {project.status}
              </p>
            </>
          ) : (
            <p className="muted">
              No project selected. <Link href={hrefOf("settings")}>Create one</Link>.
            </p>
          )}
        </article>

        <article className="card">
          <h2>You</h2>
          <p className="card-value">{me.principal.display_name ?? me.principal.subject}</p>
          <p className="muted">
            {me.capabilities.role_keys.length > 0
              ? `${me.capabilities.role_keys.join(", ")} at the tenant root`
              : "no role at the tenant root"}
          </p>
        </article>
      </section>

      <section>
        <h2>Where you stand</h2>
        {/* The anchor list, with the source the gateway gave each one. It is
            the honest answer to "why can I see this?" and it is a set of
            real decisions rather than a shape derived from a plan — which
            is the whole of ADR-0073 decision 8 and worth surfacing rather
            than hiding behind the switchers. */}
        <ul className="anchors">
          {me.anchors.map((anchor) => (
            <li key={anchor.scope_id}>
              <span className={`tag ${anchor.direct ? "direct" : "inherited"}`}>
                {anchor.source.replace(/_/g, " ")}
              </span>{" "}
              <strong>{anchor.kind}</strong>{" "}
              <span className="muted">
                {anchor.roles.length > 0 ? anchor.roles.join(", ") : "no role"} ·{" "}
                {anchor.direct ? "granted here" : "inherited"}
              </span>
            </li>
          ))}
        </ul>
        {me.anchors_not_answered ? (
          <p className="muted">
            {me.anchors_not_answered} further anchor(s) were not answered — the response bound
            dropped them rather than truncating silently.
          </p>
        ) : null}
      </section>

      <section>
        <h2>Next</h2>
        <ul className="next">
          <li>
            <Link href={hrefOf("learnings")}>Review New Learnings</Link> — decide what your
            sessions proposed before it can become active Knowledge.
          </li>
          <li>
            <Link href={hrefOf("welcome")}>Connect an agent client</Link> — the commands for
            Claude Code, Cursor, Claude Desktop or any MCP client.
          </li>
          <li>
            <Link href={hrefOf("people")}>Invite somebody</Link> — a one-time link they redeem
            with their own credential.
          </li>
          <li>
            <Link href={hrefOf("settings")}>Attach a repository</Link> — what this project is
            about, by canonical remote rather than by a path on your machine.
          </li>
        </ul>
      </section>
    </>
  );
}
