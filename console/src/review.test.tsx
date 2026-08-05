/**
 * **The acceptance criterion, this half of it** (CNSL-1, ADR-0056 decision 7).
 *
 * CNSL-1 asks for full review parity with the CLI. The only version of that
 * word a test can fail is one where both surfaces answer the same corpus,
 * so `console/fixtures/<case>.facts.json` says what a review has to name,
 * `crates/synveda-cli/src/proposal.rs` asserts that `synveda proposal
 * review` names all of it, and this asserts the same file against the
 * console's own rendering.
 *
 * Every assertion is about **naming a fact**, never about layout: that a
 * blocking finding is distinguishable, not that it is a chip; that both
 * quality numbers appear, not the shape of the line they appear on.
 * ADR-0056 rejected serving a display model precisely so that a terminal
 * and a browser could differ where they should, and a parity suite that
 * pinned wording would be that display model arriving through the back
 * door.
 *
 * The rendering is `renderToStaticMarkup` over the real components,
 * reduced to the text a reader sees. Not a view model, and not a snapshot:
 * a view model would let the components drift away from the thing under
 * test, and a snapshot would fail on every whitespace change while passing
 * on a missing finding.
 */

import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { renderToStaticMarkup } from "react-dom/server";

import { Review } from "./Review.js";
import type { ProposalDetail } from "./review.mjs";
import { toLines, toText } from "./text.mjs";

const FIXTURES = new URL("../fixtures/", import.meta.url);

/**
 * Every case in the corpus. Kept in step with the list in
 * `synveda-gateway/tests/console_parity.rs`, which records them, and with
 * the CLI's; the test below refuses to let the three diverge.
 */
const CASES = [
  "memory-update",
  "memory-drifted",
  "skill-clean",
  "skill-below-bar",
  "skill-checklist-stale",
  "skill-blocking-scan",
  "skill-unknown-severity",
];

function corpus(name: string): unknown {
  return JSON.parse(readFileSync(new URL(name, FIXTURES), "utf8"));
}

interface Facts {
  state: string;
  outstanding: string;
  approvals: { subject: string; verdict: string; counts: boolean }[];
  members: {
    name: string;
    effect: "add" | "update" | "none";
    drifted: boolean;
    proposed: string | null;
    baseline: string | null;
    current: string | null;
  }[];
  scan?: {
    blocks_at: string;
    blocked: boolean;
    findings: {
      path: string;
      line: number;
      rule: string;
      severity: string;
      blocking: boolean;
    }[];
  };
  quality?: {
    score: number;
    min_score: number;
    checklist: "complete" | "partial" | "absent";
    checklist_required: boolean;
    shortfalls: string[];
    needs_override: boolean;
  };
}

/** Asserts the rendering names a fact, and says which one when it does not. */
function names(rendered: string, needle: string, what: string): void {
  assert.ok(
    rendered.includes(needle),
    `the review does not name ${what} (${JSON.stringify(needle)})\n\n${rendered}`,
  );
}

/** The rendered line that mentions `needle`. */
function lineWith(lines: string[], needle: string, what: string): string {
  const found = lines.find((line) => line.includes(needle));
  assert.ok(found !== undefined, `no line names ${what} (${JSON.stringify(needle)})\n\n${lines.join("\n")}`);
  return found;
}

/**
 * How a member is identifiable in a rendering.
 *
 * Derived from the *shape* of the name rather than by calling the
 * component's own helper, which would make the assertion agree with
 * whatever the component did.
 */
function identifier(name: string): string {
  return /^[0-9a-fA-F-]{36}$/.test(name) ? name.slice(0, 12) : name;
}

/**
 * The part of a rendering from one heading to the next.
 *
 * Facts are asserted where a reviewer would look for them. A scan finding
 * names the file it was found in and a member names the same file, so a
 * search over the whole page would let the scan block satisfy an assertion
 * about the diff, and the suite would pass with no members rendered at all.
 */
function section(lines: string[], from: string, to?: string): string[] {
  const start = lines.findIndex((line) => line.startsWith(from));
  assert.ok(start >= 0, `no ${JSON.stringify(from)} block\n\n${lines.join("\n")}`);
  const rest = lines.slice(start);
  if (to === undefined) {
    return rest;
  }
  const end = rest.findIndex((line, index) => index > 0 && line.startsWith(to));
  return end < 0 ? rest : rest.slice(0, end);
}

test("every case in the corpus is answered here", () => {
  const found = readdirSync(fileURLToPath(FIXTURES))
    .filter((name) => name.endsWith(".facts.json"))
    .map((name) => name.slice(0, -".facts.json".length))
    .sort();
  assert.deepEqual(
    found,
    [...CASES].sort(),
    "the corpus on disk and the cases this suite answers have diverged",
  );
});

for (const name of CASES) {
  test(`the console names every fact the corpus requires: ${name}`, () => {
    const detail = corpus(`${name}.json`) as ProposalDetail;
    const facts = corpus(`${name}.facts.json`) as Facts;
    const html = renderToStaticMarkup(<Review detail={detail} />);
    const rendered = toText(html);
    const lines = toLines(html);

    names(rendered, facts.state, "the proposal's state");
    names(rendered, facts.outstanding, "what the requirement still lacks");

    const reviews = section(lines, "reviews", "effect on");
    for (const approval of facts.approvals) {
      const line = lineWith(reviews, approval.subject, "an approver");
      assert.ok(
        line.includes(approval.verdict),
        `${approval.subject}'s verdict is not on the line naming them: ${line}`,
      );
      if (!approval.counts) {
        assert.ok(
          line.includes("does not count"),
          "an approval of an earlier commit must be marked as not counting, or a " +
            `reviewer reads a requirement as met that is not: ${line}`,
        );
      }
    }

    const members = section(lines, "effect on");
    for (const member of facts.members) {
      const line = lineWith(members, identifier(member.name), "a member");
      const label = { add: "add", update: "update", none: "same" }[member.effect];
      assert.ok(
        line.includes(label),
        `what publishing would do to ${member.name} is not on its line: ${line}`,
      );
      if (member.drifted) {
        names(members.join("\n"), "publishing will refuse", "that the member drifted");
      }
      const contents: [string | null, string][] = [
        [member.baseline, "the bytes a publication would overwrite"],
        [member.proposed, "the bytes under review"],
        [member.current, "the member as it stands now"],
      ];
      for (const [text, what] of contents) {
        if (text === null) {
          continue;
        }
        for (const content of text.split("\n").filter((line) => line.trim().length > 0)) {
          names(members.join("\n"), content, what);
        }
      }
    }

    if (facts.scan) {
      const scan = section(lines, "security scan", "effect on");
      for (const finding of facts.scan.findings) {
        const line = lineWith(scan, finding.rule, "a scan finding");
        for (const part of [finding.path, String(finding.line), finding.severity]) {
          assert.ok(line.includes(part), `${finding.rule} is missing ${part} from its line: ${line}`);
        }
        // ADR-0056 decision 5: the gateway's verdict, and a reader with no
        // colour — a screen reader, a ticket, a printout — still has to be
        // able to tell which findings stop the publication.
        assert.equal(
          line.includes("blocks"),
          finding.blocking,
          `${finding.rule} is served blocking=${finding.blocking} and its line does not ` +
            `say so: ${line}`,
        );
      }
      if (facts.scan.blocked) {
        names(scan.join("\n"), "REFUSED", "that the pack in force will refuse this bundle");
      }
    }

    if (facts.quality) {
      // Two numbers, never one (ADR-0053 decision 1).
      names(rendered, `${facts.quality.score}/100`, "the rubric score");
      names(rendered, String(facts.quality.min_score), "the bar the pack asks for");
      const checklist =
        facts.quality.checklist === "complete"
          ? "complete"
          : facts.quality.checklist === "partial"
            ? "PARTIAL"
            : facts.quality.checklist_required
              ? "NONE recorded"
              : "none recorded";
      names(rendered, checklist, "the state of the checklist");
      for (const shortfall of facts.quality.shortfalls) {
        // Verbatim: the sentence is the gateway's, and a surface that
        // reworded it would be the second author decision 6 prevents.
        names(rendered, shortfall, "a bar this bundle misses");
      }
      if (facts.quality.needs_override) {
        names(rendered, "quality override", "that publishing needs an override");
      }
    }
  });
}

test("a read-only review offers no verdict, and an actionable one requires a reason to reject", () => {
  const detail = corpus("skill-clean.json") as ProposalDetail;

  const readOnly = toText(renderToStaticMarkup(<Review detail={detail} />));
  assert.ok(
    !readOnly.includes("Approve"),
    `a review with no verdict handler must offer no buttons:\n\n${readOnly}`,
  );

  const actionable = renderToStaticMarkup(<Review detail={detail} onVerdict={() => {}} />);
  assert.ok(actionable.includes("Approve"), actionable);
  assert.ok(actionable.includes("Reject"), actionable);
  // The reason box starts empty, so Reject starts disabled. The gateway
  // refuses a reasonless rejection anyway; this is what stops a reviewer
  // losing what they typed to a 400 they could have been told about first.
  const reject = actionable.slice(actionable.indexOf("<button", actionable.indexOf("Approve")));
  assert.ok(
    reject.includes("disabled"),
    `Reject must start disabled with no reason given:\n\n${actionable}`,
  );
});
