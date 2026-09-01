import { constants, closeSync, fstatSync, lstatSync, openSync, readSync } from "node:fs";

const LOGIN_SCOPES = Object.freeze(["openid", "profile", "email"]);
const OIDC_VALUE = /^[A-Za-z0-9_-]{43}$/;
const SESSION_STATE = /^[A-Za-z0-9_-]{24}$/;
const PASSWORD = /^[0-9a-f]{64}$/;
const AUTHORIZATION_PARAMETERS = Object.freeze([
  "client_id",
  "code_challenge",
  "code_challenge_method",
  "nonce",
  "redirect_uri",
  "response_type",
  "scope",
  "state",
]);
const SENSITIVE_PARAMETERS = Object.freeze([
  "access_token",
  "client_secret",
  "code_verifier",
  "id_token",
  "password",
  "refresh_token",
  "token",
]);
const FILESYSTEM = Object.freeze({
  closeSync,
  fstatSync,
  lstatSync,
  openSync,
  readSync,
});

export class BrowserContractError extends Error {
  constructor(stage) {
    super(stage);
    this.name = "BrowserContractError";
    this.stage = stage;
  }
}

function refuse(stage) {
  throw new BrowserContractError(stage);
}

function exactUrl(raw, stage) {
  if (typeof raw !== "string" || raw.length < 8 || raw.length > 512) refuse(stage);
  let url;
  try {
    url = new URL(raw);
  } catch {
    refuse(stage);
  }
  if (
    url.username !== "" ||
    url.password !== "" ||
    url.search !== "" ||
    url.hash !== "" ||
    !["http:", "https:"].includes(url.protocol)
  ) refuse(stage);
  return url;
}

export function validateSettings(appRaw, issuerRaw) {
  const app = exactUrl(appRaw, "configuration");
  const issuer = exactUrl(issuerRaw, "configuration");
  if (app.pathname !== "/" || issuer.pathname !== "/realms/synveda") {
    refuse("configuration");
  }
  if (app.protocol !== issuer.protocol || app.port !== issuer.port) {
    refuse("configuration");
  }
  if (app.hostname === issuer.hostname) refuse("configuration");
  if (app.protocol === "http:") {
    if (
      !app.hostname.endsWith(".test") ||
      !issuer.hostname.endsWith(".test") ||
      app.port === ""
    ) refuse("configuration");
  }
  return Object.freeze({
    appOrigin: app.origin,
    issuer: issuer.href,
    issuerOrigin: issuer.origin,
    authorizationPath: `${issuer.pathname}/protocol/openid-connect/auth`,
    callback: `${app.origin}/auth/callback`,
  });
}

function one(search, name, stage) {
  const values = search.getAll(name);
  if (values.length !== 1 || values[0] === "") refuse(stage);
  return values[0];
}

export function validateAuthorizationUrl(raw, settings) {
  if (typeof raw !== "string" || raw.length > 4096) refuse("authorization-request");
  let url;
  try {
    url = new URL(raw);
  } catch {
    refuse("authorization-request");
  }
  if (
    url.origin !== settings.issuerOrigin ||
    url.pathname !== settings.authorizationPath ||
    url.hash !== "" ||
    url.username !== "" ||
    url.password !== ""
  ) refuse("authorization-request");
  const names = [...url.searchParams.keys()].sort();
  if (
    names.length !== AUTHORIZATION_PARAMETERS.length ||
    names.some((name, index) => name !== AUTHORIZATION_PARAMETERS[index]) ||
    SENSITIVE_PARAMETERS.some((name) => url.searchParams.has(name))
  ) refuse("authorization-request");
  if (
    one(url.searchParams, "response_type", "authorization-request") !== "code" ||
    one(url.searchParams, "client_id", "authorization-request") !== "synveda" ||
    one(url.searchParams, "redirect_uri", "authorization-request") !== settings.callback ||
    one(url.searchParams, "scope", "authorization-request") !== LOGIN_SCOPES.join(" ") ||
    one(url.searchParams, "code_challenge_method", "authorization-request") !== "S256" ||
    !OIDC_VALUE.test(one(url.searchParams, "state", "authorization-request")) ||
    !OIDC_VALUE.test(one(url.searchParams, "nonce", "authorization-request")) ||
    !OIDC_VALUE.test(one(url.searchParams, "code_challenge", "authorization-request"))
  ) refuse("authorization-request");
  return one(url.searchParams, "state", "authorization-request");
}

export function validateCallbackUrl(raw, settings, expectedState) {
  if (typeof raw !== "string" || raw.length > 8192) refuse("callback");
  let url;
  try {
    url = new URL(raw);
  } catch {
    refuse("callback");
  }
  const names = [...url.searchParams.keys()].sort();
  if (
    url.origin !== settings.appOrigin ||
    url.pathname !== "/auth/callback" ||
    url.hash !== "" ||
    url.username !== "" ||
    url.password !== "" ||
    names.length !== 4 ||
    names[0] !== "code" ||
    names[1] !== "iss" ||
    names[2] !== "session_state" ||
    names[3] !== "state" ||
    one(url.searchParams, "code", "callback").length > 4096 ||
    one(url.searchParams, "iss", "callback") !== settings.issuer ||
    !SESSION_STATE.test(one(url.searchParams, "session_state", "callback")) ||
    !OIDC_VALUE.test(expectedState) ||
    one(url.searchParams, "state", "callback") !== expectedState
  ) refuse("callback");
  return true;
}

export function allowedRequest(raw, settings) {
  if (typeof raw !== "string" || raw.length > 8192) return false;
  try {
    const url = new URL(raw);
    if (url.username !== "" || url.password !== "" || url.hash !== "") return false;
    if (url.origin === settings.appOrigin) return true;
    if (url.origin !== settings.issuerOrigin) return false;
    return (
      url.pathname === settings.authorizationPath ||
      url.pathname.startsWith(`${new URL(settings.issuer).pathname}/login-actions/`) ||
      url.pathname.startsWith("/resources/")
    );
  } catch {
    return false;
  }
}

function exactSecretMetadata(metadata, uid) {
  return (
    metadata.isFile() &&
    !metadata.isSymbolicLink() &&
    metadata.nlink === 1 &&
    (metadata.mode & 0o7777) === 0o600 &&
    metadata.uid === uid &&
    metadata.size === 65
  );
}

function sameSecretIdentity(left, right) {
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.mode === right.mode &&
    left.uid === right.uid &&
    left.nlink === right.nlink &&
    left.size === right.size &&
    left.mtimeMs === right.mtimeMs &&
    left.ctimeMs === right.ctimeMs
  );
}

export function readDemoPassword(path, filesystem = FILESYSTEM) {
  if (typeof path !== "string" || !path.startsWith("/") || path.length > 512) {
    refuse("password-file");
  }
  if (typeof process.getuid !== "function") refuse("password-file");
  const uid = process.getuid();
  if (
    filesystem === null ||
    typeof filesystem !== "object" ||
    ["closeSync", "fstatSync", "lstatSync", "openSync", "readSync"].some(
      (name) => typeof filesystem[name] !== "function",
    )
  ) refuse("password-file");
  let before;
  try {
    before = filesystem.lstatSync(path);
  } catch {
    refuse("password-file");
  }
  if (!exactSecretMetadata(before, uid)) refuse("password-file");

  let descriptor;
  let raw;
  let result;
  try {
    descriptor = filesystem.openSync(
      path,
      constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0),
    );
    const opened = filesystem.fstatSync(descriptor);
    if (!exactSecretMetadata(opened, uid) || !sameSecretIdentity(before, opened)) {
      refuse("password-file");
    }
    raw = Buffer.alloc(66);
    let bytesRead = 0;
    while (bytesRead < raw.length) {
      const count = filesystem.readSync(
        descriptor,
        raw,
        bytesRead,
        raw.length - bytesRead,
        null,
      );
      if (!Number.isSafeInteger(count) || count < 0 || count > raw.length - bytesRead) {
        refuse("password-file");
      }
      if (count === 0) break;
      bytesRead += count;
    }
    if (bytesRead !== 65 || raw[64] !== 0x0a) refuse("password-file");
    const value = raw.subarray(0, 64);
    if (!PASSWORD.test(value.toString("ascii"))) refuse("password-file");
    const after = filesystem.fstatSync(descriptor);
    if (!exactSecretMetadata(after, uid) || !sameSecretIdentity(opened, after)) {
      refuse("password-file");
    }
    result = Buffer.from(value);
    raw.fill(0);
    raw = undefined;
    return result;
  } catch (error) {
    raw?.fill(0);
    result?.fill(0);
    if (error instanceof BrowserContractError) throw error;
    refuse("password-file");
  } finally {
    raw?.fill(0);
    if (descriptor !== undefined) {
      try {
        filesystem.closeSync(descriptor);
      } catch {
        result?.fill(0);
        refuse("password-file");
      }
    }
  }
}
