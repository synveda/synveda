/**
 * The reduction the parity suite's assertions stand on.
 *
 * Worth its own tests because a broken `toText` would not fail loudly — it
 * would make the parity suite **weaker**. Every line-level assertion there
 * is of the form "the severity, the path and the verdict share one row",
 * and a reduction that joined the whole page into a single line would
 * satisfy all of them at once while proving nothing.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import { toLines, toText } from "./text.mjs";

test("block elements end a line and inline ones do not", () => {
  // The claim the parity suite depends on: two findings are two rows, and
  // a severity beside a rule is one.
  assert.deepEqual(
    toLines("<li><span>high</span> <code>a.sh:3</code> <span>blocks</span></li><li>notice</li>"),
    ["high a.sh:3 blocks", "notice"],
  );
});

test("a page cannot collapse into one line", () => {
  const collapsed = toLines("<p>one</p><p>two</p><div>three</div>");
  assert.deepEqual(collapsed, ["one", "two", "three"]);
});

test("the entities React escapes come back as themselves", () => {
  // React escapes exactly these in text, so a shortfall sentence carrying a
  // quote or an ampersand has to survive the round trip or the parity
  // assertion would fail on the sentence rather than on the surface.
  assert.equal(
    toText("<p>a &amp; b &lt; c &gt; d &quot;e&quot; &#x27;f&#39;</p>"),
    `a & b < c > d "e" 'f'`,
  );
});

test("runs of whitespace collapse within a line but lines stay apart", () => {
  assert.deepEqual(toLines("<p>a   \t b</p>\n\n<p>c</p>"), ["a b", "c"]);
});

test("empty markup is no lines rather than one empty one", () => {
  assert.deepEqual(toLines(""), []);
  assert.deepEqual(toLines("<div></div>"), []);
});

test("attributes are not mistaken for text", () => {
  assert.equal(toText('<span class="severity high" data-x="blocks">notice</span>'), "notice");
});
