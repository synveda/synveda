/**
 * The console's half of the explorer parity corpus (CNSL-2, ADR-0058
 * decision 10; the scope re-cut CPR-7).
 *
 * The same payloads the gateway answers, recorded from the real gateway by
 * `crates/synveda-gateway/tests/explorer.rs`. Two renderers that agree on
 * the day they are written is not parity; it is a coincidence with a
 * maintenance schedule, and this is what makes the divergence fail a test.
 *
 * **It already earned its place.** The first draft of the CLI rendered an
 * inherited origin as `assigned at <uuid>` where this bundle rendered
 * `inherited` — two words for one fact, and nothing anywhere would have
 * noticed. ADR-0056 decision 5 draws the line the fix followed: the word
 * has one right answer and is shared; the id beside it is layout, so the
 * terminal keeps it and the browser does not need it.
 *
 * A fact is a **substring the rendering must contain**, never a line either
 * surface must produce.
 */

import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { renderToStaticMarkup } from "react-dom/server";

import {
  describeEnd,
  mayDo,
  mayRead,
  deniedCount,
  type Capabilities,
  type LapseListing,
} from "./explorer.mjs";
import type { EffectiveConfigurationView } from "./generated/api.js";
import { toText } from "./text.mjs";

const CASES = [
  "configuration-inherited",
  "capabilities-with-denial",
  "lapses-standing-and-ended",
] as const;

const FIXTURES = join(dirname(fileURLToPath(import.meta.url)), "..", "fixtures", "explorer");

function corpus(name: string): { asked_about: string; payload: unknown } {
  return JSON.parse(readFileSync(join(FIXTURES, `${name}.json`), "utf8"));
}

function facts(name: string): { must_name: string[]; must_not_name: string[] } {
  return JSON.parse(readFileSync(join(FIXTURES, `${name}.facts.json`), "utf8"));
}

/**
 * Renders a case the way the panel that owns it does.
 *
 * The panels themselves take an `Outcome` and a `Node` and live inside a
 * component tree; what parity is about is the *content*, so each case is
 * rendered through the same pure helpers `Explorer.tsx` renders through.
 * A panel that stopped using them would fail its own component test, and a
 * helper that stopped naming a fact fails here.
 */
function render(name: string, asked: string, payload: unknown): string {
  switch (name) {
    case "configuration-inherited": {
      const configuration = payload as EffectiveConfigurationView;
      const inherited = configuration.binding_scope_id !== asked;
      return toText(
        renderToStaticMarkup(
          <p>
            <strong>{configuration.document.policy_pack}</strong>{" "}
            <span>{configuration.fail_safe ? "enterprise fail-safe" : inherited ? "inherited" : "bound here"}</span>{" "}
            <span>{configuration.version_id}</span>{" "}
            <span>{configuration.content_hash}</span>
          </p>,
        ),
      );
    }
    case "capabilities-with-denial": {
      const caps = payload as Capabilities;
      return toText(
        renderToStaticMarkup(
          <div>
            <ul>
              {mayDo(caps).map((action) => (
                <li key={action}>{action}</li>
              ))}
            </ul>
            <dl>
              {mayRead(caps).map(([action, tiers]) => (
                <div key={action}>
                  <dt>{action}</dt>
                  <dd>{tiers.join(", ")}</dd>
                </div>
              ))}
            </dl>
            <p>
              {deniedCount(caps)} action(s) denied. Decided under{" "}
              {caps.pack ? `${caps.pack.name}@${caps.pack.version}` : "the pack in force"} — a
              forecast, not a grant: every act decides again at its own seam.
            </p>
          </div>,
        ),
      );
    }
    case "lapses-standing-and-ended": {
      const listing = payload as LapseListing;
      return toText(
        renderToStaticMarkup(
          <ul>
            {listing.lapses.map((lapse) => (
              <li key={lapse.id}>
                <span>{lapse.outcome}</span> {lapse.action}{" "}
                {describeEnd(lapse.grantee_scope_path, lapse.grantee_scope_id)} →{" "}
                {describeEnd(lapse.target_scope_path, lapse.target_scope_id)}
                <div>{lapse.reason}</div>
              </li>
            ))}
          </ul>,
        ),
      );
    }
    default:
      throw new Error(`no renderer for ${name}`);
  }
}

for (const name of CASES) {
  test(`the console names every fact the corpus requires: ${name}`, () => {
    const { asked_about, payload } = corpus(name);
    const rendered = render(name, asked_about, payload);
    const required = facts(name);

    for (const fact of required.must_name) {
      assert.ok(
        rendered.includes(fact),
        `${name}: the console never names \`${fact}\`:\n\n${rendered}`,
      );
    }
    for (const fact of required.must_not_name) {
      assert.ok(
        !rendered.includes(fact),
        `${name}: the console names \`${fact}\`, which this case says it must not — ` +
          `a denied action rendered as something the reader may do:\n\n${rendered}`,
      );
    }
  });
}

test("every recorded case is answered here", () => {
  // A case added to the corpus and not to `CASES` is a case that proves
  // nothing — the same guard the CLI's suite keeps, because the corpus is
  // shared and either surface could be the one that forgot.
  const recorded = readdirSync(FIXTURES)
    .filter((file) => file.endsWith(".json") && !file.endsWith(".facts.json"))
    .map((file) => file.replace(/\.json$/, ""))
    .sort();
  assert.deepEqual(recorded, [...CASES].sort());
});
