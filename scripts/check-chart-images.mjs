#!/usr/bin/env node
// Asserts that every container image we ship or use for a deployment fixture
// — the canonical Compose graph's, the Helm chart's, the released single-node
// profile's, and every base image in deployment Dockerfiles — appears in
// deploy/helm/IMAGES.md, tag included.
// Writes nothing, ever.
//
// The release profile joined the chart as a surface with OPS-8 (ADR-0065
// decision 9): those are images a *customer installs*, which is a stronger
// reason to know their licences than the chart's, not a weaker one. The
// file and this script keep their chart-shaped names — renaming both plus
// every reference is churn against OPS-2's artefacts for no reading.
//
// Why (OPS-2, ADR-0062 decision 11): the repository licence rule is enforced
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

import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import {
  canonicalComposeFiles,
  composeImageReferences,
  dockerfileBaseImages,
  parseComposeDefaults,
} from "./chart-image-discovery.mjs";

const INVENTORY = "deploy/helm/IMAGES.md";
const CHART = "deploy/helm/synveda/Chart.yaml";
const VALUES = "deploy/helm/synveda/values.yaml";
const RELEASE_COMPOSE = "deploy/release/docker-compose.yml";
const COMPOSE_DIRECTORY = "deploy/compose";
const COMPOSE_DEFAULTS = `${COMPOSE_DIRECTORY}/.env.example`;
const LEGACY_COMPOSE = `${COMPOSE_DIRECTORY}/docker-compose.yml`;
// The per-architecture TEI pins. They are declared here, in the one place
// that resolves them, and `synveda init` carries the same table for an
// installed operator who has no Makefile — so this is where the inventory
// learns about the arm64 build, which no compose file names.
const MAKEFILE = "Makefile";
const problems = [];
const fail = (message) => problems.push(message);

const read = (path) => readFileSync(path, "utf8");

function deploymentDockerfiles(directory, found = []) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      deploymentDockerfiles(path, found);
    } else if (entry.name === "Dockerfile") {
      if (!entry.isFile()) fail(`${path}: deployment Dockerfile is not a regular file`);
      found.push(path);
    }
  }
  return found;
}

const DOCKERFILES = deploymentDockerfiles("deploy").sort();

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

// ── What the canonical Compose graph and its fixtures run ────────────────
// The checked-in non-secret defaults resolve every canonical image selector.
// Reference deployments replace the locally built Synveda image names with
// environment-manifest digests, but the third-party Collector remains the
// same exact runtime dependency and must not escape the image inventory.
let composeDefaults = new Map();
try {
  composeDefaults = parseComposeDefaults(read(COMPOSE_DEFAULTS));
} catch (error) {
  fail(`${COMPOSE_DEFAULTS}: ${error?.code ?? "defaults could not be parsed"}`);
}
const composeFiles = canonicalComposeFiles(
  readdirSync(COMPOSE_DIRECTORY, { withFileTypes: true }),
);
if (!composeFiles.includes("compose.yaml")) {
  fail(`${COMPOSE_DIRECTORY}: canonical compose.yaml was not discovered`);
}
for (const name of composeFiles) {
  const path = join(COMPOSE_DIRECTORY, name);
  try {
    for (const ref of composeImageReferences(read(path), composeDefaults)) {
      found.set(ref, `${path} (image:)`);
    }
  } catch (error) {
    fail(`${path}: ${error?.code ?? "image selector could not be resolved"}`);
  }
}

// The contributor `make dev-up` stack remains executable until CPR-45 deletes
// its Rauthy and Temporal residue. It is not canonical, but exclusion from the
// reference graph cannot make its pulled and locally built images invisible.
try {
  for (const ref of composeImageReferences(read(LEGACY_COMPOSE), composeDefaults)) {
    found.set(ref, `${LEGACY_COMPOSE} (legacy image:)`);
  }
} catch (error) {
  fail(`${LEGACY_COMPOSE}: ${error?.code ?? "legacy image selector could not be resolved"}`);
}

// ── The per-architecture TEI pins ────────────────────────────────────────
for (const [, ref] of read(MAKEFILE).matchAll(/^TEI_IMAGE_\w+\s*=\s*(\S+:\S+)\s*$/gm)) {
  found.set(ref, `${MAKEFILE} (TEI_IMAGE_*)`);
}

// ── What the images we build are built from ──────────────────────────────
for (const path of DOCKERFILES) {
  const text = read(path);
  try {
    for (const ref of dockerfileBaseImages(text)) {
      found.set(ref, `${path} (FROM)`);
    }
  } catch (error) {
    fail(`${path}: ${error?.code ?? "base image could not be resolved"}`);
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
  console.error(`\n${problems.length} problem(s); the deployment image surface is not fully inventoried.`);
  process.exit(1);
}
console.log(`ok: ${found.size} image reference(s) across canonical Compose, the chart, the release profile and deployment Dockerfiles, all inventoried in ${INVENTORY}.`);
