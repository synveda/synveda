#!/usr/bin/env node
// Lightweight integrity checks for current first-party documentation. Historical
// ADR and spike prose remains link-checked, but is not treated as a statement
// about the current product. Remaining backlog briefs describe open work.

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, posix, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const RETIRED_IMPLEMENTATION_LEDGER =
  "docs/implementation/synveda-context-platform.md";

const FIXTURE_PREFIXES = [
  "adapters/claude-code/fixtures/",
  "console/fixtures/",
  "crates/synveda-cli/fixtures/",
  "demos/fixtures/",
  "evals/fixtures/",
];

const HISTORICAL_PREFIXES = ["docs/adr/", "docs/spikes/"];
const CURRENT_HISTORY_INDEXES = new Set([
  "docs/adr/README.md",
]);

const ROOT_PATH_PREFIXES = new Set([
  ".github",
  ".sqlx",
  "adapters",
  "console",
  "crates",
  "demos",
  "deploy",
  "docs",
  "evals",
  "policies",
  "scripts",
]);

const ROOT_FILES = new Set([
  ".gitattributes",
  ".gitignore",
  "AGENTS.md",
  "Cargo.lock",
  "Cargo.toml",
  "CLAUDE.md",
  "CONTRIBUTING.md",
  "Makefile",
  "README.md",
  "deny.toml",
  "package.json",
  "pnpm-lock.yaml",
  "pnpm-workspace.yaml",
  "rust-toolchain.toml",
  "tsconfig.base.json",
]);

function lineNumberAt(source, index) {
  let line = 1;
  for (let cursor = 0; cursor < index; cursor += 1) {
    if (source.charCodeAt(cursor) === 10) line += 1;
  }
  return line;
}

function blankExceptNewlines(value) {
  return value.replace(/[^\r\n]/gu, " ");
}

/** Masks fenced Markdown blocks without changing offsets or line numbers. */
export function maskFencedBlocks(source) {
  const lines = source.match(/[^\n]*(?:\n|$)/gu) ?? [];
  let fence = null;
  let output = "";

  for (const line of lines) {
    if (line === "") continue;
    if (fence === null) {
      const opening = line.match(/^ {0,3}(`{3,}|~{3,})/u);
      if (opening) {
        fence = { marker: opening[1][0], length: opening[1].length };
        output += blankExceptNewlines(line);
      } else {
        output += line;
      }
      continue;
    }

    const closing = new RegExp(
      `^ {0,3}${fence.marker === "`" ? "`" : "~"}{${fence.length},}[ \\t]*(?:\\r?\\n)?$`,
      "u",
    );
    output += blankExceptNewlines(line);
    if (closing.test(line)) fence = null;
  }
  return output;
}

function codeSpans(source) {
  const spans = [];
  for (let cursor = 0; cursor < source.length; cursor += 1) {
    if (source[cursor] !== "`" || source[cursor - 1] === "\\") continue;
    let width = 1;
    while (source[cursor + width] === "`") width += 1;
    const marker = "`".repeat(width);
    let closing = cursor + width;
    while (closing < source.length) {
      closing = source.indexOf(marker, closing);
      if (closing === -1) break;
      if (source[closing - 1] !== "`" && source[closing + width] !== "`") break;
      closing += width;
    }
    if (closing === -1) {
      cursor += width - 1;
      continue;
    }
    spans.push({
      start: cursor,
      end: closing + width,
      value: source.slice(cursor + width, closing),
    });
    cursor = closing + width - 1;
  }
  return spans;
}

function maskCodeSpans(source) {
  let output = "";
  let cursor = 0;
  for (const span of codeSpans(source)) {
    output += source.slice(cursor, span.start);
    output += blankExceptNewlines(source.slice(span.start, span.end));
    cursor = span.end;
  }
  return output + source.slice(cursor);
}

function destinationFrom(value) {
  const trimmed = value.trim();
  if (trimmed.startsWith("<")) {
    const end = trimmed.indexOf(">");
    return end === -1 ? null : trimmed.slice(1, end);
  }
  let escaped = false;
  for (let index = 0; index < trimmed.length; index += 1) {
    const character = trimmed[index];
    if (!escaped && /\s/u.test(character)) return trimmed.slice(0, index);
    if (!escaped && character === "\\") escaped = true;
    else escaped = false;
  }
  return trimmed;
}

/** Returns inline and reference-definition Markdown link destinations. */
export function extractMarkdownLinks(source) {
  const visible = maskCodeSpans(maskFencedBlocks(source));
  const links = [];

  const definitions = /^ {0,3}\[[^\]\n]+\]:[ \t]*(<[^>\n]+>|\S+)/gmu;
  for (const match of visible.matchAll(definitions)) {
    const target = destinationFrom(match[1]);
    if (target) links.push({ target, line: lineNumberAt(visible, match.index) });
  }

  for (let cursor = 0; cursor < visible.length - 1; cursor += 1) {
    if (visible[cursor] !== "]" || visible[cursor + 1] !== "(") continue;
    const start = cursor + 2;
    let depth = 1;
    let escaped = false;
    let closing = start;
    for (; closing < visible.length; closing += 1) {
      const character = visible[closing];
      if (!escaped && character === "(") depth += 1;
      if (!escaped && character === ")") {
        depth -= 1;
        if (depth === 0) break;
      }
      if (!escaped && character === "\\") escaped = true;
      else escaped = false;
    }
    if (depth !== 0) continue;
    const target = destinationFrom(visible.slice(start, closing));
    if (target) links.push({ target, line: lineNumberAt(visible, cursor) });
    cursor = closing;
  }

  return links;
}

function decode(value) {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

function headingText(value) {
  return value
    .replace(/!?(?:\[([^\]]*)\])\([^)]*\)/gu, "$1")
    .replace(/\[([^\]]+)\]\[[^\]]*\]/gu, "$1")
    .replace(/<[^>]+>/gu, "")
    .replace(/[`*_~]/gu, "")
    .replace(/&amp;/gu, "&")
    .replace(/&lt;/gu, "<")
    .replace(/&gt;/gu, ">");
}

export function headingSlug(value) {
  return headingText(value)
    .trim()
    .toLocaleLowerCase("en-US")
    .replace(/[^\p{Letter}\p{Number}\s_-]/gu, "")
    .replace(/\s+/gu, "-");
}

/** Collects generated heading ids and explicit HTML id/name anchors. */
export function anchorsForMarkdown(source) {
  const visible = maskFencedBlocks(source);
  const anchors = new Set();

  const explicit = /<a\s+[^>]*(?:id|name)\s*=\s*["']([^"']+)["'][^>]*>/giu;
  for (const match of visible.matchAll(explicit)) anchors.add(match[1]);

  const headings = /^ {0,3}#{1,6}[ \t]+(.+?)[ \t]*#*[ \t]*(?:\r)?$/gmu;
  for (const match of visible.matchAll(headings)) {
    const base = headingSlug(match[1]);
    if (!base) continue;
    let anchor = base;
    let suffix = 0;
    while (anchors.has(anchor)) {
      suffix += 1;
      anchor = `${base}-${suffix}`;
    }
    anchors.add(anchor);
  }
  return anchors;
}

export function isFirstPartyMarkdown(file) {
  return (
    file.endsWith(".md") &&
    !file.startsWith(".codex/") &&
    !FIXTURE_PREFIXES.some((prefix) => file.startsWith(prefix))
  );
}

/** Current product/operator prose, excluding decision and feature history. */
export function isCurrentDocument(file) {
  return (
    isFirstPartyMarkdown(file) &&
    file !== RETIRED_IMPLEMENTATION_LEDGER &&
    (CURRENT_HISTORY_INDEXES.has(file) ||
      !HISTORICAL_PREFIXES.some((prefix) => file.startsWith(prefix)))
  );
}

function directoryPaths(paths) {
  const directories = new Set([""]);
  for (const path of paths) {
    let parent = posix.dirname(path);
    while (parent !== "." && parent !== "") {
      directories.add(parent);
      parent = posix.dirname(parent);
    }
  }
  return directories;
}

function relativeDestination(file, destination) {
  if (
    /^(?:[a-z][a-z\d+.-]*:|\/\/|git@)/iu.test(destination) ||
    destination.startsWith("/")
  ) {
    return null;
  }
  const hash = destination.indexOf("#");
  const beforeHash = hash === -1 ? destination : destination.slice(0, hash);
  const query = beforeHash.indexOf("?");
  const path = decode(query === -1 ? beforeHash : beforeHash.slice(0, query));
  const anchor = hash === -1 ? null : decode(destination.slice(hash + 1));
  if (path === "" && anchor === null) return null;
  const joined = path === "" ? file : posix.normalize(posix.join(posix.dirname(file), path));
  if (joined === ".." || joined.startsWith("../")) {
    return { escaped: true, path: joined, anchor };
  }
  return { escaped: false, path: joined.replace(/\/$/u, ""), anchor };
}

export function markdownLinkFindings({ file, source, documents, trackedPaths }) {
  const findings = [];
  const directories = directoryPaths(trackedPaths);
  for (const link of extractMarkdownLinks(source)) {
    const target = relativeDestination(file, link.target);
    if (target === null) continue;
    if (target.escaped) {
      findings.push(
        `${file}:${link.line}: documentation link escapes the repository: ${link.target}`,
      );
      continue;
    }
    if (!trackedPaths.has(target.path) && !directories.has(target.path)) {
      findings.push(`${file}:${link.line}: missing documentation target: ${link.target}`);
      continue;
    }
    if (target.anchor && documents.has(target.path)) {
      const anchors = anchorsForMarkdown(documents.get(target.path));
      if (!anchors.has(target.anchor)) {
        findings.push(`${file}:${link.line}: missing anchor #${target.anchor} in ${target.path}`);
      }
    }
  }
  return findings;
}

function unambiguousRepoPath(value) {
  let candidate = value.trim();
  if (!candidate || /[\s*?<>{}|$]/u.test(candidate)) return null;
  candidate = candidate.replace(/:\d+(?:-\d+)?(?:,\d+(?:-\d+)?)*$/u, "");
  candidate = candidate.split("#", 1)[0];
  if (ROOT_FILES.has(candidate)) return candidate;
  const first = candidate.split("/", 1)[0];
  if (!ROOT_PATH_PREFIXES.has(first)) return null;
  return posix.normalize(candidate).replace(/\/$/u, "");
}

/** Extracts only code spans that unambiguously name a repository-root path. */
export function extractCodeSpanReferences(source) {
  const visible = maskFencedBlocks(source);
  const references = [];
  for (const span of codeSpans(visible)) {
    const path = unambiguousRepoPath(span.value);
    if (path) {
      references.push({ path, raw: span.value.trim(), line: lineNumberAt(visible, span.start) });
    }
  }
  return references;
}

function citedLines(raw) {
  const suffix = raw.match(/:(\d+(?:-\d+)?(?:,\d+(?:-\d+)?)*)$/u)?.[1];
  if (!suffix) return [];
  return suffix.split(",").flatMap((range) =>
    range.split("-").map((value) => Number.parseInt(value, 10)),
  );
}

export function codeSpanReferenceFindings({
  file,
  source,
  trackedPaths,
  lineCounts = new Map(),
}) {
  if (!isCurrentDocument(file) || file === "docs/implementation/context-hard-cut-inventory.md") {
    return [];
  }
  const directories = directoryPaths(trackedPaths);
  const findings = [];
  for (const reference of extractCodeSpanReferences(source)) {
    if (!trackedPaths.has(reference.path) && !directories.has(reference.path)) {
      findings.push(
        `${file}:${reference.line}: code span names no current repository path: ${reference.raw}`,
      );
      continue;
    }
    const lineCount = lineCounts.get(reference.path);
    const maximum = Math.max(0, ...citedLines(reference.raw));
    if (lineCount !== undefined && maximum > lineCount) {
      findings.push(
        `${file}:${reference.line}: cited line ${maximum} exceeds ${reference.path}'s ${lineCount} lines`,
      );
    }
  }
  return findings;
}

export function staleProseFindings({ file, source }) {
  if (!isCurrentDocument(file)) return [];
  const visible = maskFencedBlocks(source);
  const pattern =
    /(?:docs\/implementation\/|(?:\.\.\/)+implementation\/)?synveda-context-platform\.md/gu;
  const lines = new Set(
    [...visible.matchAll(pattern)].map((match) => lineNumberAt(visible, match.index)),
  );
  return [...lines].map(
    (line) =>
      `${file}:${line}: current documentation references the retired implementation ledger`,
  );
}

/** Pure repository-document validation over an injected catalogue. */
export function validateDocuments({ documents, trackedPaths, lineCounts = new Map() }) {
  const findings = [];
  const orderedDocuments = [...documents.entries()].sort(([left], [right]) =>
    left.localeCompare(right),
  );
  for (const [file, source] of orderedDocuments) {
    findings.push(...markdownLinkFindings({ file, source, documents, trackedPaths }));
    findings.push(...codeSpanReferenceFindings({ file, source, trackedPaths, lineCounts }));
    findings.push(...staleProseFindings({ file, source }));
  }
  return findings.sort();
}

function worktreeFiles(root, pattern = null) {
  const args = ["ls-files", "-z", "--cached", "--others", "--exclude-standard"];
  if (pattern !== null) args.push("--", pattern);
  return new Set(
    execFileSync("git", args, { cwd: root, encoding: "utf8" })
      .split("\0")
      .filter(
        (file) =>
          file && !file.startsWith(".codex/") && existsSync(resolve(root, file)),
      ),
  );
}

export function checkRepository(root) {
  const trackedPaths = worktreeFiles(root);
  const markdown = worktreeFiles(root, "*.md");
  const documents = new Map(
    [...markdown]
      .filter(isFirstPartyMarkdown)
      .map((file) => [file, readFileSync(resolve(root, file), "utf8")]),
  );
  const lineCounts = new Map();
  for (const [file, source] of documents) {
    if (!isCurrentDocument(file)) continue;
    for (const reference of extractCodeSpanReferences(source)) {
      if (citedLines(reference.raw).length === 0 || lineCounts.has(reference.path)) continue;
      const path = resolve(root, reference.path);
      if (!trackedPaths.has(reference.path) || !existsSync(path)) continue;
      const content = readFileSync(path, "utf8");
      const lines = content === "" ? 0 : content.split("\n").length - Number(content.endsWith("\n"));
      lineCounts.set(reference.path, lines);
    }
  }
  return {
    documents: documents.size,
    findings: validateDocuments({ documents, trackedPaths, lineCounts }),
  };
}

const entry = process.argv[1] ? resolve(process.argv[1]) : null;
if (entry === fileURLToPath(import.meta.url)) {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const result = checkRepository(root);
  if (result.findings.length > 0) {
    for (const finding of result.findings) console.error(`FAIL ${finding}`);
    console.error(`\n${result.findings.length} documentation problem(s).`);
    process.exit(1);
  }
  console.log(`ok: ${result.documents} first-party Markdown files have valid current references`);
}
