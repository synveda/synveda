import { idempotencyKey, request } from "./client.mjs";
import type { TenantProfileView } from "./generated/api.js";
import type { SeedOutcome } from "./onboarding.mjs";

/**
 * The organisation's policy profile (ADR-0162). Whether the reader may choose
 * it is the server's answer (`me.onboarding.profile`), never inferred from a
 * role name. A profile is a canonical Configuration template, not an edition.
 */

/** The three canonical templates, by their generated contract names. */
export type ProfileTemplate = NonNullable<TenantProfileView["template"]>;

export interface ProfileChoice {
  template: ProfileTemplate;
  label: string;
  /** The policy pack the template's document selects. */
  pack: string;
  /** Who the profile suits. */
  suits: string;
  /** Who approves a later Configuration change under it. */
  approvals: string;
  /** What the reader must hear before confirming it. */
  warning?: string;
}

/** ADR-0162 requires this copy to be shown before the reader chooses. */
export const PROFILE_CHOICES: readonly ProfileChoice[] = [
  {
    template: "personal",
    label: "Personal",
    pack: "open-collaboration",
    suits: "One person, or a few people who each may change settings alone.",
    approvals:
      "An administrator can approve their own Configuration change. Changes at a workspace or project need no approval.",
  },
  {
    template: "team",
    label: "Team",
    pack: "standard",
    suits: "A small team that reviews each other's changes.",
    approvals:
      "A Configuration change needs approval from one administrator other than its author. With only one administrator, you cannot change Configuration until you add a second.",
  },
  {
    template: "enterprise",
    label: "Enterprise",
    pack: "regulated-strict",
    suits: "Regulated or larger organisations. This is the default.",
    approvals:
      "A Configuration change needs approval from two administrators other than its author, and a separate person to apply it.",
    warning:
      "Leaving Enterprise later needs that four-person review: two approvers, an applier and the author.",
  },
] as const;

export function profileChoice(template: ProfileTemplate): ProfileChoice {
  const choice = PROFILE_CHOICES.find((candidate) => candidate.template === template);
  if (!choice) throw new Error(`unknown profile template ${template}`);
  return choice;
}

/** Whether the server offers this reader the choice now. */
export function profileChoosable(
  profile: TenantProfileView | null | undefined,
): boolean {
  return profile?.state === "open" || profile?.state === "prepared";
}

/** What a reader who is not offered the choice is told. */
export function profileStatus(profile: TenantProfileView | null | undefined): string | null {
  if (!profile) return null;
  switch (profile.state) {
    case "chosen":
      return profile.template
        ? `Your organisation uses the ${profileChoice(profile.template).label} profile (${profile.policy_pack}). Changing it goes through review under ${profile.policy_pack}.`
        : `Your organisation's Configuration is set at the organisation and runs ${profile.policy_pack}. Changing it goes through review under ${profile.policy_pack}.`;
    case "unavailable":
      return `Your organisation has not chosen a profile, so it runs ${profile.policy_pack}, the default. Changes go through review under ${profile.policy_pack}.`;
    default:
      return null;
  }
}

/** The tenant-unique artifact name an adoption creates. */
export function profileArtifactName(template: ProfileTemplate): string {
  return `organisation-${template}-profile`;
}

/**
 * Create the template at the tenant root and bind it there, or bind the
 * artifact a `prepared` adoption left. Reports what the server decided.
 */
export async function adoptProfile(
  profile: TenantProfileView,
  rootScopeId: string,
  template: ProfileTemplate,
  fetchImpl: typeof fetch = fetch,
): Promise<SeedOutcome> {
  const { label } = profileChoice(template);
  const what = `The ${label} profile for your organisation`;
  const why = (answer: { kind: string; message?: string }) =>
    answer.kind === "unauthenticated"
      ? "your session has expired"
      : (answer.message ?? "the gateway did not say why");

  let artifactId: string;
  if (profile.state === "prepared" && profile.template === template && profile.artifact_id) {
    artifactId = profile.artifact_id;
  } else {
    const listed = await request("list_configuration_templates", {}, fetchImpl);
    const source =
      listed.kind === "ok"
        ? listed.body.templates.find((candidate) => candidate.name === template)
        : undefined;
    if (!source) {
      return {
        kind: "refused",
        what,
        why: listed.kind === "ok" ? "the server did not offer that template" : why(listed),
      };
    }
    const created = await request(
      "create_configuration",
      {
        idempotencyKey: idempotencyKey(),
        body: {
          governing_scope_id: rootScopeId,
          name: profileArtifactName(template),
          source_template: template,
          document: source.document,
        },
      },
      fetchImpl,
    );
    if (created.kind !== "ok") return { kind: "refused", what, why: why(created) };
    if (created.body.outcome === "pending_review") {
      return { kind: "pending", what, changeId: created.body.change_id };
    }
    if (created.body.outcome !== "applied" || !created.body.artifact_id) {
      return { kind: "refused", what, why: "the governed change was rejected" };
    }
    artifactId = created.body.artifact_id;
  }

  const bound = await request(
    "create_configuration_binding",
    {
      idempotencyKey: idempotencyKey(),
      body: { scope_id: rootScopeId, artifact_id: artifactId, enabled: true },
    },
    fetchImpl,
  );
  if (bound.kind !== "ok") return { kind: "refused", what, why: why(bound) };
  switch (bound.body.outcome) {
    case "applied":
      return { kind: "applied", what };
    case "pending_review":
      return { kind: "pending", what, changeId: bound.body.change_id };
    default:
      return { kind: "refused", what, why: "the governed change was rejected" };
  }
}
