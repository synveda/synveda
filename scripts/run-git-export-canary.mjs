#!/usr/bin/env node
// FLOW-8: opt-in public-CLI export/replay with an independent private GitHub clone.
// This records provider evidence, not credential rotation or joint recovery.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  closeSync, constants, fstatSync, mkdirSync, mkdtempSync, openSync,
  readSync, realpathSync, rmSync, writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { isAbsolute, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const CHANNELS = ["prompt/published", "context-pack/published"];
const HASH = /^[0-9a-f]{64}$/;
const OID = /^[0-9a-f]{40}$/;
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const CONFIG_KEYS = [
  "format", "profile", "tenant_id", "scope_id", "target", "repository",
  "repository_id", "deployment_artifact", "cli_sha256", "source_heads",
  "synthetic_scope",
];

function fail(message) {
  throw new Error(`FLOW-8 canary: ${message}`);
}

function exactKeys(value, keys) {
  return value && typeof value === "object" && !Array.isArray(value)
    && Object.keys(value).sort().join("\0") === [...keys].sort().join("\0");
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function boundedFile(path, limit) {
  let fd;
  try {
    fd = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW);
    const info = fstatSync(fd);
    if (!info.isFile() || info.size > limit) fail("input file is not regular or exceeds its bound");
    const buffer = Buffer.alloc(info.size + 1);
    let length = 0;
    while (length < buffer.length) {
      const count = readSync(fd, buffer, length, buffer.length - length, null);
      if (count === 0) break;
      length += count;
    }
    if (length !== info.size) fail("input file changed while being read");
    return buffer.subarray(0, length);
  } catch {
    fail("cannot read the bounded input file");
  } finally {
    if (fd !== undefined) closeSync(fd);
  }
}

function json(stdout, label) {
  try {
    return JSON.parse(stdout);
  } catch {
    // Provider/CLI parse errors can include echoed data or credentials.
    fail(`${label} returned invalid JSON`);
  }
}

export function validateConfig(config) {
  const keys = config?.trusted_keys_file === undefined
    ? CONFIG_KEYS : [...CONFIG_KEYS, "trusted_keys_file"];
  const [owner, repository, ...extra] = typeof config?.repository === "string"
    ? config.repository.split("/") : [];
  if (!exactKeys(config, keys)
      || config.format !== "synveda-github-canary-v1" || config.synthetic_scope !== true
      || !UUID.test(config.tenant_id) || !UUID.test(config.scope_id)
      || !/^[a-z0-9][a-z0-9-]{0,47}$/.test(config.target)
      || !/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(config.profile)
      || !owner || !/^[a-z0-9](?:[a-z0-9-]{0,37}[a-z0-9])?$/.test(owner)
      || owner.includes("--") || !repository || extra.length !== 0
      || !/^[a-z0-9][a-z0-9_.-]{0,99}$/.test(repository) || repository.endsWith(".git")
      || !Number.isSafeInteger(config.repository_id) || config.repository_id <= 0
      || !/^sha256:[0-9a-f]{64}$/.test(config.deployment_artifact)
      || !HASH.test(config.cli_sha256) || !exactKeys(config.source_heads, CHANNELS)
      || CHANNELS.some((name) => !HASH.test(config.source_heads[name]))
      || (config.trusted_keys_file !== undefined
        && (typeof config.trusted_keys_file !== "string" || !isAbsolute(config.trusted_keys_file)))) {
    fail("configuration must pin a dedicated synthetic scope, repository, artifacts and both source heads");
  }
  return config;
}

function childEnvironment() {
  const env = { ...process.env };
  // A raw bearer overrides a CLI profile and can silently choose another host.
  delete env.SYNVEDA_TOKEN;
  delete env.SYNVEDA_GATEWAY;
  for (const name of Object.keys(env)) {
    if (name.startsWith("GIT_") || /^(?:https?|all)_proxy$/i.test(name)) delete env[name];
  }
  return {
    ...env, GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_SYSTEM: "/dev/null",
    GIT_CONFIG_GLOBAL: "/dev/null", GIT_TERMINAL_PROMPT: "0",
    GIT_NO_REPLACE_OBJECTS: "1", GIT_ATTR_NOSYSTEM: "1",
  };
}

function command(program, args, label, env, cwd = ROOT) {
  const result = spawnSync(program, args, {
    cwd, env, encoding: "utf8", timeout: 60_000, maxBuffer: 256 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
  });
  if (result.error || result.status !== 0) fail(`${label} failed; captured command output is withheld`);
  return result.stdout;
}

function metadata(config, env) {
  const value = json(command("gh", [
    "api", "--hostname", "github.com", `repos/${config.repository}`,
    "--jq", "{id,full_name,private,visibility,archived,disabled,fork,size}",
  ], "GitHub metadata read", env), "GitHub metadata read");
  if (value.id !== config.repository_id || typeof value.full_name !== "string"
      || value.full_name.toLowerCase() !== config.repository || value.private !== true
      || value.visibility !== "private" || value.archived !== false
      || value.disabled !== false || value.fork !== false
      || !Number.isSafeInteger(value.size) || value.size < 0 || value.size > 65_536) {
    fail("GitHub repository identity/privacy or reported canary size differs");
  }
}

function receipt(value, config, name) {
  if (value?.provider !== "github" || value.repository_id !== config.repository_id
      || value.target !== config.target || value.source_head !== config.source_heads[name]
      || value.source_pin !== null || value.git_ref !== `refs/heads/synveda/${name}`
      || !OID.test(value.git_head) || !HASH.test(value.mapping_digest)
      || !HASH.test(value.destination_digest)
      || !["completed", "resumed", "no_op"].includes(value.outcome)
      || !Number.isInteger(value.commits) || value.commits < 1 || value.commits > 128
      || !Number.isInteger(value.objects) || value.objects < 1 || value.objects > 1024) {
    fail("export receipt differs from the pinned canary fixture or destination");
  }
  // Select only validated, content-free fields; never retain an entire response.
  return Object.fromEntries([
    "target", "provider", "repository_id", "source_head", "source_pin", "git_ref",
    "git_head", "mapping_digest", "destination_digest", "commits", "objects", "outcome",
  ].map((key) => [key, value[key]]));
}

function writeReport(directory, name, value) {
  writeFileSync(join(directory, name), `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
}

export function run(configPath, outputPath) {
  const configBytes = boundedFile(resolve(configPath), 16_384);
  const config = validateConfig(json(configBytes, "canary configuration"));
  const cli = realpathSync(process.env.SYNVEDA_BIN || join(ROOT, "target/debug/synveda"));
  if (sha256(boundedFile(cli, 512 * 1024 * 1024)) !== config.cli_sha256) {
    fail("CLI bytes differ from the pinned artifact");
  }
  if (config.trusted_keys_file) boundedFile(config.trusted_keys_file, 65_536);
  const env = childEnvironment();
  const cliJson = (args, label) => json(command(cli, args, label, env), label);
  const codeRevision = command("git", ["rev-parse", "HEAD"], "source revision", env).trim();
  if (!OID.test(codeRevision)) fail("source revision is unavailable");
  const codeDirty = command("git", ["status", "--porcelain=v1"], "source status", env).trim() !== "";
  const version = command(cli, ["--version"], "CLI version", env).trim();
  if (!/^synveda [A-Za-z0-9.+-]{1,48}$/.test(version)) fail("CLI version is unavailable");
  const whoami = cliJson(["whoami", "--profile", config.profile, "--json"], "caller read");
  if (whoami?.tenant?.id !== config.tenant_id) fail("CLI profile resolves another tenant");
  const effective = cliJson([
    "configuration", "effective", config.scope_id, "--profile", config.profile, "--json",
  ], "Configuration read");
  if (effective?.scope_id !== config.scope_id
      || !Array.isArray(effective.document?.git_export_targets)
      || !effective.document.git_export_targets.includes(config.target)
      || !Array.isArray(effective.document?.allowed_external_providers)
      || !effective.document.allowed_external_providers.includes("github")) {
    fail("effective Configuration does not enable the exact target/provider");
  }
  const channels = cliJson([
    "channel", "status", config.scope_id, "--profile", config.profile, "--json",
  ], "source channel read");
  if (channels?.scope_id !== config.scope_id || !Array.isArray(channels.channels)
      || CHANNELS.some((name) => {
        const selected = channels.channels.filter((entry) => entry.name === name);
        return selected.length !== 1 || selected[0].commit !== config.source_heads[name]
          || selected[0].pin != null;
      })) fail("source heads moved or pins are present; quiesce the synthetic fixture");
  metadata(config, env);
  const output = resolve(outputPath);
  mkdirSync(output, { mode: 0o700 }); // Exclusive creation preserves earlier evidence.
  const scratch = mkdtempSync(join(tmpdir(), "synveda-github-canary-"));
  const report = {
    schema_version: 1, feature: "FLOW-8", evidence_tier: "incomplete_live_github_canary",
    started_at: new Date().toISOString(), config_sha256: sha256(configBytes),
    code_revision: codeRevision, code_dirty: codeDirty, cli_version: version,
    cli_sha256: config.cli_sha256, gateway_artifact_operator_pin: config.deployment_artifact,
    tenant_id: config.tenant_id, scope_id: config.scope_id, target: config.target,
    repository_id: config.repository_id, phase: "export", channels: [],
    not_measured: [
      "credential rotation/revocation", "outage and uncertain acknowledgement recovery",
      "branch protection/divergence", "deployment egress/retention", "paired database/key recovery",
      "independent binding of the running gateway to the operator artifact pin",
    ],
  };
  try {
    for (const name of CHANNELS) {
      report.phase = `export:${name}`;
      writeReport(output, "partial.json", report);
      const args = ["git-bridge", "export", config.scope_id,
        "--target", config.target, "--channel", name, "--profile", config.profile];
      const first = receipt(cliJson(args, "canary export"), config, name);
      if (report.channels.length && first.destination_digest !== report.channels[0].first.destination_digest) {
        fail("destination custody changed between channels");
      }
      const row = { channel: name, first };
      report.channels.push(row);
      report.phase = `replay:${name}`;
      writeReport(output, "partial.json", report);
      const replay = receipt(cliJson(args, "canary replay"), config, name);
      if (replay.outcome !== "no_op"
          || JSON.stringify({ ...first, outcome: "no_op" }) !== JSON.stringify(replay)) {
        fail("replay changed the receipt instead of returning the exact no-op");
      }
      row.replay = replay;
      report.phase = `clone:${name}`;
      writeReport(output, "partial.json", report);
      const repository = join(scratch, name.split("/")[0]);
      command("git", [
        "-c", "core.hooksPath=/dev/null", "-c", "credential.helper=",
        "-c", "credential.helper=!gh auth git-credential",
        "-c", "protocol.allow=never", "-c", "protocol.https.allow=always",
        "-c", "http.sslVerify=true", "-c", "http.followRedirects=false", "-c", "http.proxy=",
        "clone", "--bare", "--single-branch", "--template=", "--branch", `synveda/${name}`,
        `https://github.com/${config.repository}.git`, repository,
      ], "independent GitHub clone", env, scratch);
      const observed = command("git", ["--git-dir", repository,
        "rev-parse", "--verify", first.git_ref], "clone head read", env, scratch).trim();
      if (observed !== first.git_head) fail("independent clone head differs from the export receipt");
      report.phase = `verify:${name}`;
      writeReport(output, "partial.json", report);
      const verified = cliJson(["git-bridge", "verify", repository, "--channel", name,
        ...(config.trusted_keys_file ? ["--keys", config.trusted_keys_file] : [])], "offline verification");
      if (verified?.valid !== true || verified.format !== "synveda-git-export-v1"
          || verified.source_head !== first.source_head || verified.mapping_digest !== first.mapping_digest
          || verified.commits !== first.commits || verified.objects !== first.objects) {
        fail("independent verification differs from the export receipt");
      }
      row.verified = true;
    }
    report.phase = "final_repository_metadata";
    writeReport(output, "partial.json", report);
    metadata(config, env);
    report.phase = "completed";
    report.evidence_tier = "live_github_export_replay";
    report.completed_at = new Date().toISOString();
    writeReport(output, "report.json", report);
    rmSync(join(output, "partial.json"));
    process.stdout.write("FLOW-8 canary: both exports, exact no-op replays and independent clone verification passed\n");
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 4) {
    process.stderr.write("FLOW-8 canary PENDING: supply the prepared canary JSON and a new private evidence directory\n");
    process.exitCode = 77;
  } else {
    try {
      run(...process.argv.slice(2));
    } catch (error) {
      process.stderr.write(`${error.message.startsWith("FLOW-8 canary:") ? error.message : "FLOW-8 canary: prerequisite or private evidence storage failed"}\n`);
      process.exitCode = 1;
    }
  }
}
