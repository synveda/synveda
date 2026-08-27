#!/usr/bin/env node
// Asserts no ADR still reads `Proposed` after its feature shipped.
//
// AGENTS.md requires an ADR before implementation, so an ADR spends its
// early life as `Proposed` — correctly. What nothing checked was the other
// end: the feature lands, STATUS.md gets its `[x]`, and the ADR keeps
// saying the decision is a proposal. Two of them drifted that way
// (ADR-0046 for EVAL-2, delivered 2026-07-30; ADR-0060 for AUTH-5,
// delivered 2026-08-07) and were only found by cross-checking every header
// against STATUS.md, which is exactly the kind of sweep nobody does twice.
//
// **One direction only, and the asymmetry is the design.** The mirror
// check — `Accepted` before the feature ships — would fire on every
// feature in flight, because writing the ADR first is the rule rather than
// the exception. A gate that fails during normal work is a gate someone
// turns off, so this one stays silent there. ADR-0061 sat in exactly that
// state on the day this was written.
//
// Run by `make ci` beside check-backlog.mjs, which validates the feature
// inventory and open briefs but never reads an ADR header.
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const ADRS = "docs/adr";
const STATUS = "docs/backlog/STATUS.md";
const TEMPLATE = "adr-0000-template.md";

/// The statuses an ADR may declare. Closed, and checked, because a typo'd
/// one would read as "not Proposed" and pass this gate silently — the same
/// vacuity the gate exists to remove.
const STATUSES = ["Proposed", "Accepted", "Superseded"];

/// How far into a file the header lives. Enough for the longest one here,
/// short enough that a mention of "Status" in the prose cannot be mistaken
/// for the header.
const HEADER_CHARS = 900;

const fail = (message) => {
  console.error(`FAIL: ${message}`);
  failed = true;
};
let failed = false;

const status = readFileSync(STATUS, "utf8");
const delivered = new Set([...status.matchAll(/^- \[x\] ([A-Z]+-\d+):/gm)].map((m) => m[1]));
const pending = new Set([...status.matchAll(/^- \[ \] \[([A-Z]+-\d+):/gm)].map((m) => m[1]));

const files = readdirSync(ADRS)
  .filter((name) => name.startsWith("adr-") && name.endsWith(".md") && name !== TEMPLATE)
  .sort();

let proposed = 0;
for (const name of files) {
  const head = readFileSync(join(ADRS, name), "utf8").slice(0, HEADER_CHARS);
  const at = `${ADRS}/${name}`;

  const declared = head.match(/^- \*\*Status\*\*:\s*([A-Za-z]+)/m);
  const features = head.match(/^- \*\*Feature\(s\)\*\*:\s*(.+)$/m);
  if (!declared) {
    fail(`${at} declares no **Status** in its header`);
    continue;
  }
  if (!features) {
    fail(`${at} declares no **Feature(s)** in its header, so nothing can tell whether its decisions have shipped`);
    continue;
  }
  if (!STATUSES.includes(declared[1])) {
    fail(
      `${at} declares status '${declared[1]}', which is not one of ${STATUSES.join(" / ")}. ` +
        `A status this script does not know reads as 'not Proposed' and passes silently.`,
    );
    continue;
  }
  if (declared[1] !== "Proposed") continue;
  proposed += 1;

  // `ADR-0056` is a cross-reference, not a feature — ADR-0058's header
  // names one. Everything else of the shape `AAA-9` is a feature id.
  const named = [...features[1].matchAll(/\b([A-Z]+-\d+)\b/g)]
    .map((m) => m[1])
    .filter((id) => !id.startsWith("ADR-"));
  const known = named.filter((id) => delivered.has(id) || pending.has(id));

  if (known.length === 0) {
    fail(
      `${at} is Proposed and names no feature ${STATUS} knows (found: ${named.join(", ") || "none"}). ` +
        `This gate cannot tell whether its decisions have shipped, so it would skip this ADR ` +
        `forever — name a feature id that exists.`,
    );
    continue;
  }
  const outstanding = known.filter((id) => pending.has(id));
  if (outstanding.length === 0) {
    fail(
      `${at} still reads Proposed, but ${known.join(", ")} ${known.length === 1 ? "has" : "have"} ` +
        `shipped. An ADR whose consequences have all been paid is not a proposal — set it to ` +
        `Accepted, or to Superseded if something replaced it.`,
    );
  }
}

if (failed) process.exit(1);
console.log(
  `ADR statuses hold for ${files.length} ADRs ` +
    `(${proposed} still Proposed, ${files.length - proposed} Accepted or Superseded)`,
);
