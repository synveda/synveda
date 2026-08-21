/**
 * Skills (CPR-8): the governed skills an agent may load at the selected
 * scope.
 *
 * The one primary page whose plane already exists. It reads `GET /v1/skills`
 * at the selected project's scope — the same route `synveda skill available`
 * uses — and shows what came back.
 *
 * Two things the listing means, and the page says both rather than leaving
 * them to be inferred. A skill the caller may not read **at their tier** is
 * omitted by the gateway rather than refused, so an empty list is "nothing
 * you may load here" and not "nothing exists". And a bundle with no quality
 * score was authored before the rubric existed — "not scored yet" rather
 * than "scored zero", which is a distinction a listing must not collapse.
 *
 * It is a hand-written call: skills are not on the OpenAPI contract yet
 * (`api.mts`), and this page moves onto the generated client with the rest
 * of `/v1` at Prompt 19.
 */

import { skillsAt } from "./api.mjs";
import { Loaded, useQuery, useRefresh } from "./Query.js";
import { PageHeading, useApp } from "./Shell.js";
import { whenOf } from "./people.mjs";

/** `GET /v1/skills`'s shape. Hand-written because the contract has none. */
interface SkillListing {
  scope_id: string;
  scope_path: string;
  skills: {
    name: string;
    description: string;
    sensitivity: string;
    quality?: { score?: number } | null;
    files: { path: string }[];
    updated_at: string;
  }[];
}

export function Skills() {
  const { project, workspace } = useApp();
  // The project's scope when there is one, else the workspace's: a skill is
  // resolved against a scope chain, and the nearest scope the reader has
  // selected is the one they mean.
  const scopeId = project?.scope_id ?? workspace?.scope_id ?? null;
  const cacheKey = `skills/${scopeId ?? "self"}`;
  const entry = useQuery(cacheKey, () => skillsAt(scopeId));
  const retry = useRefresh(cacheKey);

  return (
    <>
      <PageHeading route="skills" />
      <Loaded<SkillListing> entry={entry} what="the skills" onRetry={retry}>
        {(body) => (
          <>
            <p className="muted">At {body.scope_path}</p>
            {body.skills.length === 0 ? (
              <p className="muted">
                Nothing you may load here. A skill you are not cleared to read at your tier is
                omitted rather than refused, so this is not the same as "none exist".
              </p>
            ) : (
              <ul className="skills">
                {body.skills.map((skill) => (
                  <li key={skill.name}>
                    <strong>{skill.name}</strong>{" "}
                    <span className={`tag ${skill.sensitivity}`}>{skill.sensitivity}</span>
                    <div>{skill.description}</div>
                    <div className="muted">
                      {skill.files.length} file{skill.files.length === 1 ? "" : "s"} ·{" "}
                      {skill.quality && typeof skill.quality.score === "number"
                        ? `quality ${skill.quality.score}`
                        : "not scored yet"}{" "}
                      · updated {whenOf(skill.updated_at)}
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </>
        )}
      </Loaded>
    </>
  );
}
