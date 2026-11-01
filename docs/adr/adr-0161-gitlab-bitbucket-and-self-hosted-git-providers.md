# ADR-0161: Extend the Git bridge to GitLab, Bitbucket and self-hosted forges

- **Status**: Proposed
- **Date**: 2026-11-01
- **Feature(s)**: Git bridge — GitLab and Bitbucket providers
- **Deciders**: Owner hosting choice of 2026-10-31; provider facts from current
  documentation and read-only protocol probes retrieved 2026-11-01

## Context

[ADR-0138](adr-0138-governed-git-export.md) and
[ADR-0139](adr-0139-private-github-git-export.md) export authored channels to a
local bare repository or a pinned private GitHub.com repository.
[ADR-0148](adr-0148-inbound-git-source-reconciliation.md) and
[ADR-0149](adr-0149-private-github-source-identity-and-custody.md) read
configured Markdown paths from the same two places. ADR-0139 deferred other
forges, self-hosted origins and private CAs to a separate decision; ADR-0149
asked for one when another provider needs a different contract. The
[open brief](../backlog/FLOW-9.md) records the owner's choice: GitLab.com and
Bitbucket Cloud, then self-managed GitLab, Bitbucket Data Center and GitHub
Enterprise Server.

The export push is already forge-neutral: `wire.rs` speaks smart-HTTP
receive-pack, the pack is built in process, and push refuses any advertised
object format other than SHA-1. Inbound reads are not. ADR-0149 accepts a source
only after rebuilding each tree object from its entries and hashing it. Current
provider documentation shows that this check cannot be repeated over the new
providers' REST APIs:

- GitLab commit and branch responses carry parent IDs but no tree ID; trees list
  only by path and ref, not by tree ID; JSON blob content is converted to UTF-8.
- Bitbucket Cloud `/commit` has no tree hash; `/src` entries carry no blob or
  tree object ID and no file mode.
- Bitbucket Data Center commits have no tree ID; the directory schema is
  undocumented and `contentId` is not documented as a Git object ID.

All five hosts serve Git smart HTTP. Read-only probes on 2026-11-01 found that
GitHub.com, GitLab.com and Bitbucket Cloud each advertise protocol v2 with
`fetch=shallow wait-for-done filter`. Each returned a commit, its root tree and a
blob in separate one-object fetches by object ID. Bitbucket Data Center and
GitHub Enterprise Server were not probed.

## Decision

1. **Hosts.** Add GitLab.com, Bitbucket Cloud, self-managed GitLab, Bitbucket
   Data Center and GitHub Enterprise Server. GitHub.com transports, receipts,
   digests and document formats are unchanged. GitHub Enterprise Cloud with data
   residency (`SUBDOMAIN.ghe.com`) is excluded: its REST origin is
   `api.SUBDOMAIN.ghe.com` without `/api/v3`, and it was not selected. Azure
   DevOps, generic hosts, caller-supplied URLs and SSH stay excluded.

2. **Deployment allowlists.** Two new optional settings,
   `SYNVEDA_GIT_EXPORT_FORGES` and `SYNVEDA_GIT_SOURCE_FORGES` (each with
   `_FILE`), carry formats `synveda-git-forge-export-targets-v1` and
   `synveda-git-forge-source-targets-v1`. A mapping has `tenant_id`, `scope_id`,
   `target`, `provider`, `repository_path`, `repository_id`,
   `secret_reference`, and for self-hosted providers `base_url` and an optional
   `ca_certificate_pem`. Unknown fields refuse, as in the GitHub formats.
   - `provider` is closed: `gitlab`, `bitbucket_cloud`,
     `bitbucket_data_center`, `github_enterprise_server`. A `gitlab` mapping
     without `base_url` means GitLab.com and with one means self-managed.
     `bitbucket_cloud` never carries `base_url`; `bitbucket_data_center` and
     `github_enterprise_server` always do. A cloud host is never accepted as
     a `base_url`.
   - `base_url` is `https://host[/path]`: lower-case host, default port (the
     repository identity grammar refuses others), no user info, query,
     fragment or trailing slash. The path carries a GitLab relative root or a
     Bitbucket context path.
   - `repository_id` is a string in the provider's canonical form: decimal for
     GitLab, Bitbucket Data Center and GitHub Enterprise Server; a lower-case
     braced UUID for Bitbucket Cloud.
   - Startup refuses a malformed document, more than 128 mappings or 1 MiB, a
     PEM over 16 KiB, a target or repository mapped twice across this file and
     the GitHub file, and two mappings at one `base_url` with different trust.
     A remote mapping never falls back to local custody.

3. **Configuration narrowing.** Append `gitlab` (GitLab.com), `bitbucket`
   (Bitbucket Cloud) and `self_hosted_git` (any operator-pinned `base_url`) to
   the closed `ExternalProvider` vocabulary, after `github`. The sortedness rule
   uses declaration order, so appending keeps stored documents valid. `github`
   keeps meaning GitHub.com only, so no approved document silently gains a
   destination. `RepositoryProvider` stays an identity label covering both cloud
   and self-hosted variants. Configuration still grants nothing: export needs
   ChannelExport and every asset read; inbound needs SourceRead and
   KnowledgeWrite.

4. **Credential custody.** Secret kinds stay `import_export` (export) and
   `source_read` (inbound); the stored provider label is the mapping's
   `provider`. New sealed documents `synveda-git-forge-export-credential-v1` and
   `synveda-git-forge-source-credential-v1` bind `provider`, `base_url` when
   self-hosted, `repository_id` and `token`. A secret whose kind, provider,
   origin or repository identity differs from the mapping's refuses with the
   existing non-oracular error. ADR-0139/0149 custody rules hold: reopen custody
   per effect, rotate under exclusive admission, never put a token in argv,
   environment, logs, audit or Git objects.

5. **Egress.** Clients keep ADR-0139's policy: HTTPS only, TLS 1.2 or later, no
   redirects, no environment proxy, bounded bodies, a five-second connect
   deadline and the existing per-request and per-operation deadlines.
   - Cloud origins are constructed: `gitlab.com`; `api.bitbucket.org` and
     `bitbucket.org`.
   - A self-hosted mapping reaches only its `base_url` host. With
     `ca_certificate_pem`, that root is the only trust (built-in roots
     disabled); without it, public PKI applies. One client exists per distinct
     origin and trust.
   - Response-supplied URLs, clone links and `Location` headers are ignored. A
     redirect, 429 or 5xx is a failed attempt; retries stay explicit requests,
     and no `Retry-After` handling is added.

6. **Preflight.** Before every effect, no-op and read, and again immediately
   before content leaves the process, the provider's REST metadata must match
   the pinned identity and show a private, active, non-fork repository. The
   provider sections below give the exact fields. A mismatch, including a
   rename or transfer, refuses until the operator updates the mapping.

7. **Export.** Push the unchanged v1 projection through the existing
   receive-pack exchange, with the old-head compare-and-swap, explicit
   unpack/ref report and exact-head rediscovery. Branch protection stays
   authoritative: a token that may not push to an export branch fails the
   export; nothing is forced, deleted or bypassed. Each provider's framing is
   proven by its own fixture, because GitHub needed an extra flush.

8. **Inbound reads.**
   - GitHub Enterprise Server uses the existing REST Git-data reader with
     `{base_url}/api/v3` and `X-GitHub-Api-Version: 2022-11-28`. GitHub's
     OpenAPI descriptions for Enterprise Server 3.19 and 3.22 define these
     endpoints identically to GitHub.com.
   - GitLab, Bitbucket Cloud and Bitbucket Data Center use a new forge-neutral
     Git protocol v2 reader in the replaceable `synveda-git` leaf. It uses the
     existing `reqwest`, `flate2` and `sha1` dependencies, adds no dependency,
     and keeps ADR-0148's limits.

   The Git reader's invariants:
   - **Transport.** `GET {git_url}/info/refs?service=git-upload-pack`, then
     `POST {git_url}/git-upload-pack`, both with `Git-Protocol: version=2`
     and the provider's authentication.
   - **Advertisement.** It must begin `version 2` and advertise `ls-refs` and
     `fetch` with the `shallow` and `filter` features; `object-format` must be
     absent or `sha1`. Anything else refuses.
   - **Branch resolution.** `ls-refs` with
     `ref-prefix refs/heads/{branch}`; exactly one line must equal the
     configured ref.
   - **One object per request.** Each request names one `want` and then
     `no-progress` and `done`:
     - commit: `deepen 1` and `filter tree:0`;
     - tree: `filter tree:0`;
     - blob: `filter blob:none`.

     It never sends `thin-pack`, `ofs-delta`, `include-tag` or `sideband-all`.
   - **Derived wants only.** A `want` is only the resolved ref, the `tree` line
     of a hashed commit, or an entry of a hashed tree. Git's v2 server accepts
     any existing object ID, so the reader never requests one it did not derive.
   - **Response sections.** An optional `shallow-info` naming exactly the
     wanted commit (commit requests only), then `packfile`. `acknowledgments`,
     `wanted-refs`, `packfile-uris` or any unknown section refuses. Band 1
     carries data, a bounded band 2 is discarded, and band 3 refuses.
   - **Pack checks.** Signature `PACK`, version 2, exactly one entry of the
     expected type, verified trailer checksum. Delta types 6 and 7 refuse; a
     one-entry non-thin pack cannot hold one, and the reader does not rely on
     that.
   - **Size checks.** The entry header's size is checked against the object
     bound (128 KiB per blob) before inflation. Inflation is capped at that
     size, and the compressed response is capped at the bound plus fixed
     overhead.
   - **Object hash.** `"{type} {size}\0" + bytes` must hash to the wanted ID.
     Trees parse with ADR-0149's mode and name rules; a submodule is an entry,
     never a fetch.

   This verifies more than the GitHub REST path does: the commit bytes are
   hashed, so the commit-to-tree link is proven rather than trusted. A partial
   or failed read stays incomplete and advances no cursor.

9. **Source identity.** An additive forward migration adds a
   provider-qualified identity projection keyed by tenant, project, provider,
   `base_url` (empty for cloud) and the text `repository_id`, mapped to the
   permanent root and actual attachment with ADR-0149's binding rules:
   - the first binding adopts the existing URI root;
   - a different ID at a bound URI refuses;
   - ambiguous roots refuse.

   Self-hosted IDs are unique only per instance, hence `base_url` in the key.
   Migration `0008` and every row in it are untouched, and GitHub.com stays
   there. The single resolver that governs source heads, import identity,
   suppression and deduplication consults both projections.

10. **Evidence and audit.** Each provider has its own custody digest domain
    covering provider, `base_url`, `repository_path`, `repository_id` and
    secret ID; local and GitHub digest inputs stay byte-stable, so no existing
    receipt appears retargeted. Audit, responses and metrics stay content-free
    and name the closed provider. A provider is named in integration docs,
    readiness or support claims only after its own credentialed canary; GitHub.com
    evidence does not transfer.

### Provider contracts

Evidence tags: **Doc** is current provider documentation; **Observed** is the
2026-11-01 read-only probe; **Untested** means the live canary must establish it.

**GitLab (GitLab.com and self-managed).**
- **REST.** `GET {api}/api/v4/projects/{id}` with `PRIVATE-TOKEN`, where `{api}`
  is `https://gitlab.com` or `base_url`. Address by numeric ID, because GET by an
  old path answers 301 (Doc).
- **Preflight.**
  - `id` and `path_with_namespace` equal the mapping.
  - `visibility == "private"`, which refuses `internal` and `public`.
  - `archived` is false.
  - `forked_from_project` is absent.
  - `marked_for_deletion_on` and `marked_for_deletion_at` are null.
  - `repository_object_format` is `sha1` when present (Doc).
- **Git.** `{api}/{path_with_namespace}.git` with Basic auth, username `oauth2`;
  any non-empty username is accepted (Doc).
- **Token.** A project access token: Developer role, or Maintainer if an export
  branch is protected.
  - Scopes: `read_api` with `write_repository` for export; `read_api` with
    `read_repository` for inbound (Doc).
  - Availability: GitLab.com Premium or Ultimate; any licence on self-managed
    (Doc).
  - A fine-grained personal access token limited to the project, generally
    available since 19.2, is accepted (Doc).
- **Protocol v2 and filters.** Documented for self-managed HTTP (Doc); observed
  on GitLab.com.
- **Residuals.**
  - A fork link to an upstream the token cannot see may be hidden.
  - That the ID survives rename or transfer is implied, not stated, and
    transfers are asynchronous since 19.4.
  - A project token created by an internal user can reach Internal projects
    (Doc), so operators should prefer a mapping where that does not apply.

**Bitbucket Cloud.**
- **REST.** `GET https://api.bitbucket.org/2.0/repositories/{workspace}/{uuid}`
  with a Bearer token. The UUID in the slug position is documented to survive
  renames (Doc).
- **Preflight.**
  - `uuid` and `full_name` equal the mapping.
  - `is_private` is true.
  - `parent` is absent.
  - `scm == "git"`.
- **Git.** `https://bitbucket.org/{workspace}/{slug}.git` with Basic auth,
  username `x-token-auth` (Doc).
- **Token.** A repository access token, the only Cloud token limited to one
  repository: `repository:write` for export, `repository` for inbound (Doc).
  - Atlassian API tokens are user-wide, and app passwords were removed on
    2026-07-28 (Doc), so neither is supported.
  - A transfer to another workspace revokes the token (Doc).
- **Protocol v2 and filters.** Stated only by Atlassian staff on its issue
  tracker (2024); observed.
- **Residuals.**
  - No archived state exists.
  - `parent` is undescribed and the schema admits extra fields.
  - Whether access tokens can push to restricted branches is untested: a
    15 September 2026 changelog allows it, the API introduction still forbids
    it, and users reported denials in October.

**Bitbucket Data Center (supported 9.4 LTS through 10.5).**
- **REST.** `GET {base_url}/rest/api/latest/projects/{KEY}/repos/{slug}` with a
  Bearer token. No core endpoint fetches a repository by ID, so the returned
  `id` is compared on every call (Doc).
- **Preflight.**
  - `id`, `slug` and `project.key` equal the mapping.
  - `public` and `project.public` are both false.
  - `archived` is false.
  - `state == "AVAILABLE"`.
  - `origin` is absent.
- **Git.** `{base_url}/scm/{KEY}/{slug}.git` with `Authorization: Bearer`. This
  form is the documented one; the same page contradicts itself about Basic with
  `x-token-auth` (Doc).
- **Token.** A repository HTTP access token, which needs a Data Center licence:
  Repository write for export, Repository read for inbound (Doc).
- **Protocol v2 and filters.** Supported since 5.13; partial clone on by
  default since 7.13 (Doc). `tree:0` needs server Git 2.20 or later, per
  Atlassian's issue tracker. Untested.
- **Residuals.**
  - `public` is "not the definitive answer". Public access is also governed by
    a global setting, which defaults to off since 8.18 and operators must keep
    off.
  - Mirrors redirect writes, so operators map the primary.
  - Whether a token can be exempted from branch permissions is undocumented
    and untested.

**GitHub Enterprise Server (supported 3.19 through 3.22).**
- **REST.** `GET {base_url}/api/v3/repos/{owner}/{repo}` with
  `X-GitHub-Api-Version: 2022-11-28`. Versions 3.19 and 3.20 list only that
  version, and a request naming an absent version is documented to fail (Doc).
- **Preflight.** GitHub.com's fields (`id`, canonical name, `private`, not
  `archived`, `disabled` or `fork`), plus `visibility == "private"` to refuse
  `internal`. REST does not document how `private` reports internal
  repositories (Doc).
- **Git.** `{base_url}/{owner}/{repo}.git` with Basic auth, username
  `x-access-token`. Documented only for installation tokens, and live-qualified
  on GitHub.com with fine-grained tokens; untested on Enterprise Server. If the
  canary refuses it, using the token owner's username is a separate amendment.
- **Token.** A fine-grained personal access token with selected repositories,
  Metadata read, and Contents read or read/write; generally available since
  3.17 (Doc). A token still pending organization approval reads only public
  resources, so preflight refuses it (Doc).
- **Residuals.** That subdomain isolation leaves `/api/v3` and repository paths
  in place is inferred from GitHub's moved-path table.

## Options considered

1. **Per-provider REST readers.** They match the GitHub path's shape, but cannot
   prove a commit's tree on GitLab, Bitbucket Cloud or Bitbucket Data Center,
   and two of them expose no object IDs or modes. Rejected.
2. **Trust REST listings and hash only the blobs.** That fails ADR-0149's
   complete-object rule and cannot prove a path absent, so deletion would rest
   on an unverified listing. Rejected.
3. **gitoxide (`gix`, MIT OR Apache-2.0).**
   - Its HTTP transport is blocking only.
   - Its reqwest backend pulls reqwest 0.13 beside the pinned 0.12.28 and builds
     its own client, so this crate's egress policy cannot be reused.
   - Its crates declare an initial-development stability tier with frequent
     breaking releases.
   - It adds 51 to 187 crates, and it implements no push.

   One-object fetches need only pkt-line framing, one inflate and one hash,
   which the crate already has. Rejected.
4. **libgit2 (`git2`).** C code, and partial clone is unsupported. Rejected.
5. **Native Git with credentials.** Rejected by ADR-0139: a secret-bearing
   subprocess.
6. **Move GitHub.com inbound onto the Git reader now.** It would change a
   qualified path without evidence that it needs changing. Deferred: see
   reversal triggers.
7. **Reuse `github`, `gitlab` and `bitbucket` for self-hosted origins.** That
   would silently widen documents that already allow `github`. Rejected in
   favour of `self_hosted_git`.

## Consequences

- **Positive.**
  - Five more hosts behind one egress, custody and identity contract.
  - Inbound verification stronger than today's REST path, with no new
    dependency.
  - Export logic is reused unchanged.
- **Negative / accepted.**
  - Two inbound readers coexist.
  - Every provider and self-hosted product needs its own fixtures and canary.
  - Preflight establishes observed private state, not a guarantee. The
    residuals above remain operator custody, as ADR-0139 accepted for GitHub.
  - Bitbucket Data Center and GitHub Enterprise Server rest on documentation
    until their canaries pass.
  - No proxy support may block some self-hosted networks.
- **Reversal triggers.**
  - A selected host refuses `tree:0` or protocol v2, or bans filters through
    `uploadpackfilter`.
  - An owner requires an egress proxy.
  - A canary refuses a documented auth form.
  - Once the Git reader is live-qualified on two hosts, decide separately
    whether GitHub.com and Enterprise Server inbound move onto it.

## Compliance notes

Embedded Cedar, current Configuration narrowing, forced tenant RLS, exclusive
source admission, prepared/completed receipts and content-free audit apply
unchanged to every provider. Static SQL stays in `synveda-store`; the identity
migration is additive and rewrites no published row. Fetched bytes are inert
data that reach Knowledge only through Capture review. No policy relaxation,
compatibility path or credential fallback is introduced.

Primary sources: GitLab [projects](https://docs.gitlab.com/api/projects/),
[repositories](https://docs.gitlab.com/api/repositories/) and
[project access tokens](https://docs.gitlab.com/user/project/settings/project_access_tokens/);
Bitbucket Cloud [REST](https://developer.atlassian.com/cloud/bitbucket/rest/intro/)
and [access tokens](https://support.atlassian.com/bitbucket-cloud/docs/using-access-tokens/);
Bitbucket Data Center [REST](https://developer.atlassian.com/server/bitbucket/rest/v1005/intro/)
and [HTTP access tokens](https://confluence.atlassian.com/bitbucketserver/http-access-tokens-939515499.html);
GitHub Enterprise Server [API versions](https://docs.github.com/en/enterprise-server@3.19/rest/about-the-rest-api/api-versions)
and [releases](https://docs.github.com/en/enterprise-server@latest/admin/all-releases);
Git [protocol v2](https://git-scm.com/docs/gitprotocol-v2) and
[pack format](https://git-scm.com/docs/gitformat-pack).
