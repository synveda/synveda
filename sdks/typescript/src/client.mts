import { randomBytes } from "node:crypto";
import { contract } from "./generated/contract.js";
import type { ApiErrorBody, OperationId, Operations } from "./generated/api.js";
export type * from "./generated/api.js";

export type TokenProvider = (refresh: boolean, signal: AbortSignal) => Promise<string>;
export interface ApiResponse<T> { data: T; status: number; traceId: string; retryAfter?: string }
export class ApiError extends Error {
  constructor(readonly status: number, readonly detail: ApiErrorBody,
    readonly traceId: string, readonly retryAfter?: string) {
    super(`Synveda request failed (${status}, ${detail.kind})`);
    this.name = "ApiError";
  }
}
export class TransportError extends Error {
  constructor(readonly kind: "cancelled" | "timeout" | "transport" | "invalid_response" | "response_too_large") {
    super(`Synveda request failed (${kind})`); this.name = "TransportError";
  }
}
interface CommonOptions {
  path?: Record<string, string>;
  query?: Record<string, string | number | boolean | undefined>;
  traceparent?: string;
  signal?: AbortSignal;
}
export type RequestOptions<K extends OperationId> = CommonOptions &
  (Operations[K] extends { body: infer B } ? { body: B } : { body?: never }) &
  (Operations[K] extends { idempotent: true } ? { idempotencyKey: string } : { idempotencyKey?: never });
type LooseOptions = CommonOptions & { body?: unknown; idempotencyKey?: string };
type Reply<K extends OperationId> = Operations[K]["response"];

export interface ClientOptions { timeoutMs?: number; maxResponseBytes?: number }
const MAX_BYTES = 8 * 1024 * 1024;

/** Authenticated public API client. Pagination is explicit; this owns no task lifecycle. */
export class Client {
  private readonly base: string;
  private readonly timeoutMs: number;
  private readonly maxBytes: number;
  constructor(baseUrl: string, private readonly token: TokenProvider, options: ClientOptions = {}) {
    let base: URL;
    try { base = new URL(baseUrl); }
    catch { throw new TypeError("Use a valid HTTP(S) gateway URL"); }
    if (!["https:", "http:"].includes(base.protocol) || base.username || base.password || base.search || base.hash) {
      throw new TypeError("Use an HTTP(S) gateway URL without credentials, query or fragment");
    }
    this.base = base.href.replace(/\/$/, "");
    this.timeoutMs = bounded(options.timeoutMs ?? 30_000, 1, 120_000, "timeoutMs");
    this.maxBytes = bounded(options.maxResponseBytes ?? MAX_BYTES, 1, 64 * 1024 * 1024, "maxResponseBytes");
  }

  async request<K extends OperationId>(operation: K, options: RequestOptions<K>): Promise<ApiResponse<Reply<K>>> {
    const route = contract.operations[operation];
    if (!route) throw new TypeError("Operation is outside this SDK contract slice");
    const url = this.url(operation, options);
    const body = options.body === undefined ? undefined : JSON.stringify(options.body);
    if (body !== undefined && Buffer.byteLength(body) > this.maxBytes) throw new TypeError("Request body too large");
    if (route.body && body === undefined) throw new TypeError("Request body is required");
    if (!route.accepts_body && body !== undefined) throw new TypeError("This operation takes no body");
    if (!route.idempotent && options.idempotencyKey !== undefined) throw new TypeError("This operation has no request idempotency contract");
    if (route.idempotent && !validHeader(options.idempotencyKey)) throw new TypeError("Idempotency key is required (1–200 ASCII characters)");
    const parent = options.traceparent ?? `00-${randomBytes(16).toString("hex")}-${randomBytes(8).toString("hex")}-01`;
    validateTrace(parent);
    const timeout = AbortSignal.timeout(this.timeoutMs);
    const signal = options.signal ? AbortSignal.any([timeout, options.signal]) : timeout;
    try {
      let bearer = await abortable(this.token(false, signal), signal);
      for (let attempt = 0; attempt < 2; attempt += 1) {
        if (!validHeader(bearer, 16_384)) throw new TypeError("Bearer provider returned an invalid token");
        const response = await fetch(url, {
          method: route.method, body, signal, redirect: "error", credentials: "omit",
          headers: { authorization: `Bearer ${bearer}`, traceparent: parent,
            "x-synveda-client": "synveda-typescript/0.1.0", accept: "application/json",
            ...(body === undefined ? {} : { "content-type": "application/json" }),
            ...(route.idempotent ? { "idempotency-key": options.idempotencyKey! } : {}) },
        });
        const data = await readJson(response, this.maxBytes);
        const retryAfter = response.headers.get("retry-after") ?? undefined;
        const serverTrace = response.headers.get("x-synveda-trace-id");
        const traceId = serverTrace !== null && /^[0-9a-f]{32}$/.test(serverTrace) && !/^0+$/.test(serverTrace)
          ? serverTrace : parent.slice(3, 35);
        if (response.ok) return { data: data as Reply<K>, status: response.status, traceId, retryAfter };
        // Refresh once only where replay cannot duplicate a governed effect.
        // Other retry decisions (including Retry-After) remain explicit to the application.
        if (response.status === 401 && attempt === 0 && (route.method === "GET" || route.idempotent)) {
          const refreshed = await abortable(this.token(true, signal), signal);
          if (refreshed !== bearer) { bearer = refreshed; continue; }
        }
        const detail = typeof data === "object" && data !== null && "kind" in data && typeof data.kind === "string"
          ? data as ApiErrorBody : { kind: "invalid_response" };
        throw new ApiError(response.status, detail, traceId, retryAfter);
      }
      throw new TransportError("transport");
    } catch (error) {
      if (options.signal?.aborted) throw new TransportError("cancelled");
      if (timeout.aborted) throw new TransportError("timeout");
      if (error instanceof ApiError || error instanceof TransportError) throw error;
      // HTTP libraries may retain request headers in exceptions. Do not expose those.
      throw new TransportError("transport");
    }
  }

  private url(operation: OperationId, options: LooseOptions): string {
    const route = contract.operations[operation];
    const path = options.path ?? {};
    const query = options.query ?? {};
    for (const [location, supplied] of [["path", path], ["query", query]] as const) {
      const parameters = route.parameters.filter((p) => p.location === location);
      for (const name of Object.keys(supplied)) {
        if (!parameters.some((p) => p.name === name)) throw new TypeError(`Unknown ${location} parameter: ${name}`);
      }
      for (const parameter of parameters) {
        if (parameter.required && supplied[parameter.name] === undefined) throw new TypeError(`Missing ${location} parameter: ${parameter.name}`);
      }
    }
    const suffix = route.path.replace(/\{([^}]+)\}/g, (_match, name: string) => {
      const value = path[name];
      if (typeof value !== "string" || !value || value === "." || value === "..") throw new TypeError(`Invalid path parameter: ${name}`);
      return encodeURIComponent(value).replace(/[!'()*]/g, (char) => `%${char.charCodeAt(0).toString(16).toUpperCase()}`);
    });
    const url = new URL(`${this.base}${suffix}`);
    for (const [name, value] of Object.entries(query)) {
      if (value !== undefined) {
        if (!["string", "number", "boolean"].includes(typeof value) || typeof value === "number" && !Number.isFinite(value)) throw new TypeError("Invalid query value");
        url.searchParams.set(name, String(value));
      }
    }
    return url.href;
  }
}

function bounded(value: number, min: number, max: number, name: string): number {
  if (!Number.isInteger(value) || value < min || value > max) throw new TypeError(`${name} is outside its supported bounds`);
  return value;
}
function validHeader(value: string | undefined, max = 200): value is string {
  return typeof value === "string" && value.length > 0 && value.length <= max && /^[\x21-\x7e]+$/.test(value);
}
function validateTrace(value: string): void {
  if (!/^00-[0-9a-f]{32}-[0-9a-f]{16}-0[01]$/.test(value) || /^0+$/.test(value.slice(3, 35)) || /^0+$/.test(value.slice(36, 52))) {
    throw new TypeError("Invalid W3C traceparent");
  }
}
async function abortable<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
  signal.throwIfAborted();
  let listener: () => void;
  const stopped = new Promise<never>((_resolve, reject) => {
    listener = () => reject(signal.reason); signal.addEventListener("abort", listener, { once: true });
  });
  try { return await Promise.race([promise, stopped]); }
  finally { signal.removeEventListener("abort", listener!); }
}
async function readJson(response: Response, maxBytes: number): Promise<unknown> {
  if (response.status === 204) return undefined;
  if (!response.body) throw new TransportError("invalid_response");
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    for (;;) {
      const chunk = await reader.read();
      if (chunk.done) break;
      size += chunk.value.byteLength;
      if (size > maxBytes) throw new TransportError("response_too_large");
      chunks.push(chunk.value);
    }
    try { return JSON.parse(Buffer.concat(chunks).toString("utf8")); }
    catch { throw new TransportError("invalid_response"); }
  } finally { await reader.cancel(); reader.releaseLock(); }
}
