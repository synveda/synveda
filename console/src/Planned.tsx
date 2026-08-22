/**
 * The planes this epoch has not built yet (CPR-8, ADR-0075 decision 7).
 *
 * Knowledge, New Learnings and Tools are in the primary navigation because
 * they are what the product *is* — and none of them has an API at this
 * commit. The context-platform programme's own order says why: candidates
 * and knowledge versions are the next two prompts, the tool registry two
 * stages later. This page is what those three routes render meanwhile.
 *
 * **Sessions left this page at CPR-10** (ADR-0076), which is what the page is
 * for: an entry here is a debt with a name, and it is paid by deleting the
 * entry rather than by editing it.
 *
 * # Why a page rather than a hidden nav item
 *
 * Because the shape of the product is the thing this feature exists to
 * show, and a navigation that grew items as the backend caught up would
 * teach every reader a different shape. The honest cost of showing them is
 * one page that says what will be here and which piece of work brings it —
 * and the honest alternative, a nav item that renders an empty list, is
 * worse: an empty list is indistinguishable from a plane that works and has
 * nothing in it, which is precisely the wrong thing to tell somebody whose
 * agent has been running all week.
 *
 * So: no fabricated rows, no placeholder counts, no "0 sessions". A
 * sentence, and what it is waiting on.
 */

import { PageHeading } from "./Shell.js";
import type { RouteId } from "./routes.mjs";

/** What each planned plane will hold, and what delivers it. */
const PLANNED: Record<string, { what: string; owed: string }> = {
  knowledge: {
    what:
      "What has been reviewed and published, as immutable versions: a stable id, a revision " +
      "chain, and the proposal that put each one there.",
    owed: "knowledge versions and the candidate → knowledge promotion path",
  },
  learnings: {
    what:
      "What your sessions extracted and nobody has stood behind yet — the candidates, on " +
      "their own side of the trust boundary, where you accept, edit or drop them.",
    owed: "the candidate plane, separated from published knowledge",
  },
  tools: {
    what:
      "The tool registry: which tools your agents may call here, at which version, under " +
      "whose review.",
    owed: "tool versions, governed like skills",
  },
};

export function Planned({ route }: { route: RouteId }) {
  const plane = PLANNED[route];
  return (
    <>
      <PageHeading route={route} />
      <section className="planned">
        <p>{plane?.what}</p>
        <p className="muted">
          This plane is not built yet. It is waiting on {plane?.owed}. Nothing is shown here
          because there is nothing to show — not an empty list, which would look the same as a
          quiet week.
        </p>
      </section>
    </>
  );
}
