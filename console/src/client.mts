/**
 * The typed client over the generated contract (CPR-8, ADR-0075 decision 3).
 *
 * `console/src/generated/api.ts` is derived from `docs/api/openapi.json`,
 * which the gateway derives from its own handlers. This turns that type
 * table into calls: an operation id, the path parameters its template
 * names, its query string and its body — all checked by the compiler
 * against the document, so a route that changes shape in Rust fails
 * `pnpm --filter @synveda/console build` rather than at a customer.
 *
 * # What it will not let you do
 *
 * - Call an operation the document does not declare (`OperationId` is
 *   `keyof Operations`).
 * - Send a body to an operation that takes none, or omit one that does.
 * - Omit an `Idempotency-Key` on an operation whose document says it is
 *   required. The generator emits `idempotent: true` for exactly those, and
 *   the option is required by the type there and forbidden everywhere else.
 *   A creation retried after a timeout is the failure this removes, and a
 *   client that could forget the header is a client that will.
 * - Leave a path placeholder unfilled: {@link fillPath} throws rather than
 *   sending a literal `{workspace_id}` to the gateway.
 *
 * # What it deliberately does not do
 *
 * It does not wrap routes absent from the contract. The document currently
 * covers 106 operations, including the immutable Skill, trusted MCP and OKF
 * exchange planes. Older governance calls still in `api.mts` remain visibly
 * hand-written until the public-contract convergence package declares them;
 * this facade never makes an undeclared call look generated.
 */

import { call, type Outcome } from "./api.mjs";
import { OPERATIONS, type OperationId, type Operations } from "./generated/api.js";

/** The transport's answer, narrowed to the operation's own success type. */
export type Answer<T> = { kind: "ok"; body: T } | Exclude<Outcome, { kind: "ok" }>;

type Op<K extends OperationId> = Operations[K];

/** The request body an operation takes, or `never` when it takes none. */
type BodyOf<K extends OperationId> = Op<K> extends { body: infer B } ? B : never;

/** The success body an operation answers with. */
type ResponseOf<K extends OperationId> = Op<K> extends { response: infer R } ? R : never;

type BodyOption<K extends OperationId> = Op<K> extends { body: unknown }
  ? { body: BodyOf<K> }
  : { body?: never };

type IdempotencyOption<K extends OperationId> = Op<K> extends { idempotent: true }
  ? { idempotencyKey: string }
  : { idempotencyKey?: never };

interface CommonOptions {
  /** Values for the `{name}` placeholders in the operation's path. */
  path?: Record<string, string>;
  /**
   * Query parameters. `undefined` drops the parameter entirely rather than
   * sending an empty one — a filter nobody set and a filter set to nothing
   * are different requests.
   */
  query?: Record<string, string | undefined>;
}

export type Options<K extends OperationId> = CommonOptions & BodyOption<K> & IdempotencyOption<K>;

/**
 * Substitutes a path template's placeholders.
 *
 * Throws on a placeholder nothing filled. That is a programming error the
 * type system cannot catch — the document does not encode which names a
 * template contains — so it is caught loudly here rather than sent to the
 * gateway, where it would arrive as a 404 that reads like a missing row.
 */
export function fillPath(template: string, params: Record<string, string> = {}): string {
  return template.replace(/\{([^}]+)\}/g, (_match, name: string) => {
    const value = params[name];
    if (value === undefined) {
      throw new Error(`${template}: no value for path parameter {${name}}`);
    }
    return encodeURIComponent(value);
  });
}

/** Renders a query object, dropping the parameters nobody set. */
export function queryString(query: Record<string, string | undefined> = {}): string {
  const params = new URLSearchParams();
  for (const [name, value] of Object.entries(query)) {
    if (value !== undefined) {
      params.set(name, value);
    }
  }
  const rendered = params.toString();
  return rendered.length === 0 ? "" : `?${rendered}`;
}

/** The loosened option shape {@link describe} works over. */
interface LooseOptions extends CommonOptions {
  body?: unknown;
  idempotencyKey?: string;
}

/**
 * The `/v1`-relative path and `RequestInit` an operation call becomes.
 *
 * Exported and pure so a test can assert the wire shape of a call — method,
 * path, headers, body — without a fetch, which is where the interesting
 * mistakes are. It takes the loosened options because the per-operation
 * shape is a conditional type the compiler cannot resolve for a generic
 * `K`; the *caller-facing* precision lives on {@link request}, which is the
 * signature anybody writes against.
 */
export function describe(
  operation: OperationId,
  options: LooseOptions = {},
): { path: string; init: RequestInit } {
  const declared = OPERATIONS[operation];
  if (!declared) {
    throw new Error(`${operation}: not an operation this contract declares`);
  }
  const idempotent = "idempotent" in declared;
  if (idempotent && options.idempotencyKey === undefined) {
    // The type already forbids this; the check is what catches an untyped
    // caller, and the document is the authority either way.
    throw new Error(`${operation}: the contract requires an Idempotency-Key`);
  }
  if (!idempotent && options.idempotencyKey !== undefined) {
    throw new Error(`${operation}: the contract declares no Idempotency-Key`);
  }
  // The document's paths are absolute (`/v1/...`) and `call` prepends the
  // base, so the prefix is stripped here rather than duplicated there.
  const absolute = fillPath(declared.path, options.path);
  const headers: Record<string, string> = {};
  if (options.body !== undefined) {
    headers["content-type"] = "application/json";
  }
  if (options.idempotencyKey !== undefined) {
    headers["idempotency-key"] = options.idempotencyKey;
  }
  return {
    path: `${absolute.replace(/^\/v1/, "")}${queryString(options.query)}`,
    init: {
      method: declared.method,
      headers,
      ...(options.body === undefined ? {} : { body: JSON.stringify(options.body) }),
    },
  };
}

/** Calls one contract operation. */
export async function request<K extends OperationId>(
  operation: K,
  options: Options<K>,
  fetchImpl: typeof fetch = fetch,
): Promise<Answer<ResponseOf<K>>> {
  const { path, init } = describe(operation, options as LooseOptions);
  const outcome = await call(path, init, fetchImpl);
  // The cast is the one place the generated type is asserted rather than
  // proven: the transport parses JSON and cannot know which shape it got.
  // It is sound exactly as far as the document is honest about the route,
  // which is what `crates/synveda-gateway/tests/openapi.rs` is for.
  return outcome as Answer<ResponseOf<K>>;
}

/**
 * A fresh idempotency key.
 *
 * `crypto.randomUUID` is in every browser that runs this bundle and needs
 * no dependency. A key is the client's claim that "this is that request
 * again", so it is minted once per *attempt the user makes* and reused
 * across retries of it — the callers here mint one when a form is
 * submitted, not when a request is sent.
 */
export function idempotencyKey(): string {
  return crypto.randomUUID();
}
