#!/usr/bin/env node
import assert from "node:assert/strict";
import fs, { readFileSync, readdirSync } from "node:fs";
import { syncBuiltinESMExports } from "node:module";
import { dirname, join, relative, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import * as effectGeneration from "../deploy/compose/scripts/clean-engine-live-provider-effect-generation.mjs";
import {
  COLIMA_LIVE_MUTATION_SURFACE_ROLES,
} from "../deploy/compose/scripts/clean-engine-colima-live-schemas.mjs";
import {
  buildColimaLiveProviderProcessStartEffectFreshAdmissionStructure,
} from "../deploy/compose/scripts/clean-engine-live-provider-process-start.mjs";
import {
  cleanEngineLiveProviderProcessStartFreshAdmissionFixture,
  cleanEngineLiveProviderProcessStartRootObservationFixture,
  cleanEngineLiveProviderProcessStartSourceFixture,
} from "./fixtures/clean-engine-live-provider-process-start-fixture.mjs";

const {
  COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_GENERATION_PREREQUISITE_PROJECTION_SCHEMA,
  COLIMA_LIVE_PROVIDER_EFFECT_GENERATION_PREREQUISITE_PROJECTION_SCHEMA,
  LiveProviderEffectGenerationFailure,
  buildColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure,
  liveProviderEffectGenerationBytes,
  liveProviderEffectGenerationDigest,
  validateColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure,
} = effectGeneration;

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(HERE, "..");
const MODULE_PATH = resolve(
  REPO_ROOT,
  "deploy/compose/scripts/clean-engine-live-provider-effect-generation.mjs",
);
const TEST_PATH = fileURLToPath(import.meta.url);

function clone(value) {
  return structuredClone(value);
}

function digest(value) {
  return liveProviderEffectGenerationDigest(
    liveProviderEffectGenerationBytes(value),
  );
}

function prepared(fixtureOnly = false) {
  const upstream =
    cleanEngineLiveProviderProcessStartFreshAdmissionFixture(fixtureOnly);
  const input = { admission: upstream.admission, source: upstream.source };
  return {
    ...upstream,
    input,
    projection:
      buildColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(
        input,
      ),
  };
}

function assertRecursivelyFrozen(value) {
  if (value === null || typeof value !== "object") return;
  assert.equal(Object.isFrozen(value), true);
  for (const child of Object.values(value)) assertRecursivelyFrozen(child);
}

function changedValue(value) {
  if (typeof value === "boolean") return !value;
  if (typeof value === "string") {
    if (/^[0-9a-f]{64}$/u.test(value)) {
      return `${value[0] === "a" ? "b" : "a"}${value.slice(1)}`;
    }
    return `${value}-changed`;
  }
  if (Array.isArray(value)) return [...value, "unexpected"];
  if (value !== null && typeof value === "object") return null;
  return "unexpected";
}

function expectRefusal(operation, pattern = /effect-generation/u) {
  assert.throws(operation, (error) => {
    assert.ok(error instanceof LiveProviderEffectGenerationFailure);
    assert.match(error.message, pattern);
    assert.equal(error.exitStatus >= 69, true);
    return true;
  });
}

function expectStructuralRefusal(operation, pattern) {
  assert.throws(operation, (error) => {
    assert.ok(error instanceof LiveProviderEffectGenerationFailure);
    assert.equal(error.exitStatus, 70);
    assert.match(error.message, pattern);
    return true;
  });
}

function nestedArray(depth) {
  let value = "leaf";
  for (let index = 0; index < depth; index += 1) value = [value];
  return value;
}

function binaryTree(depth) {
  if (depth === 0) return "leaf";
  return [binaryTree(depth - 1), binaryTree(depth - 1)];
}

test("structural validation performs no filesystem observation", () => {
  const observedMethods = [
    "closeSync",
    "fstatSync",
    "lstatSync",
    "openSync",
    "opendirSync",
    "readSync",
    "realpathSync",
  ];
  const originals = new Map(
    observedMethods.map((name) => [name, fs[name]]),
  );
  let attemptedMethod;
  try {
    for (const name of observedMethods) {
      fs[name] = () => {
        attemptedMethod = name;
        throw new Error(`unexpected filesystem observation through ${name}`);
      };
    }
    syncBuiltinESMExports();
    prepared(false);
    prepared(true);
  } finally {
    for (const [name, implementation] of originals) {
      fs[name] = implementation;
    }
    syncBuiltinESMExports();
  }
  assert.equal(attemptedMethod, undefined);
});

test("production and fixture prerequisite projections are exact and non-authorizing", () => {
  const variants = [
    {
      evidenceClass: "production-pinned",
      fixtureOnly: false,
      projectionDigest:
        "1502c4bf0ef93336f2d6c887ed588bd363d1a5b9210508ce496b42a583831853",
      schema:
        COLIMA_LIVE_PROVIDER_EFFECT_GENERATION_PREREQUISITE_PROJECTION_SCHEMA,
    },
    {
      evidenceClass: "fixture-only",
      fixtureOnly: true,
      projectionDigest:
        "d9b0745d63b2954296e8e8d4ec38f8e6a69e2f0f60316e708fba650ed8728b20",
      schema:
        COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_GENERATION_PREREQUISITE_PROJECTION_SCHEMA,
    },
  ];
  assert.equal(
    COLIMA_LIVE_PROVIDER_EFFECT_GENERATION_PREREQUISITE_PROJECTION_SCHEMA,
    "synveda.clean-engine.colima-live-provider-effect-generation-prerequisite-projection.v1",
  );
  assert.equal(
    COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_GENERATION_PREREQUISITE_PROJECTION_SCHEMA,
    "synveda.clean-engine.colima-live-fixture-provider-effect-generation-prerequisite-projection.v1",
  );
  for (const variant of variants) {
    const fixture = prepared(variant.fixtureOnly);
    const value = fixture.projection;
    assert.equal(value.schema, variant.schema);
    assert.equal(value.evidence_class, variant.evidenceClass);
    assert.equal(value.authority, "none-prerequisite-projection-only");
    assert.equal(value.branch, "sibling-effect-prerequisite-only");
    assert.equal(value.predecessor, "completed-start-decision-only");
    assert.equal(
      value.completed_no_spawn_predecessor,
      "forbidden-terminal-sibling",
    );
    assert.equal(
      value.fixed_marker_name,
      ".synveda-clean-engine-provider-reservation",
    );
    assert.equal(value.state_integration, "not-integrated");
    assert.equal(value.future_atomic_generation.required, true);
    assert.deepEqual(value.future_atomic_generation, {
      mutation_close_schema: "synveda.clean-engine.mutation-close.v8",
      mutation_recovery_root_schema:
        "synveda.clean-engine.mutation-recovery-root.v6",
      mutation_recovery_schema: "synveda.clean-engine.mutation-recovery.v6",
      mutation_slot_schema: "synveda.clean-engine.mutation-slot.v7",
      receipt_schema: "synveda.clean-engine.receipt.v6",
      required: true,
    });
    assert.deepEqual(value.future_execution_prerequisites.causal_role_contract, [
      "outer",
      "hostagent",
      "usernet",
      "ssh-controlmaster",
    ]);
    assert.equal(
      value.future_execution_prerequisites
        .complete_bounded_causal_graph_required,
      true,
    );
    assert.equal(
      value.future_effect_witness.marker_continuity_boundary,
      "before-start-authority-through-terminal-receipt",
    );
    for (const [field, authorized] of Object.entries(value.authorities)) {
      assert.equal(authorized, false, field);
    }
    const upstreamAuthorityFields = Object.keys(
      fixture.admission.process_start_effect_candidate,
    ).filter(
      (field) => field.endsWith("_authorized") || field === "lifecycle_exposed",
    );
    for (const field of upstreamAuthorityFields) {
      assert.equal(
        Object.hasOwn(value.authorities, field),
        true,
        `upstream authority ${field}`,
      );
    }
    assert.equal(value.future_effect_witness.publication_authorized, false);
    assert.equal(
      value.future_execution_prerequisites.prerequisites_satisfied,
      false,
    );
    validateColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(
      value,
      fixture.input,
    );
    assert.equal(digest(value), variant.projectionDigest);
    assertRecursivelyFrozen(value);
  }
  assert.notEqual(variants[0].schema, variants[1].schema);
  assert.notEqual(variants[0].projectionDigest, variants[1].projectionDigest);
});

test("every projection field and nested requirement fails closed on drift", () => {
  const fixture = prepared();
  const value = fixture.projection;
  for (const field of Object.keys(value)) {
    const changed = clone(value);
    changed[field] = changedValue(changed[field]);
    expectRefusal(
      () =>
        validateColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(
          changed,
          fixture.input,
        ),
      /effect-generation/u,
    );
  }
  for (const group of [
    "authorities",
    "future_atomic_generation",
    "future_effect_witness",
    "future_execution_prerequisites",
  ]) {
    for (const field of Object.keys(value[group])) {
      const changed = clone(value);
      changed[group][field] = changedValue(changed[group][field]);
      expectRefusal(() =>
        validateColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(
          changed,
          fixture.input,
        ),
      );
    }
    const missing = clone(value);
    delete missing[group][Object.keys(missing[group])[0]];
    expectRefusal(() =>
      validateColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(
        missing,
        fixture.input,
      ),
    );
    const extra = clone(value);
    extra[group].unexpected = true;
    expectRefusal(() =>
      validateColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(
        extra,
        fixture.input,
      ),
    );
  }
  const missing = clone(value);
  delete missing.schema;
  expectRefusal(() =>
    validateColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(
      missing,
      fixture.input,
    ),
  );
  const extra = { ...clone(value), unexpected: true };
  expectRefusal(() =>
    validateColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(
      extra,
      fixture.input,
    ),
  );
});

test("class crossing collision and upstream drift cannot become prerequisites", () => {
  const production = prepared(false);
  const fixture = prepared(true);
  expectRefusal(() =>
    validateColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(
      production.projection,
      fixture.input,
    ),
  );
  expectRefusal(() =>
    validateColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(
      fixture.projection,
      production.input,
    ),
  );

  const changedSource = clone(production.source);
  changedSource.closeAuthority = "recovery";
  expectRefusal(
    () =>
      buildColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure({
        admission: production.admission,
        source: changedSource,
      }),
    /source was refused/u,
  );
  const changedAdmission = clone(production.admission);
  changedAdmission.process_start_effect_candidate.process_start_authorized = true;
  expectRefusal(
    () =>
      buildColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure({
        admission: changedAdmission,
        source: production.source,
      }),
    /source was refused/u,
  );

  const collisionSource =
    cleanEngineLiveProviderProcessStartSourceFixture(false);
  const collisionRoot =
    cleanEngineLiveProviderProcessStartRootObservationFixture(
      collisionSource.source,
      COLIMA_LIVE_MUTATION_SURFACE_ROLES[0],
    );
  const collisionAdmission =
    buildColimaLiveProviderProcessStartEffectFreshAdmissionStructure({
      completedStartDecisionProjection:
        collisionSource.completedStartDecisionProjection,
      rootObservation: collisionRoot,
      source: collisionSource.source,
    });
  assert.equal(collisionAdmission.process_start_effect_candidate, null);
  expectRefusal(
    () =>
      buildColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure({
        admission: collisionAdmission,
        source: collisionSource.source,
      }),
    /prerequisite was refused/u,
  );
  expectRefusal(() =>
    buildColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure({
      admission: production.projection,
      source: production.source,
    }),
  );
  expectRefusal(() =>
    buildColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure({
      ...production.input,
      unexpected: true,
    }),
  );
  for (const input of [
    null,
    [],
    { admission: null, source: production.source },
    { admission: [], source: production.source },
    { admission: production.admission, source: null },
    { admission: production.admission, source: [] },
  ]) {
    expectRefusal(() =>
      buildColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(
        input,
      ),
    );
  }
});

test("hidden inherited symbol and accessor inputs are refused without evaluation", () => {
  const fixture = prepared();
  const cases = [];
  const hidden = clone(fixture.projection);
  Object.defineProperty(hidden, "hidden", {
    enumerable: false,
    value: "unexpected",
  });
  cases.push(hidden);
  const symbol = clone(fixture.projection);
  symbol[Symbol("unexpected")] = true;
  cases.push(symbol);
  cases.push(
    Object.assign(Object.create({ unexpected: true }), clone(fixture.projection)),
  );
  let getterRead = false;
  const accessor = clone(fixture.projection);
  Object.defineProperty(accessor, "schema", {
    enumerable: true,
    get() {
      getterRead = true;
      return fixture.projection.schema;
    },
  });
  cases.push(accessor);
  for (const value of cases) {
    expectRefusal(() =>
      validateColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(
        value,
        fixture.input,
      ),
    );
  }
  assert.equal(getterRead, false);

  let inputGetterRead = false;
  const input = {};
  Object.defineProperty(input, "admission", {
    enumerable: true,
    get() {
      inputGetterRead = true;
      return fixture.admission;
    },
  });
  Object.defineProperty(input, "source", {
    enumerable: true,
    value: fixture.source,
  });
  expectRefusal(() =>
    buildColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(input),
  );
  assert.equal(inputGetterRead, false);
});

test("canonical graphs have exact cycle depth node entry and byte bounds", () => {
  const cyclicRecord = {};
  cyclicRecord.self = cyclicRecord;
  const cyclicArray = [];
  cyclicArray.push(cyclicArray);
  const indirectLeft = {};
  const indirectRight = { left: indirectLeft };
  indirectLeft.right = indirectRight;
  for (const value of [cyclicRecord, cyclicArray, indirectLeft]) {
    expectStructuralRefusal(
      () => liveProviderEffectGenerationBytes(value),
      /canonical cycle was refused/u,
    );
  }

  assert.doesNotThrow(() =>
    liveProviderEffectGenerationBytes(nestedArray(16)),
  );
  expectStructuralRefusal(
    () => liveProviderEffectGenerationBytes(nestedArray(17)),
    /canonical depth was exceeded/u,
  );

  assert.doesNotThrow(() =>
    liveProviderEffectGenerationBytes([binaryTree(8)]),
  );
  expectStructuralRefusal(
    () => liveProviderEffectGenerationBytes([binaryTree(8), "leaf"]),
    /canonical node budget was exceeded/u,
  );

  assert.doesNotThrow(() =>
    liveProviderEffectGenerationBytes(
      Array.from({ length: 64 }, (_, index) => index),
    ),
  );
  const overwideArray = Array.from({ length: 65 }, (_, index) => index);
  assert.doesNotThrow(() =>
    liveProviderEffectGenerationBytes(
      Object.fromEntries(
        Array.from({ length: 64 }, (_, index) => [`field-${index}`, index]),
      ),
    ),
  );
  const overwideRecord = Object.fromEntries(
    Array.from({ length: 65 }, (_, index) => [`field-${index}`, index]),
  );
  const originalDescriptor = Object.getOwnPropertyDescriptor;
  let overwideDescriptorReads = 0;
  try {
    Object.getOwnPropertyDescriptor = (value, key) => {
      if (value === overwideArray || value === overwideRecord) {
        overwideDescriptorReads += 1;
      }
      return originalDescriptor(value, key);
    };
    for (const value of [overwideArray, overwideRecord]) {
      expectStructuralRefusal(
        () => liveProviderEffectGenerationBytes(value),
        /container entry budget was exceeded/u,
      );
    }
  } finally {
    Object.getOwnPropertyDescriptor = originalDescriptor;
  }
  assert.equal(overwideDescriptorReads, 0);

  assert.equal(
    liveProviderEffectGenerationBytes("x".repeat(65_533)).length,
    65_536,
  );
  expectStructuralRefusal(
    () => liveProviderEffectGenerationBytes("x".repeat(65_534)),
    /canonical byte budget was exceeded/u,
  );
  const escapedBoundary = `${"\0".repeat(10_922)}a`;
  assert.equal(
    liveProviderEffectGenerationBytes(escapedBoundary).length,
    65_536,
  );
  expectStructuralRefusal(
    () => liveProviderEffectGenerationBytes(`${escapedBoundary}a`),
    /canonical byte budget was exceeded/u,
  );
  assert.equal(
    liveProviderEffectGenerationBytes({ ["k".repeat(65_526)]: true }).length,
    65_536,
  );
  expectStructuralRefusal(
    () =>
      liveProviderEffectGenerationBytes({ ["k".repeat(65_527)]: true }),
    /canonical byte budget was exceeded/u,
  );
});

test("canonical graphs preserve aliases and reject exotic arrays without access", () => {
  const shared = { value: "same" };
  assert.deepEqual(
    liveProviderEffectGenerationBytes({ left: shared, right: shared }),
    liveProviderEffectGenerationBytes({
      left: { value: "same" },
      right: { value: "same" },
    }),
  );
  const sharedSubtree = binaryTree(7);
  assert.doesNotThrow(() =>
    liveProviderEffectGenerationBytes([sharedSubtree, sharedSubtree]),
  );
  expectStructuralRefusal(
    () =>
      liveProviderEffectGenerationBytes([
        sharedSubtree,
        sharedSubtree,
        "first-extra-occurrence",
        "second-extra-occurrence",
      ]),
    /canonical node budget was exceeded/u,
  );

  let getterRead = false;
  const accessor = [];
  Object.defineProperty(accessor, "0", {
    enumerable: true,
    get() {
      getterRead = true;
      return "unexpected";
    },
  });
  accessor.length = 1;
  const sparse = new Array(1);
  const named = [];
  named.unexpected = true;
  const symbol = [];
  symbol[Symbol("unexpected")] = true;
  const customPrototype = [];
  Object.setPrototypeOf(customPrototype, Object.create(Array.prototype));
  for (const value of [accessor, sparse, named, symbol, customPrototype]) {
    expectStructuralRefusal(
      () => liveProviderEffectGenerationBytes(value),
      /canonical value/u,
    );
  }
  assert.equal(getterRead, false);
});

test("root nested and revoked proxies are refused before any trap or upstream call", () => {
  let trapCalls = 0;
  const handler = Object.fromEntries(
    ["get", "getOwnPropertyDescriptor", "getPrototypeOf", "ownKeys"].map(
      (name) => [
        name,
        () => {
          trapCalls += 1;
          throw new Error(`unexpected proxy trap ${name}`);
        },
      ],
    ),
  );
  const transparent = new Proxy({}, {});
  const throwing = new Proxy({}, handler);
  const nested = { value: new Proxy([], handler) };
  const revocable = Proxy.revocable({}, handler);
  revocable.revoke();
  for (const value of [transparent, throwing, nested, revocable.proxy]) {
    expectStructuralRefusal(
      () => liveProviderEffectGenerationBytes(value),
      /contained a proxy/u,
    );
  }

  const fixture = prepared();
  const rootInput = new Proxy(fixture.input, handler);
  const admission = new Proxy(fixture.admission, handler);
  expectStructuralRefusal(
    () =>
      buildColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(
        rootInput,
      ),
    /contained a proxy/u,
  );
  expectStructuralRefusal(
    () =>
      buildColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure({
        admission,
        source: fixture.source,
      }),
    /contained a proxy/u,
  );
  expectStructuralRefusal(
    () =>
      validateColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(
        fixture.projection,
        { admission, source: fixture.source },
      ),
    /contained a proxy/u,
  );
  assert.equal(trapCalls, 0);
});

test("builder and validator preflight hostile graphs without freezing caller data", () => {
  const upstream =
    cleanEngineLiveProviderProcessStartFreshAdmissionFixture(false);
  const input = {
    admission: clone(upstream.admission),
    source: clone(upstream.source),
  };
  const projection =
    buildColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(
      input,
    );
  for (const value of [input, input.admission, input.source]) {
    assert.equal(Object.isFrozen(value), false);
  }
  assertRecursivelyFrozen(projection);

  const cyclicBuildInput = clone(input);
  cyclicBuildInput.source.unexpected = cyclicBuildInput;
  expectStructuralRefusal(
    () =>
      buildColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(
        cyclicBuildInput,
      ),
    /canonical cycle was refused/u,
  );
  assert.equal(Object.isFrozen(cyclicBuildInput), false);
  assert.equal(Object.isFrozen(cyclicBuildInput.source), false);

  const overwideBuildInput = clone(input);
  overwideBuildInput.source.unexpected = Object.fromEntries(
    Array.from({ length: 65 }, (_, index) => [`field-${index}`, index]),
  );
  expectStructuralRefusal(
    () =>
      buildColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(
        overwideBuildInput,
      ),
    /container entry budget was exceeded/u,
  );
  assert.equal(Object.isFrozen(overwideBuildInput.source), false);
  assert.equal(
    Object.isFrozen(overwideBuildInput.source.unexpected),
    false,
  );

  const cyclicExpected = clone(input);
  cyclicExpected.source.unexpected = cyclicExpected;
  expectStructuralRefusal(
    () =>
      validateColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(
        projection,
        cyclicExpected,
      ),
    /canonical cycle was refused/u,
  );
  assert.equal(Object.isFrozen(cyclicExpected), false);

  const cyclicProjection = clone(projection);
  cyclicProjection.future_execution_prerequisites.unexpected = cyclicProjection;
  expectStructuralRefusal(
    () =>
      validateColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(
        cyclicProjection,
        input,
      ),
    /canonical cycle was refused/u,
  );
  assert.equal(Object.isFrozen(cyclicProjection), false);
});

test("serialized prerequisites contain no private or executable identity", () => {
  for (const fixtureOnly of [false, true]) {
    const raw = liveProviderEffectGenerationBytes(
      prepared(fixtureOnly).projection,
    ).toString("utf8");
    assert.doesNotMatch(
      raw,
      /(?:\/Users\/|\/home\/|binding_key|credential|password|secret|token)/iu,
    );
    assert.doesNotMatch(
      raw,
      /"(?:argv|command|cwd|environment|executable|home|path|pgid|pid|sid|socket|uid)"\s*:/iu,
    );
  }
});

function firstPartyFiles(root) {
  const files = [];
  const pending = [root];
  const skipped = new Set([
    ".git",
    ".next",
    "build",
    "coverage",
    "dist",
    "node_modules",
    "target",
  ]);
  while (pending.length > 0) {
    const current = pending.pop();
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      if (skipped.has(entry.name)) continue;
      const path = join(current, entry.name);
      if (entry.isDirectory()) pending.push(path);
      else if (entry.isFile()) files.push(path);
    }
  }
  return files;
}

function dependencyClosure(entryPath) {
  const files = new Set();
  const builtins = new Set();
  const pending = [entryPath];
  while (pending.length > 0) {
    const path = pending.pop();
    if (files.has(path)) continue;
    files.add(path);
    const source = readFileSync(path, "utf8");
    for (const match of source.matchAll(/from "([^"]+)"/gu)) {
      const specifier = match[1];
      if (specifier.startsWith("node:")) {
        builtins.add(specifier);
      } else if (specifier.startsWith("./")) {
        pending.push(resolve(dirname(path), specifier));
      } else {
        assert.fail(`unexpected dependency ${specifier} from ${path}`);
      }
    }
  }
  return {
    builtins: [...builtins].sort(),
    files: [...files]
      .map((path) => relative(REPO_ROOT, path))
      .sort(),
  };
}

test("the prerequisite module remains outside production authority", () => {
  assert.deepEqual(Object.keys(effectGeneration), [
    "COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_GENERATION_PREREQUISITE_PROJECTION_SCHEMA",
    "COLIMA_LIVE_PROVIDER_EFFECT_GENERATION_PREREQUISITE_PROJECTION_SCHEMA",
    "LiveProviderEffectGenerationFailure",
    "buildColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure",
    "liveProviderEffectGenerationBytes",
    "liveProviderEffectGenerationDigest",
    "validateColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure",
  ]);
  const source = readFileSync(MODULE_PATH, "utf8");
  const imports = [...source.matchAll(/from "([^"]+)"/gu)].map(
    (match) => match[1],
  );
  assert.deepEqual(imports, [
    "node:crypto",
    "node:util/types",
    "./clean-engine-colima-live-schemas.mjs",
    "./clean-engine-live-provider-process-start.mjs",
  ]);
  assert.deepEqual(dependencyClosure(MODULE_PATH), {
    builtins: ["node:crypto", "node:fs", "node:path", "node:util/types"],
    files: [
      "deploy/compose/scripts/clean-engine-colima-live-contract.mjs",
      "deploy/compose/scripts/clean-engine-colima-live-schemas.mjs",
      "deploy/compose/scripts/clean-engine-live-provider-effect-generation.mjs",
      "deploy/compose/scripts/clean-engine-live-provider-intent.mjs",
      "deploy/compose/scripts/clean-engine-live-provider-plan.mjs",
      "deploy/compose/scripts/clean-engine-live-provider-process-start.mjs",
      "deploy/compose/scripts/clean-engine-live-provider-start-decision.mjs",
      "deploy/compose/scripts/clean-engine-provider-adapter-registry.mjs",
    ],
  });
  assert.doesNotMatch(
    source,
    /node:(?:child_process|dgram|fs|http|https|net|tls|worker_threads)/u,
  );
  assert.doesNotMatch(
    source,
    /clean-engine-(?:state|receipts|live-provider-reservation|provider-adapter-registry|provider-process-contract)|controlled-background|\/fixtures\//u,
  );
  assert.doesNotMatch(source, /operation_(?:contract|kind)/u);
  assert.doesNotMatch(
    source,
    /export function (?:authorize|cleanup|execute|finalize|launch|publish|recover|settle|signal|spawn)/u,
  );
  assert.doesNotMatch(
    source,
    /\b(?:exec|fork|kill|link|mkdir|rename|rmdir|spawn|unlink|writeFile)[A-Za-z0-9_]*\s*\(/u,
  );

  const excluded = new Set([MODULE_PATH, TEST_PATH]);
  const forbiddenReference =
    /clean-engine-live-provider-effect-generation\.mjs/u;
  const roots = [
    "adapters",
    "console",
    "crates",
    "demos",
    "deploy",
    "policies",
    "scripts",
  ];
  const authorityPaths = [
    ...roots.flatMap((root) => firstPartyFiles(resolve(REPO_ROOT, root))),
    ...[
      ".dockerignore",
      "Cargo.lock",
      "Cargo.toml",
      "deny.toml",
      "package.json",
      "pnpm-lock.yaml",
      "pnpm-workspace.yaml",
      "rust-toolchain.toml",
      "tsconfig.base.json",
    ].map((path) => resolve(REPO_ROOT, path)),
  ].filter((path) => !excluded.has(path));
  assert.ok(authorityPaths.length > 100, "authority scan was unexpectedly small");
  for (const path of authorityPaths) {
    const bytes = readFileSync(path);
    if (bytes.includes(0)) continue;
    let consumer;
    try {
      consumer = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    } catch {
      continue;
    }
    assert.doesNotMatch(
      consumer,
      forbiddenReference,
      relative(REPO_ROOT, path),
    );
  }

  const state = readFileSync(
    resolve(REPO_ROOT, "deploy/compose/scripts/clean-engine-state.mjs"),
    "utf8",
  );
  const receipts = readFileSync(
    resolve(REPO_ROOT, "deploy/compose/scripts/clean-engine-receipts.mjs"),
    "utf8",
  );
  const reservation = readFileSync(
    resolve(
      REPO_ROOT,
      "deploy/compose/scripts/clean-engine-live-provider-reservation.mjs",
    ),
    "utf8",
  );
  const registry = readFileSync(
    resolve(
      REPO_ROOT,
      "deploy/compose/scripts/clean-engine-provider-adapter-registry.mjs",
    ),
    "utf8",
  );
  const lifecycle = readFileSync(
    resolve(REPO_ROOT, "deploy/compose/scripts/clean-engine-acceptance.sh"),
    "utf8",
  );
  assert.match(
    receipts,
    /RECEIPT_SCHEMA = "synveda\.clean-engine\.receipt\.v5"/u,
  );
  assert.match(
    state,
    /MUTATION_SLOT_SCHEMA = "synveda\.clean-engine\.mutation-slot\.v6"/u,
  );
  assert.match(
    state,
    /MUTATION_CLOSE_SCHEMA = "synveda\.clean-engine\.mutation-close\.v7"/u,
  );
  assert.match(
    state,
    /MUTATION_RECOVERY_SCHEMA = "synveda\.clean-engine\.mutation-recovery\.v5"/u,
  );
  assert.match(
    state,
    /schema: "synveda\.clean-engine\.mutation-recovery-root\.v5"/u,
  );
  assert.match(
    state,
    /completedLiveProviderReservations\[0\] !==\s*inventory\.mutationSlots\.at\(-1\)/u,
  );
  assert.match(
    reservation,
    /"mutation-journal-v6-no-spawn-reservation-only"/u,
  );
  assert.match(lifecycle, /plan\|status\|verify/u);
  for (const consumer of [state, receipts, reservation, registry, lifecycle]) {
    assert.doesNotMatch(
      consumer,
      /clean-engine-live-provider-effect-generation/u,
    );
  }
});
