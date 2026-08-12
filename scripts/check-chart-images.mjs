#!/usr/bin/env node
// Asserts that every container image we ship — the Helm chart's, the
// released single-node profile's, and every base image the images we build
// are built from — appears in deploy/helm/IMAGES.md, tag included. Writes
// nothing, ever.
//
// The release profile joined the chart as a surface with OPS-8 (ADR-0065
// decision 9): those are images a *customer installs*, which is a stronger
// reason to know their licences than the chart's, not a weaker one. The
// file and this script keep their chart-shaped names — renaming both plus
// every reference is churn against OPS-2's artefacts for no reading.
//
// Why (OPS-2, ADR-0062 decision 11): CLAUDE.md's licence rule is enforced
// by cargo-deny over crates, check-npm-licences over packages and
// check-corpus-licences over corpora. A chart introduces a fourth artefact
// class — container images — that none of those look at. This is the same
// gap in the same shape as the one that let a CC BY-NC corpus reach a
// published phase demo goal (EVAL-7).
//
// What it can and cannot do: an image carries no machine-readable licence,
// so this proves the inventory is *complete*, not that the licences in it
// are admissible. Matching includes the tag on purpose — a bump to
// text-embeddings-inference is exactly the event that should make somebody
// re-read a licence, and a check that ignored tags would let it through.
//
// Usage: node scripts/check-chart-images.mjs   (exit 0 clean, 1 with findings)

import { readFileSync } from "node:fs";

const INVENTORY = "deploy/helm/IMAGES.md";
const CHART = "deploy/helm/synveda/Chart.yaml";
const VALUES = "deploy/helm/synveda/values.yaml";
const RELEASE_COMPOSE = "deploy/release/docker-compose.yml";
// The per-architecture TEI pins. They are declared here, in the one place
// that resolves them, and `synveda init` carries the same table for an
// installed operator who has no Makefile — so this is where the inventory
// learns about the arm64 build, which no compose file names.
const MAKEFILE = "Makefile";
const DOCKERFILES = [
  "deploy/helm/postgres/Dockerfile",
  "deploy/compose/gateway/Dockerfile",
];

const problems = [];
const fail = (message) => problems.push(message);

const read = (path) => readFileSync(path, "utf8");

// ── The inventory ────────────────────────────────────────────────────────
// Every backticked token that looks like `repo:tag`. The gateway's own tag
// is the chart's appVersion, which the inventory writes as the literal
// `<appVersion>` because pinning it here would mean editing this file on
// every release for no reading.
// The file also records the images the install test runs, which the chart
// never references. Those count for *matching* — naming one is never an
// error — but not for the orphan note at the end, or every run would
// report them as unreferenced forever.
const text = read(INVENTORY);
const shippedText = text.split(/^## Images the install test runs/m)[0];
const inventory = new Set();
const shipped = new Set();
for (const [, ref] of text.matchAll(/`([^`\s]+:[^`\s]+)`/g)) inventory.add(ref);
for (const [, ref] of shippedText.matchAll(/`([^`\s]+:[^`\s]+)`/g)) shipped.add(ref);
if (inventory.size === 0) fail(`${INVENTORY}: no image references found — the format changed`);

// ── What the chart references ────────────────────────────────────────────
const appVersion = read(CHART).match(/^appVersion:\s*"?([^"\s]+)"?/m)?.[1];
if (!appVersion) fail(`${CHART}: no appVersion to resolve the product image's tag against`);

const found = new Map(); // ref → where it came from

const values = read(VALUES);
// `image: repo:tag` — a scalar, never the mapping key of the same name.
for (const [, ref] of values.matchAll(/^\s*image:\s+(\S+)\s*$/gm)) {
  found.set(ref, `${VALUES} (image:)`);
}
// The product image is split across repository/tag, and an empty tag means
// the chart's appVersion (values.yaml says so, and _helpers.tpl does it).
const repository = values.match(/^\s*repository:\s*(\S+)\s*$/m)?.[1];
if (repository) {
  const tag = values.match(/^\s*tag:\s*"(.*)"\s*$/m)?.[1] ?? "";
  found.set(`${repository}:${tag === "" ? "<appVersion>" : tag}`, `${VALUES} (image.repository)`);
}

// ── What the released single-node profile runs ───────────────────────────
// `image: <ref>`, where <ref> may be `${VAR:-default}` (the TEI image, which
// `synveda init` overrides per architecture) and may carry the packager's
// `__SYNVEDA_VERSION__` placeholder. The placeholder is inventoried as
// `<version>` for the same reason the chart's tag is inventoried as
// `<appVersion>`: pinning it here would mean editing the inventory on every
// release for no reading.
const release = read(RELEASE_COMPOSE);
for (const [, raw] of release.matchAll(/^\s*image:\s+(\S+)\s*$/gm)) {
  const defaulted = raw.match(/^\$\{[A-Z_]+:-(.+)\}$/);
  const ref = (defaulted ? defaulted[1] : raw).replace("__SYNVEDA_VERSION__", "<version>");
  if (ref.includes("$")) {
    fail(`${RELEASE_COMPOSE}: image ${raw} has no default, so it cannot be inventoried`);
    continue;
  }
  found.set(ref, `${RELEASE_COMPOSE} (image:)`);
}

// ── The per-architecture TEI pins ────────────────────────────────────────
for (const [, ref] of read(MAKEFILE).matchAll(/^TEI_IMAGE_\w+\s*=\s*(\S+:\S+)\s*$/gm)) {
  found.set(ref, `${MAKEFILE} (TEI_IMAGE_*)`);
}

// ── What the images we build are built from ──────────────────────────────
for (const path of DOCKERFILES) {
  const text = read(path);
  const args = new Map();
  for (const [, name, value] of text.matchAll(/^ARG\s+(\w+)=(\S+)\s*$/gm)) args.set(name, value);
  for (const [, raw] of text.matchAll(/^FROM\s+(\S+)/gm)) {
    // `FROM ${CNPG_BASE}` resolves against the ARG default above it.
    const ref = raw.replace(/\$\{(\w+)\}/g, (whole, name) => args.get(name) ?? whole);
    if (ref.includes("$")) {
      fail(`${path}: FROM ${raw} references a build arg with no default, so it cannot be inventoried`);
      continue;
    }
    found.set(ref, `${path} (FROM)`);
  }
}

// ── The check ────────────────────────────────────────────────────────────
for (const [ref, where] of found) {
  if (!inventory.has(ref)) {
    fail(
      `${ref} — referenced by ${where} — is not in ${INVENTORY}.\n` +
        `      Add it with its licence and the reason it is there. If this is a version bump,\n` +
        `      the licence is the thing to re-read before updating the row.`,
    );
  }
}

// The other direction is a note, not a failure: an inventory row can
// outlive the reference that needed it (a base image dropped from a
// Dockerfile), and deleting the licence somebody read is worse than
// carrying a stale row until they say so.
const orphans = [...shipped].filter((ref) => !found.has(ref));

if (orphans.length) {
  console.log(`note: in ${INVENTORY} but referenced by nothing: ${orphans.join(", ")}`);
}
if (problems.length) {
  for (const p of problems) console.error(`FAIL ${p}`);
  console.error(`\n${problems.length} problem(s); we ship images the inventory does not name.`);
  process.exit(1);
}
console.log(`ok: ${found.size} image reference(s) across the chart, the release profile and their Dockerfiles, all inventoried in ${INVENTORY}.`);
