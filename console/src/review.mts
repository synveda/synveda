/**
 * What a `ProposalDetail` is, and the few pure things the review screen
 * needs from one (CNSL-1, ADR-0056).
 *
 * The types here describe **the payload the gateway already serves** —
 * CNSL-1 adds no governed API (decision 9), so this file is a reading of
 * something that existed before the console did, not a contract the console
 * negotiated.
 *
 * Two rules govern what is allowed in this file.
 *
 * The old Skill-only scan/checklist payload was removed with the mutable
 * Skill publication model in CPR-23/24. Scan, rubric and harness evidence is
 * version metadata in the Skills Library; this type deliberately contains
 * only the common VedaFlow review model.
 */

import type {
  ApprovalRequirementView,
  ProposalApprovalView,
  ProposalDetail as GeneratedProposalDetail,
  ProposalMemberView,
  ProposalSummary,
} from "./generated/api.js";

export type Proposal = ProposalSummary;
export type Requirement = ApprovalRequirementView;
export type ProposalDetail = GeneratedProposalDetail;
export type Member = ProposalMemberView;
export type Approval = ProposalApprovalView;

// ── The few pure readings the screen needs ──────────────────────────────

/**
 * How a member is named on screen.
 *
 * An address a reader cannot type is abbreviated; a path somebody chose is
 * not, because a name a person typed is the whole point of the name. The
 * CLI makes the same call for the same reason, and it is a coincidence of
 * good sense rather than shared code — it is layout, and each surface owns
 * its own.
 */
export function label(member: string): string {
  const uuidShaped = /^[0-9a-fA-F-]{36}$/.test(member);
  return uuidShaped ? member.slice(0, 12) : member;
}

/** What publishing would do to this member, as a word. */
export function effectLabel(effect: Member["effect"]): string {
  switch (effect) {
    case "add":
      return "add";
    case "update":
      return "update";
    case "apply":
      return "apply";
    case "none":
      return "same";
  }
}

/**
 * Whether this member has a diff to show.
 *
 * A member the publication would not touch has none — the channel already
 * names it at exactly this address — and showing one anyway would invite a
 * reviewer to look for a change that is not there.
 */
export function showsDiff(member: Member): boolean {
  return member.effect !== "none";
}

/**
 * An instant, as a reviewer reads it.
 *
 * UTC and explicit, rather than the viewer's own zone. A review surface is
 * read beside an audit trail and quoted into incident notes, and two people
 * in two offices comparing "09:01" is exactly the confusion the zone suffix
 * costs one word to prevent. The microseconds the wire carries are dropped:
 * they are real, and no reviewer has ever needed them.
 *
 * An unparseable value is shown as it arrived. A surface that rendered
 * `Invalid Date` over a timestamp it did not understand would be hiding the
 * only evidence of what went wrong.
 */
export function instant(raw: string): string {
  const parsed = new Date(raw);
  if (Number.isNaN(parsed.getTime())) {
    return raw;
  }
  return `${parsed.toISOString().slice(0, 19).replace("T", " ")} UTC`;
}

/** The requirement in one line: what is asked for, and where it came from. */
export function describeRequirement(required: Requirement): string {
  const parts = required.roles.map(({ count, role }) => `${count} × ${role}`);
  if (required.distinct_approvers > 1) {
    parts.push(`${required.distinct_approvers} distinct approvers`);
  }
  for (const subject of required.subjects ?? []) {
    parts.push(`@${subject}`);
  }
  if (required.forbid_author_approval) {
    parts.push("reviewer distinct from author");
  }
  if (required.separate_effect_actor) {
    parts.push("effect actor distinct from author and reviewers");
  }
  if (parts.length === 0) {
    parts.push("nothing");
  }
  return `${parts.join(" + ")}  (from: ${required.origins.join(", ")})`;
}
