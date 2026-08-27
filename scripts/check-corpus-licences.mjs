#!/usr/bin/env node
// The repository licence rule on the *corpus* side (EVAL-3, ADR-0061
// compliance notes) — the gate that did not exist when it was needed.
//
// ADR-0061 decision 1 found LoCoMo's corpus is licensed CC BY-NC 4.0: it
// grants rights "for NonCommercial purposes only", and this feature's own
// acceptance criterion calls its published scores a marketing artefact.
// Publishing LoCoMo numbers to sell an enterprise product is the paradigm
// case of the use that licence withholds — and nothing in the build would
// have caught it. `cargo deny` governs crates. `check-npm-licences.mjs`
// governs packages. A corpus is data, so it passed through the feature
// specification and phase demo goal untouched by any gate,
// and was caught by somebody reading a LICENSE.txt.
//
// This is that gap closed where the build can see it. Three shapes of
// failure, in the order they would actually happen:
//
//   1. A corpus arrives in a directory nobody declared. It fails until
//      somebody writes down where it came from and under what licence,
//      which puts the licence in a diff a person reviews. This is the one
//      that would have caught LoCoMo.
//   2. A declared licence is not one the core path admits. Same list
//      `deny.toml` holds, for the same reason.
//   3. A licence *file* on disk carries a non-commercial or
//      no-derivatives grant. This one fires on the machine that fetched
//      the corpus, before any score is computed from it — which matters
//      because the corpora are too large to commit and the licence file
//      arrives with the data rather than in the history.
//
// Run by `make ci` beside check-npm-licences.mjs and check-crate-deps.mjs.
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const ROOT = "evals/fixtures";

/// deny.toml's `allow`, verbatim. A corpus whose numbers we publish is on
/// the core path whatever else it is.
const PERMITTED = ["MIT", "Apache-2.0", "PostgreSQL", "Unicode-3.0"];

/// The licence families this repo may not carry at all, matched
/// case-insensitively against the text of any licence file found under
/// ROOT. Named rather than inferred: these are the two that withhold
/// exactly what a published benchmark score does with a corpus.
const WITHHELD = [
  ["noncommercial", "a non-commercial grant cannot cover a score published to sell a product"],
  ["no derivatives", "a no-derivatives grant cannot cover a corpus this harness reads and reports on"],
  ["noderivatives", "a no-derivatives grant cannot cover a corpus this harness reads and reports on"],
];

/// Every directory under ROOT, and where its material came from. A
/// directory absent from this table fails — the deny.toml discipline, one
/// entry at a time and never a widened default.
///
///   first-party  — written in this repo, carries the repo's licence.
///   third-party  — somebody else's material. Needs `licence` (from
///                  PERMITTED), `source`, a committed NOTICE.md, and — on
///                  any machine that has actually fetched the data —
///                  upstream's own licence file beside it.
const ORIGINS = {
  extraction: { origin: "first-party" },
  judge: { origin: "first-party" },
  qa: { origin: "first-party" },
  reader: { origin: "first-party" },
  security: { origin: "first-party" },
  longmemeval: {
    origin: "third-party",
    licence: "MIT",
    source: "https://github.com/xiaowu0162/LongMemEval",
    // The data is fetched rather than committed (NOTICE.md says why), so
    // the licence-file rule is conditional on the data being here.
    data: /^longmemeval_.*\.json$/,
  },
};

const LICENCE_FILE = /^(licen[cs]e|copying)(\..*)?$/i;

const directories = readdirSync(ROOT).filter((entry) =>
  statSync(join(ROOT, entry)).isDirectory(),
);

let failed = false;
const fail = (message) => {
  console.error(`FAIL: ${message}`);
  failed = true;
};

for (const name of directories) {
  const declared = ORIGINS[name];
  if (declared === undefined) {
    fail(
      `${ROOT}/${name} is not declared in scripts/check-corpus-licences.mjs. ` +
        `Add it as first-party, or as third-party with its licence and source — ` +
        `a corpus whose provenance nobody wrote down is how a CC BY-NC corpus ` +
        `reached a feature specification (ADR-0061 decision 1).`,
    );
    continue;
  }

  const files = readdirSync(join(ROOT, name));
  const licenceFiles = files.filter((file) => LICENCE_FILE.test(file));

  if (declared.origin === "first-party") {
    if (licenceFiles.length > 0) {
      fail(
        `${ROOT}/${name} is declared first-party but carries ${licenceFiles.join(", ")}. ` +
          `First-party fixtures take the repository's licence; a licence file here means ` +
          `somebody else's material arrived without the table being updated.`,
      );
    }
    continue;
  }

  if (declared.origin !== "third-party") {
    fail(`${ROOT}/${name} declares an unknown origin '${declared.origin}'`);
    continue;
  }

  if (!PERMITTED.includes(declared.licence)) {
    fail(
      `${ROOT}/${name} is declared ${declared.licence}, which the core path does not admit ` +
        `(${PERMITTED.join(" / ")}, per deny.toml). There is no exception ` +
        `mechanism here: a corpus whose licence withholds the use we make of it is a corpus ` +
        `that does not enter the repository.`,
    );
  }
  if (!existsSync(join(ROOT, name, "NOTICE.md"))) {
    fail(
      `${ROOT}/${name} is third-party and has no NOTICE.md recording its source, licence ` +
        `and attribution.`,
    );
  }

  // Upstream's licence file travels with the data. On a machine that has
  // not fetched the corpus there is nothing to assert; on one that has,
  // "vendored with its licence file intact" is a checkable claim.
  const data = files.filter((file) => declared.data?.test(file));
  if (data.length > 0 && licenceFiles.length === 0) {
    fail(
      `${ROOT}/${name} holds ${data.join(", ")} but none of upstream's licence file. ` +
        `Fetch it alongside the data — see ${ROOT}/${name}/NOTICE.md.`,
    );
  }
}

// Every licence file under ROOT, committed or fetched, read for the two
// grants that cannot cover what this repository does with a corpus. This
// is the check that fires on a developer's machine rather than in CI,
// because the corpora are too large to commit and their licence files
// arrive with them.
for (const name of directories) {
  for (const file of readdirSync(join(ROOT, name))) {
    if (!LICENCE_FILE.test(file)) continue;
    const path = join(ROOT, name, file);
    const text = readFileSync(path, "utf8").toLowerCase().replace(/[-_]/g, " ");
    for (const [marker, why] of WITHHELD) {
      if (text.includes(marker)) {
        fail(
          `${path} carries a '${marker}' grant — ${why}. This corpus does not enter the ` +
            `repository; ADR-0061 decision 1 and EVAL-7 record the two paths that would ` +
            `change that.`,
        );
      }
    }
  }
}

// An entry nobody needs is an entry nobody re-reads — the same reasoning
// as check-npm-licences.mjs's BUILD_EXCEPTIONS sweep and deny.toml's
// annotations.
for (const name of Object.keys(ORIGINS)) {
  if (!directories.includes(name)) {
    fail(
      `scripts/check-corpus-licences.mjs declares ${ROOT}/${name}, which no longer exists — ` +
        `remove it.`,
    );
  }
}

if (failed) process.exit(1);
const third = directories.filter((name) => ORIGINS[name]?.origin === "third-party");
console.log(
  `corpus licences hold for ${directories.length} fixture director${directories.length === 1 ? "y" : "ies"} ` +
    `(${third.length} third-party: ${third.map((name) => `${name} ${ORIGINS[name].licence}`).join(", ") || "none"})`,
);
