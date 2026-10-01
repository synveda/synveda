import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { checkConsoleFiles, consoleNotices, consolePolicy, helperNoticeName, inputBindings, inputPaths,
  inventoryName, noticesName, sbomName, sha256 } from "../../console/build/dependency-contract.mjs";

const repository = fileURLToPath(new URL("../../", import.meta.url));
export function consolePackageFixture(t, { source = "a".repeat(40), version = "0.4.3" } = {}) {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-console-inventory-"));
  t.after(() => rmSync(scratch, { recursive: true, force: true }));
  const root = join(scratch, "source"), stage = join(scratch, "stage"), output = join(stage, "console");
  const helperNotice = Buffer.from("Complete generated-helper notice fixture.\n");
  for (const path of [...inputPaths, "assets/brand/fonts/InterVariable.woff2", "assets/brand/fonts/OFL.txt"]) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    copyFileSync(join(repository, path), join(root, path));
  }
  const policyPath = join(root, "console/dependency-policy.json"), policy = JSON.parse(readFileSync(policyPath));
  policy.helper_notice_sha256 = sha256(helperNotice);
  writeFileSync(policyPath, JSON.stringify(policy));
  const expected = consolePolicy(root), licence = readFileSync(new URL("./console-runtime-LICENSE", import.meta.url));
  const rootRef = `pkg:npm/%40synveda/console@${version}`;
  const components = expected.packages.map(({ name, version }) => ({
    name, version, type: "library", purl: `pkg:npm/${name}@${version}`, "bom-ref": `pkg:npm/${name}@${version}`,
    licenses: [{ license: { id: "MIT", acknowledgement: "declared" } }],
    evidence: { licenses: [{ license: { name: "file: LICENSE", text: { content: licence.toString("base64"), contentType: "text/plain", encoding: "base64" } } }] },
  }));
  const ref = name => components.find(c => c.name === name).purl;
  const document = {
    bomFormat: "CycloneDX", specVersion: "1.6", version: 1,
    metadata: { component: { name: "console", group: "@synveda", version, type: "application", purl: rootRef, "bom-ref": rootRef },
      tools: [{ name: "rollup-plugin-sbom", version: "4.0.0" }, { name: "vite", version: expected.vite_version }, { name: "rollup", version: expected.rollup_version }] },
    components, dependencies: [
      { ref: rootRef, dependsOn: [ref("react"), ref("react-dom")] }, { ref: ref("react") },
      { ref: ref("react-dom"), dependsOn: [ref("react"), ref("scheduler")] }, { ref: ref("scheduler") },
    ],
  };
  const files = new Map([
    ["index.html", Buffer.from('<script src="assets/index.js"></script>')],
    ["assets/index.js", Buffer.from("/* synthetic packaged console */")],
    ["assets/InterVariable.woff2", readFileSync(join(root, "assets/brand/fonts/InterVariable.woff2"))],
    ["assets/Inter-OFL.txt", readFileSync(join(root, "assets/brand/fonts/OFL.txt"))],
    [helperNoticeName, helperNotice], [sbomName, Buffer.from(JSON.stringify(document))],
    [noticesName, consoleNotices(document, expected, helperNotice, readFileSync(join(root, "assets/brand/fonts/OFL.txt")))],
  ]);
  const inventory = {
    schema_version: 1, evidence: "rendered-console-dependency-inventory", version, source_sha: source,
    inputs: inputBindings(root), runtime_packages: expected.packages.map(p => ({ ...p, modules: ["index.js"] })),
    generated_helpers: { provider: "vite", version: expected.vite_version, kinds: ["commonjs", "modulepreload-polyfill"], notice_sha256: expected.helper_notice_sha256 },
  };
  function seal() {
    inventory.files = [...files].filter(([path]) => path !== inventoryName).map(([path, bytes]) =>
      ({ path, bytes: bytes.length, sha256: sha256(bytes) })).sort((a, b) => a.path.localeCompare(b.path));
    files.set(inventoryName, Buffer.from(JSON.stringify(inventory)));
  }
  seal();
  const archive = join(scratch, "console.tar.gz");
  function pack(extra = [], image = false) {
    rmSync(stage, { recursive: true, force: true });
    for (const [path, bytes] of files) { mkdirSync(dirname(join(output, path)), { recursive: true }); writeFileSync(join(output, path), bytes); }
    if (!image) for (const name of ["LICENSE", "NOTICE"]) copyFileSync(join(repository, name), join(output, name));
    execFileSync("tar", [image ? "-cf" : "-czf", archive, "-C", stage, "console", ...extra]);
  }
  return { root, stage, output, files, inventory, document, archive, seal, pack,
    imageTar: () => { pack([], true); return readFileSync(archive); },
    check: () => checkConsoleFiles(files, root, version, source) };
}
