//! The `synveda` admin/dev CLI (`synveda init`, `synveda policy apply`,
//! `synveda proposal review`, ...). Talks to the gateway API as a client for
//! everything a running gateway serves; the bootstrap commands below
//! (TEN-1, ADR-0008) go to the database directly because they exist
//! precisely for the moment there is no usable gateway yet — applying
//! migrations, admitting the first tenant, minting a dev token.
//!
//! Since ADPT-1 it is also the credential authority (ADR-0027 decision 4):
//! `synveda login` runs the loopback flow and `synveda auth token` hands a
//! currently-valid bearer to whoever asks — the Claude Code adapter's
//! hooks, a script, a human. One implementation of PKCE, expiry, and
//! refresh, here, rather than a second drifting one per adapter.

// `unsafe` is forbidden in the product code; the credentials tests set
// process environment variables, which is unsafe in edition 2024, and
// they hold a lock while they do it.
#![cfg_attr(not(test), forbid(unsafe_code))]

mod api;
mod channel;
mod credentials;
mod diff;
mod hierarchy;
mod init;
mod login;
mod mcp;
mod pack;
mod prompt;
mod proposal;
mod recall;
mod skill;
#[cfg(test)]
mod testing;

use std::process::ExitCode;
use std::time::Duration;

use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use synveda_audit::{Actor, AuditAction, AuditEvent, Outcome};
use synveda_identity::{Hs256Verifier, personal_slug};
use synveda_types::{
    CompositionConfig, DedupConfig, DedupMode, IdentityId, IdentityKind, IndexTier, InjectChannels,
    PackConfig, PromotionConfig, ProposalId, ProposalState, RecordId, RedactionConfig,
    RedactionMode, RetentionConfig, Role, ScanSeverity, ScopeId, ScopeKind, SkillIndex,
    SkillScanConfig, TenantId, TenantStatus,
};

#[derive(Parser)]
#[command(name = "synveda", about = "Synveda admin/dev CLI", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Bring up a single-node deployment and admit its first tenant
    /// (OPS-1, ADR-0055) — the SMB profile of tech plan §4.
    ///
    /// What it does is deliberately small, because everything else the
    /// product has a governed surface for is created *through* that
    /// surface: `init` applies migrations, admits the tenant, configures
    /// the issuer, and starts the stack. The org root, the operator's
    /// identity and their `org-admin` binding all arrive on the first
    /// `synveda login`, from AUTH-2's provisioning transaction, chained
    /// under the operator's own subject. Departments and teams are
    /// `synveda hierarchy create` after that.
    ///
    /// There is no path in here that writes a scope, an identity, a role
    /// binding or a record behind the PDP's back — an installer runs once,
    /// as root-equivalent, before anybody is watching, which makes it the
    /// worst place in the product to keep a shortcut (seed §2.2).
    Init {
        /// Tenant slug to admit: lowercase, hyphenated. Also becomes the
        /// org root's slug when the first admin logs in.
        #[arg(long, default_value = "acme")]
        slug: String,
        /// Tenant display name; becomes the org root's name.
        #[arg(long, default_value = "ACME")]
        name: String,
        /// Which embedder the corpus will be written with. Permanent in
        /// practice: `record_embeddings` stores the model, embed-or-fail
        /// is unconditional, and nothing re-embeds a corpus that changed
        /// its mind (ADR-0055 decision 5). `deterministic` needs no model
        /// download; `tei` serves BGE-M3 and downloads ~2.3 GB once.
        #[arg(long, value_parser = ["deterministic", "tei"], default_value = "deterministic")]
        embedder: String,
        /// An external OIDC issuer URL. Omitted, the bundled Rauthy is
        /// configured for you; given, nothing is created in your directory
        /// and the client registration you must perform there is printed
        /// (ADR-0055 decision 4).
        #[arg(long)]
        issuer: Option<String>,
        /// Also build the ACME demo organisation — two departments, three
        /// teams, four people, and material that arrives through the
        /// observe → extract → embed pipeline like anybody else's. Never
        /// use on a deployment that will hold real memory.
        #[arg(long)]
        demo: bool,
        /// Print what would happen and change nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// The scopes an organisation is made of (OPS-1, ADR-0055 decision 3).
    ///
    /// Gateway calls under the bearer `synveda login` stored, like
    /// `proposal` and `recall`: creating a department is a governed act
    /// whose `HierarchyCreate` decision the PDP takes at the parent scope
    /// and whose event the gateway chains under your own identity.
    ///
    /// The org root is not creatable here — it arrives with the first
    /// admin login, from the tenant's own slug and name (ADR-0055
    /// decision 2), so every `create` has a parent.
    #[command(subcommand)]
    Hierarchy(HierarchyCommand),
    /// Log in to a gateway through your browser (AUTH-1 end to end,
    /// ADR-0027 decision 5) and store the session under a profile. This
    /// is the whole of "zero-config": every other Synveda client on this
    /// machine reads its bearer from what this writes.
    Login {
        /// Gateway base URL. Defaults to $SYNVEDA_GATEWAY, else
        /// http://127.0.0.1:8120.
        #[arg(long)]
        gateway: Option<String>,
        /// Which configured issuer to log in against; optional when the
        /// gateway has exactly one.
        #[arg(long)]
        issuer: Option<String>,
        /// Credential profile to write. Defaults to $SYNVEDA_PROFILE,
        /// else `default`.
        #[arg(long)]
        profile: Option<String>,
        /// Print the login URL instead of opening a browser (headless
        /// machines, SSH sessions).
        #[arg(long)]
        no_browser: bool,
    },
    /// Stored credentials (ADR-0027 decisions 4 and 6).
    #[command(subcommand)]
    Auth(AuthCommand),
    /// Serve governed memory to any MCP client over stdio (ADPT-2,
    /// ADR-0057 as amended).
    ///
    /// Two tools — `recall` and `remember` — for clients that have no hook
    /// seam and whose only extension point is a tool the model chooses to
    /// call. A client launches this as a subprocess and speaks JSON-RPC on
    /// its stdin and stdout, so nothing here listens on a port and no
    /// credential leaves the machine.
    ///
    /// It is an adapter that happens to live in this binary: a gateway
    /// client over `/v1` holding the bearer `synveda login` stored, three
    /// primitives only, no database connection and no core-crate call.
    Mcp {
        /// Configure a client to launch this server. Bare `synveda mcp`
        /// *is* the server, which is what a client's config execs.
        #[command(subcommand)]
        command: Option<McpCommand>,
        /// Who owns the write at this host. `tool` advertises `remember`
        /// as well as `recall`, because nothing else writes; `host`
        /// advertises `recall` only, because the harness or framework
        /// launching this already observes its own turns.
        ///
        /// Get it wrong towards `tool` on a harness with hooks and the
        /// same turn is stored twice — once as the model composed it, once
        /// as the hook saw it — with different payloads, so nothing
        /// downstream can tell they were the same turn.
        #[arg(long, value_enum, default_value_t = mcp::Writes::Tool)]
        writes: mcp::Writes,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Database administration (dev bootstrap).
    #[command(subcommand)]
    Db(DbCommand),
    /// Tenant administration (dev bootstrap).
    #[command(subcommand)]
    Tenant(TenantCommand),
    /// Dev-mode bearer tokens (HS256, ADR-0008). AUTH-1 makes real IdPs the
    /// issuer; this command never applies to a production deployment.
    #[command(subcommand)]
    Token(TokenCommand),
    /// Per-tenant policy packs (AUTHZ-1, ADR-0012). Dev/admin plumbing:
    /// AUTHZ-2 owns the product surface, and VedaFlow eventually governs
    /// packs as reviewed assets.
    #[command(subcommand)]
    Policy(PolicyCommand),
    /// Role bindings (AUTHZ-3, ADR-0015). Dev plumbing and the documented
    /// break-glass: a tenant that revoked its last org-admin recovers
    /// here, at the store level — the product surface is `/v1/roles/*`.
    #[command(subcommand)]
    Role(RoleCommand),
    /// Service identities (AUTH-3, ADR-0018). Dev plumbing and the
    /// break-glass at the store level — the product surface is
    /// `/v1/service-identities`. Credentials live in the IdP: register
    /// the OAuth2 client there, then bind its subject to an anchor here.
    #[command(subcommand)]
    Service(ServiceCommand),
    /// The hash-chained audit log (AUD-1, ADR-0019): verify a tenant's
    /// chain or tail its recent events. Read-only; the operator's and
    /// auditor's chain check ahead of AUD-2's query surface.
    #[command(subcommand)]
    Audit(AuditCommand),
    /// The VedaFlow review flow (FLOW-6, ADR-0035): read, review, and
    /// decide promotion proposals from a terminal.
    ///
    /// These are not dev plumbing, and they open no database connection.
    /// Every verb is a call to /v1/proposals under the bearer `synveda
    /// login` stored, so the PDP decides who may act exactly as it would
    /// for a console, and the gateway chains the event under your own
    /// identity. Run `synveda login` first.
    #[command(subcommand)]
    Proposal(ProposalCommand),
    /// VedaFlow channels (FLOW-7, ADR-0036): what a scope publishes, the
    /// states it has published, rewinding to one of them, and holding
    /// what it serves at a commit.
    ///
    /// Gateway calls under the bearer `synveda login` stored, like
    /// `proposal` and for the same reason: a rewind reaches every agent
    /// under the scope on their next session, and an act that large is
    /// one the PDP must decide and the gateway must chain under your own
    /// identity — never a row a laptop wrote.
    #[command(subcommand)]
    Channel(ChannelCommand),
    /// The prompt registry (PRMT-1, ADR-0049): author a draft, resolve the
    /// version a consumer would be served, and open the review that carries
    /// one across the trust boundary.
    ///
    /// Gateway calls under the bearer `synveda login` stored, like every
    /// other governed verb: authoring is a `PromptWrite` decision and
    /// resolution a `PromptRead` at the tier the served version carries.
    /// Reviewing and publishing stay `synveda proposal`'s — a prompt
    /// proposal is an ordinary proposal.
    #[command(subcommand)]
    Prompt(PromptCommand),
    /// The context-pack registry (PRMT-2, ADR-0050): author a bundle of
    /// documents, list what a scope drafts and publishes, and open the
    /// review that carries it across the trust boundary.
    ///
    /// There is deliberately **no `show`**: a prompt is fetched by name,
    /// and a pack's content reaches a session through `synveda inject`, as
    /// pinned records ranked against everything else the reader may see.
    ///
    /// Gateway calls under the bearer `synveda login` stored, like every
    /// other governed verb: authoring is a `ContextPackWrite` decision, and
    /// the server does the chunking, the secret scan and the embedding, so
    /// a terminal can never disagree with it about a document's address.
    #[command(subcommand, name = "context-pack")]
    ContextPack(ContextPackCommand),
    /// The skills registry (SKIL-1, ADR-0051): import an
    /// agentskills.io-format bundle, list what a scope holds, open the
    /// review that carries it, and **install** it into a client's own
    /// skills directory.
    ///
    /// `install` is the only thing in the product that writes a skill onto
    /// a disk, and it is here rather than in the gateway because the
    /// harness is a guest (seed §2.6): a client moving its folder should
    /// cost a CLI release, not a server one. What it writes is exactly the
    /// reviewed files — the receipt goes beside your credentials, never
    /// inside the bundle, because a file no reviewer approved in a
    /// directory a client walks is the modification "installs unmodified"
    /// forbids.
    #[command(subcommand)]
    Skill(SkillCommand),
    /// Fetch records in full by id (CTX-4, ADR-0041) — the other half of
    /// tiered injection.
    ///
    /// An inject block's index tier ends its lines with `(recall <id>)`;
    /// this is that instruction. A gateway call under the bearer `synveda
    /// login` stored, so the PDP decides per scope and the read is chained
    /// under your own identity.
    ///
    /// A handle is a name, not a capability: what you may read is decided
    /// now, not when the block was composed, so an id you have since lost
    /// access to simply does not come back.
    Recall {
        /// The record ids, as the block printed them. Omit when asking a
        /// question with --query.
        #[arg(num_args = 0..)]
        ids: Vec<RecordId>,
        /// Ask a question instead of naming records: hybrid retrieval over
        /// every scope your policy lets you read, which is wider than the
        /// scopes an inject block composes from.
        ///
        /// Omit both this and the ids, with --as-of, to sweep everything
        /// you may read as it stood at that instant — the complete
        /// historical read, including material the live corpus no longer
        /// holds.
        #[arg(long, conflicts_with = "ids")]
        query: Option<String>,
        /// Serve bodies as the database held them at this instant —
        /// "what did the agent know on March 3rd" (RFC 3339, e.g.
        /// 2026-03-03T00:00:00Z).
        ///
        /// It rewinds the corpus, never your access: what you may read is
        /// decided now, so this cannot return material you lost access to.
        #[arg(long)]
        as_of: Option<DateTime<Utc>>,
        /// Valid time — which assertions were true *about the world* at
        /// this instant. Defaults to --as-of, so one flag asks the
        /// diagonal question.
        #[arg(long)]
        valid_at: Option<DateTime<Utc>>,
        /// How many records a --query may return.
        #[arg(long)]
        limit: Option<usize>,
        /// Print the gateway's answer verbatim.
        #[arg(long)]
        json: bool,
        /// Skip the "reading as ..." line — for a harness piping the
        /// bodies straight into a session.
        #[arg(long)]
        quiet: bool,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
    },
}

#[derive(Subcommand)]
enum PromptCommand {
    /// The registry at one scope: what is drafted, what is published, and
    /// whether they are the same bytes.
    List {
        /// The scope UUID.
        scope: ScopeId,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Resolve a prompt the way a consumer would: by name, walking your own
    /// placement chain nearest-first, unless you name a scope.
    ///
    /// `--commit` is the consumer's pin — the version you were built
    /// against — and it is refused, naming both commits, if a rewind has
    /// since taken it off the channel (ADR-0049 decision 10).
    Show {
        /// The prompt's name, e.g. `support/triage-reply`.
        name: String,
        /// Resolve at this scope instead of walking your chain. Required
        /// with --draft and with --commit.
        #[arg(long)]
        scope: Option<ScopeId>,
        /// Read the authoring copy at --scope rather than the reviewed
        /// version. Unreviewed by construction.
        #[arg(long)]
        draft: bool,
        /// Pin to a commit that scope's channel has held.
        #[arg(long)]
        commit: Option<String>,
        /// Render with these values, `name=value`. A missing required
        /// variable and an undeclared value are both refusals.
        #[arg(long = "var", value_name = "NAME=VALUE")]
        values: Vec<String>,
        /// Print the gateway's answer verbatim.
        #[arg(long)]
        json: bool,
        /// Print only the prompt (or the render) — for piping.
        #[arg(long)]
        quiet: bool,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Write a draft: create it, or replace the content of the one that is
    /// there. This moves nothing a consumer reads.
    Author {
        /// The prompt's name, e.g. `support/triage-reply`.
        name: String,
        /// The scope that will stand behind it.
        #[arg(long)]
        scope: ScopeId,
        /// The template, read from a file. `-` reads standard input.
        #[arg(long)]
        file: String,
        /// One line, read in a listing and at review.
        #[arg(long, default_value = "")]
        description: String,
        /// Declare a variable: `name` (required) or `name=default`
        /// (optional). The schema must agree with the template exactly.
        #[arg(long = "var", value_name = "NAME[=DEFAULT]")]
        variables: Vec<String>,
        /// public | internal | confidential. Defaults to internal;
        /// `restricted` is refused — nothing in the product mints that tier
        /// for an authored asset.
        #[arg(long)]
        sensitivity: Option<String>,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Open the review that can carry a draft onto a scope's published
    /// channel. Everything after this is `synveda proposal`.
    Propose {
        /// The prompt's name.
        name: String,
        /// The scope whose channel would move. Requirements resolve here.
        #[arg(long)]
        scope: ScopeId,
        /// What this proposes, in one line. Defaults to the name.
        #[arg(long)]
        title: Option<String>,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
    },
}

#[derive(Subcommand)]
enum SkillCommand {
    /// The registry at one scope: every skill, its files, and whether what
    /// a client would install is what was last written.
    List {
        /// The scope UUID.
        scope: ScopeId,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Resolve a bundle the way a client would: by name, walking your own
    /// placement chain nearest-first, unless you name a scope.
    ///
    /// Because the name is also the installed directory name and a client's
    /// skills root is flat, that walk is what decides which of two
    /// same-named skills exists on your disk at all.
    Show {
        /// The skill's name, e.g. `code-review`.
        name: String,
        /// Resolve at this scope instead of walking your chain. Required
        /// with --draft and with --commit.
        #[arg(long)]
        scope: Option<ScopeId>,
        /// Read the authoring copy at --scope rather than the reviewed
        /// version. Unreviewed by construction.
        #[arg(long)]
        draft: bool,
        /// Pin to a commit that scope's channel has held.
        #[arg(long)]
        commit: Option<String>,
        /// Print the gateway's answer verbatim.
        #[arg(long)]
        json: bool,
        /// Suppress the connection banner — for piping.
        #[arg(long)]
        quiet: bool,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Read an anthropics/skills-format directory and write it as a draft.
    /// This moves nothing a client installs.
    ///
    /// The request is the bundle: a file you removed from the directory is
    /// removed from the draft, because a client loads a skill whole and a
    /// leftover file would be published back onto a laptop.
    Import {
        /// The directory holding SKILL.md and its bundled files.
        dir: std::path::PathBuf,
        /// The scope that will stand behind it.
        #[arg(long)]
        scope: ScopeId,
        /// Override the skill name. Defaults to the directory's own name,
        /// which is the spec's rule; the frontmatter `name` must agree
        /// either way.
        #[arg(long)]
        name: Option<String>,
        /// public | internal | confidential. Defaults to internal;
        /// `restricted` is refused — nothing in the product mints that tier
        /// for an authored asset.
        #[arg(long)]
        sensitivity: Option<String>,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Write a published bundle into a client's own skills directory, byte
    /// for byte, and re-hash every file against the address the commit
    /// named.
    Install {
        /// The skill's name — also the directory this creates.
        name: String,
        /// Which client's layout to write. The per-client difference is the
        /// root and nothing else.
        #[arg(long, default_value = "claude-code")]
        client: String,
        /// Write under this directory instead of the client's own root.
        #[arg(long)]
        root: Option<std::path::PathBuf>,
        /// Install this scope's copy instead of walking your chain.
        #[arg(long)]
        scope: Option<ScopeId>,
        /// Install the commit you were built against. A rewind that took it
        /// off the channel refuses this, naming both commits.
        #[arg(long)]
        commit: Option<String>,
        /// Print the receipt as JSON.
        #[arg(long)]
        json: bool,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
    },
    /// What you may install: every published skill on your own placement
    /// chain, nearest scope first (SKIL-4, ADR-0054).
    ///
    /// The plural of `show`, and the same walk — a scope you may not read
    /// skills at is skipped as though it published nothing, so it cannot
    /// shadow a copy further up that you can read. Another team's skills
    /// are absent because that team is not on your chain at all.
    Available {
        /// Print the gateway's answer verbatim.
        #[arg(long)]
        json: bool,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Make a client's governed skills directory match what you may
    /// install: write every available skill, remove what is no longer
    /// available (SKIL-4, ADR-0054 decision 15).
    ///
    /// The removal is the half that makes a rollback mean something on a
    /// laptop. It is bounded to directories this command wrote — the root
    /// is a directory Synveda owns, never a client's own skills folder —
    /// and to names the registry no longer serves you.
    Sync {
        /// Which client's layout to write. The per-client difference is the
        /// root and nothing else.
        #[arg(long, default_value = "claude-code")]
        client: String,
        /// Write under this directory instead of the client's own root.
        /// The adapter passes its plugin's own `skills/` here.
        #[arg(long)]
        root: Option<std::path::PathBuf>,
        /// Report what would change and write nothing.
        #[arg(long)]
        dry_run: bool,
        /// Print the outcome as JSON — what an adapter reads.
        #[arg(long)]
        json: bool,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Open the review that can carry a bundle onto a scope's published
    /// channel. Everything after this is `synveda proposal`.
    ///
    /// Under every pack this is the only route: the invariant floor asks
    /// for a security reviewer and two distinct approvers, so no pack makes
    /// shipping executable code a one-signature act.
    Propose {
        /// The skill's name. The proposal names the bundle, never a file.
        name: String,
        /// The scope whose channel would move. Requirements resolve here.
        #[arg(long)]
        scope: ScopeId,
        /// The scope that holds it, when climbing. Defaults to --scope.
        #[arg(long)]
        source: Option<ScopeId>,
        /// What this proposes, in one line. Defaults to the name.
        #[arg(long)]
        title: Option<String>,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
    },
}

#[derive(Subcommand)]
enum ContextPackCommand {
    /// The registry at one scope: every pack, its documents, and whether
    /// what a session composes is what was last written.
    List {
        /// The scope UUID.
        scope: ScopeId,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Write a bundle: create it, or replace the documents named here.
    /// This moves nothing a session composes.
    ///
    /// The server chunks, scans for secrets and embeds every document
    /// whose bytes moved — so a re-run over an unchanged bundle costs one
    /// address lookup per file and no vectors at all.
    Author {
        /// The pack's name — one segment, e.g. `payments`.
        name: String,
        /// The scope that will stand behind it.
        #[arg(long)]
        scope: ScopeId,
        /// A document to put in it. Repeatable.
        #[arg(long = "file", value_name = "PATH", required = true)]
        files: Vec<std::path::PathBuf>,
        /// Name documents by their path relative to this directory, so a
        /// bundle keeps the shape it has on disk.
        #[arg(long)]
        root: Option<std::path::PathBuf>,
        /// One line, read in a listing and at review.
        #[arg(long, default_value = "")]
        description: String,
        /// public | internal | confidential. Defaults to internal;
        /// `restricted` is refused — nothing in the product mints that tier
        /// for an authored asset.
        #[arg(long)]
        sensitivity: Option<String>,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Open the review that can carry a bundle onto a scope's published
    /// channel. Everything after this is `synveda proposal`.
    Propose {
        /// The pack's name.
        name: String,
        /// The scope whose channel would move. Requirements resolve here —
        /// and under `regulated-strict` above a team that is a curator and
        /// a steward, two distinct people (ADR-0050 decision 15).
        #[arg(long)]
        scope: ScopeId,
        /// What this proposes, in one line. Defaults to the name.
        #[arg(long)]
        title: Option<String>,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
    },
}

#[derive(Subcommand)]
enum ChannelCommand {
    /// What stands at a scope: each channel, where it points, and the
    /// pin holding its readers if there is one.
    Status {
        /// The scope UUID.
        scope: ScopeId,
        /// Print the gateway's answer verbatim.
        #[arg(long)]
        json: bool,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
    },
    /// The states a channel has held, newest first — the menu a rewind is
    /// chosen from. Everything listed but the head is a legal `--to`, and
    /// nothing outside the listing is (ADR-0036 decisions 1 and 11).
    History {
        /// The scope UUID.
        scope: ScopeId,
        /// Which channel, as its ref name. Defaults to
        /// `memory/published`.
        #[arg(long)]
        channel: Option<String>,
        /// How many states, 1..=200.
        #[arg(long)]
        limit: Option<u32>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Rewind a channel to a state it has already held. Every session
    /// that starts afterwards composes that state; nothing else has to
    /// happen, and nothing can be installed that was not approved when it
    /// was published.
    Rollback {
        /// The scope UUID.
        scope: ScopeId,
        /// The commit being abandoned — the head as `history` showed it.
        /// Required: a rewind is a decision about which state to leave,
        /// and that decision is stale if someone else moved the ref.
        #[arg(long)]
        from: String,
        /// The state to install: one of the commits `history` lists.
        #[arg(long)]
        to: String,
        /// Why. An auditor reads this, and so does whoever asks next week
        /// why a record stopped being published.
        #[arg(long)]
        message: String,
        #[arg(long)]
        channel: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Hold what a channel serves at a commit. Publications keep landing
    /// and the channel keeps advancing; what stops moving is what readers
    /// compose (ADR-0036 decision 6).
    Pin {
        /// The scope UUID.
        scope: ScopeId,
        /// The commit to hold readers at — one of the commits `history`
        /// lists, the head included.
        #[arg(long)]
        commit: String,
        /// Why this scope is holding its readers. The pin's only record:
        /// the ref carries who and when and nothing else.
        #[arg(long)]
        reason: String,
        #[arg(long)]
        channel: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Release the hold. Readers catch up to the channel's head on their
    /// next session.
    Unpin {
        /// The scope UUID.
        scope: ScopeId,
        /// Why the hold is being released.
        #[arg(long)]
        reason: String,
        #[arg(long)]
        channel: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        profile: Option<String>,
    },
}

#[derive(Subcommand)]
enum ProposalCommand {
    /// List proposals, newest first.
    ///
    /// Without `--scope` this is a tenant-wide listing, which the packs
    /// grant to tenant-wide review and admin roles only; a curator bound
    /// at one team passes `--scope` instead.
    List {
        /// Restrict to proposals targeting this scope.
        #[arg(long)]
        scope: Option<ScopeId>,
        /// Restrict to one stored state (open/rejected/withdrawn/
        /// published). `approved` is computed, not stored: filter on
        /// `open` and read each row's state.
        #[arg(long)]
        state: Option<ProposalState>,
        /// How many, 1..=500.
        #[arg(long)]
        limit: Option<i64>,
        /// Print the gateway's answer verbatim.
        #[arg(long)]
        json: bool,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Show one proposal in full: what it needs, who has acted, and what
    /// publishing it would do to the target's channel — with a diff.
    Show {
        /// The proposal UUID.
        id: ProposalId,
        /// Print the gateway's answer verbatim.
        #[arg(long)]
        json: bool,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Walk the open proposals one at a time, rendering each in full and
    /// asking for a verdict. Oldest first. End of input casts nothing.
    Review {
        /// Review just this one.
        id: Option<ProposalId>,
        /// Restrict the queue to one scope.
        #[arg(long)]
        scope: Option<ScopeId>,
        /// How many to queue, 1..=500.
        #[arg(long)]
        limit: Option<i64>,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Approve one proposal.
    Approve {
        /// The proposal UUID.
        id: ProposalId,
        /// What you want to say about it.
        #[arg(long)]
        comment: Option<String>,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Reject one proposal, with a reason. Terminal — a revision is a new
    /// proposal.
    Reject {
        /// The proposal UUID.
        id: ProposalId,
        /// Why. Mandatory: a rejection an auditor cannot read the reason
        /// for is not a review.
        #[arg(long)]
        reason: String,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Close your own proposal without a verdict. The proposer's act; a
    /// reviewer rejects with a reason instead.
    Withdraw {
        /// The proposal UUID.
        id: ProposalId,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Run an approved proposal's effect: move the target's published
    /// channel. A separate act from the deciding approval by design
    /// (ADR-0032 decision 9) — it takes `ChannelPublish` and `MemoryRead`
    /// at the target, which the deciding reviewer may not hold.
    Publish {
        /// The proposal UUID.
        id: ProposalId,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Record a decision to publish a skill the quality gate refuses
    /// (SKIL-3, ADR-0053 decision 8).
    ///
    /// Takes `SkillQualityOverride` at the target scope — deliberately a
    /// *different* authority from the one that publishes. Under the
    /// product packs a `curator` publishes skills and a `steward` grants
    /// this, so the person who decided a bundle was good enough is never
    /// the person who records that it was not.
    ///
    /// It is its own act rather than a flag on `publish` for a plainer
    /// reason too: a steward holds no content read, so a steward cannot
    /// publish a skill at all. Grant the override, then let whoever
    /// ordinarily publishes publish.
    ///
    /// The reason is what an auditor will read in a year to find out why
    /// the product shipped something it had itself marked down, so write
    /// it for them. It never waves the *security* scan through: that has
    /// no override at any tier and must not acquire one.
    OverrideQuality {
        /// The proposal UUID.
        id: ProposalId,
        /// Why. Mandatory.
        #[arg(long)]
        reason: String,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Record the reviewer's quality checklist for a skill proposal
    /// (SKIL-3): the half of a skill's score no machine can supply.
    ///
    /// Its own act rather than part of approving, because a reviewer may
    /// legitimately work through the checklist without yet deciding to
    /// ship — indeed under `regulated-strict` that is the point, since the
    /// pack requires one to exist *before* anybody publishes.
    ///
    /// The answers are bound to the bundle's bytes, not to this proposal:
    /// if the author edits a file afterwards, the answers are not found
    /// and the checklist has to be redone. That is deliberate — a review
    /// of bytes nobody has since changed is the only kind worth recording.
    Checklist {
        /// The proposal UUID.
        id: ProposalId,
        /// An answer, repeatable: `--item tested=yes`. Items are
        /// `instructions-correct`, `scope-appropriate`, `not-duplicate`,
        /// `dependencies-available` and `tested`; verdicts are `yes`,
        /// `no` and `n/a`.
        #[arg(long = "item", value_name = "ITEM=VERDICT", required = true)]
        items: Vec<String>,
        /// Anything you want the record to say.
        #[arg(long)]
        note: Option<String>,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Run an approved *classification* proposal's effect: move its
    /// records to the tier the review approved (AUTHZ-5).
    Classify {
        /// The proposal UUID.
        id: ProposalId,
        #[arg(long)]
        profile: Option<String>,
    },
}

#[derive(Subcommand)]
enum McpCommand {
    /// Write a client's own config so it launches this server (ADPT-2,
    /// ADR-0057 decision 10).
    ///
    /// The acceptance criterion is "works in", and "paste this JSON into
    /// that file" is exactly where two clients diverge into a support
    /// burden nobody can test — so the product writes the file and says
    /// what it wrote.
    ///
    /// It changes one key. Your other MCP servers, and everything else in
    /// the file, are written back as they were found; an existing
    /// `synveda` entry that differs is refused rather than replaced.
    Install {
        /// Which client to configure. Pass an unknown name to be told the
        /// ones this installation knows.
        ///
        /// The list is data, not code: it ships in the binary and
        /// `~/.config/synveda/mcp-clients.jsonc` adds to it or overrides
        /// it, so a client we have never heard of needs a file rather than
        /// a release (seed §2 principle 6).
        ///
        /// Claude Code is absent on purpose — its plugin already carries
        /// the entry, and carries it with the write tool switched off.
        #[arg(long)]
        client: String,
        /// Write this file instead of the client's own — a project-level
        /// `.cursor/mcp.json` or `.zed/settings.json`, or a layout this
        /// release has not heard of.
        #[arg(long)]
        config: Option<std::path::PathBuf>,
        /// Report what would change and write nothing.
        #[arg(long)]
        dry_run: bool,
        /// Replace an existing `synveda` entry that differs from this one.
        #[arg(long)]
        force: bool,
        /// Print the entry as JSON and write nothing — for a client this
        /// release does not know, or a config kept somewhere unusual.
        #[arg(long)]
        print: bool,
        /// Credential profile the generated entry names. Defaults to
        /// $SYNVEDA_PROFILE, else `default`.
        #[arg(long)]
        profile: Option<String>,
    },
}

#[derive(Subcommand)]
enum AuthCommand {
    /// Print a currently-valid bearer for the profile, refreshing it
    /// through the gateway first if it has expired. Exits non-zero when
    /// there is nothing to print and says what to run.
    Token {
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
        /// Print the token with its expiry, tenant, and gateway as one
        /// JSON object — the shape the Claude Code adapter reads.
        #[arg(long)]
        json: bool,
    },
    /// Forget a profile's credentials.
    Logout {
        /// Credential profile to forget.
        #[arg(long)]
        profile: Option<String>,
        /// Forget every profile.
        #[arg(long, conflicts_with = "profile")]
        all: bool,
    },
}

#[derive(Subcommand)]
enum AuditCommand {
    /// Walk the tenant's whole chain, recomputing every hash, and report
    /// the first divergence. Exits non-zero on a broken chain.
    Verify {
        /// Tenant UUID.
        #[arg(long)]
        tenant: TenantId,
    },
    /// Print the tenant's most recent events, newest first, as JSON.
    Tail {
        /// Tenant UUID.
        #[arg(long)]
        tenant: TenantId,
        /// How many events.
        #[arg(long, default_value_t = 20)]
        limit: i64,
    },
}

#[derive(Subcommand)]
enum DbCommand {
    /// Apply all pending migrations to DATABASE_URL.
    Migrate,
}

#[derive(Subcommand)]
enum HierarchyCommand {
    /// Create a scope under a parent you may write to.
    Create {
        /// Parent scope UUID. Required — the org root is provisioned by
        /// the first admin login, not created here.
        #[arg(long)]
        parent: ScopeId,
        /// Level: division, department, team. (`org` is the root, and
        /// `user` scopes are provisioned per person, never authored.)
        #[arg(long, value_parser = scope_kind_below_root)]
        kind: ScopeKind,
        /// Human-stable handle, unique among siblings, immutable.
        #[arg(long)]
        slug: String,
        /// Display name.
        #[arg(long)]
        name: String,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else `default`.
        #[arg(long)]
        profile: Option<String>,
        /// Print the gateway's JSON rather than a line.
        #[arg(long)]
        json: bool,
    },
    /// Draw the subtree under a scope, or under the org root.
    List {
        /// Anchor scope UUID. Defaults to the tenant's org root.
        #[arg(long)]
        under: Option<ScopeId>,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// One scope, in full.
    Show {
        /// Scope UUID.
        id: ScopeId,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// The org root's id — the parent every first `create` needs.
    Root {
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

/// Refuses `org` and `user` at the surface (ADR-0011's rank rule and
/// ADR-0055 decision 2): the root has no parent to pass, and a personal
/// scope is provisioned with its identity — an authored one would be a
/// leaf nobody is placed at.
fn scope_kind_below_root(value: &str) -> Result<ScopeKind, String> {
    match value {
        "division" => Ok(ScopeKind::Division),
        "department" => Ok(ScopeKind::Department),
        "team" => Ok(ScopeKind::Team),
        "org" => Err(
            "the org root is created by the first admin login, from the tenant's \
                      own slug and name — there is nothing to create here"
                .to_owned(),
        ),
        "user" => Err(
            "personal scopes are provisioned with their identity at login, \
                       never authored"
                .to_owned(),
        ),
        other => Err(format!(
            "unknown scope kind `{other}` (division, department, team)"
        )),
    }
}

#[derive(Subcommand)]
enum TenantCommand {
    /// Admit a tenant; prints the created tenant as JSON.
    Create {
        /// Human-stable handle: lowercase, hyphenated, unique.
        #[arg(long)]
        slug: String,
        /// Display name.
        #[arg(long)]
        name: String,
        /// Admit in suspended state (its tokens will not resolve).
        #[arg(long)]
        suspended: bool,
    },
}

#[derive(Subcommand)]
enum PolicyCommand {
    /// Compile-check a Cedar pack and apply it for a tenant (bumps the
    /// version; the gateway hot-reloads it within its refresh interval).
    /// Prints the stored pack row as JSON.
    Apply {
        /// Tenant UUID the pack applies to.
        #[arg(long)]
        tenant: TenantId,
        /// Pack name (slug grammar), e.g. `acme-strict`. Product names
        /// (`regulated-strict`, `standard`, `open-collaboration`) are
        /// reserved (ADR-0014).
        #[arg(long)]
        name: String,
        /// Redaction mode for secret findings on observe ingest
        /// (deny/redact/quarantine — MEM-2, ADR-0021). Both redaction
        /// flags must be given together or neither; unconfigured packs
        /// get the strict default (secrets quarantine, PII redact).
        #[arg(long, requires = "redaction_pii")]
        redaction_secrets: Option<RedactionMode>,
        /// Redaction mode for PII findings on observe ingest.
        #[arg(long, requires = "redaction_secrets")]
        redaction_pii: Option<RedactionMode>,
        /// Estimated-token inject budget at scopes this pack governs
        /// (CTX-2, ADR-0025). Both composition flags must be given
        /// together or neither; unconfigured packs get the product
        /// default (1500, published-and-derived).
        #[arg(long, requires = "composition_channels")]
        composition_budget: Option<u32>,
        /// Inject channel rule (published-and-derived/published-only —
        /// published-only is the bank-mode switch).
        #[arg(long, requires = "composition_budget")]
        composition_channels: Option<InjectChannels>,
        /// What happens to material that does not fit the budget
        /// (off/demote — CTX-4, ADR-0041). `demote` names it and hands the
        /// reader a recall handle; `off` drops it in silence, which is how
        /// composition behaved before CTX-4. Omitted keeps the product
        /// config, which demotes. Only meaningful alongside the
        /// composition pair.
        #[arg(long, requires = "composition_budget")]
        composition_index_tier: Option<IndexTier>,
        /// How wide an index line's content is, in characters (default
        /// 320 — the feature's "~80 tokens each" through the chars/4
        /// estimator). Lower it where the corpus is short: a record is
        /// only ever named instead of shown when naming it is genuinely
        /// cheaper, so a width near the median record length turns the
        /// tier off in practice.
        #[arg(long, requires = "composition_budget")]
        composition_index_chars: Option<u32>,
        /// Whether a block names the skills this identity may install
        /// (off/names — SKIL-4, ADR-0054). Unlike the index tier this is
        /// new content rather than a rendering of material the block was
        /// already carrying, so `off` is the switch for a scope whose
        /// readers are already told this by their own client. Omitted
        /// keeps the product config, which names them.
        #[arg(long, requires = "composition_budget")]
        composition_skill_index: Option<SkillIndex>,
        /// The severity at which a skill bundle's security scan refuses
        /// rather than reports (notice/high/critical — SKIL-2, ADR-0052).
        /// Omitted keeps the invariant floor, which is what an
        /// unconfigured pack gets: `critical` always refuses and the rest
        /// is a reviewer's to weigh. `high` is `regulated-strict`'s
        /// reading. There is deliberately no value that permits
        /// `critical` — that band is not a pack's to move.
        #[arg(long)]
        scan_block_at: Option<ScanSeverity>,
        /// Path to a JSON file of auto-promotion rules (FLOW-4,
        /// ADR-0033): `{"rules":[{"name":..., "min_recalls":...,
        /// "min_distinct_members":..., "max_sensitivity":...}]}`. A file
        /// rather than flags because a rule set is a list, not a scalar.
        /// Omitted means the pack carries no rules and nothing
        /// auto-promotes at the scopes it governs — a trigger's fail-safe
        /// is silence.
        #[arg(long)]
        promotion: Option<std::path::PathBuf>,
        /// What the ingestion pipeline does with a restatement or a
        /// contradiction at scopes this pack governs (off/merge/supersede
        /// — MEM-5, ADR-0039). Omitted keeps the product config, which
        /// supersedes; the thresholds are product constants and are tuned
        /// through `--dedup-config` rather than one flag each.
        #[arg(long)]
        dedup_mode: Option<DedupMode>,
        /// Path to a JSON file holding a full `DedupConfig` — the mode
        /// plus the three thresholds in per mille and the nomination
        /// depth. Takes precedence over `--dedup-mode`; a file rather than
        /// five flags for the reason `--promotion` is one.
        #[arg(long, conflicts_with = "dedup_mode")]
        dedup_config: Option<std::path::PathBuf>,
        /// Path to a JSON file holding a full `RetentionConfig` (MEM-6,
        /// ADR-0040): the mode, the per-class record horizons in days,
        /// the destruction and staging horizons, and the staleness
        /// half-life. A file rather than a flag per class for the reason
        /// `--promotion` is one — a schedule is a table, not a scalar.
        ///
        /// Omitted keeps the product config, whose record horizons are
        /// all unset: nothing this CLI does by default can expire or
        /// destroy a tenant's memory.
        #[arg(long)]
        retention: Option<std::path::PathBuf>,
        /// Path to the Cedar policy source file.
        file: std::path::PathBuf,
    },
    /// Remove one of a tenant's stored packs. Refused while assignments
    /// or the tenant default still reference it; scopes it governed fall
    /// back to their inherited pack (ADR-0014).
    Clear {
        /// Tenant UUID.
        #[arg(long)]
        tenant: TenantId,
        /// Stored pack name to remove.
        #[arg(long)]
        name: String,
    },
}

#[derive(Subcommand)]
enum RoleCommand {
    /// Bind a role to a subject; prints the binding as JSON. Without
    /// --scope the binding is tenant-wide (in force everywhere, the
    /// tenant plane included).
    Bind {
        /// Tenant UUID.
        #[arg(long)]
        tenant: TenantId,
        /// The token subject to bind.
        #[arg(long)]
        subject: String,
        /// The role (viewer/contributor/curator/steward/org-admin/
        /// auditor/security-reviewer/compliance).
        #[arg(long)]
        role: Role,
        /// Hierarchy node UUID to bind at; omit for tenant-wide.
        #[arg(long)]
        scope: Option<ScopeId>,
    },
    /// Remove one binding (exact subject + role + scope).
    Unbind {
        /// Tenant UUID.
        #[arg(long)]
        tenant: TenantId,
        /// The bound subject.
        #[arg(long)]
        subject: String,
        /// The bound role.
        #[arg(long)]
        role: Role,
        /// The bound node UUID; omit for the tenant-wide binding.
        #[arg(long)]
        scope: Option<ScopeId>,
    },
    /// List every binding of the tenant as JSON.
    List {
        /// Tenant UUID.
        #[arg(long)]
        tenant: TenantId,
    },
}

#[derive(Subcommand)]
enum ServiceCommand {
    /// Register a service identity at an anchor node: creates its
    /// personal leaf under the anchor and the identity row (kind
    /// `service`); prints the identity as JSON. Tokens for its subject
    /// are then confined to the anchor's subtree.
    Register {
        /// Tenant UUID.
        #[arg(long)]
        tenant: TenantId,
        /// The `sub` the IdP puts in the agent's client-credentials
        /// tokens (for Rauthy, the client id).
        #[arg(long)]
        subject: String,
        /// The anchor node UUID.
        #[arg(long)]
        scope: ScopeId,
        /// Display name; defaults to the subject.
        #[arg(long)]
        name: Option<String>,
    },
    /// Revoke a service identity: deletes the row and its personal leaf.
    Remove {
        /// Tenant UUID.
        #[arg(long)]
        tenant: TenantId,
        /// The registered identity UUID (see `service list`).
        #[arg(long)]
        id: IdentityId,
    },
    /// List the tenant's service identities as JSON.
    List {
        /// Tenant UUID.
        #[arg(long)]
        tenant: TenantId,
    },
}

#[derive(Subcommand)]
enum TokenCommand {
    /// Issue a token signed with SYNVEDA_DEV_JWT_SECRET; prints the raw
    /// token to stdout.
    Issue {
        /// Tenant UUID for the `tid` claim.
        #[arg(long)]
        tenant: TenantId,
        /// Subject for the `sub` claim.
        #[arg(long, default_value = "dev")]
        subject: String,
        /// Lifetime in seconds for the `exp` claim.
        #[arg(long, default_value_t = 3600)]
        ttl_secs: u64,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("synveda: {message}");
            ExitCode::FAILURE
        }
    }
}

/// The credential profile a command acts on: `--profile`, then
/// `SYNVEDA_PROFILE`, then `default`.
/// Reads a template from a file, or from standard input for `-`.
///
/// A flag rather than an inline argument: a prompt is a document, and a
/// shell that had to quote one would mangle exactly the `{{ }}` the schema
/// is about.
fn read_template(path: &str) -> Result<String, String> {
    if path == "-" {
        use std::io::Read;
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|err| format!("read the template from stdin: {err}"))?;
        return Ok(text);
    }
    std::fs::read_to_string(path).map_err(|err| format!("read {path}: {err}"))
}

fn profile_name(flag: Option<String>) -> String {
    flag.or_else(|| std::env::var("SYNVEDA_PROFILE").ok())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| credentials::DEFAULT_PROFILE.to_owned())
}

#[tokio::main(flavor = "current_thread")]
async fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Init {
            slug,
            name,
            embedder,
            issuer,
            demo,
            dry_run,
        } => {
            init::init(init::Plan {
                slug,
                name,
                embedder,
                issuer,
                demo,
                dry_run,
            })
            .await
        }
        Command::Hierarchy(HierarchyCommand::Create {
            parent,
            kind,
            slug,
            name,
            profile,
            json,
        }) => {
            hierarchy::create(
                &profile_name(profile),
                hierarchy::NewNode {
                    parent,
                    kind,
                    slug: &slug,
                    name: &name,
                },
                json,
            )
            .await
        }
        Command::Hierarchy(HierarchyCommand::List {
            under,
            profile,
            json,
        }) => hierarchy::list(&profile_name(profile), under, json).await,
        Command::Hierarchy(HierarchyCommand::Show { id, profile, json }) => {
            hierarchy::show(&profile_name(profile), id, json).await
        }
        Command::Hierarchy(HierarchyCommand::Root { profile, json }) => {
            hierarchy::root(&profile_name(profile), json).await
        }
        Command::Login {
            gateway,
            issuer,
            profile,
            no_browser,
        } => {
            login::login(
                login::gateway_url(gateway),
                issuer,
                profile_name(profile),
                !no_browser,
            )
            .await
        }
        Command::Auth(AuthCommand::Token { profile, json }) => {
            login::auth_token(profile_name(profile), json).await
        }
        Command::Mcp {
            command: None,
            writes,
            profile,
        } => mcp::serve(profile_name(profile), writes).await,
        Command::Mcp {
            command:
                Some(McpCommand::Install {
                    client,
                    config,
                    dry_run,
                    force,
                    print,
                    profile,
                }),
            ..
        } => mcp::install::install(&mcp::install::Plan {
            client,
            config,
            profile: profile_name(profile),
            dry_run,
            force,
            print,
        }),
        Command::Auth(AuthCommand::Logout { profile, all }) => {
            let mut stored = credentials::load()?;
            if all {
                let count = stored.profiles.len();
                stored.profiles.clear();
                credentials::save(&stored)?;
                eprintln!("synveda: forgot {count} profile(s)");
            } else {
                let name = profile_name(profile);
                if stored.profiles.remove(&name).is_none() {
                    return Err(format!("no credentials for profile `{name}`"));
                }
                credentials::save(&stored)?;
                eprintln!("synveda: forgot profile `{name}`");
            }
            // The gateway is not told: the IdP owns revocation, and a
            // local forget is exactly that — local (ADR-0027 decision 6).
            eprintln!(
                "         the tokens themselves remain valid at the issuer until they expire"
            );
            Ok(())
        }
        Command::Db(DbCommand::Migrate) => {
            let pool = connect().await?;
            synveda_store::migrate(&pool)
                .await
                .map_err(|err| err.to_string())?;
            eprintln!("migrations applied");
            Ok(())
        }
        Command::Tenant(TenantCommand::Create {
            slug,
            name,
            suspended,
        }) => {
            let status = if suspended {
                TenantStatus::Suspended
            } else {
                TenantStatus::Active
            };
            let pool = connect().await?;
            let tenant = create_tenant(&pool, &slug, &name, status).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&tenant).map_err(|err| err.to_string())?
            );
            Ok(())
        }
        Command::Policy(PolicyCommand::Apply {
            tenant,
            name,
            redaction_secrets,
            redaction_pii,
            composition_budget,
            composition_channels,
            composition_index_tier,
            composition_index_chars,
            composition_skill_index,
            scan_block_at,
            promotion,
            dedup_mode,
            dedup_config,
            retention,
            file,
        }) => {
            let source = std::fs::read_to_string(&file)
                .map_err(|err| format!("read {}: {err}", file.display()))?;
            // Compile-check before storing: same schema, same validation
            // the gateway's reloader applies (ADR-0012 decision 2).
            synveda_policy::Pdp::new()
                .and_then(|pdp| pdp.compile_check(&name, &source))
                .map_err(|err| err.to_string())?;
            // clap's `requires` makes each config's flags all-or-nothing.
            let redaction = redaction_secrets
                .zip(redaction_pii)
                .map(|(secrets, pii)| RedactionConfig { secrets, pii });
            let composition =
                composition_budget
                    .zip(composition_channels)
                    .map(|(budget_tokens, channels)| CompositionConfig {
                        budget_tokens,
                        channels,
                        // Omitted keeps the product config's tier rather
                        // than turning the index off: a flag nobody passed
                        // must not silently remove a rendering the pack it
                        // replaces was serving (ADR-0041 decision 11).
                        index_tier: composition_index_tier
                            .unwrap_or(CompositionConfig::DEFAULT.index_tier),
                        index_entry_chars: composition_index_chars
                            .unwrap_or(CompositionConfig::DEFAULT.index_entry_chars),
                        // Same rule as the tier above, and it matters more
                        // here: omitting the flag must not silently stop a
                        // fleet being told which skills it may install.
                        skill_index: composition_skill_index
                            .unwrap_or(CompositionConfig::DEFAULT.skill_index),
                    });
            let scan = scan_block_at.map(|block_at| SkillScanConfig { block_at });
            // Validated here as well as at install: a rule that asks for
            // zero recalls, or names an asset with no usage signal, is
            // refused when it is written rather than discovered when a
            // sweep silently does nothing (ADR-0033 decision 6).
            let promotion = promotion
                .map(|path| {
                    let raw = std::fs::read_to_string(&path)
                        .map_err(|err| format!("read {}: {err}", path.display()))?;
                    let config: PromotionConfig = serde_json::from_str(&raw)
                        .map_err(|err| format!("parse {}: {err}", path.display()))?;
                    config
                        .validate()
                        .map_err(|err| format!("{}: {err}", path.display()))?;
                    Ok::<_, String>(config)
                })
                .transpose()?;
            // A threshold outside `0..=1` makes a band unreachable, which
            // reads as "dedup is off" without the pack ever saying so —
            // refused here as well as at install (ADR-0039 decision 12).
            let dedup = match dedup_config {
                Some(path) => {
                    let raw = std::fs::read_to_string(&path)
                        .map_err(|err| format!("read {}: {err}", path.display()))?;
                    let config: DedupConfig = serde_json::from_str(&raw)
                        .map_err(|err| format!("parse {}: {err}", path.display()))?;
                    config
                        .validate()
                        .map_err(|err| format!("{}: {err}", path.display()))?;
                    Some(config)
                }
                None => dedup_mode.map(|mode| DedupConfig {
                    mode,
                    ..DedupConfig::DEFAULT
                }),
            };
            // A schedule written in seconds, or a staging horizon that
            // would spend MEM-1's idempotency guarantee for nothing, is
            // refused here as well as at install — before it destroys
            // something, rather than after (ADR-0040 decision 7).
            let retention = retention
                .map(|path| {
                    let raw = std::fs::read_to_string(&path)
                        .map_err(|err| format!("read {}: {err}", path.display()))?;
                    let config: RetentionConfig = serde_json::from_str(&raw)
                        .map_err(|err| format!("parse {}: {err}", path.display()))?;
                    config
                        .validate()
                        .map_err(|err| format!("{}: {err}", path.display()))?;
                    Ok::<_, String>(config)
                })
                .transpose()?;
            let pool = connect().await?;
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let pack = synveda_store::policy_packs::apply(
                &mut *tx,
                tenant,
                &name,
                &source,
                &PackConfig {
                    redaction,
                    composition,
                    promotion,
                    dedup,
                    retention,
                    scan,
                    ..Default::default()
                },
            )
            .await
            .map_err(|err| err.to_string())?;
            record_break_glass(
                &mut tx,
                tenant,
                AuditAction::PolicyPackApplied,
                format!("tenant {tenant}"),
                json!({
                    "pack": pack.name,
                    "version": pack.version,
                    "redaction": pack.config.redaction,
                    "composition": pack.config.composition,
                    "scan": pack.config.scan,
                    "promotion": pack.config.promotion,
                    "retention": pack.config.retention,
                }),
            )
            .await?;
            tx.commit().await.map_err(|err| err.to_string())?;
            println!(
                "{}",
                serde_json::json!({
                    "tenant_id": pack.tenant_id,
                    "name": pack.name,
                    "version": pack.version,
                    "updated_at": pack.updated_at,
                })
            );
            Ok(())
        }
        Command::Policy(PolicyCommand::Clear { tenant, name }) => {
            let pool = connect().await?;
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let removed = synveda_store::policy_packs::clear(&mut tx, tenant, &name)
                .await
                .map_err(|err| err.to_string())?;
            if removed {
                record_break_glass(
                    &mut tx,
                    tenant,
                    AuditAction::PolicyPackCleared,
                    format!("tenant {tenant}"),
                    json!({"pack": name}),
                )
                .await?;
            }
            tx.commit().await.map_err(|err| err.to_string())?;
            eprintln!(
                "{}",
                if removed {
                    "policy pack cleared; it unloads on the next reload sweep"
                } else {
                    "no stored pack by that name"
                }
            );
            Ok(())
        }
        Command::Role(RoleCommand::Bind {
            tenant,
            subject,
            role,
            scope,
        }) => {
            let pool = connect().await?;
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let binding =
                synveda_store::role_bindings::bind(&mut *tx, tenant, &subject, scope, role)
                    .await
                    .map_err(|err| err.to_string())?;
            record_break_glass(
                &mut tx,
                tenant,
                AuditAction::RoleBound,
                scope.map_or_else(|| format!("tenant {tenant}"), |id| format!("scope {id}")),
                json!({"binding": {"subject": subject, "role": role, "scope_id": scope}}),
            )
            .await?;
            tx.commit().await.map_err(|err| err.to_string())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&binding).map_err(|err| err.to_string())?
            );
            Ok(())
        }
        Command::Role(RoleCommand::Unbind {
            tenant,
            subject,
            role,
            scope,
        }) => {
            let pool = connect().await?;
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let removed =
                synveda_store::role_bindings::unbind(&mut *tx, tenant, &subject, scope, role)
                    .await
                    .map_err(|err| err.to_string())?;
            if removed {
                record_break_glass(
                    &mut tx,
                    tenant,
                    AuditAction::RoleUnbound,
                    scope.map_or_else(|| format!("tenant {tenant}"), |id| format!("scope {id}")),
                    json!({"binding": {"subject": subject, "role": role, "scope_id": scope}}),
                )
                .await?;
            }
            tx.commit().await.map_err(|err| err.to_string())?;
            eprintln!(
                "{}",
                if removed {
                    "role binding removed; it is out of force on the next request"
                } else {
                    "no such binding"
                }
            );
            Ok(())
        }
        Command::Role(RoleCommand::List { tenant }) => {
            let pool = connect().await?;
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let bindings = synveda_store::role_bindings::all(&mut *tx, tenant)
                .await
                .map_err(|err| err.to_string())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&bindings).map_err(|err| err.to_string())?
            );
            Ok(())
        }
        Command::Service(ServiceCommand::Register {
            tenant,
            subject,
            scope,
            name,
        }) => {
            let pool = connect().await?;
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let anchor = synveda_store::hierarchy::node(&mut *tx, scope)
                .await
                .map_err(|err| err.to_string())?
                .filter(|node| node.tenant_id == tenant)
                .ok_or_else(|| format!("no scope {scope} in tenant {tenant}"))?;
            if anchor.slug == synveda_store::identities::QUARANTINE_SLUG && anchor.depth == 1 {
                return Err(
                    "service identities cannot be anchored at the quarantine scope".to_owned(),
                );
            }
            let identity_id = IdentityId::new();
            let display_name = name.as_deref().unwrap_or(&subject);
            let leaf = synveda_store::hierarchy::create(
                &mut tx,
                ScopeId::new(),
                tenant,
                Some(anchor.id),
                ScopeKind::User,
                &personal_slug(None, &subject, identity_id),
                display_name,
            )
            .await
            .map_err(|err| err.to_string())?;
            let identity = synveda_store::identities::create(
                &mut tx,
                identity_id,
                tenant,
                &subject,
                IdentityKind::Service,
                None,
                name.as_deref(),
                leaf.id,
            )
            .await
            .map_err(|err| err.to_string())?;
            record_break_glass(
                &mut tx,
                tenant,
                AuditAction::ServiceIdentityRegistered,
                format!("scope {}", anchor.id),
                json!({
                    "identity": {"id": identity.id, "subject": identity.subject},
                    "leaf_scope_id": leaf.id,
                    "anchor": {"slug": anchor.slug, "path": anchor.path},
                }),
            )
            .await?;
            tx.commit().await.map_err(|err| err.to_string())?;
            eprintln!(
                "note: a running gateway caches hierarchy out-of-process; restart it or use the API path"
            );
            println!(
                "{}",
                serde_json::to_string_pretty(&identity).map_err(|err| err.to_string())?
            );
            Ok(())
        }
        Command::Service(ServiceCommand::Remove { tenant, id }) => {
            let pool = connect().await?;
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let identity = synveda_store::identities::by_id(&mut *tx, tenant, id)
                .await
                .map_err(|err| err.to_string())?
                .filter(|identity| identity.kind == IdentityKind::Service)
                .ok_or_else(|| format!("no service identity {id} in tenant {tenant}"))?;
            // The anchor (the leaf's parent) names the event's resource,
            // read before the leaf goes.
            let anchor_id = synveda_store::hierarchy::node(&mut *tx, identity.scope_id)
                .await
                .map_err(|err| err.to_string())?
                .and_then(|leaf| leaf.parent_id);
            // Row first (its FK pins the leaf), then the leaf.
            synveda_store::identities::delete_service(&mut *tx, tenant, id)
                .await
                .map_err(|err| err.to_string())?;
            synveda_store::hierarchy::delete(&mut tx, identity.scope_id)
                .await
                .map_err(|err| err.to_string())?;
            record_break_glass(
                &mut tx,
                tenant,
                AuditAction::ServiceIdentityRevoked,
                anchor_id.map_or_else(|| format!("tenant {tenant}"), |id| format!("scope {id}")),
                json!({"identity": {"id": identity.id, "subject": identity.subject}}),
            )
            .await?;
            tx.commit().await.map_err(|err| err.to_string())?;
            eprintln!("service identity revoked; its tokens are quarantined from the next request");
            Ok(())
        }
        Command::Service(ServiceCommand::List { tenant }) => {
            let pool = connect().await?;
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let identities = synveda_store::identities::services(&mut *tx, tenant)
                .await
                .map_err(|err| err.to_string())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&identities).map_err(|err| err.to_string())?
            );
            Ok(())
        }
        Command::Audit(AuditCommand::Verify { tenant }) => {
            let pool = connect().await?;
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let verification = synveda_audit::verify(&mut tx, tenant)
                .await
                .map_err(|err| err.to_string())?;
            println!("{verification}");
            match verification {
                synveda_audit::ChainVerification::Valid { .. } => Ok(()),
                synveda_audit::ChainVerification::Broken { .. } => {
                    Err("audit chain verification failed".to_owned())
                }
            }
        }
        Command::Audit(AuditCommand::Tail { tenant, limit }) => {
            let pool = connect().await?;
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let events = synveda_audit::tail(&mut tx, tenant, limit)
                .await
                .map_err(|err| err.to_string())?;
            for event in events {
                println!(
                    "{}",
                    json!({
                        "seq": event.seq,
                        "occurred_at": event.occurred_at,
                        "actor": {"kind": event.actor_kind, "subject": event.actor_subject},
                        "action": event.action,
                        "resource": event.resource,
                        "outcome": event.outcome,
                        "payload": event.payload,
                        "trace_id": event.trace_id,
                        "hash": event.hash_hex(),
                    })
                );
            }
            Ok(())
        }
        Command::Proposal(command) => match command {
            ProposalCommand::List {
                scope,
                state,
                limit,
                json,
                profile,
            } => proposal::list(&profile_name(profile), scope, state, limit, json).await,
            ProposalCommand::Show { id, json, profile } => {
                proposal::show(&profile_name(profile), id, json).await
            }
            ProposalCommand::Review {
                id,
                scope,
                limit,
                profile,
            } => proposal::review(&profile_name(profile), id, scope, limit).await,
            ProposalCommand::Approve {
                id,
                comment,
                profile,
            } => proposal::approve(&profile_name(profile), id, comment).await,
            ProposalCommand::Reject {
                id,
                reason,
                profile,
            } => proposal::reject(&profile_name(profile), id, reason).await,
            ProposalCommand::Withdraw { id, profile } => {
                proposal::withdraw(&profile_name(profile), id).await
            }
            ProposalCommand::Publish { id, profile } => {
                proposal::publish(&profile_name(profile), id).await
            }
            ProposalCommand::OverrideQuality {
                id,
                reason,
                profile,
            } => proposal::override_quality(&profile_name(profile), id, &reason).await,
            ProposalCommand::Checklist {
                id,
                items,
                note,
                profile,
            } => proposal::checklist(&profile_name(profile), id, &items, note).await,
            ProposalCommand::Classify { id, profile } => {
                proposal::classify(&profile_name(profile), id).await
            }
        },
        Command::Prompt(command) => match command {
            PromptCommand::List { scope, profile } => {
                prompt::list(&profile_name(profile), scope).await
            }
            PromptCommand::Show {
                name,
                scope,
                draft,
                commit,
                values,
                json,
                quiet,
                profile,
            } => {
                prompt::show(
                    &profile_name(profile),
                    prompt::Ask {
                        name: &name,
                        scope,
                        draft,
                        commit: commit.as_deref(),
                        values: &values,
                    },
                    json,
                    quiet,
                )
                .await
            }
            PromptCommand::Author {
                name,
                scope,
                file,
                description,
                variables,
                sensitivity,
                profile,
            } => {
                let template = read_template(&file)?;
                let sensitivity = sensitivity
                    .as_deref()
                    .map(str::parse::<synveda_types::Sensitivity>)
                    .transpose()
                    .map_err(|err| err.to_string())?;
                prompt::author(
                    &profile_name(profile),
                    prompt::Draft {
                        name: &name,
                        scope,
                        description: &description,
                        template,
                        variables: &variables,
                        sensitivity,
                    },
                )
                .await
            }
            PromptCommand::Propose {
                name,
                scope,
                title,
                profile,
            } => prompt::propose(&profile_name(profile), &name, scope, title.as_deref()).await,
        },
        Command::Skill(command) => match command {
            SkillCommand::List { scope, profile } => {
                skill::list(&profile_name(profile), scope).await
            }
            SkillCommand::Show {
                name,
                scope,
                draft,
                commit,
                json,
                quiet,
                profile,
            } => {
                skill::show(
                    &profile_name(profile),
                    skill::Ask {
                        name: &name,
                        scope,
                        draft,
                        commit: commit.as_deref(),
                    },
                    json,
                    quiet,
                )
                .await
            }
            SkillCommand::Import {
                dir,
                scope,
                name,
                sensitivity,
                profile,
            } => {
                let sensitivity = sensitivity
                    .as_deref()
                    .map(str::parse::<synveda_types::Sensitivity>)
                    .transpose()
                    .map_err(|err| err.to_string())?;
                skill::import(
                    &profile_name(profile),
                    &dir,
                    scope,
                    name.as_deref(),
                    sensitivity,
                )
                .await
            }
            SkillCommand::Install {
                name,
                client,
                root,
                scope,
                commit,
                json,
                profile,
            } => {
                skill::install(
                    &profile_name(profile),
                    skill::Ask {
                        name: &name,
                        scope,
                        draft: false,
                        commit: commit.as_deref(),
                    },
                    &client,
                    root.as_deref(),
                    json,
                )
                .await
            }
            SkillCommand::Available { json, profile } => {
                skill::available(&profile_name(profile), json).await
            }
            SkillCommand::Sync {
                client,
                root,
                dry_run,
                json,
                profile,
            } => {
                skill::sync(
                    &profile_name(profile),
                    &client,
                    root.as_deref(),
                    dry_run,
                    json,
                )
                .await
            }
            SkillCommand::Propose {
                name,
                scope,
                source,
                title,
                profile,
            } => {
                skill::propose(
                    &profile_name(profile),
                    &name,
                    scope,
                    source,
                    title.as_deref().unwrap_or(&name),
                )
                .await
            }
        },
        Command::ContextPack(command) => match command {
            ContextPackCommand::List { scope, profile } => {
                pack::list(&profile_name(profile), scope).await
            }
            ContextPackCommand::Author {
                name,
                scope,
                files,
                root,
                description,
                sensitivity,
                profile,
            } => {
                let sensitivity = sensitivity
                    .as_deref()
                    .map(str::parse::<synveda_types::Sensitivity>)
                    .transpose()
                    .map_err(|err| err.to_string())?;
                pack::author(
                    &profile_name(profile),
                    pack::Bundle {
                        name: &name,
                        scope,
                        description: &description,
                        files: &files,
                        root: root.as_ref(),
                        sensitivity,
                    },
                )
                .await
            }
            ContextPackCommand::Propose {
                name,
                scope,
                title,
                profile,
            } => pack::propose(&profile_name(profile), &name, scope, title.as_deref()).await,
        },
        Command::Recall {
            ids,
            query,
            as_of,
            valid_at,
            limit,
            json,
            quiet,
            profile,
        } => {
            recall::recall(
                &profile_name(profile),
                recall::Ask {
                    ids: &ids,
                    query: query.as_deref(),
                    as_of,
                    valid_at,
                    limit,
                },
                json,
                quiet,
            )
            .await
        }
        Command::Channel(command) => match command {
            ChannelCommand::Status {
                scope,
                json,
                profile,
            } => channel::status(&profile_name(profile), scope, json).await,
            ChannelCommand::History {
                scope,
                channel,
                limit,
                json,
                profile,
            } => channel::history(&profile_name(profile), scope, channel, limit, json).await,
            ChannelCommand::Rollback {
                scope,
                from,
                to,
                message,
                channel,
                json,
                profile,
            } => {
                channel::rollback(
                    &profile_name(profile),
                    scope,
                    from,
                    to,
                    message,
                    channel,
                    json,
                )
                .await
            }
            ChannelCommand::Pin {
                scope,
                commit,
                reason,
                channel,
                json,
                profile,
            } => channel::pin(&profile_name(profile), scope, commit, reason, channel, json).await,
            ChannelCommand::Unpin {
                scope,
                reason,
                channel,
                json,
                profile,
            } => channel::unpin(&profile_name(profile), scope, reason, channel, json).await,
        },
        Command::Token(TokenCommand::Issue {
            tenant,
            subject,
            ttl_secs,
        }) => {
            let secret = std::env::var("SYNVEDA_DEV_JWT_SECRET")
                .ok()
                .filter(|secret| !secret.is_empty())
                .ok_or("SYNVEDA_DEV_JWT_SECRET must be set to issue dev tokens")?;
            let token = Hs256Verifier::new(secret.as_bytes()).issue(
                &subject,
                tenant,
                Duration::from_secs(ttl_secs),
            );
            println!("{token}");
            Ok(())
        }
    }
}

/// The break-glass actor: whoever holds the database credentials, named
/// as well as the OS names them — honest about attribution being weaker
/// than the IdP-authenticated plane (AUD-1, ADR-0019 decision 7).
fn break_glass() -> Actor {
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "unknown".to_owned());
    Actor::break_glass(user)
}

/// Admits a tenant and chains `tenant.created` in the same transaction.
///
/// Shared by `synveda tenant create` and `synveda init` (OPS-1, ADR-0055
/// decision 1) so that the installer takes the *existing* audited
/// break-glass path rather than a second one that could drift from it —
/// this and `db migrate` are the only store-level writes on the install
/// path, and both predate it.
pub(crate) async fn create_tenant(
    pool: &sqlx::PgPool,
    slug: &str,
    name: &str,
    status: TenantStatus,
) -> Result<synveda_types::Tenant, String> {
    let tenant_id = TenantId::new();
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant_id)
        .await
        .map_err(|err| err.to_string())?;
    let tenant = synveda_store::tenants::create(&mut *tx, tenant_id, slug, name, status)
        .await
        .map_err(|err| err.to_string())?;
    record_break_glass(
        &mut tx,
        tenant_id,
        AuditAction::TenantCreated,
        format!("tenant {tenant_id}"),
        json!({"slug": tenant.slug, "name": tenant.name, "status": tenant.status}),
    )
    .await?;
    tx.commit().await.map_err(|err| err.to_string())?;
    Ok(tenant)
}

/// Chains a break-glass event in the same transaction as the mutation it
/// records: the CLI audits itself like the gateway does (ADR-0019
/// decision 7) — a failed append fails the command.
async fn record_break_glass(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    action: AuditAction,
    resource: String,
    payload: serde_json::Value,
) -> Result<(), String> {
    synveda_audit::append(
        tx,
        tenant,
        &AuditEvent {
            occurred_at: chrono::Utc::now(),
            actor: break_glass(),
            action,
            resource,
            outcome: Outcome::Success,
            payload,
            trace_id: None,
        },
    )
    .await
    .map(|_| ())
    .map_err(|err| err.to_string())
}

async fn connect() -> Result<sqlx::PgPool, String> {
    let url = std::env::var("DATABASE_URL")
        .map_err(|_| "DATABASE_URL must be set (dev default is in the Makefile)")?;
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .map_err(|err| format!("connect to DATABASE_URL: {err}"))
}
