// OPS-12 / ADR-0134: validate independently of the SBOM plugin's package discovery.
import assert from "node:assert/strict";
import { existsSync, readdirSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, isAbsolute, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import {
  checkConsoleFiles, checkConsoleSbom, consoleNotices, consolePolicy, helperNoticeName,
  inputBindings, inventoryName, maxFiles, maxFileBytes, noticesName, packageLicence,
  relativePath, sbomName, sha256, sourceBytes,
} from "./dependency-contract.mjs";

const root = fileURLToPath(new URL("../../", import.meta.url));
const require = createRequire(join(root, "console/package.json"));

function packageFor(path) {
  assert.ok(isAbsolute(path), "rendered package path must be absolute");
  let directory = dirname(path);
  for (let depth = 0; depth < 16; depth++) {
    if (existsSync(join(directory, "package.json"))) {
      const manifest = JSON.parse(sourceBytes(directory, "package.json"));
      assert.ok(typeof manifest.name === "string" && typeof manifest.version === "string");
      return { directory, manifest };
    }
    const parent = dirname(directory);
    if (parent === directory) break;
    directory = parent;
  }
  assert.fail("rendered module has no bounded package identity");
}

export function renderedConsolePackages(bundle) {
  const packages = new Map(), helpers = new Set();
  assert.ok(Object.keys(bundle).length <= maxFiles);
  for (const chunk of Object.values(bundle)) {
    if (chunk.type !== "chunk") continue;
    assert.ok(Buffer.byteLength(chunk.code) <= maxFileBytes && Object.keys(chunk.modules).length <= 1024);
    for (const [id, module] of Object.entries(chunk.modules)) {
      if (module.renderedLength === 0) continue;
      assert.ok(Number.isSafeInteger(module.renderedLength) && module.renderedLength > 0 && id.length <= 4096);
      let physical = id;
      if (id === "\0vite/modulepreload-polyfill.js") { helpers.add("modulepreload-polyfill"); continue; }
      if (id === "\0commonjsHelpers.js") { helpers.add("commonjs"); continue; }
      if (id.startsWith("\0")) {
        const wrapper = /^\0(.+)\?commonjs-(?:module|exports|es-import|proxy)$/.exec(id);
        assert.ok(wrapper && isAbsolute(wrapper[1]) && wrapper[1].includes("/node_modules/"), "unreviewed rendered virtual console module");
        physical = wrapper[1]; helpers.add("commonjs");
      }
      if (!physical.includes("/node_modules/")) {
        assert.ok(!physical.includes("?"), "foreign rendered console source");
        relativePath(relative(join(root, "console"), physical).split(sep).join("/"));
        continue;
      }
      const { directory, manifest } = packageFor(physical);
      const key = `${manifest.name}@${manifest.version}`;
      if (!packages.has(key)) packages.set(key, { directory, manifest, modules: new Set() });
      assert.equal(packages.get(key).directory, directory, "duplicate rendered package location");
      packages.get(key).modules.add(relativePath(relative(directory, physical).split(sep).join("/")));
    }
  }
  assert.deepEqual([...helpers].sort(), ["commonjs", "modulepreload-polyfill"], "generated console helper inventory differs");
  return [...packages.values()].sort((a, b) => a.manifest.name.localeCompare(b.manifest.name));
}

export function consoleInventoryPlugin() {
  return {
    name: "synveda-console-dependency-inventory",
    apply: "build",
    generateBundle: {
      // Vite's HTML/CSS and licence emitters must finish before the closed file inventory.
      order: "post",
      handler(_options, bundle) {
        const policy = consolePolicy(root);
        const version = JSON.parse(sourceBytes(root, "console/package.json")).version;
        const document = JSON.parse(bundle[sbomName]?.source ?? "null");
        const observed = renderedConsolePackages(bundle);
        assert.deepEqual(observed.map(({ manifest: { name, version } }) => ({ name, version })), policy.packages,
          "independently rendered console packages differ from the reviewed lockfile inventory");
        checkConsoleSbom(document, policy, version);
        for (const { directory, manifest } of observed) {
          assert.equal(manifest.license, policy.runtime_license);
          assert.deepEqual(readdirSync(directory).filter(name => /^(?:license|copying|notice)(?:\.|$)/i.test(name)).sort(), ["LICENSE"],
            "rendered package has unreviewed notice files");
          const component = document.components.find(entry => entry.name === manifest.name);
          assert.deepEqual(packageLicence(component, policy), sourceBytes(directory, "LICENSE"), "SBOM licence differs from installed rendered package");
        }
        const vite = packageFor(require.resolve("vite"));
        assert.equal(vite.manifest.version, policy.vite_version);
        const helperNotice = sourceBytes(vite.directory, "LICENSE.md");
        const fontNotice = sourceBytes(root, "assets/brand/fonts/OFL.txt");
        const emit = (fileName, source) => {
          assert.ok(!Object.hasOwn(bundle, fileName), "duplicate console metadata output");
          this.emitFile({ type: "asset", fileName, source });
        };
        emit(helperNoticeName, helperNotice);
        emit(noticesName, consoleNotices(document, policy, helperNotice, fontNotice));
        const files = new Map(Object.entries(bundle).map(([path, item]) => [relativePath(path),
          Buffer.from(item.type === "chunk" ? item.code : item.source)]));
        // Rollup adds emitted assets to the bundle synchronously during this hook.
        assert.ok(files.has(helperNoticeName) && files.has(noticesName));
        const inventory = {
          schema_version: 1, evidence: "rendered-console-dependency-inventory", version,
          source_sha: process.env.SYNVEDA_BUILD_SOURCE_SHA || null,
          inputs: inputBindings(root),
          runtime_packages: observed.map(({ manifest: { name, version }, modules }) => ({ name, version, modules: [...modules].sort() })),
          generated_helpers: { provider: "vite", version: policy.vite_version,
            kinds: ["commonjs", "modulepreload-polyfill"], notice_sha256: sha256(helperNotice) },
          files: [...files].map(([path, bytes]) => ({ path, bytes: bytes.length, sha256: sha256(bytes) })).sort((a, b) => a.path.localeCompare(b.path)),
        };
        const bytes = Buffer.from(JSON.stringify(inventory, null, 2) + "\n");
        files.set(inventoryName, bytes);
        checkConsoleFiles(files, root, version, inventory.source_sha, false);
        emit(inventoryName, bytes);
      },
    },
  };
}
