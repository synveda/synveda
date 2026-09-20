# Security policy

Report suspected vulnerabilities privately through
[GitHub's Report a vulnerability form](https://github.com/synveda/synveda/security/advisories/new).
Private vulnerability reporting is enabled for this repository (verified
2026-09-20). A GitHub account is required. Do not put exploit details, secrets,
tenant content or unredacted logs in public issues or pull requests. If the
private form is unavailable, do not fall back to a public report; no alternative
private contact is currently published.

Include the affected version or commit, deployment shape, expected and observed
behavior, security impact, and a minimal synthetic reproduction where possible.
Share only the information needed to reproduce the issue. Never test against
another person's deployment without permission.

Synveda is a pre-1.0 evaluation release. There is no published supported-version
window, backport commitment or response SLA. The current release and its tested
platform limits are in [the installation guide](deploy/compose/PREBUILT.md);
known production gaps remain in [production readiness](docs/PRODUCTION_READINESS.md).

Please coordinate public disclosure in the private report so maintainers can
assess the impact and prepare a correction. The separate
[security model](docs/SECURITY.md) describes Cedar, RLS, audit, input boundaries
and adversarial tests; it is not a claim of production certification.
