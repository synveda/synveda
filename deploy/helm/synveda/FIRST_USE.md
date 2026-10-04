# First useful workflow

Use Getting started to create/select a project, optionally attach its repository,
and choose a registry-backed client. You can explore the console before client
setup. Its checks keep browser/server access, sign-in, project access, your
setup confirmation and authenticated Session/context delivery separate.

Install a CLI matching the server candidate from the verified client artifact
or your reviewed source candidate; see [client-only installation](../../../docs/CONSUMER_CLI.md).
Run setup at the actual repository root with the selected project ID. The
console prints this gateway's actual origin and project. Observation starts
off; explicitly opt in to trusted hooks before recording transcripts/tools.
MCP/native trust remains your client's normal approval. Detailed tested
versions, platform limits and manual routes appear with every client choice.

After a client context/recall operation, rerun the console check. Inspect the
corresponding Session and Context trace to see the delivered revision/source.
No context found on the checked page is an unverified result, not a declaration
that the client never ran. A self-reported client label cannot prove vendor
loading or model use.

## Shared installation: publish one useful project fact

Use real identities and an authorised project. These steps run in the console
and the selected client; no evaluation accounts or seed command are required.

1. In **Knowledge**, choose **Add Knowledge**, select the project under
   **Publish at**, and enter a short fact the team has checked, with a title,
   Markdown body and appropriate sensitivity. Submit **Create governed change**.
   The result is either an applied revision under the effective Configuration
   or a pending change. A pending change is not available for retrieval yet.
2. For a pending change, open **Advanced → Reviews** at its printed change ID.
   The required authorised reviewers inspect it and use **Approve** under their
   own identities. Follow the displayed separation rules; the author cannot
   approve when the matrix requires another person. Once approved, the permitted
   effect actor uses **Apply approved change**. A refusal gives the missing role or review state;
   resolve it through People/Access or the required reviewer, then retry that
   same change. Do not create a replacement merely to avoid review.
3. Open the resulting **Knowledge** item. Record its current immutable revision
   and check **Revision history**. This route is explicitly authored Knowledge. **Provenance**
   may be empty; a URL typed into its body is not a verified repository or
   Session source descriptor.
4. Ask the configured client to call Synveda's **recall** for that fact, using
   its selected project and stable task/Session binding. Complete the native
   trust prompt. Inspect the actual tool result rather than assuming the model
   used it. No result means check the selected project, current revision,
   sensitivity and read grants; an empty Knowledge catalogue is a valid empty
   result, not a connection failure.
5. In **Context**, open the corresponding recorded context run and its selected
   item. Compare the exact Knowledge revision and follow its item/history/source
   links. A preview computes a plan without recording delivery; **Request context**
   records a console request for an existing Session and does not itself prove
   that the client called a tool. Recheck Getting started after the real call.

For the full source-evidence learning path, first opt into trusted observation
hooks for the project, or explicitly consent to an MCP **remember** call for a
specific finding. Recall-only MCP setup records no turns. In **Sessions**, open
the resulting Session and inspect its timeline, then use **Capture this Session**.
In **New Learnings**, inspect the proposed text and authorised source event,
select its publishing scope, and use **Accept** (or edit and accept). A pending
VedaFlow change follows the same distinct review and apply steps above. Capture
freezes evidence; it does not automatically approve the proposed learning.
Repeat recall and compare that applied revision with its Session-event source.
The server's initial deterministic extractor/retrieval is synthetic/lexical;
provider-backed extraction requires its separately configured provider.

## Optional repeatable learning sample

This sample works with the Kubernetes **local evaluation** forwards; it needs
no Docker command or paid model. It uses ordinary authenticated public APIs
and existing review rules. It is deliberately refused on shared remote targets.
The current staged commands require the source candidate CLI. Published
packages must be qualified separately.

Keep both forwards running. Use a new private browser context for each of the
four explicitly selected evaluation identities. Retrieve each person's
password from the protected prepared Secret file in a private terminal, using
the main guide's command with `admin`, `member`, `approver` or `viewer` as its
last key. Never put a password/token in CLI arguments.

Run on the client workstation, using your actual selected APP_PORT:

```sh
export SYNVEDA_GATEWAY="http://localhost:$APP_PORT"
export SYNVEDA_INSECURE_DEVELOPMENT_HTTP=true
synveda login --gateway "$SYNVEDA_GATEWAY" --profile author --no-browser
# synveda-demo-admin (Avery Author)
synveda login --gateway "$SYNVEDA_GATEWAY" --profile reviewer --no-browser
# synveda-demo-member (Riley Reviewer)
synveda login --gateway "$SYNVEDA_GATEWAY" --profile approver --no-browser
# synveda-demo-approver (Morgan Approver)
synveda login --gateway "$SYNVEDA_GATEWAY" --profile viewer --no-browser
# synveda-demo-viewer (Vera Restricted Viewer)
```

Seed is deliberately incomplete. It creates or reopens the stable
`northstar-delivery-demo` workspace and `ingestion-api` project, verifies the
four grants, creates one already-approved Knowledge revision with repository
provenance, opens a versioned Skill install proposal, and appends one synthetic
finding to a Session. It does not capture that Session:

```sh
synveda demo retry-review seed \
  --author-credentials author \
  --reviewer-credentials reviewer \
  --approver-credentials approver \
  --viewer-credentials viewer \
  --confirm-target "$SYNVEDA_GATEWAY"
synveda demo retry-review inspect --author-credentials author
synveda demo retry-review capture \
  --author-credentials author \
  --confirm-target "$SYNVEDA_GATEWAY"
```

The expected finding says that retries reuse the original `Idempotency-Key`,
return 409 while the first ingestion runs, and replay its stored response once
complete. Capture prints two exact IDs: `<learning-change-id>` and
`<skill-change-id>`. Riley inspects each proposal. Vera's attempted learning
approval must exit non-zero without granting or applying anything. Riley alone
can approve the Knowledge change. Skill changes require Riley and Morgan as two
distinct approvers before Avery applies each typed change:

```sh
synveda proposal show <learning-change-id> --profile reviewer
synveda proposal approve <learning-change-id> --profile viewer \
  --comment "Not authorised to review"
# expected: denied; no state transition
synveda proposal approve <learning-change-id> --profile reviewer \
  --comment "Retry contract matches the reviewed Session evidence"
synveda proposal apply <learning-change-id> --profile author

synveda proposal show <skill-change-id> --profile reviewer
synveda proposal approve <skill-change-id> --profile reviewer \
  --comment "Skill instructions preserve idempotency and content-free evidence"
synveda proposal approve <skill-change-id> --profile approver \
  --comment "Distinct administrator approval for the reviewed Skill"
synveda proposal apply <skill-change-id> --profile author
```

Now open the project binding, substitute the printed
`<binding-change-id>`, and keep review and effect execution separate:

```sh
synveda demo retry-review bind-skill \
  --author-credentials author \
  --confirm-target "$SYNVEDA_GATEWAY"
synveda proposal show <binding-change-id> --profile reviewer
synveda proposal approve <binding-change-id> --profile reviewer \
  --comment "Pin the reviewed Skill version at the ingestion project"
synveda proposal approve <binding-change-id> --profile approver \
  --comment "Distinct administrator approval for the exact binding"
synveda proposal apply <binding-change-id> --profile author

synveda demo retry-review verify \
  --author-credentials author \
  --reviewer-credentials reviewer \
  --confirm-target "$SYNVEDA_GATEWAY" \
  --json
synveda demo retry-review status --author-credentials author --json
```

`verify` first reads the applied Knowledge head and its Session-event
provenance, then requests Riley's authorised context, checks that it selected
that exact revision, reads the enabled pinned Skill version, and finally reads
content-free Knowledge, Skill, Session and context audit pages plus the chain
verification result. The team's redacted Context trace does not retain task
or Knowledge text; its selected-item links still address the exact immutable
Knowledge revisions. `status` reopens the recorded Knowledge, Session,
Capture, Skill, Context and audit addresses through the public API. Re-running
`seed` uses stable idempotency keys and must retain the same addresses; it
refuses a different gateway, different identities, a
different effective curator file or changed receipt ownership. There is no
fixture reset endpoint and no business-table SQL.


On interruption, rerun the same stage with the same profiles and receipt.
`status` identifies the recorded addresses; read them before applying a printed
change ID. Do not seed a different tenant, reset data or silently approve to
advance. A refusal names the required identity/review state. Correct it through
the displayed governed action. Synthetic observations and deterministic/lexical
retrieval demonstrate the workflow; they do not qualify a live agent or model.
