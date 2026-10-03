// OPS-12 / ADR-0134: one contract for build output and archived console bytes.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { lstatSync, readFileSync } from "node:fs";
import { join } from "node:path";

export const inventoryName = "dependency-inventory.json";
export const sbomName = "sbom.cdx.json";
export const noticesName = "THIRD-PARTY-NOTICES.txt";
export const helperNoticeName = "licenses/vite.txt";
export const inputPaths = [
  "console/package.json", "pnpm-lock.yaml", "console/vite.config.ts",
  "console/dependency-policy.json", "console/build/dependency-contract.mjs",
  "console/build/dependency-plugin.mjs",
];
export const maxFileBytes = 8 * 1024 * 1024;
export const maxFiles = 256;
const maxMetadata = 1024 * 1024;
const hashPattern = /^[0-9a-f]{64}$/;
const sourcePattern = /^[0-9a-f]{40}$/;

export function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

export function relativePath(path) {
  assert.ok(typeof path === "string" && path.length <= 256 && /^[a-zA-Z0-9_./-]+$/.test(path) &&
    !path.startsWith("/") && path.split("/").every(part => part && part !== "." && part !== ".."),
  "console path must be a bounded safe relative file");
  return path;
}

export function sourceBytes(root, path, limit = maxMetadata) {
  relativePath(path);
  const full = join(root, path), stat = lstatSync(full);
  assert.ok(stat.isFile() && !stat.isSymbolicLink() && stat.size > 0 && stat.size <= limit,
    "console source input must be a bounded regular file");
  const bytes = readFileSync(full);
  assert.ok(bytes.length <= limit, "console source input grew beyond its bound");
  return bytes;
}

export function inputBindings(root) {
  return inputPaths.map(path => {
    const bytes = sourceBytes(root, path);
    return { path, bytes: bytes.length, sha256: sha256(bytes) };
  });
}

export function consolePolicy(root) {
  const policy = JSON.parse(sourceBytes(root, "console/dependency-policy.json"));
  assert.equal(policy.schema_version, 1);
  const lock = sourceBytes(root, "pnpm-lock.yaml").toString("utf8").replaceAll("\r\n", "\n");
  const packages = lock.split("\npackages:\n")[1]?.split("\nsnapshots:\n")[0];
  assert.ok(packages, "expected pnpm package inventory");
  const version = name => {
    assert.match(name, /^[a-z-]+$/);
    const matches = [...packages.matchAll(new RegExp(`^  ${name}@([0-9]+\\.[0-9]+\\.[0-9]+):$`, "gm"))];
    assert.equal(matches.length, 1, `expected one locked ${name} identity`);
    return matches[0][1];
  };
  assert.deepEqual(policy.runtime_packages, ["react", "react-dom", "scheduler"]);
  assert.equal(policy.runtime_license, "MIT");
  assert.equal(policy.helper_provider, "vite");
  assert.match(policy.runtime_license_sha256, hashPattern);
  assert.match(policy.helper_notice_sha256, hashPattern);
  return { ...policy, packages: policy.runtime_packages.map(name => ({ name, version: version(name) })),
    vite_version: version("vite"), rollup_version: version("rollup") };
}

export function packageLicence(component, policy) {
  assert.deepEqual(component.licenses, [{ license: { id: policy.runtime_license, acknowledgement: "declared" } }],
    "console runtime licence declaration differs");
  const evidence = component.evidence?.licenses;
  assert.ok(Array.isArray(evidence) && evidence.length === 1, "expected complete single-file package licence evidence");
  const { name, text } = evidence[0].license ?? {};
  assert.equal(name, "file: LICENSE");
  assert.equal(text?.encoding, "base64");
  assert.equal(text?.contentType, "text/plain");
  assert.ok(typeof text.content === "string" && text.content.length <= 128 * 1024);
  const bytes = Buffer.from(text.content, "base64");
  assert.equal(bytes.toString("base64"), text.content, "noncanonical licence encoding");
  assert.equal(sha256(bytes), policy.runtime_license_sha256, "console package licence is incomplete or changed");
  return bytes;
}

export function checkConsoleSbom(document, policy, version) {
  assert.equal(document.bomFormat, "CycloneDX");
  assert.equal(document.specVersion, "1.6");
  assert.equal(document.version, 1);
  assert.equal(document.metadata?.component?.name, "console");
  assert.equal(document.metadata.component.group, "@synveda");
  assert.equal(document.metadata.component.version, version);
  assert.equal(document.metadata.component.type, "application");
  assert.equal(document.metadata.component.purl, `pkg:npm/%40synveda/console@${version}`);
  assert.equal(document.metadata.component["bom-ref"], document.metadata.component.purl);
  assert.equal(document.metadata.timestamp, undefined);
  assert.equal(document.serialNumber, undefined);
  assert.ok(Array.isArray(document.components) && document.components.length === policy.packages.length,
    "console SBOM package inventory differs");
  const identities = document.components.map(({ name, version }) => ({ name, version })).sort((a, b) => a.name.localeCompare(b.name));
  assert.deepEqual(identities, policy.packages, "console SBOM is missing or misversions rendered packages");
  const refs = new Set([document.metadata.component["bom-ref"]]);
  for (const component of document.components) {
    assert.equal(component.type, "library");
    assert.ok(typeof component.purl === "string" &&
      (component.purl === `pkg:npm/${component.name}@${component.version}` ||
        component.purl.startsWith(`pkg:npm/${component.name}@${component.version}?vcs_url=`)), "console package URL differs");
    assert.equal(component["bom-ref"], component.purl);
    assert.ok(!refs.has(component.purl), "duplicate console component");
    refs.add(component.purl);
    packageLicence(component, policy);
  }
  assert.ok(Array.isArray(document.dependencies) && document.dependencies.length === refs.size);
  assert.deepEqual([...document.dependencies.map(entry => entry.ref)].sort(), [...refs].sort(), "console dependency graph differs");
  for (const edge of document.dependencies) {
    assert.ok(!edge.dependsOn || (Array.isArray(edge.dependsOn) && edge.dependsOn.length <= refs.size));
    assert.ok((edge.dependsOn ?? []).every(ref => refs.has(ref)), "foreign console dependency edge");
    assert.equal(new Set(edge.dependsOn ?? []).size, (edge.dependsOn ?? []).length);
  }
  const ref = name => document.components.find(component => component.name === name).purl;
  const expectedEdges = [
    { ref: document.metadata.component.purl, dependsOn: [ref("react"), ref("react-dom")].sort() },
    { ref: ref("react"), dependsOn: [] },
    { ref: ref("react-dom"), dependsOn: [ref("react"), ref("scheduler")].sort() },
    { ref: ref("scheduler"), dependsOn: [] },
  ].sort((a, b) => a.ref.localeCompare(b.ref));
  assert.deepEqual(document.dependencies.map(edge => ({ ref: edge.ref, dependsOn: [...(edge.dependsOn ?? [])].sort() }))
    .sort((a, b) => a.ref.localeCompare(b.ref)), expectedEdges, "console runtime dependency relationships differ");
  for (const [name, expected] of [["rollup-plugin-sbom", "4.0.0"], ["vite", policy.vite_version], ["rollup", policy.rollup_version]]) {
    const tools = document.metadata.tools?.filter(tool => tool.name === name) ?? [];
    assert.equal(tools.length, 1, `expected one ${name} build tool`);
    assert.equal(tools[0].version, expected);
  }
  return identities;
}

export function consoleNotices(document, policy, helperNotice, fontNotice) {
  assert.equal(sha256(helperNotice), policy.helper_notice_sha256, "generated helper notice changed");
  const runtime = packageLicence(document.components[0], policy);
  return Buffer.concat([
    Buffer.from(`Synveda console third-party notices\n\nRuntime packages (MIT):\n${policy.packages.map(p => `  ${p.name}@${p.version}`).join("\n")}\n\n`),
    runtime,
    Buffer.from(`\nGenerated helpers: Vite ${policy.vite_version} module-preload polyfill and CommonJS wrappers.\nThe following complete upstream distribution notice includes their attribution.\n\n`),
    helperNotice, Buffer.from("\nInter variable font (OFL-1.1):\n\n"), fontNotice,
  ]);
}

export function checkConsoleFiles(files, root, version, source, requireSource = true) {
  assert.ok(files instanceof Map && files.size > 5 && files.size <= maxFiles);
  let total = 0;
  for (const [path, bytes] of files) {
    relativePath(path);
    assert.ok(Buffer.isBuffer(bytes) && bytes.length > 0 && bytes.length <= maxFileBytes, "console file size differs");
    total += bytes.length;
  }
  assert.ok(total <= 32 * 1024 * 1024, "console output exceeds its aggregate bound");
  const read = name => { assert.ok(files.has(name), `missing console ${name}`); return files.get(name); };
  assert.ok(read(inventoryName).length <= maxMetadata && read(sbomName).length <= maxMetadata);
  const inventory = JSON.parse(read(inventoryName));
  assert.equal(inventory.schema_version, 1);
  assert.equal(inventory.evidence, "rendered-console-dependency-inventory");
  assert.equal(inventory.version, version);
  if (requireSource) {
    assert.match(source ?? "", sourcePattern);
    assert.equal(inventory.source_sha, source, "console build source differs or is unqualified");
  } else if (inventory.source_sha !== null) assert.match(inventory.source_sha, sourcePattern);
  assert.deepEqual(inventory.inputs, inputBindings(root), "console build input bindings differ from source");
  const actual = [...files].filter(([path]) => path !== inventoryName).map(([path, bytes]) =>
    ({ path, bytes: bytes.length, sha256: sha256(bytes) })).sort((a, b) => a.path.localeCompare(b.path));
  assert.deepEqual(inventory.files, actual, "console output inventory differs from actual bytes");
  read("index.html");
  const javascript = actual.filter(item => item.path.endsWith(".js"));
  assert.ok(javascript.length > 0 && javascript.length <= 16, "console must contain bounded JavaScript chunks");
  const policy = consolePolicy(root), document = JSON.parse(read(sbomName));
  const identities = checkConsoleSbom(document, policy, version);
  assert.deepEqual(inventory.runtime_packages.map(({ name, version }) => ({ name, version })), identities);
  for (const item of inventory.runtime_packages) {
    assert.ok(Array.isArray(item.modules) && item.modules.length > 0 && item.modules.length <= 128);
    assert.deepEqual(item.modules, [...new Set(item.modules)].sort());
    item.modules.forEach(relativePath);
  }
  assert.deepEqual(inventory.generated_helpers, {
    provider: "vite", version: policy.vite_version,
    kinds: ["commonjs", "modulepreload-polyfill"], notice_sha256: policy.helper_notice_sha256,
  });
  assert.deepEqual(read("assets/Inter-OFL.txt"), sourceBytes(root, "assets/brand/fonts/OFL.txt"));
  const fonts = actual.filter(item => item.path.endsWith(".woff2"));
  assert.equal(fonts.length, 1, "expected one console font");
  assert.equal(fonts[0].sha256, sha256(sourceBytes(root, "assets/brand/fonts/InterVariable.woff2", maxFileBytes)), "console font differs");
  assert.deepEqual(read(noticesName), consoleNotices(document, policy, read(helperNoticeName), read("assets/Inter-OFL.txt")),
    "console third-party notice carriage differs");
  return { packages: identities, files: actual.length + 1, inventory_sha256: sha256(read(inventoryName)),
    sbom_sha256: sha256(read(sbomName)), notices_sha256: sha256(read(noticesName)) };
}
