/**
 * Rendered markup, as the text a reader sees (CNSL-1, test support).
 *
 * The parity suite asserts that the console **names** a fact, and a fact is
 * named in the text of the page rather than in its tags. Reducing the
 * markup the way a browser lays it out — block elements ending a line,
 * inline ones running on with a space — is what makes an assertion about
 * "the row for this finding" possible without the suite knowing which
 * element that row happens to be.
 *
 * It lives here rather than inside the test file because the same reduction
 * is what a copy-paste out of the browser produces, which is the thing the
 * assertions are really about: whether a review pasted into a ticket still
 * carries the verdict.
 */

/** Elements that end a line, because a browser puts them on their own. */
const BLOCK =
  /<\/?(?:div|p|li|ul|ol|section|article|h[1-6]|dl|dt|dd|pre|tr|table|thead|tbody|header|footer|nav|form|label|textarea|br)\b[^>]*>/gi;

/**
 * The named entities React emits. React escapes exactly these five in text
 * (`&`, `<`, `>`, `"`, `'`) and nothing else, so this is the whole set
 * rather than an abbreviation of one.
 */
const ENTITIES: Record<string, string> = {
  "&amp;": "&",
  "&lt;": "<",
  "&gt;": ">",
  "&quot;": '"',
  "&#x27;": "'",
  "&#39;": "'",
};

export function toText(html: string): string {
  return html
    .replace(BLOCK, "\n")
    .replace(/<[^>]+>/g, " ")
    .replace(/&(?:amp|lt|gt|quot|#x27|#39);/g, (entity) => ENTITIES[entity] ?? entity)
    .split("\n")
    // Collapse runs of whitespace *within* a line but keep the lines apart:
    // the assertions care that a severity, a path and a verdict share one
    // row, which is a claim a joined-up blob could not falsify.
    .map((line) => line.replace(/[ \t ]+/g, " ").trim())
    .filter((line) => line.length > 0)
    .join("\n");
}

/** The lines of `toText`, for assertions about what shares a row. */
export function toLines(html: string): string[] {
  const text = toText(html);
  return text.length === 0 ? [] : text.split("\n");
}
