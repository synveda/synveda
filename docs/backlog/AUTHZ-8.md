# AUTHZ-8: Delegated agent authority

## Problem and evidence

At `8a22b0e5` an agent reaches Synveda in one of two ways, and neither lets a
person or administrator give one agent less authority than its owner, or stop
one agent without stopping others.

- **Interactive agents act as the person.** Claude Code hooks, the Codex and
  Copilot adapters, `synveda mcp` and SDK clients present the person's own
  bearer from `synveda login`. The MCP server's own source notes that the
  gateway cannot tell one of its tool calls from a `synveda recall`
  (`crates/synveda-cli/src/mcp.rs`). Cedar therefore decides every agent
  request with the person's full grants. Sessions record harness identifiers
  (`crates/synveda-types/src/session.rs`), so evidence names the client, but
  authority does not depend on it.
- **Service identities have no owner or lifetime.** ADR-0018 registers an
  IdP client-credentials subject at an anchor scope with an optional display
  name, and confines its tokens to the anchor's subtree. Nothing records a
  responsible person, an expiry or a review date, and revocation deletes the
  row (`crates/synveda-gateway/src/service_identities.rs`).
- **Revocation is per person, not per agent.** The session and token hygiene
  work lets a person revoke their own console sessions and the presented
  bearer or its refresh family, on the bundled Keycloak profile only and in
  no published release. Every local agent on a machine shares the CLI
  credential, so revoking it stops all of them. Administrator revocation is
  deferred.

The surrounding practice moved in 2026. The external facts below come from a
survey checked on 2026-11-01:

- OWASP's Top 10 for Agentic Applications (2025-12-09) lists ASI03 Identity
  and Privilege Abuse
  ([OWASP](https://genai.owasp.org/resource/owasp-top-10-for-agentic-applications-for-2026/)).
- Microsoft Entra Agent ID is generally available, with a human sponsor per
  agent, lifecycle workflows against orphaned agents, separate on-behalf-of
  and autonomous access, and Conditional Access that can block a risky agent
  ([Entra](https://learn.microsoft.com/en-us/entra/agent-id/whats-new-agent-id)).
- Okta Agent SSO, built on the identity assertion authorization grant
  (ID-JAG), became generally available on 2026-08-24
  ([Okta](https://okta.com/newsroom/press-releases/okta-brings-first-class-identity-to-ai-agents-with-agent-sso/)).
- ID-JAG is an IETF OAuth working-group draft. The agent-specific drafts for
  actor chains, attenuated delegation and agent revocation are individual
  submissions the working group has not adopted
  ([datatracker](https://datatracker.ietf.org/group/oauth/documents/)), so no
  standard token format for attenuation exists yet.
- The Five Eyes guidance "Careful Adoption of Agentic AI Services"
  (2026-05-01) asks for least privilege and strong agent identity
  ([CISA](https://content.govdelivery.com/accounts/USDHSCISA/bulletins/41544ff)).

## Scope

1. **ADR.** Decide:
   - how an interactive agent is identified to the gateway separately from its
     person: a per-client OAuth client at the IdP, an IdP token exchange that
     names the agent, or a Synveda-issued delegation the client presents
     beside the person's bearer;
   - the attenuation rule: an agent's effective authority is the intersection
     of its person's grants and the agent's own grant, so it can only narrow;
   - how audit records "this agent, for this person" without content;
   - which IdP mechanisms are qualified first.
2. **Agent grants.** A person can create, list and revoke a named grant for
   one client: scopes, actions, maximum sensitivity and expiry. An
   administrator can list and revoke any agent grant in their scope.
3. **Service identity lifecycle.** Add a responsible person, an expiry and a
   review date to service identities, refuse expired ones, and keep a
   content-free revocation record instead of only deleting the row.
4. **Stop one agent.** Revoking one agent grant or service identity refuses
   that agent's next request and leaves the person's other agents working.
5. Surface agent grants and service identity owners in the console and CLI.

## Non-goals

- A Synveda-specific token format published as a standard, ahead of the IETF
  work.
- Multi-hop agent-to-agent delegation chains. One agent acting for one person
  or one service is the first contract.
- Widening any decision: an agent grant never adds authority its person lacks.
- Replacing an IdP's agent registry. Synveda consumes the IdP's identity and
  adds governed authority.
- SAML.

## Architecture seam

Cedar stays the only decision point. The agent becomes a principal attribute
or second entity in the request, and a policy pack decides with both; forced
RLS and VedaFlow are unchanged. Agent grants are governed, tenant-scoped,
forced-RLS rows changed only through audited commands. Revocation reuses the
bounded ledger and credential-verification path the session and token hygiene
work added. Clients learn to present the agent's identity; nothing else in the
adapters changes.

## Acceptance criteria

- The ADR is accepted before implementation.
- An agent with a narrower grant is refused outside it while its person, using
  another client, is allowed; the refusal leaks no resource existence.
- Audit for every agent request names the person, the agent and the grant,
  and the audit chain verifies.
- An expired service identity or agent grant is refused on the next request.
- Revoking one agent stops it within the measured revocation bound and leaves
  the person's other agents and console session working.
- A service identity without a responsible person cannot be registered once
  the owner enables that rule.

## Required tests

- Cedar policy tests for the intersection rule, including an attempt to widen.
- Two-agent, one-person database tests for grant, expiry and revocation.
- Gateway tests on every agent route (sessions, context, Knowledge, MCP tools)
  for allow, deny, expiry and revoke.
- Audit chain and content-free payload tests for agent attribution.
- A live run on each qualified IdP mechanism.

## Rollout and rollback

Ship agent attribution in audit first, because it changes no decision. Then
ship agent grants as opt-in per tenant; a tenant without them keeps today's
behaviour. Rollback disables agent grant enforcement for a tenant and keeps
attribution and existing grants as evidence.

## Dependencies

The owner must choose the identification mechanism and the first IdPs to
qualify. [Session & token hygiene](AUTH-6.md) supplies the revocation ledger.
The [additional API transport](ADPT-3.md) uses MCP authorization, whose
enterprise-managed path carries an IdP-issued agent assertion; the two must
agree on how an agent is identified. [Governed admin-plane mutation](AUTHZ-7.md)
decides whether administrator changes to agent grants need review.
