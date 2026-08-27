/** Pure client-side envelope and presentation rules for OKF exchange (CPR-28). */

import type {
  OkfImportJobView,
  OkfMappingView,
  PlanOkfImportBody,
} from "./generated/api.js";

export const OKF_VERSION = "0.2";
export const OKF_SPEC_COMMIT = "ad30107c31c06aec8a7d5636e0d1058118604e6f";
const MAX_ARCHIVE_BYTES = 1_500_000;
const MAX_ARTIFACT_BYTES = 262_144;
const MAX_ARTIFACTS = 2_000;
const MAX_EXPANDED_BYTES = 4_000_000;

export interface UploadFile {
  name: string;
  size: number;
  webkitRelativePath?: string;
  arrayBuffer(): Promise<ArrayBuffer>;
}

export interface ClassificationCounts {
  addition: number;
  update: number;
  duplicate: number;
  conflict: number;
}

/** Count only the four persisted dry-run outcomes; never infer candidates. */
export function classificationCounts(mappings: OkfMappingView[]): ClassificationCounts {
  const counts: ClassificationCounts = { addition: 0, update: 0, duplicate: 0, conflict: 0 };
  for (const mapping of mappings) {
    if (mapping.classification in counts) {
      counts[mapping.classification as keyof ClassificationCounts] += 1;
    }
  }
  return counts;
}

/** Human state for immutable import history. */
export function importProgress(job: OkfImportJobView): string {
  switch (job.state) {
    case "planned":
      return "Dry-run complete · no candidates created";
    case "materialized":
      return `Candidate materialisation complete · ${job.candidate_count} reviewable`;
    case "failed":
      return "Import failed before publication";
    default:
      return `Import state: ${job.state}`;
  }
}

/**
 * Package browser-selected files into the generated public request.
 *
 * Browser paths are untrusted. This repeats the path grammar before a request
 * and excludes `.git` administration data. The gateway then performs the
 * authoritative bounded archive/Markdown/YAML validation again.
 */
export async function importBody(
  files: readonly UploadFile[],
  sourceLocator: string,
  sourceRevision: string,
): Promise<PlanOkfImportBody> {
  const locator = sourceLocator.trim();
  if (!locator) throw new Error("Source name is required.");
  if (files.length === 0) throw new Error("Choose an OKF directory or archive.");
  const revision = sourceRevision.trim();
  const archive = files.length === 1 ? archiveEncoding(files[0]?.name ?? "") : null;
  if (archive) {
    if (revision) throw new Error("A source revision applies only to checked-out directory files.");
    const file = files[0] as UploadFile;
    if (file.size > MAX_ARCHIVE_BYTES) {
      throw new Error(`The OKF archive exceeds ${MAX_ARCHIVE_BYTES} bytes.`);
    }
    return {
      source_kind: archive.kind,
      source_locator: locator,
      encoding: archive.encoding,
      entries: [],
      archive_base64: bytesToBase64(new Uint8Array(await file.arrayBuffer())),
    };
  }

  if (files.length > MAX_ARTIFACTS) {
    throw new Error(`The OKF directory exceeds ${MAX_ARTIFACTS} selected files.`);
  }
  const seen = new Set<string>();
  const entries: NonNullable<PlanOkfImportBody["entries"]> = [];
  let totalBytes = 0;
  for (const file of files) {
    const raw = file.webkitRelativePath?.trim() || file.name;
    const logicalPath = normaliseLogicalPath(raw);
    if (logicalPath.split("/").includes(".git")) continue;
    if (file.size > MAX_ARTIFACT_BYTES) {
      throw new Error(`${logicalPath} exceeds ${MAX_ARTIFACT_BYTES} bytes.`);
    }
    totalBytes += file.size;
    if (totalBytes > MAX_EXPANDED_BYTES) {
      throw new Error(`The OKF directory exceeds ${MAX_EXPANDED_BYTES} bytes.`);
    }
    const folded = logicalPath.toLocaleLowerCase("en-US");
    if (seen.has(folded)) throw new Error(`Two selected files resolve to ${logicalPath}.`);
    seen.add(folded);
    entries.push({
      logical_path: logicalPath,
      kind: "file",
      content_base64: bytesToBase64(new Uint8Array(await file.arrayBuffer())),
    });
  }
  if (entries.length === 0) throw new Error("The selection contains no OKF files outside .git.");
  entries.sort((left, right) =>
    left.logical_path < right.logical_path ? -1 : left.logical_path > right.logical_path ? 1 : 0,
  );
  return {
    source_kind: revision ? "git" : "directory",
    source_locator: locator,
    source_revision: revision || undefined,
    encoding: "entries",
    entries,
    archive_base64: null,
  };
}

/** A safe bundle-relative path. The server repeats this check. */
export function normaliseLogicalPath(raw: string): string {
  if (!raw || raw.startsWith("/") || raw.includes("\\") || raw.includes("\0")) {
    throw new Error(`Unsafe OKF path: ${raw || "(empty)"}.`);
  }
  const parts = raw.split("/");
  if (parts.some((part) => !part || part === "." || part === "..")) {
    throw new Error(`Unsafe OKF path: ${raw}.`);
  }
  return parts.join("/");
}

function archiveEncoding(name: string): { kind: string; encoding: string } | null {
  const lower = name.toLocaleLowerCase("en-US");
  if (lower.endsWith(".zip")) return { kind: "zip", encoding: "zip" };
  if (lower.endsWith(".tar.gz") || lower.endsWith(".tgz")) {
    return { kind: "tar", encoding: "tar_gzip" };
  }
  if (lower.endsWith(".tar")) return { kind: "tar", encoding: "tar" };
  return null;
}

function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  const chunk = 0x8000;
  for (let offset = 0; offset < bytes.length; offset += chunk) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + chunk));
  }
  return btoa(binary);
}
