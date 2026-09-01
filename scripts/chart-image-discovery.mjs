function refuse(code) {
  const error = new Error(code);
  error.code = code;
  throw error;
}

export function parseComposeDefaults(source) {
  const defaults = new Map();
  for (const raw of source.split(/\r?\n/)) {
    if (raw === "" || raw.startsWith("#")) continue;
    const match = raw.match(/^([A-Z][A-Z0-9_]*)=(\S*)$/);
    if (!match) refuse("compose-default-syntax");
    const [, name, value] = match;
    if (defaults.has(name)) refuse("compose-default-duplicate");
    defaults.set(name, value);
  }
  return defaults;
}

export function resolveComposeImage(raw, defaults) {
  if (typeof raw !== "string" || raw === "" || raw.length > 1024) {
    refuse("compose-image-syntax");
  }
  if (!raw.includes("$")) {
    if (/\s/.test(raw)) refuse("compose-image-syntax");
    return raw;
  }

  let match = raw.match(/^\$\{([A-Z][A-Z0-9_]*)\}$/);
  if (match) {
    const value = defaults.get(match[1]);
    if (value === undefined || value === "") refuse("compose-image-default-missing");
    return value;
  }

  match = raw.match(/^\$\{([A-Z][A-Z0-9_]*):\?[^}]+\}$/);
  if (match) {
    const value = defaults.get(match[1]);
    if (value === undefined || value === "") refuse("compose-image-default-missing");
    return value;
  }

  match = raw.match(/^\$\{([A-Z][A-Z0-9_]*):-(\S+)\}$/);
  if (match) {
    const configured = defaults.get(match[1]);
    return configured === undefined || configured === "" ? match[2] : configured;
  }

  refuse("compose-image-expression");
}

export function composeImageReferences(source, defaults) {
  const references = [];
  for (const line of source.split(/\r?\n/)) {
    const trimmed = line.trimStart();
    if (trimmed === "" || trimmed.startsWith("#")) continue;
    const withoutExpressions = line.replace(/\$\{[^}\r\n]*\}/g, "");
    if (
      /[{}]/.test(withoutExpressions) &&
      !/^\s*[A-Za-z0-9_.-]+:\s*\{\}\s*$/.test(withoutExpressions)
    ) refuse("compose-flow-mapping");
    if (
      /^(?:["']|\?|!|&)/.test(trimmed) ||
      /(?:^|[^A-Za-z0-9_-])(?:image|"image"|'image')\s*:/.test(trimmed) ||
      /^image\s+:/.test(trimmed)
    ) {
      const canonical = line.match(/^\s*image:\s+(.+?)\s*$/);
      if (canonical === null) refuse("compose-key-syntax");
      references.push(resolveComposeImage(canonical[1], defaults));
      continue;
    }
    const match = line.match(/^\s*image:\s+(.+?)\s*$/);
    if (match) references.push(resolveComposeImage(match[1], defaults));
  }
  return references;
}

export function canonicalComposeFiles(entries) {
  return entries
    .filter((entry) => entry.isFile() && /^compose(?:\.[a-z0-9-]+)?\.yaml$/.test(entry.name))
    .map((entry) => entry.name)
    .sort();
}

export function dockerfileBaseImages(source) {
  const defaults = new Map();
  const stages = new Set();
  const references = [];
  let sawFrom = false;

  for (const rawLine of source.split(/\r?\n/)) {
    if (/^\s*#\s*(?:syntax|escape|check)\s*=/i.test(rawLine)) {
      refuse("dockerfile-parser-directive");
    }
    const line = rawLine.trim();
    if (line === "" || line.startsWith("#")) continue;

    if (!sawFrom && /^ARG\b/i.test(line)) {
      const argument = line.match(/^ARG\s+([A-Za-z_][A-Za-z0-9_]*)=(\S+)$/i);
      if (argument === null) refuse("dockerfile-base-default-missing");
      const [, name, value] = argument;
      if (defaults.has(name)) refuse("dockerfile-base-default-duplicate");
      defaults.set(name, value);
      continue;
    }

    const from = line.match(
      /^FROM\s+(?:--platform=\S+\s+)?(\S+)(?:\s+AS\s+(\S+))?$/i,
    );
    if (from === null) {
      if (!sawFrom) refuse("dockerfile-instruction-before-from");
      continue;
    }

    sawFrom = true;
    const [, raw, alias] = from;
    if (!stages.has(raw.toLowerCase())) {
    const resolved = raw.replace(
      /\$\{([A-Za-z_][A-Za-z0-9_]*)\}/g,
      (whole, name) => defaults.get(name) ?? whole,
    );
    if (resolved.includes("$")) refuse("dockerfile-base-default-missing");
    references.push(resolved);
    }
    if (alias !== undefined) {
      const canonicalAlias = alias.toLowerCase();
      if (stages.has(canonicalAlias)) refuse("dockerfile-stage-alias-duplicate");
      stages.add(canonicalAlias);
    }
  }

  if (!sawFrom) refuse("dockerfile-base-missing");
  return references;
}
