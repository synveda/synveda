/** The organisation's profile choice, in onboarding and Configuration. */

import { useState } from "react";

import type { TenantProfileView } from "./generated/api.js";
import type { SeedOutcome } from "./onboarding.mjs";
import {
  PROFILE_CHOICES,
  adoptProfile,
  profileChoice,
  type ProfileTemplate,
} from "./profile.mjs";

export function ProfileChoice({
  profile,
  rootScopeId,
  onDone,
  onSkip,
}: {
  profile: TenantProfileView;
  rootScopeId: string;
  onDone: (outcome: SeedOutcome) => void;
  /** Offered during onboarding only: keep the default and decide later. */
  onSkip?: () => void;
}) {
  const prepared =
    profile.state === "prepared" && profile.template ? profile.template : null;
  const [chosen, setChosen] = useState<ProfileTemplate>(prepared ?? "enterprise");
  const [busy, setBusy] = useState(false);
  const choice = profileChoice(chosen);

  const confirm = async () => {
    setBusy(true);
    const outcome = await adoptProfile(profile, rootScopeId, chosen);
    setBusy(false);
    onDone(outcome);
  };

  return (
    <section aria-labelledby="profile-heading">
      <h2 id="profile-heading">How should your organisation review changes?</h2>
      <p className="muted">
        Choose a profile for your whole organisation. Every workspace uses it.
        It decides who approves later changes to settings such as Capture,
        Context and review rules. Until you choose, Synveda uses Enterprise,
        the strictest profile.
      </p>
      {prepared ? (
        <p>
          An earlier choice of <strong>{profileChoice(prepared).label}</strong>{" "}
          was saved but not finished. Finish it to apply{" "}
          {profileChoice(prepared).label} to your organisation. Choosing a
          different profile now goes through review.
        </p>
      ) : (
        <fieldset className="choice">
          <legend>Profile</legend>
          {PROFILE_CHOICES.map((option) => (
            <label key={option.template}>
              <input
                type="radio"
                name="profile"
                checked={chosen === option.template}
                onChange={() => setChosen(option.template)}
              />
              <strong>{option.label}</strong>{" "}
              <span className="muted">({option.pack})</span>
              <div className="muted">{option.suits}</div>
              <div>{option.approvals}</div>
            </label>
          ))}
        </fieldset>
      )}
      {choice.warning ? (
        <div className="banner warning" role="note">
          {choice.warning}
        </div>
      ) : null}
      <p className="muted">
        Only an administrator for the whole organisation can make this choice
        without review, and only until a profile is set. After that, changing
        the profile goes through review under the profile you chose.
      </p>
      <p>
        <button
          type="button"
          className="primary"
          disabled={busy}
          onClick={() => void confirm()}
        >
          {busy
            ? "Applying…"
            : prepared
              ? `Finish choosing ${choice.label}`
              : `Use ${choice.label}`}
        </button>{" "}
        {onSkip ? (
          <button type="button" onClick={onSkip} disabled={busy}>
            Decide later
          </button>
        ) : null}
      </p>
      {onSkip ? (
        <p className="muted">
          Deciding later keeps Enterprise, the default. Any administrator for
          the whole organisation can choose under Advanced ▸ Configuration
          until a profile is set.
        </p>
      ) : null}
    </section>
  );
}
