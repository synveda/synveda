// Focused publication checks; reuse the repository's Markdown anchor rules.
import assert from "node:assert/strict";
import { readFile, readdir, stat } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { anchorsForMarkdown } from "../scripts/check-docs.mjs";
import { publicFiles, repository, siteUrl } from "./config.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const dist = resolve(root, "website/dist");
const html = await readFile(resolve(dist, "index.html"), "utf8");
const css = await readFile(resolve(dist, "styles.css"), "utf8");
const base = siteUrl();
const ids = [...html.matchAll(/\bid="([^"]+)"/g)].map((m) => m[1]);
assert.equal(ids.length, new Set(ids).size, "Duplicate HTML id");
assert.equal((html.match(/<h1\b/g) || []).length, 1, "Exactly one h1");
assert.match(html, /<html lang="en">/);
assert.match(html, /class="skip-link" href="#main"/);
assert.ok(
  !/<script\b|<iframe\b|\son\w+=|\{\{/.test(html),
  "No scripts, embeds or unresolved tokens",
);
assert.ok(!/https?:\/\//.test(css), "CSS must use local assets");

const localTargets = new Set();
for (const raw of [
  ...html.matchAll(/\b(?:href|src)="([^"]+)"/g),
  ...css.matchAll(/url\("([^"\)]+)"\)/g),
].map((m) => m[1])) {
  assert.ok(!raw.startsWith("/"), `Root-relative URL: ${raw}`);
  if (raw.startsWith("#")) {
    assert.ok(
      raw === "#" || ids.includes(raw.slice(1)),
      `Missing section ${raw}`,
    );
  } else if (raw.startsWith(repository)) {
    const url = new URL(raw);
    let path = url.pathname
      .slice("/synveda/synveda".length)
      .replace(/^\/(?:blob|tree)\/main\//, "");
    if (!path || path === "/") path = "README.md";
    assert.ok(!path.startsWith("/"), `Unrecognised repository path: ${raw}`);
    const target = resolve(root, decodeURIComponent(path));
    await stat(target);
    if (url.hash) {
      const anchors = anchorsForMarkdown(await readFile(target, "utf8"));
      assert.ok(
        anchors.has(decodeURIComponent(url.hash.slice(1))),
        `Missing repository anchor: ${raw}`,
      );
    }
  } else if (raw.startsWith("https://")) {
    assert.equal(
      raw,
      base.href,
      `Unexpected external asset or destination: ${raw}`,
    );
  } else {
    const url = new URL(raw, base);
    assert.ok(
      url.pathname.startsWith(base.pathname),
      `Escaped Pages base: ${raw}`,
    );
    localTargets.add(
      decodeURIComponent(url.pathname.slice(base.pathname.length)),
    );
  }
}
for (const target of localTargets) await stat(resolve(dist, target));
for (const tag of html.matchAll(/<img\b[^>]*>/g)) {
  assert.match(tag[0], /\balt="[^"]*"/);
  assert.match(tag[0], /\bwidth="\d+"/);
  assert.match(tag[0], /\bheight="\d+"/);
}
for (const tag of html.matchAll(
  /<meta\b[^>]*(?:og:url|og:image"|twitter:image")[^>]*>/g,
)) {
  const value = tag[0].match(/content="([^"]+)"/)[1];
  assert.ok(
    value === base.href ||
      value === new URL("assets/social-preview.png", base).href,
    `Wrong social URL: ${value}`,
  );
}

const registry = JSON.parse(
  await readFile(resolve(root, "adapters/registry.json"), "utf8"),
);
const supportRows = [
  ...html.matchAll(
    /data-client="([^"]+)"\s+data-level="([^"]+)"\s+data-version="([^"]+)"/g,
  ),
];
assert.equal(supportRows.length, 3, "Expected the three named client claims");
for (const [, id, level, version] of supportRows) {
  const client = registry.clients.find((item) => item.id === id);
  assert.equal(client?.support_level, level, `Support label drift: ${id}`);
  assert.ok(
    client.tested_versions.includes(version),
    `Untested client version: ${id} ${version}`,
  );
}

const files = await readdir(dist, { recursive: true, withFileTypes: true });
const actual = files
  .filter((file) => !file.isDirectory())
  .map((file) => {
    assert.ok(
      file.isFile(),
      "Public output cannot contain links or special files",
    );
    return resolve(file.parentPath, file.name).slice(dist.length + 1);
  })
  .sort();
assert.deepEqual(
  actual,
  [...publicFiles.map(([, target]) => target), ".nojekyll"].sort(),
);
let bytes = 0;
for (const file of actual) bytes += (await stat(resolve(dist, file))).size;
assert.ok(bytes < 750_000, `Public output exceeds 750 kB: ${bytes}`);

for (const [name, width, height] of [
  ["synveda-avatar-512.png", 512, 512],
  ["synveda-avatar-256.png", 256, 256],
  ["favicon-32.png", 32, 32],
  ["social-preview.png", 1280, 640],
]) {
  const png = await readFile(resolve(root, "assets/brand", name));
  assert.equal(png.subarray(1, 4).toString(), "PNG");
  assert.equal(png.readUInt32BE(16), width);
  assert.equal(png.readUInt32BE(20), height);
  assert.ok(
    png.length < 1_000_000,
    `${name} must be under GitHub's 1 MB limit`,
  );
}
for (const name of (await readdir(resolve(root, "assets/brand"))).filter(
  (name) => name.endsWith(".svg"),
)) {
  const svg = await readFile(resolve(root, "assets/brand", name), "utf8");
  assert.ok(
    !/<(?:image|text|script)\b|data:image|@font-face/.test(svg),
    `SVG must be self-contained vector paths: ${name}`,
  );
}
console.log(
  `Checked Pages links, fragments, claims, metadata, image dimensions and public allowlist: ${actual.length} files, ${bytes} bytes, base ${base.pathname}`,
);
