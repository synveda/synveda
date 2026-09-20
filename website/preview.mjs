import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { resolve, extname, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { siteUrl } from "./config.mjs";

const root = fileURLToPath(new URL("./dist/", import.meta.url));
const base = siteUrl().pathname;
const mime = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".woff2": "font/woff2",
  ".txt": "text/plain; charset=utf-8",
  ".md": "text/plain; charset=utf-8",
};
const port = Number(process.env.PORT || 4173);
createServer(async (request, response) => {
  try {
    const path = decodeURIComponent(
      new URL(request.url, "http://localhost").pathname,
    );
    if (path === base.slice(0, -1)) {
      response.writeHead(302, { Location: base }).end();
      return;
    }
    if (!path.startsWith(base)) throw new Error("Not found");
    const relative = path.slice(base.length) || "index.html";
    const file = resolve(root, relative);
    if (!file.startsWith(root.endsWith(sep) ? root : root + sep))
      throw new Error("Not found");
    const content = await readFile(file);
    response.writeHead(200, {
      "Content-Type": mime[extname(file)] || "application/octet-stream",
    });
    response.end(content);
  } catch {
    response.writeHead(404, { "Content-Type": "text/plain" }).end("Not found");
  }
}).listen(port, "127.0.0.1", () =>
  console.log(`Preview: http://127.0.0.1:${port}${base}`),
);
