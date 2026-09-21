/** OPS-12 / ADR-0117: Windows hooks reuse the CLI's native storage boundary. */
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { isAbsolute } from "node:path";

export interface PrivateResult {
  bytes?: string | null;
  digest?: string | null;
  names?: string[];
  id?: string;
  created?: boolean;
}

/** No shell, network, bearer lookup, payload arguments or child diagnostics. */
export function privateState(request: Record<string, unknown>): PrivateResult {
  const binary = process.env.SYNVEDA_CLI;
  if (binary === undefined || !isAbsolute(binary) || !binary.toLowerCase().endsWith(".exe")) {
    throw new Error("Windows private state requires an absolute SYNVEDA_CLI executable");
  }
  const input = JSON.stringify({ version: 1, request });
  if (Buffer.byteLength(input) > 24 * 1024 * 1024) throw new Error("private input exceeds its bound");
  const child = spawnSync(binary, ["private-state"], {
    input, encoding: "utf8", timeout: 3000, maxBuffer: 24 * 1024 * 1024,
    windowsHide: true, shell: false,
  });
  if (child.error || child.status !== 0) throw new Error("private storage refused");
  let response: { version?: unknown; result?: PrivateResult };
  try { response = JSON.parse(child.stdout); }
  catch { throw new Error("invalid private storage response"); }
  if (response.version !== 1 || response.result === null || typeof response.result !== "object") {
    throw new Error("unsupported private storage response");
  }
  return response.result;
}

export function privateBytes(result: PrivateResult): Buffer | undefined {
  if (result.bytes === null && result.digest === null) return undefined;
  if (typeof result.bytes !== "string" || typeof result.digest !== "string" || !/^[0-9a-f]{64}$/.test(result.digest)) {
    throw new Error("invalid private snapshot");
  }
  const bytes = Buffer.from(result.bytes, "base64");
  if (bytes.toString("base64") !== result.bytes || createHash("sha256").update(bytes).digest("hex") !== result.digest) {
    throw new Error("invalid private snapshot digest");
  }
  return bytes;
}
