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
 * **No verdicts.** `blocking` is read, never computed; a shortfall's
 * sentence is read, never composed. Decisions 5 and 6 moved those to the
 * gateway so that two renderers could not disagree about them, and a
 * `severityRank` here would put one straight back — including the case that
 * makes it matter, a severity from a gateway newer than this bundle, where
 * a client can only guess and the guess is wrong half the time.
 *
 * **No optional fields invented for convenience.** Where the gateway omits
 * something it means something: `scan` is absent for an artifact family
 * that has no executable content, and `checklist` is absent both when
 * nobody answered one and when somebody did and the
 * bundle has changed since — which are the same fact from a publication's
 * point of view (ADR-0053 decision 4).
 */

export interface Proposal {
  id: string;
  title: string;
  state: string;
  asset: string;
  effect: string;
  sensitivity: string;
  commit: string;
  proposer_subject: string;
  created_at: string;
  target_scope_id: string;
  source_scope_id: string;
  target_scope_path?: string;
  source_scope_path?: string;
  /** What the matrix asks for here, resolved now. */
  required: Requirement;
  /** What it still lacks, in one line the gateway wrote. */
  outstanding: string;
  close_reason?: string;
  promotion?: Promotion;
}

export interface Requirement {
  roles: { role: string; count: number }[];
  distinct_approvers: number;
  subjects?: string[];
  origins: string[];
}

/** Why a rule opened this, when one did (FLOW-4, ADR-0033 decision 12). */
export interface Promotion {
  rule: string;
  from_seq: number;
  to_seq: number;
}

export interface ProposalDetail extends Proposal {
  members: Member[];
  approvals: Approval[];
  scan?: ScanReport;
  quality?: QualityReport;
}

export interface Member {
  /** Stable member id or authored path. */
  member: string;
  asset: string;
  object_hash: string;
  /** `false` means the content moved after the proposal opened. */
  unchanged: boolean;
  sensitivity: string;
  effect: "add" | "update" | "none";
  /** The canonical bytes at the proposed address — what the approvals bind. */
  proposed: string;
  /** The member's text as it stands now. */
  content: string;
  baseline?: { object_hash: string; text: string };
}

export interface Approval {
  approver_subject: string;
  verdict: string;
  roles: string[];
  /** `false` once the proposal's commit has moved past this act. */
  counts: boolean;
  comment?: string;
  created_at: string;
}

export interface ScanReport {
  ruleset_version: number;
  worst?: string;
  blocks_at: string;
  /** Whether the pack in force would refuse this bundle. */
  blocked: boolean;
  findings: Finding[];
}

export interface Finding {
  path: string;
  rule: string;
  severity: string;
  title: string;
  line: number;
  count: number;
  /**
   * Whether *this* finding is one the pack in force refuses (ADR-0056
   * decision 5). Served, because the gateway is the only participant
   * holding both the severity order and the pack.
   */
  blocking: boolean;
}

export interface QualityReport {
  rubric_version: number;
  score: number;
  min_score: number;
  requires_checklist: boolean;
  checks: QualityCheck[];
  checklist?: Checklist;
  shortfalls: Shortfall[];
  needs_override: boolean;
}

export interface QualityCheck {
  check: string;
  passed: boolean;
  weight: number;
  title: string;
  detail?: string;
}

export interface Checklist {
  answers: Record<string, string>;
  note?: string;
  complete: boolean;
  concerns: string[];
  reviewed_at: string;
}

export interface Shortfall {
  kind: string;
  /** The gateway's own sentence (ADR-0056 decision 6). */
  detail: string;
}

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
  if (parts.length === 0) {
    parts.push("nothing");
  }
  return `${parts.join(" + ")}  (from: ${required.origins.join(", ")})`;
}
