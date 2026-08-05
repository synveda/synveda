/**
 * A line diff for the review screen (CNSL-1).
 *
 * The console's own, and deliberately not a port of the CLI's. ADR-0056
 * draws the line at **verdicts and sentences** — the things with one right
 * answer, which the gateway now serves — and leaves layout to each client,
 * because a terminal and a browser have genuinely different affordances. A
 * diff is layout: the CLI renders a memory's canonical object field by
 * field because eighty columns cannot hold two versions side by side, and
 * a browser is under no such constraint.
 *
 * What both surfaces owe the corpus is that the bytes are *named* — every
 * line of the baseline, of the proposal, and of the record where it has
 * drifted. How they are arranged is theirs.
 */

export type Mark = "added" | "removed" | "same";

export interface DiffLine {
  mark: Mark;
  text: string;
}

/**
 * Above this many lines on either side the quadratic table is abandoned.
 *
 * A skill bundle is a handful of files a person wrote, so this is not a
 * limit anybody reaches by writing a skill; it is a limit that stops a
 * pathological input from locking the tab. Crossing it degrades to
 * "everything removed, everything added", which is honest — it is what a
 * diff of two unrelated files looks like anyway — rather than slow.
 */
const MAX_LINES = 2000;

/**
 * Splits text into lines for diffing.
 *
 * A trailing newline is dropped rather than becoming an empty final line:
 * every file in the corpus ends in one, and rendering a blank row at the
 * bottom of every diff would be an artefact of the format rather than
 * anything about the change.
 */
export function lines(text: string): string[] {
  const split = text.split("\n");
  if (split.length > 1 && split[split.length - 1] === "") {
    split.pop();
  }
  return split;
}

/**
 * The diff, as rows to render.
 *
 * `before` is `null` for an addition — material the channel does not hold
 * at all — which renders as an addition of every line rather than as a
 * removal of nothing. That is the CLI's rule too (ADR-0035's reasoning in
 * `crates/synveda-cli/src/diff.rs`): a reviewer admitting new content
 * should not have to read a column of absences to learn there was no old
 * version.
 */
export function diffLines(before: string | null, after: string): DiffLine[] {
  const right = lines(after);
  if (before === null) {
    return right.map((text) => ({ mark: "added" as const, text }));
  }
  const left = lines(before);
  if (left.length > MAX_LINES || right.length > MAX_LINES) {
    return [
      ...left.map((text) => ({ mark: "removed" as const, text })),
      ...right.map((text) => ({ mark: "added" as const, text })),
    ];
  }
  return walk(left, right, common(left, right));
}

/** Lengths of the longest common subsequence of every suffix pair. */
function common(left: string[], right: string[]): number[][] {
  const table: number[][] = Array.from({ length: left.length + 1 }, () =>
    new Array<number>(right.length + 1).fill(0),
  );
  for (let i = left.length - 1; i >= 0; i -= 1) {
    for (let j = right.length - 1; j >= 0; j -= 1) {
      table[i][j] =
        left[i] === right[j]
          ? table[i + 1][j + 1] + 1
          : Math.max(table[i + 1][j], table[i][j + 1]);
    }
  }
  return table;
}

/**
 * Walks the table into rows.
 *
 * Removals are emitted before additions at the same position, so a changed
 * line reads as the old one struck through and the new one under it, which
 * is the order every diff a reviewer has ever read uses.
 */
function walk(left: string[], right: string[], table: number[][]): DiffLine[] {
  const rows: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < left.length && j < right.length) {
    if (left[i] === right[j]) {
      rows.push({ mark: "same", text: left[i] });
      i += 1;
      j += 1;
    } else if (table[i + 1][j] >= table[i][j + 1]) {
      rows.push({ mark: "removed", text: left[i] });
      i += 1;
    } else {
      rows.push({ mark: "added", text: right[j] });
      j += 1;
    }
  }
  for (; i < left.length; i += 1) {
    rows.push({ mark: "removed", text: left[i] });
  }
  for (; j < right.length; j += 1) {
    rows.push({ mark: "added", text: right[j] });
  }
  return rows;
}
