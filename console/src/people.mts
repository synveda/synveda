/**
 * The People page's judgements (CPR-8, ADR-0075 decision 5).
 *
 * The page answers one question — *who may act here, and why* — and the
 * "why" is the part the API went to trouble to make answerable. `MemberView`
 * carries `source`, `scope_id`, `inherited`, `via_group` and
 * `directory_managed` precisely so a reader can see that Robin is here
 * because somebody granted them `member` at the workspace, or because they
 * are in the `engineering` group, or because a directory said so — without
 * opening an audit log (CPR-5, ADR-0072).
 *
 * Rendering that is a judgement, so it lives here rather than in the
 * component: what a row says about its own origin is a claim about
 * authority, and a claim about authority belongs somewhere a test can read
 * it.
 *
 * # The one rule that is not cosmetic
 *
 * {@link mayRemove} decides whether the page offers a remove control. An
 * inherited grant cannot be removed at the scope it is *seen* at — the row
 * is written above — and a directory-managed one cannot be removed at all,
 * because the directory would put it straight back. Offering the control
 * anyway would be offering an act the API refuses, which ADR-0056's
 * "absent rather than disabled" already rejected for verdicts and rejects
 * again here: a disabled button is a promise that trying harder would work.
 *
 * It is still not enforcement. The gateway decides, at the act's own seam.
 */

import type { GroupRefView, InviteView, MemberView } from "./generated/api.js";

/** The six grant keys, in the order a role picker should offer them. */
export const ROLE_KEYS = [
  "owner",
  "administrator",
  "curator",
  "reviewer",
  "member",
  "viewer",
] as const;

export type RoleKey = (typeof ROLE_KEYS)[number];

/**
 * The group a membership came through, when it came through one.
 *
 * `via_group` is generated as `unknown | null | GroupRefView` — utoipa
 * renders an optional referenced schema that way — so the narrowing happens
 * once, here, rather than at every call site with a cast.
 */
export function viaGroup(member: MemberView): GroupRefView | null {
  const candidate = member.via_group;
  if (typeof candidate !== "object" || candidate === null) return null;
  const record = candidate as Record<string, unknown>;
  return typeof record.id === "string" && typeof record.slug === "string"
    ? { id: record.id, slug: record.slug }
    : null;
}

/**
 * The members whose grant is written **at this scope** rather than above it.
 *
 * What the page calls "project-only": people the project's own member list
 * names and the workspace's does not, which is exactly the set whose access
 * ends at this project. Derived from `inherited` rather than by diffing the
 * two lists, because the API already answers it per row and a diff would
 * silently disagree with it the first time somebody holds two roles.
 */
export function directMembers(members: MemberView[]): MemberView[] {
  return members.filter((member) => !member.inherited);
}

/** The members whose grant reaches here from an ancestor scope. */
export function inheritedMembers(members: MemberView[]): MemberView[] {
  return members.filter((member) => member.inherited);
}

/**
 * Where a member's access came from, in words.
 *
 * The sentence names the *mechanism* first, because that is what a reader
 * has to change to change the access: a directory-managed row is changed in
 * the directory, a group row by editing the group, a direct row by revoking
 * it here.
 */
export function accessSource(member: MemberView): string {
  const clauses = [member.inherited ? "inherited from a scope above" : "granted here"];
  // The group clause survives beside the directory one rather than being
  // replaced by it, and that is deliberate. "Managed by your directory"
  // tells a reader they cannot change this here; **which group** tells them
  // what to change instead. Dropping the second because the first is true
  // is dropping the actionable half.
  const group = viaGroup(member);
  if (group) {
    clauses.push(`through the ${group.slug} group`);
  }
  if (member.directory_managed) {
    clauses.push("managed by your directory");
  } else {
    switch (member.source) {
      case "owner":
        clauses.push("as its creator");
        break;
      case "invite":
        clauses.push("by redeeming an invitation");
        break;
      case "automation":
        clauses.push("by automation");
        break;
      case "directory":
        clauses.push("from your directory");
        break;
      case "direct":
      default:
        // A group grant is already explained by its group clause; saying
        // "granted directly" beside it would contradict it.
        if (!group) clauses.push("granted directly");
        break;
    }
  }
  return clauses.join(", ");
}

/** Whether the page should offer to remove this membership here. */
export function mayRemove(member: MemberView): boolean {
  return !member.inherited && !member.directory_managed;
}

/**
 * A stable key for a member row.
 *
 * One entry per (principal, role) is what the API serves — somebody holding
 * two roles appears twice, because the two came from different grants and
 * are revoked separately — so the grant id is the identity of a row, not
 * the principal.
 */
export function memberKey(member: MemberView): string {
  return member.grant_id;
}

/**
 * The invitations still worth acting on, newest first.
 *
 * `pending` only: an accepted invitation is a grant now and appears in the
 * member list, and a revoked or expired one is history. The listing serves
 * all four states on purpose ("who was invited here and what happened"), so
 * the page shows the rest under their own heading rather than dropping them.
 */
export function pendingInvites(invites: InviteView[]): InviteView[] {
  return invites.filter((invite) => invite.status === "pending");
}

/** The invitations that are no longer redeemable. */
export function settledInvites(invites: InviteView[]): InviteView[] {
  return invites.filter((invite) => invite.status !== "pending");
}

/**
 * Whether an invitation has run out, given the time now.
 *
 * The server owns `status`, and a `pending` invitation whose `expires_at`
 * has passed is one the server has not looked at yet rather than one that
 * still works. Said in the row rather than corrected in it: the console does
 * not get to overrule a status, only to point out that the clock disagrees.
 */
export function hasLapsed(invite: InviteView, now: number): boolean {
  const expires = Date.parse(invite.expires_at);
  return Number.isFinite(expires) && expires <= now;
}

/** `2026-08-21 14:03 UTC`, the format every other surface here uses. */
export function whenOf(timestamp: string): string {
  const parsed = new Date(timestamp);
  if (Number.isNaN(parsed.getTime())) return timestamp;
  return `${parsed.toISOString().slice(0, 16).replace("T", " ")} UTC`;
}
