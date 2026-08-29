#!/usr/bin/env node
// CPR-13: demos are executable documentation, so their command and route
// vocabulary must be generated from the product rather than remembered by a
// shell script. This checker executes no demo and interprets no shell
// expansion. It reads only command-shaped shell text outside comments and
// heredoc bodies, compares `synveda` invocations with Clap's own recursive
// `--help`, and compares production `/v1` paths with the generated OpenAPI
// document.

import { execFileSync } from "node:child_process";
import {
  existsSync,
  readFileSync,
  readdirSync,
  statSync,
} from "node:fs";
import { basename, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));
const DEFAULT_DEMOS = join(ROOT, "demos");
const DEFAULT_OPENAPI = join(ROOT, "docs/api/openapi.json");
const DEFAULT_BINARY = join(ROOT, "target/debug/synveda");

const CONTROL_WORDS = new Set([
  "!",
  "do",
  "done",
  "elif",
  "else",
  "fi",
  "if",
  "then",
  "time",
  "until",
  "while",
]);
const COMMAND_PREFIXES = new Set(["command", "env", "exec", "sudo", "timeout"]);
const DOCUMENT_COMMANDS = new Set([
  "awk",
  "cat",
  "echo",
  "grep",
  "head",
  "jq",
  "printf",
  "sed",
  "tail",
]);

function commandName(token) {
  return basename(token.replaceAll("\\", "/"));
}

function isSynvedaCommand(token) {
  const name = commandName(token).replace(/^['"]|['"]$/g, "");
  if (name === "synveda") return true;
  return /^\$\{?(?:CLI|BIN|SYNVEDA_CLI)\}?$/.test(name);
}

function stripComment(line) {
  let single = false;
  let double = false;
  let escaped = false;
  for (let index = 0; index < line.length; index += 1) {
    const char = line[index];
    if (escaped) {
      escaped = false;
      continue;
    }
    if (char === "\\" && !single) {
      escaped = true;
      continue;
    }
    if (char === "'" && !double) {
      single = !single;
      continue;
    }
    if (char === '"' && !single) {
      double = !double;
      continue;
    }
    if (char === "#" && !single && !double) {
      const before = index === 0 ? "" : line[index - 1];
      if (index === 0 || /\s/.test(before)) return line.slice(0, index);
    }
  }
  return line;
}

function heredocDelimiters(line) {
  const delimiters = [];
  const matcher = /<<-?\s*(['"]?)([A-Za-z_][A-Za-z0-9_]*)\1/g;
  for (const match of line.matchAll(matcher)) delimiters.push(match[2]);
  return delimiters;
}

function logicalLines(source) {
  const output = [];
  const heredocs = [];
  let pending = "";
  let pendingLine = 0;
  const lines = source.split(/\r?\n/);
  for (let index = 0; index < lines.length; index += 1) {
    const raw = lines[index];
    if (heredocs.length > 0) {
      if (raw.trim() === heredocs[0]) heredocs.shift();
      continue;
    }
    const withoutComment = stripComment(raw);
    const trimmed = withoutComment.trim();
    if (!trimmed && !pending) continue;
    if (!pending) pendingLine = index + 1;
    const continued = /\\\s*$/.test(withoutComment);
    pending += `${pending ? " " : ""}${withoutComment.replace(/\\\s*$/, "").trim()}`;
    if (continued) continue;
    if (pending.trim()) {
      output.push({ line: pendingLine, text: pending.trim() });
      heredocs.push(...heredocDelimiters(pending));
    }
    pending = "";
  }
  if (pending.trim()) output.push({ line: pendingLine, text: pending.trim() });
  return output;
}

function commandSegments(line) {
  const segments = [];
  let current = "";
  let single = false;
  let double = false;
  let escaped = false;
  const flush = () => {
    if (current.trim()) segments.push(current.trim());
    current = "";
  };
  for (let index = 0; index < line.length; index += 1) {
    const char = line[index];
    const next = line[index + 1] ?? "";
    if (escaped) {
      current += char;
      escaped = false;
      continue;
    }
    if (char === "\\" && !single) {
      current += char;
      escaped = true;
      continue;
    }
    if (char === "'" && !double) {
      single = !single;
      current += char;
      continue;
    }
    if (char === '"' && !single) {
      double = !double;
      current += char;
      continue;
    }
    if (!single && (char === "`" || (char === "$" && next === "("))) {
      flush();
      if (char === "$") index += 1;
      continue;
    }
    if (!single && !double && (char === ";" || char === "|" || char === "&" || char === ")")) {
      flush();
      if ((char === "|" || char === "&") && next === char) index += 1;
      continue;
    }
    current += char;
  }
  flush();
  return segments;
}

function shellWords(segment) {
  const words = [];
  let value = "";
  let single = false;
  let double = false;
  let escaped = false;
  let started = false;
  const flush = () => {
    if (started) words.push(value);
    value = "";
    started = false;
  };
  for (const char of segment) {
    if (escaped) {
      value += char;
      started = true;
      escaped = false;
      continue;
    }
    if (char === "\\" && !single) {
      escaped = true;
      started = true;
      continue;
    }
    if (char === "'" && !double) {
      single = !single;
      started = true;
      continue;
    }
    if (char === '"' && !single) {
      double = !double;
      started = true;
      continue;
    }
    if (/\s/.test(char) && !single && !double) {
      flush();
      continue;
    }
    value += char;
    started = true;
  }
  flush();
  return words;
}

function commandStart(words) {
  let index = 0;
  while (index < words.length) {
    const word = words[index];
    if (CONTROL_WORDS.has(word) || /^\w+=/.test(word)) {
      index += 1;
      continue;
    }
    if (COMMAND_PREFIXES.has(commandName(word))) {
      index += 1;
      while (index < words.length && (/^\w+=/.test(words[index]) || words[index].startsWith("-"))) {
        index += 1;
      }
      continue;
    }
    return index;
  }
  return -1;
}

function parseHelpCommands(help) {
  const commands = [];
  let inside = false;
  for (const line of help.split(/\r?\n/)) {
    if (line === "Commands:") {
      inside = true;
      continue;
    }
    if (!inside) continue;
    if (/^[A-Z][A-Za-z ]+:/.test(line)) break;
    const match = line.match(/^  ([a-z][a-z0-9-]*)\s{2,}/);
    if (match && match[1] !== "help") commands.push(match[1]);
  }
  return commands;
}

function parseHelpOptions(help) {
  const options = new Set();
  for (const line of help.split(/\r?\n/)) {
    const match = line.match(/^\s+(?:-[A-Za-z],\s+)?(--[a-z][a-z0-9-]*)\b/);
    if (match) options.add(match[1]);
  }
  return options;
}

function helpText(path) {
  const args = [...path, "--help"];
  if (existsSync(DEFAULT_BINARY)) {
    return execFileSync(DEFAULT_BINARY, args, { encoding: "utf8" });
  }
  return execFileSync(
    "cargo",
    ["run", "--quiet", "-p", "synveda-cli", "--bin", "synveda", "--", ...args],
    {
      cwd: ROOT,
      encoding: "utf8",
      env: { ...process.env, SQLX_OFFLINE: process.env.SQLX_OFFLINE ?? "true" },
    },
  );
}

function buildCli() {
  execFileSync("cargo", ["build", "--quiet", "-p", "synveda-cli", "--bin", "synveda"], {
    cwd: ROOT,
    env: { ...process.env, SQLX_OFFLINE: process.env.SQLX_OFFLINE ?? "true" },
    stdio: "inherit",
  });
}

export function generatedCliInventory(readHelp = helpText) {
  const children = new Map();
  const visit = (path) => {
    const help = readHelp(path);
    const commands = parseHelpCommands(help);
    children.set(path.join(" "), {
      commands: new Set(commands),
      options: parseHelpOptions(help),
    });
    for (const command of commands) visit([...path, command]);
  };
  visit([]);
  return children;
}

export function openApiRoutes(documentPath = DEFAULT_OPENAPI) {
  const document = JSON.parse(readFileSync(documentPath, "utf8"));
  return Object.keys(document.paths ?? {});
}

function routeMatcher(path) {
  const escaped = path.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`^${escaped.replace(/\\\{[^}]+\\\}/g, "[^/]+")}/?$`);
}

function pathsIn(words) {
  const paths = [];
  for (const word of words) {
    // Rauthy is a separate product with its own `/auth/v1` API. Those calls
    // exercise Synveda's OIDC boundary but are not Synveda production routes
    // and therefore cannot appear in Synveda's OpenAPI document.
    if (word.includes("/auth/v1/")) continue;
    for (const match of word.matchAll(/\/v1\/[A-Za-z0-9_{}.$/:-]+/g)) {
      paths.push(match[0].replace(/[),.:]+$/, "").split(/[?#]/, 1)[0]);
    }
  }
  return paths;
}

function shellScripts(directory) {
  const output = [];
  for (const entry of readdirSync(directory).sort()) {
    const path = join(directory, entry);
    if (statSync(path).isDirectory()) {
      output.push(...shellScripts(path));
    } else if (entry.endsWith(".sh")) {
      output.push(path);
    }
  }
  return output;
}

function yamlFiles(directory) {
  const output = [];
  for (const entry of readdirSync(directory).sort()) {
    const path = join(directory, entry);
    if (statSync(path).isDirectory()) {
      output.push(...yamlFiles(path));
    } else if (entry.endsWith(".yaml") || entry.endsWith(".yml")) {
      output.push(path);
    }
  }
  return output;
}

function yamlCommandArrays(source) {
  const output = [];
  const matcher = /^\s*(?:command|args):\s*(\[[^\n]*\])\s*$/gmu;
  for (const match of source.matchAll(matcher)) {
    try {
      const words = JSON.parse(match[1]);
      if (Array.isArray(words) && words.every((word) => typeof word === "string")) {
        output.push({ line: source.slice(0, match.index).split("\n").length, words });
      }
    } catch {
      // YAML permits more than JSON. Non-JSON command forms remain the shell
      // script's responsibility; only exact arrays can be checked safely here.
    }
  }
  return output;
}

function inventoryEntry(inventory, path) {
  const entry = inventory.get(path);
  if (entry instanceof Set) return { commands: entry, options: new Set() };
  return entry ?? { commands: new Set(), options: new Set() };
}

function cliFinding(file, line, words, start, inventory) {
  if (start < 0 || !isSynvedaCommand(words[start])) return null;
  let parent = "";
  let cursor = start + 1;
  const accepted = [];
  while (cursor < words.length) {
    const token = words[cursor];
    if (token.startsWith("-") || token.includes("=") || token.startsWith("$")) break;
    const available = inventoryEntry(inventory, parent).commands;
    if (!available.has(token)) {
      if (available.size === 0) break;
      const attempted = [...accepted, token].join(" ");
      return `${file}:${line}: synveda ${attempted}: absent from generated --help inventory`;
    }
    accepted.push(token);
    parent = accepted.join(" ");
    cursor += 1;
  }
  if (accepted.length === 0) {
    return `${file}:${line}: synveda ${words[start + 1] ?? "<missing>"}: absent from generated --help inventory`;
  }
  const options = new Set([
    ...inventoryEntry(inventory, "").options,
    ...inventoryEntry(inventory, parent).options,
  ]);
  for (const token of words.slice(cursor)) {
    if (!token.startsWith("--") || token === "--" || token.includes("$")) continue;
    const option = token.split("=", 1)[0];
    if (!options.has(option)) {
      return `${file}:${line}: synveda ${accepted.join(" ")} ${option}: absent from generated --help options`;
    }
  }
  return null;
}

export function checkCorpus({ demoDir, routes, cliInventory, repositoryRoot = resolve(demoDir, "..") }) {
  const findings = [];
  const matchers = routes.map((path) => [path, routeMatcher(path)]);
  const files = shellScripts(demoDir);
  for (const path of files) {
    const file = relative(demoDir, path);
    const source = readFileSync(path, "utf8");
    for (const match of source.matchAll(/\bDEMO_(?:COMPOSE|DATABASE)\b/gu)) {
      const line = source.slice(0, match.index).split("\n").length;
      findings.push(
        `${file}:${line}: ${match[0]}: retired owner-style database probe bypasses the exact-role demo fixture`,
      );
    }
    for (const match of source.matchAll(/\bdocs\/[A-Za-z0-9_./-]+\.(?:md|json)\b/gu)) {
      if (!existsSync(resolve(repositoryRoot, match[0]))) {
        const line = source.slice(0, match.index).split("\n").length;
        findings.push(`${file}:${line}: ${match[0]}: absent from the current repository`);
      }
    }
    for (const logical of logicalLines(source)) {
      for (const segment of commandSegments(logical.text)) {
        const words = shellWords(segment);
        if (words.length === 0) continue;
        const start = commandStart(words);
        const cli = cliFinding(file, logical.line, words, start, cliInventory);
        if (cli) findings.push(cli);
        const command = start >= 0 ? commandName(words[start]) : "";
        if (DOCUMENT_COMMANDS.has(command)) continue;
        for (const candidate of pathsIn(words)) {
          if (!matchers.some(([, matcher]) => matcher.test(candidate))) {
            findings.push(
              `${file}:${logical.line}: ${candidate}: absent from generated OpenAPI paths`,
            );
          }
        }
      }
    }
  }
  for (const path of yamlFiles(demoDir)) {
    const file = relative(demoDir, path);
    const source = readFileSync(path, "utf8");
    for (const invocation of yamlCommandArrays(source)) {
      const start = commandStart(invocation.words);
      const cli = cliFinding(file, invocation.line, invocation.words, start, cliInventory);
      if (cli) findings.push(cli);
    }
  }
  return findings;
}

function main() {
  // Do not trust a possibly stale target binary: the inventory is useful only
  // when Cargo has first made it current with the checked-out CLI sources.
  buildCli();
  const findings = checkCorpus({
    demoDir: DEFAULT_DEMOS,
    routes: openApiRoutes(),
    cliInventory: generatedCliInventory(),
  });
  if (findings.length > 0) {
    for (const finding of findings) console.error(`FAIL ${finding}`);
    console.error(`\n${findings.length} dead demo command/path reference(s).`);
    process.exitCode = 1;
    return;
  }
  const count = shellScripts(DEFAULT_DEMOS).length;
  console.log(`ok: ${count} demo scripts use only generated CLI commands and OpenAPI paths`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
