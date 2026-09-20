// FND-7: all production variants derive from the editable mark and licensed font.
import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { create } from "fontkit";
import { rasterize } from "./rasterize.mjs";

const brand = new URL("../assets/brand/", import.meta.url);
const check = process.argv.includes("--check");
const master = await readFile(new URL("synveda-mark.svg", brand), "utf8");
const mark = master.match(/<g id="mark">([\s\S]*?)<\/g>/)?.[1];
if (!mark) throw new Error("Canonical mark must contain the mark group");
const font = create(await readFile(new URL("fonts/InterVariable.ttf", brand)));
const navy = "#0B1F3B";
const darkMark = mark.replaceAll(navy, "#173963");
const monoMark = mark.replaceAll(/#[0-9A-F]{6}/g, navy);
const svg = (width, height, body, title) =>
  `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" role="img" aria-labelledby="title"><title id="title">${title}</title>${body}</svg>\n`;

function textPath(
  text,
  x,
  y,
  size,
  weight = 600,
  fill = navy,
  tracking = 0,
  features = [],
) {
  const instance = font.getVariation({ wght: weight, opsz: 32 });
  const run = instance.layout(text, features);
  const scale = size / font.unitsPerEm;
  let cursor = 0;
  return run.glyphs
    .map((glyph, i) => {
      const pos = run.positions[i];
      const path = `<path transform="translate(${(x + cursor + pos.xOffset * scale).toFixed(3)} ${(y - pos.yOffset * scale).toFixed(3)}) scale(${scale} ${-scale})" d="${glyph.path.toSVG()}" fill="${fill}"/>`;
      cursor += pos.xAdvance * scale + tracking;
      return path;
    })
    .join("");
}

function wordmark(fill = navy) {
  // Inter's single-storey a follows the approved lowercase wordmark.
  return textPath("synveda", 0, 81, 102, 650, fill, -4.3, ["cv11"]);
}

const lockup = (dark = false) =>
  svg(
    488,
    112,
    `<g transform="translate(0 2) scale(.40625)">${dark ? darkMark : mark}</g><g transform="translate(130 9) scale(.94)">${wordmark(dark ? "#FFFFFF" : navy)}</g>`,
    "Synveda",
  );
const avatar = svg(
  512,
  512,
  `<path fill="${navy}" d="M0 0h512v512H0z"/><g transform="translate(100 94) scale(1.21875)">${darkMark}</g>`,
  "Synveda avatar",
);
const favicon = svg(
  256,
  256,
  `<rect width="256" height="256" rx="48" fill="${navy}"/><g transform="translate(27 24) scale(.79)">${darkMark}</g>`,
  "Synveda",
);
const social = svg(
  1280,
  640,
  `<path fill="#F5F7F8" d="M0 0h1280v640H0z"/><path fill="${navy}" d="M896 0h384v640H896z"/>
  <g transform="translate(76 60) scale(.64)">${lockup().match(/<title[^>]*>.*?<\/title>([\s\S]*)<\/svg>/)[1]}</g>
  ${textPath("Context for", 76, 282, 76, 600, navy, -2)}
  ${textPath("capable agents.", 76, 370, 76, 600, navy, -2)}
  ${textPath("Governed memory, knowledge, skills and tools.", 80, 437, 24, 400)}
  <path stroke="#CDD5DF" d="M80 517h735"/>
  ${textPath("Postgres-first. Self-hosted. Apache-2.0.", 80, 564, 21, 450)}
  <g transform="translate(952 180) scale(1.08)">${darkMark}</g>`,
  "Synveda — Context for capable agents",
);

const outputs = new Map([
  [
    "synveda-mark-dark.svg",
    svg(256, 256, darkMark, "Synveda layered S for dark backgrounds"),
  ],
  [
    "synveda-mark-mono.svg",
    svg(256, 256, monoMark, "Synveda layered S in monochrome"),
  ],
  ["synveda-wordmark.svg", svg(380, 105, wordmark(), "Synveda")],
  ["synveda-lockup.svg", lockup()],
  ["synveda-lockup-dark.svg", lockup(true)],
  ["synveda-avatar.svg", avatar],
  ["favicon.svg", favicon],
]);
for (const [name, source, width, height] of [
  ["synveda-avatar-512.png", avatar, 512, 512],
  ["synveda-avatar-256.png", avatar, 256, 256],
  ["favicon-32.png", favicon, 32, 32],
  ["social-preview.png", social, 1280, 640],
]) {
  outputs.set(name, await rasterize(source, width, height));
}
for (const [name, content] of outputs) {
  const path = new URL(name, brand);
  if (check) {
    if (!Buffer.from(content).equals(await readFile(path))) {
      throw new Error(
        `${name} is stale; run pnpm --filter @synveda/website brand`,
      );
    }
  } else {
    await writeFile(path, content);
  }
}
console.log(
  `${check ? "Checked" : "Wrote"} ${outputs.size} canonical brand exports in ${fileURLToPath(brand)}`,
);
