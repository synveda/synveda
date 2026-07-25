#!/usr/bin/env node
// Enforces the crate layering rule (seed §8; synveda-vedaflow added by tech plan §5):
//
//   types ← {policy, store, identity, audit, vedaflow} ← retrieval/ingest ← gateway
//
// Nothing imports upward. Fails if any synveda crate declares a dependency on a
// synveda crate outside its allowed set, or if a workspace crate is unknown here
// (new crates must be placed in a tier deliberately).
import { execFileSync } from "node:child_process";

const MIDDLE = [
  "synveda-policy",
  "synveda-store",
  "synveda-identity",
  "synveda-audit",
  "synveda-vedaflow",
];

const ALLOWED = {
  "synveda-types": [],
  "synveda-policy": ["synveda-types"],
  "synveda-store": ["synveda-types"],
  "synveda-identity": ["synveda-types"],
  "synveda-audit": ["synveda-types"],
  "synveda-vedaflow": ["synveda-types"],
  "synveda-retrieval": ["synveda-types", ...MIDDLE],
  "synveda-ingest": ["synveda-types", ...MIDDLE],
  "synveda-gateway": ["synveda-types", ...MIDDLE, "synveda-retrieval", "synveda-ingest"],
  // The CLI is a client of the gateway API, plus direct store/identity access
  // for the dev-bootstrap commands (db migrate, tenant create, token issue)
  // that exist precisely when no usable gateway does. Reviewed in ADR-0008.
  // Policy added with AUTHZ-1 (ADR-0012): `synveda policy apply` compile-checks
  // a pack against the same schema the gateway's reloader enforces.
  // Audit added with AUD-1 (ADR-0019): the break-glass audits itself, and
  // `synveda audit verify` is the operator's chain check.
  // The eval harness depends on no Synveda crate at all, and this empty
  // set is the enforcement (EVAL-1, ADR-0028 decision 1). An eval that can
  // link the store can seed and read around the PDP and would then report
  // quality the product cannot deliver; one that speaks only `/v1` measures
  // what a caller gets. It is the standing the seed gives adapters and SDKs,
  // applied to the thing that grades the product.
  "synveda-eval": [],
  "synveda-cli": [
    "synveda-types",
    "synveda-store",
    "synveda-identity",
    "synveda-policy",
    "synveda-audit",
  ],
};

const metadata = JSON.parse(
  execFileSync("cargo", ["metadata", "--format-version", "1", "--no-deps"], {
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  }),
);

let failed = false;
for (const pkg of metadata.packages) {
  const allowed = ALLOWED[pkg.name];
  if (allowed === undefined) {
    console.error(
      `FAIL: unknown workspace crate '${pkg.name}' — assign it a tier in scripts/check-crate-deps.mjs`,
    );
    failed = true;
    continue;
  }
  const allowedSet = new Set(allowed);
  for (const dep of pkg.dependencies) {
    if (dep.name.startsWith("synveda-") && !allowedSet.has(dep.name)) {
      console.error(
        `FAIL: ${pkg.name} -> ${dep.name} violates the layering rule (seed §8)`,
      );
      failed = true;
    }
  }
}

if (failed) process.exit(1);
console.log(`crate layering rule holds for ${metadata.packages.length} crates (seed §8)`);
