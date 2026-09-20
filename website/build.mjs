import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { publicFiles, repository, siteUrl } from "./config.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const output = resolve(root, "website/dist");
const canonical = siteUrl().href;
await rm(output, { recursive: true, force: true });
for (const [source, target] of publicFiles) {
  let content = await readFile(resolve(root, source));
  if (target === "index.html") {
    content = content
      .toString()
      .replaceAll("{{SITE_URL}}", canonical)
      .replaceAll("{{REPOSITORY}}", repository);
    if (/\{\{.*?\}\}/.test(content))
      throw new Error("Unresolved HTML build token");
  }
  const destination = resolve(output, target);
  await mkdir(dirname(destination), { recursive: true });
  await writeFile(destination, content);
}
await writeFile(resolve(output, ".nojekyll"), "");
console.log(`Built ${publicFiles.length + 1} public files for ${canonical}`);
