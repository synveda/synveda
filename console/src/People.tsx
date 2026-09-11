/**
 * People (CPR-8, ADR-0075 decision 5): who may act here, why, and what you
 * can change about it.
 *
 * The listings and controls stay on the access plane CPR-5 built: effective
 * workspace and project members, exact grant rows, groups, and pending or
 * settled invitations, with only the supported grant and invitation changes
 * beside them.
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
import { PageHeading, useApp } from "./Shell.js";
import {
  ROLE_KEYS,
  accessLabels,
  accessSource,
  directMembers,
  hasLapsed,
  inheritedMembers,
  mayManageAccessAt,
  mayRemove,
  memberKey,
  pendingInvites,
  settledInvites,
  viaGroup,
  whenOf,
  type AccessLabels,
  type RoleKey,
} from "./people.mjs";
import type {
  CreatedInviteView,
  GrantList,
  GrantView,
  GroupList,
  GroupView,
  InviteList,
  ListResponse,
  MeView,
  MemberList,
  MemberView,
  ProjectView,
  WorkspaceView,
} from "./generated/api.js";

export function People() {
  const { me, workspace, project } = useApp();

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

  return (
    <>
      <PageHeading route="people" />
      <PeopleAtSelection me={me} workspace={workspace} project={project} />
    </>
  );
}

function PeopleAtSelection({
  me,
  workspace,
  project,
}: {
  me: MeView;
  workspace: WorkspaceView;
  project: ProjectView | null;
}) {
  const scopes = useQuery("access/reference/scopes", () => request("list_scopes", {}));
  const groups = useQuery("access/reference/groups", () => request("list_groups", {}));
  const scopeLevel = readyBody<ListResponse>(scopes);
  const groupPage = readyBody<GroupList>(groups);
  const labels = accessLabels(me, scopeLevel, groupPage?.groups ?? null);
  const workspaceCanChange = mayManageAccessAt(me, workspace.scope_id);
  const projectCanChange = project ? mayManageAccessAt(me, project.scope_id) : false;
  const membersKey = `workspaces/${workspace.id}/members`;
  const invitesKey = `workspaces/${workspace.id}/invites`;

  return (
    <>
      <p>
        Display names are labels already disclosed by Synveda's governed scope and group APIs.
        The exact token subject, role and grant scope remain visible because those—not an
        identity-provider group—are the application authority.
      </p>
      <EnrichmentStatus scopes={scopes} groups={groups} />
      <WorkspaceMembers workspaceId={workspace.id} cacheKey={membersKey} labels={labels} />
      {project ? (
        <ProjectMembers
          projectId={project.id}
          cacheKey={`projects/${project.id}/members`}
          labels={labels}
          canChange={projectCanChange}
        />
      ) : (
        <section>
          <h2>Project-only members</h2>
          <p className="muted">No project selected.</p>
        </section>
      )}
      <ExactGrants
        workspace={workspace}
        project={project}
        labels={labels}
        workspaceCanChange={workspaceCanChange}
        projectCanChange={projectCanChange}
      />
      <Groups entry={groups} labels={labels} />
      <Invitations
        workspaceId={workspace.id}
        scopeId={workspace.scope_id}
        cacheKey={invitesKey}
        canChange={workspaceCanChange}
        labels={labels}
      />
      <section>
        <h2>Identity-provider boundary</h2>
        <p className="muted">
          Keycloak owns sign-in profiles and local password administration. Synveda owns the
          grants above. This browser session receives no Keycloak administrator credential and
          does not call privileged identity-provider administration APIs.
        </p>
      </section>
    </>
  );
}

function readyBody<T>(entry: ReturnType<typeof useQuery>): T | null {
  return entry.status === "ready" && entry.outcome.kind === "ok"
    ? (entry.outcome.body as T)
    : null;
}

function EnrichmentStatus({
  scopes,
  groups,
}: {
  scopes: ReturnType<typeof useQuery>;
  groups: ReturnType<typeof useQuery>;
}) {
  const unavailable = [scopes, groups].some(
    (entry) => entry.status === "ready" && entry.outcome.kind !== "ok",
  );
  return unavailable ? (
    <div className="banner warning" role="status">
      Some display-name enrichment is unavailable under the current policy. Exact principal,
      group and scope identifiers remain shown; no name has been guessed.
    </div>
  ) : null;
}

function WorkspaceMembers({
  workspaceId,
  cacheKey,
  labels,
}: {
  workspaceId: string;
  cacheKey: string;
  labels: AccessLabels;
}) {
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
            <MemberTable members={body.members} labels={labels} onRemove={null} />
          )
        }
      </Loaded>
    </section>
  );
}

function ProjectMembers({
  projectId,
  cacheKey,
  labels,
  canChange,
}: {
  projectId: string;
  cacheKey: string;
  labels: AccessLabels;
  canChange: boolean;
}) {
  const entry = useQuery(cacheKey, () =>
    request("list_project_members", { path: { project_id: projectId } }),
  );
  const retry = useRefresh(cacheKey);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const remove = useCallback(
    async (member: MemberView) => {
      setBusy(true);
      setError(null);
      setResult(null);
      const outcome = await request("revoke_grant", {
        path: { grant_id: member.grant_id },
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
      setResult(`Revoked the exact ${member.role} grant for ${member.principal_id}.`);
      invalidate(cacheKey, "access/grants/");
    },
    [cacheKey],
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
      {result ? <div className="banner success" role="status">{result}</div> : null}
      <Loaded<MemberList> entry={entry} what="the project's members" onRetry={retry}>
        {(body) => {
          const direct = directMembers(body.members);
          const above = inheritedMembers(body.members);
          return (
            <>
              {direct.length === 0 ? (
                <p className="muted">Nobody has project-only access here.</p>
              ) : (
                <MemberTable
                  members={direct}
                  labels={labels}
                  onRemove={!busy ? (m) => void remove(m) : null}
                />
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
      {!canChange ? (
        <p className="muted">
          Your current capability forecast does not offer project access changes. This
          administration route remains visible; the gateway will decide the submitted request and
          return the actionable refusal.
        </p>
      ) : null}
      <AddProjectMember projectId={projectId} cacheKey={cacheKey} />
    </section>
  );
}

function MemberTable({
  members,
  labels,
  onRemove,
}: {
  members: MemberView[];
  labels: AccessLabels;
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
          const personName = labels.principals.get(member.principal_id);
          const groupName = group ? labels.groups.get(group.id) : undefined;
          const scopeName = labels.scopes.get(member.scope_id);
          return (
            <tr key={memberKey(member)}>
              <td>
                <strong>{personName ?? "Unresolved principal"}</strong>
                <div className="mono breakable">{member.principal_id}</div>
              </td>
              <td>
                <span className="tag role">{member.role}</span>
              </td>
              <td>
                {accessSource(member, groupName)}
                <div className="muted">
                  exact scope {scopeName ?? "unresolved"} · since {whenOf(member.granted_at)}
                </div>
                <div className="mono breakable">{member.scope_id}</div>
                {group ? (
                  <div className="muted">
                    group handle {group.slug} · <span className="mono">{group.id}</span>
                  </div>
                ) : null}
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
                      {member.directory_managed
                        ? "your directory owns this"
                        : member.inherited
                          ? "granted above"
                          : "change the group grant below"}
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

function ExactGrants({
  workspace,
  project,
  labels,
  workspaceCanChange,
  projectCanChange,
}: {
  workspace: WorkspaceView;
  project: ProjectView | null;
  labels: AccessLabels;
  workspaceCanChange: boolean;
  projectCanChange: boolean;
}) {
  return (
    <section>
      <h2>Exact access grants</h2>
      <p className="muted">
        These are stored grant rows, not effective-access totals. A workspace grant appears once
        here and is inherited by its projects. Revocation names one exact grant and retains the
        gateway's policy, directory and last-owner safeguards.
      </p>
      <ExactScopeGrants
        scopeId={workspace.scope_id}
        scopeLabel={`Workspace · ${workspace.display_name}`}
        labels={labels}
        canChange={workspaceCanChange}
      />
      {project ? (
        <ExactScopeGrants
          scopeId={project.scope_id}
          scopeLabel={`Project · ${project.display_name}`}
          labels={labels}
          canChange={projectCanChange}
        />
      ) : null}
    </section>
  );
}

function ExactScopeGrants({
  scopeId,
  scopeLabel,
  labels,
  canChange,
}: {
  scopeId: string;
  scopeLabel: string;
  labels: AccessLabels;
  canChange: boolean;
}) {
  const key = `access/grants/${scopeId}`;
  const entry = useQuery(key, () => request("list_grants", { query: { scope_id: scopeId } }));
  const retry = useRefresh(key);
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<{ error: boolean; text: string } | null>(null);

  const revoke = async (grant: GrantView) => {
    const subject = grant.principal_id ?? grant.group_id ?? "this subject";
    if (!window.confirm(`Revoke only the ${grant.role} grant for ${subject} at ${scopeLabel}?`)) {
      return;
    }
    setBusy(grant.id);
    setMessage(null);
    const outcome = await request("revoke_grant", { path: { grant_id: grant.id } });
    setBusy(null);
    if (outcome.kind !== "ok") {
      setMessage({
        error: true,
        text: outcome.kind === "unauthenticated" ? "Your session has expired." : outcome.message,
      });
      return;
    }
    setMessage({ error: false, text: `Revoked the exact ${grant.role} grant.` });
    invalidate("access/grants/", "workspaces/", "projects/");
  };

  return (
    <div className="node-detail">
      <h3>{scopeLabel}</h3>
      <div className="mono breakable">{scopeId}</div>
      {message ? (
        <div className={`banner ${message.error ? "error" : "success"}`} role={message.error ? "alert" : "status"}>
          {message.text}
        </div>
      ) : null}
      <Loaded<GrantList> entry={entry} what={`grants at ${scopeLabel}`} onRetry={retry}>
        {(body) =>
          body.grants.length === 0 ? (
            <p className="muted">No stored grant is written at this exact scope.</p>
          ) : (
            <table className="members">
              <thead>
                <tr>
                  <th>Subject</th>
                  <th>Role</th>
                  <th>Source and state</th>
                  <th>Exact grant</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {body.grants.map((grant) => {
                  const subjectId = grant.principal_id ?? grant.group_id ?? "unavailable";
                  const subjectName = grant.principal_id
                    ? labels.principals.get(grant.principal_id)
                    : grant.group_id
                      ? labels.groups.get(grant.group_id)
                      : undefined;
                  return (
                    <tr key={grant.id}>
                      <td>
                        <strong>
                          {subjectName ??
                            (grant.subject_kind === "group" ? "Unresolved group" : "Unresolved principal")}
                        </strong>
                        <div className="muted">{grant.subject_kind}</div>
                        <div className="mono breakable">{subjectId}</div>
                      </td>
                      <td><span className="tag role">{grant.role}</span></td>
                      <td>
                        {grant.source}
                        <div className="muted">
                          {grant.directory_managed
                            ? `directory owned${grant.directory_source ? ` · ${grant.directory_source}` : ""}`
                            : "managed in Synveda"}
                        </div>
                      </td>
                      <td>
                        <div className="mono breakable">{grant.id}</div>
                        <div className="muted">created {whenOf(grant.created_at)}</div>
                      </td>
                      <td>
                        {grant.directory_managed ? (
                          <span className="muted">change in directory</span>
                        ) : (
                          <button
                            type="button"
                            disabled={busy !== null}
                            onClick={() => void revoke(grant)}
                          >
                            {busy === grant.id ? "Revoking…" : "Revoke exact grant"}
                          </button>
                        )}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )
        }
      </Loaded>
      {!canChange ? (
        <p className="muted">
          This session does not currently forecast <code>membership.grant</code> at the exact
          scope. The gateway remains authoritative and will refuse an unauthorised attempt.
        </p>
      ) : null}
    </div>
  );
}

function Groups({ entry, labels }: { entry: ReturnType<typeof useQuery>; labels: AccessLabels }) {
  const retry = useRefresh("access/reference/groups");
  return (
    <section>
      <h2>Groups</h2>
      <p className="muted">
        Group names and membership state come from Synveda's access API. Directory-owned groups
        are changed at their directory source; this page does not reproduce that administration
        plane.
      </p>
      <Loaded<GroupList>
        entry={entry}
        what="access groups"
        onRetry={retry}
      >
        {(body) =>
          body.groups.length === 0 ? (
            <p className="muted">No access group is visible.</p>
          ) : (
            <ul className="packs">
              {body.groups.map((group) => (
                <GroupRow key={group.id} group={group} labels={labels} />
              ))}
            </ul>
          )
        }
      </Loaded>
    </section>
  );
}

function GroupRow({ group, labels }: { group: GroupView; labels: AccessLabels }) {
  return (
    <li>
      <strong>{group.display_name}</strong> <span className={`tag ${group.status}`}>{group.status}</span>
      <div className="muted">
        handle {group.slug} · {group.source === "directory" ? "directory owned" : "managed in Synveda"} · revision {group.revision}
      </div>
      <div className="mono breakable">{group.id}</div>
      <details className="technical-details">
        <summary>{group.members.length} member{group.members.length === 1 ? "" : "s"}</summary>
        {group.members.length === 0 ? (
          <p className="muted">No members.</p>
        ) : (
          <ul>
            {group.members.map((member) => (
              <li key={member.identity_id}>
                <strong>
                  {member.principal_id
                    ? labels.principals.get(member.principal_id) ?? "Unresolved principal"
                    : "Directory identity not yet bound to a token subject"}
                </strong>
                {member.principal_id ? <div className="mono breakable">{member.principal_id}</div> : null}
                <div className="muted mono breakable">identity {member.identity_id}</div>
              </li>
            ))}
          </ul>
        )}
      </details>
    </li>
  );
}

function AddProjectMember({ projectId, cacheKey }: { projectId: string; cacheKey: string }) {
  const [principal, setPrincipal] = useState("");
  const [role, setRole] = useState<RoleKey>("member");
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = useCallback(async () => {
    const subject = principal.trim();
    if (subject.length === 0) return;
    setBusy(true);
    setError(null);
    setResult(null);
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
    setResult(`Granted the ${role} role at this exact project scope.`);
    setPrincipal("");
    invalidate(cacheKey, "access/grants/");
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
        {busy ? "Granting…" : "Grant at this project"}
      </button>
      {error ? <span className="form-error">{error}</span> : null}
      {result ? <span className="tag done" role="status">{result}</span> : null}
    </form>
  );
}

function Invitations({
  workspaceId,
  scopeId,
  cacheKey,
  canChange,
  labels,
}: {
  workspaceId: string;
  scopeId: string;
  cacheKey: string;
  canChange: boolean;
  labels: AccessLabels;
}) {
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
    if (minted) return;
    setBusy(true);
    setError(null);
    setMinted(null);
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
  }, [workspaceId, role, email, cacheKey, minted]);

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
      <p className="muted">
        Invitations grant one existing role at {labels.scopes.get(scopeId) ?? "this workspace"}.
        Synveda creates a copyable link; it does not send email or create an identity-provider
        account.
      </p>
      <div className="mono breakable">{scopeId}</div>
      {error ? (
        <div className="banner error" role="alert">
          {error}
        </div>
      ) : null}

      {!canChange ? (
        <p className="muted">
          This session does not currently forecast invitation changes. The gateway remains the
          authority and will return a refusal if this request is not allowed.
        </p>
      ) : null}
      <form
        className="inline-form"
        onSubmit={(event) => {
          event.preventDefault();
          void issue();
        }}
      >
          <label>
            <span className="switcher-label">Email label (optional)</span>
            <input
              type="email"
              value={email}
              onChange={(event) => setEmail(event.target.value)}
              placeholder="nobody is emailed — this is a label"
            />
          </label>
          <label>
            <span className="switcher-label">Exact role to grant</span>
            <select value={role} onChange={(event) => setRole(event.target.value as RoleKey)}>
              {ROLE_KEYS.map((key) => (
                <option key={key} value={key}>
                  {key}
                </option>
              ))}
            </select>
          </label>
          <button type="submit" disabled={busy || minted !== null}>
            {busy ? "Creating…" : "Create pending invitation"}
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
          <button type="button" onClick={() => setMinted(null)}>
            Clear this one-time link
          </button>
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
                      <span className="tag pending">pending</span>{" "}
                      <span className="tag role">{invite.role}</span>{" "}
                      {invite.email ?? <span className="muted">no address — a copyable link</span>}
                      <div className="muted">
                        issued {whenOf(invite.created_at)} · {" "}
                        expires {whenOf(invite.expires_at)}
                        {hasLapsed(invite, now)
                          ? " — the clock says this has run out; the server will settle it"
                          : ""}
                      </div>
                      <button type="button" disabled={busy} onClick={() => void revoke(invite.id)}>
                        Withdraw pending invitation
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

    </section>
  );
}
