import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { publicFiles, repository, siteUrl } from "./config.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const output = resolve(root, "website/dist");
const canonical = siteUrl().href;
const installation = JSON.parse(await readFile(resolve(root, "docs/installation.json"), "utf8"));
if (installation.pagesUrl !== canonical && !process.env.SITE_URL) throw new Error("Pages URL drift");
const releaseStatus = installation.publication === "published"
  ? `Release ${installation.sourceVersion}.`
  : `Candidate ${installation.sourceVersion} — awaiting qualification and publication.`;
await rm(output, { recursive: true, force: true });
for (const [source, target] of publicFiles) {
  let content = await readFile(resolve(root, source));
  if (target === "index.html") {
    content = content
      .toString()
      .replaceAll("{{SITE_URL}}", canonical)
      .replaceAll("{{REPOSITORY}}", repository)
      .replaceAll("{{RELEASE_STATUS}}", releaseStatus)
      .replaceAll("{{DOCKER_COMMAND}}", installation.dockerCommand)
      .replaceAll("{{SAMPLE_COMMAND}}", installation.sampleCommand);
    if (/\{\{.*?\}\}/.test(content))
      throw new Error("Unresolved HTML build token");
  }
  const destination = resolve(output, target);
  await mkdir(dirname(destination), { recursive: true });
  await writeFile(destination, content);
}
await writeFile(resolve(output, ".nojekyll"), "");
console.log(`Built ${publicFiles.length + 1} public files for ${canonical}`);
