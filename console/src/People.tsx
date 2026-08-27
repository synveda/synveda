/**
 * People (CPR-8, ADR-0075 decision 5): who may act here, why, and what you
 * can change about it.
 *
 * Four listings and three controls, over the access plane CPR-5 built:
 * workspace members, the project's **project-only** members, the invitations
 * standing, and the invitations that have already been settled — with
 * invite, revoke and remove beside them.
 *
 * # Why the two member lists are different questions
 *
 * `GET /v1/workspaces/{id}/members` answers "who may act in this workspace".
 * `GET /v1/projects/{id}/members` answers "who may act in this project" —
 * **including** everybody the workspace above it grants, because a grant is
 * inherited by the scope's subtree with no row written below (ADR-0072). So
 * the project list is a superset, and the interesting part of it is the
 * complement: the people whose access **ends here**. That is what the page
 * calls project-only, and it is derived from each row's own `inherited`
 * flag rather than by diffing the two lists (`people.mts`).
 *
 * # Every row says where it came from
 *
 * That is the feature. A member list that shows names and roles makes
 * somebody open an audit log to find out why Robin can see this; one that
 * shows `source`, the scope the grant is actually written at, the group it
 * came through and whether a directory owns it does not.
 */

import { useCallback, useState } from "react";

import { idempotencyKey, request } from "./client.mjs";
import { Loaded, invalidate, useQuery, useRefresh } from "./Query.js";
import { hrefOf } from "./routes.mjs";
import { Link } from "./Router.js";
import { PageHeading, useApp } from "./Shell.js";
import {
  ROLE_KEYS,
  accessSource,
  directMembers,
  hasLapsed,
  inheritedMembers,
  mayRemove,
  memberKey,
  pendingInvites,
  settledInvites,
  viaGroup,
  whenOf,
  type RoleKey,
} from "./people.mjs";
import type { CreatedInviteView, InviteList, MemberList, MemberView } from "./generated/api.js";

export function People() {
  const { workspace, project } = useApp();

  if (!workspace) {
    return (
      <>
        <PageHeading route="people" />
        <p className="muted">
          Choose a workspace first — membership is a fact about a scope, so there is nothing to
          list until there is one.
        </p>
      </>
    );
  }

  const membersKey = `workspaces/${workspace.id}/members`;
  const invitesKey = `workspaces/${workspace.id}/invites`;
  const projectKey = project ? `projects/${project.id}/members` : null;

  return (
    <>
      <PageHeading route="people" />
      <WorkspaceMembers workspaceId={workspace.id} cacheKey={membersKey} />
      {project ? (
        <ProjectMembers projectId={project.id} cacheKey={projectKey as string} />
      ) : (
        <section>
          <h2>Project-only members</h2>
          <p className="muted">No project selected.</p>
        </section>
      )}
      <Invitations workspaceId={workspace.id} cacheKey={invitesKey} />
    </>
  );
}

function WorkspaceMembers({ workspaceId, cacheKey }: { workspaceId: string; cacheKey: string }) {
  const entry = useQuery(cacheKey, () =>
    request("list_workspace_members", { path: { workspace_id: workspaceId } }),
  );
  const retry = useRefresh(cacheKey);
  return (
    <section>
      <h2>Workspace members</h2>
      <Loaded<MemberList> entry={entry} what="the member list" onRetry={retry}>
        {(body) =>
          body.members.length === 0 ? (
            <p className="muted">Nobody holds a role here yet.</p>
          ) : (
            <MemberTable members={body.members} onRemove={null} />
          )
        }
      </Loaded>
    </section>
  );
}

function ProjectMembers({ projectId, cacheKey }: { projectId: string; cacheKey: string }) {
  const entry = useQuery(cacheKey, () =>
    request("list_project_members", { path: { project_id: projectId } }),
  );
  const retry = useRefresh(cacheKey);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const remove = useCallback(
    async (member: MemberView) => {
      setBusy(true);
      setError(null);
      const outcome = await request("remove_project_member", {
        path: { project_id: projectId, principal_id: member.principal_id },
      });
      setBusy(false);
      if (outcome.kind !== "ok") {
        // The gateway's own sentence. A refusal reworded here is a refusal
        // nothing keeps in step with the one the CLI shows.
        setError(
          outcome.kind === "unauthenticated" ? "Your session has expired." : outcome.message,
        );
        return;
      }
      invalidate(cacheKey);
    },
    [projectId, cacheKey],
  );

  return (
    <section>
      <h2>Project-only members</h2>
      <p className="muted">
        People whose access ends at this project. Everybody the workspace grants is above, and
        reaches this project without a row here.
      </p>
      {error ? (
        <div className="banner error" role="alert">
          {error}
        </div>
      ) : null}
      <Loaded<MemberList> entry={entry} what="the project's members" onRetry={retry}>
        {(body) => {
          const direct = directMembers(body.members);
          const above = inheritedMembers(body.members);
          return (
            <>
              {direct.length === 0 ? (
                <p className="muted">Nobody has project-only access here.</p>
              ) : (
                <MemberTable members={direct} onRemove={busy ? null : (m) => void remove(m)} />
              )}
              {/* The other half of the answer. Without it, an empty list
                  above reads as "nobody can act here", which is the
                  opposite of true when a workspace grant reaches every
                  project inside it with no row written below. */}
              <p className="muted">
                {above.length === 0
                  ? "Nobody reaches this project from a scope above it."
                  : `${above.length} more reach this project from a scope above it, listed under
                     Workspace members.`}
              </p>
            </>
          );
        }}
      </Loaded>
      <AddProjectMember projectId={projectId} cacheKey={cacheKey} />
    </section>
  );
}

function MemberTable({
  members,
  onRemove,
}: {
  members: MemberView[];
  onRemove: ((member: MemberView) => void) | null;
}) {
  return (
    <table className="members">
      <thead>
        <tr>
          <th>Principal</th>
          <th>Role</th>
          <th>Access source</th>
          <th>Managed</th>
          {onRemove ? <th /> : null}
        </tr>
      </thead>
      <tbody>
        {members.map((member) => {
          const group = viaGroup(member);
          return (
            <tr key={memberKey(member)}>
              <td className="mono">{member.principal_id}</td>
              <td>
                <span className="tag role">{member.role}</span>
              </td>
              <td>
                {accessSource(member)}
                <div className="muted">
                  at scope {member.scope_id.slice(0, 8)} · since {whenOf(member.granted_at)}
                  {group ? ` · group ${group.slug}` : ""}
                </div>
              </td>
              <td>
                {/* The word a reader needs before they try to change it:
                    a directory-managed row goes back the way it was on the
                    next sync, so this page does not offer to touch it. */}
                {member.directory_managed ? "directory" : "direct"}
              </td>
              {onRemove ? (
                <td>
                  {mayRemove(member) ? (
                    <button type="button" onClick={() => onRemove(member)}>
                      Remove
                    </button>
                  ) : (
                    // Absent rather than disabled (ADR-0056): a disabled
                    // button promises that trying harder would work.
                    <span className="muted">
                      {member.directory_managed ? "your directory owns this" : "granted above"}
                    </span>
                  )}
                </td>
              ) : null}
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

function AddProjectMember({ projectId, cacheKey }: { projectId: string; cacheKey: string }) {
  const [principal, setPrincipal] = useState("");
  const [role, setRole] = useState<RoleKey>("member");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = useCallback(async () => {
    const subject = principal.trim();
    if (subject.length === 0) return;
    setBusy(true);
    setError(null);
    const outcome = await request("add_project_member", {
      path: { project_id: projectId },
      // Minted once per submission, so a retry of *this* attempt replays
      // rather than granting twice.
      idempotencyKey: idempotencyKey(),
      body: { principal_id: subject, role },
    });
    setBusy(false);
    if (outcome.kind !== "ok") {
      setError(outcome.kind === "unauthenticated" ? "Your session has expired." : outcome.message);
      return;
    }
    setPrincipal("");
    invalidate(cacheKey);
  }, [principal, role, projectId, cacheKey]);

  return (
    <form
      className="inline-form"
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      <label>
        <span className="switcher-label">Principal (token subject)</span>
        <input
          value={principal}
          onChange={(event) => setPrincipal(event.target.value)}
          placeholder="robin@example.test"
        />
      </label>
      <label>
        <span className="switcher-label">Role</span>
        <select value={role} onChange={(event) => setRole(event.target.value as RoleKey)}>
          {ROLE_KEYS.map((key) => (
            <option key={key} value={key}>
              {key}
            </option>
          ))}
        </select>
      </label>
      <button type="submit" disabled={busy || principal.trim().length === 0}>
        Grant at this project
      </button>
      {error ? <span className="form-error">{error}</span> : null}
    </form>
  );
}

function Invitations({ workspaceId, cacheKey }: { workspaceId: string; cacheKey: string }) {
  const entry = useQuery(cacheKey, () =>
    request("list_workspace_invites", { path: { workspace_id: workspaceId } }),
  );
  const retry = useRefresh(cacheKey);
  const [minted, setMinted] = useState<CreatedInviteView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [email, setEmail] = useState("");
  const [role, setRole] = useState<RoleKey>("member");

  const issue = useCallback(async () => {
    setBusy(true);
    setError(null);
    const address = email.trim();
    const outcome = await request("create_workspace_invite", {
      path: { workspace_id: workspaceId },
      idempotencyKey: idempotencyKey(),
      body: { role, ...(address.length > 0 ? { email: address } : {}) },
    });
    setBusy(false);
    if (outcome.kind !== "ok") {
      setError(outcome.kind === "unauthenticated" ? "Your session has expired." : outcome.message);
      return;
    }
    // The token appears exactly once, in this response, and on no other
    // route ever (ADR-0072). It is held in component state so the inviter
    // can copy it, and it is gone the moment they navigate — which is the
    // whole of the product's story about it, so the screen says so.
    setMinted(outcome.body);
    setEmail("");
    invalidate(cacheKey);
  }, [workspaceId, role, email, cacheKey]);

  const revoke = useCallback(
    async (inviteId: string) => {
      setBusy(true);
      setError(null);
      const outcome = await request("revoke_workspace_invite", {
        path: { workspace_id: workspaceId, invite_id: inviteId },
      });
      setBusy(false);
      if (outcome.kind !== "ok") {
        setError(
          outcome.kind === "unauthenticated" ? "Your session has expired." : outcome.message,
        );
        return;
      }
      invalidate(cacheKey);
    },
    [workspaceId, cacheKey],
  );

  const now = Date.now();
  return (
    <section>
      <h2>Invitations</h2>
      {error ? (
        <div className="banner error" role="alert">
          {error}
        </div>
      ) : null}

      <form
        className="inline-form"
        onSubmit={(event) => {
          event.preventDefault();
          void issue();
        }}
      >
        <label>
          <span className="switcher-label">Email (optional)</span>
          <input
            value={email}
            onChange={(event) => setEmail(event.target.value)}
            placeholder="nobody is emailed — this is a label"
          />
        </label>
        <label>
          <span className="switcher-label">Role</span>
          <select value={role} onChange={(event) => setRole(event.target.value as RoleKey)}>
            {ROLE_KEYS.map((key) => (
              <option key={key} value={key}>
                {key}
              </option>
            ))}
          </select>
        </label>
        <button type="submit" disabled={busy}>
          Create invitation
        </button>
      </form>

      {minted ? (
        <div className="banner" role="status">
          <p>
            <strong>Copy this link now.</strong> It is shown once and exists nowhere else — not
            in the listing below, not in the audit log, not on any other route. If it is lost,
            withdraw the invitation and issue another.
          </p>
          <p className="mono breakable">{minted.accept_url}</p>
          <p className="muted">
            The recipient redeems it with their own credential. Nothing is emailed by this
            product.
          </p>
        </div>
      ) : null}

      <Loaded<InviteList> entry={entry} what="the invitations" onRetry={retry}>
        {(body) => {
          const pending = pendingInvites(body.invites);
          const settled = settledInvites(body.invites);
          return (
            <>
              <h3>Pending</h3>
              {pending.length === 0 ? (
                <p className="muted">None standing.</p>
              ) : (
                <ul className="invites">
                  {pending.map((invite) => (
                    <li key={invite.id}>
                      <span className="tag role">{invite.role}</span>{" "}
                      {invite.email ?? <span className="muted">no address — a copyable link</span>}
                      <div className="muted">
                        expires {whenOf(invite.expires_at)}
                        {hasLapsed(invite, now)
                          ? " — the clock says this has run out; the server will settle it"
                          : ""}
                      </div>
                      <button type="button" disabled={busy} onClick={() => void revoke(invite.id)}>
                        Withdraw
                      </button>
                    </li>
                  ))}
                </ul>
              )}
              <h3>Settled</h3>
              {settled.length === 0 ? (
                <p className="muted">Nothing yet.</p>
              ) : (
                <ul className="invites">
                  {settled.map((invite) => (
                    <li key={invite.id}>
                      <span className={`tag ${invite.status}`}>{invite.status}</span>{" "}
                      {invite.email ?? <span className="muted">a link</span>}
                      <div className="muted">
                        {invite.accepted_at
                          ? `redeemed ${whenOf(invite.accepted_at)}`
                          : `expired or withdrawn · issued ${whenOf(invite.created_at)}`}
                      </div>
                    </li>
                  ))}
                </ul>
              )}
            </>
          );
        }}
      </Loaded>

      <p className="muted">
        Groups and tenant-wide grants live under <Link href={hrefOf("scopes")}>Advanced ▸ Scopes</Link>
        , where a grant can be written at any scope you hold authority over.
      </p>
    </section>
  );
}
