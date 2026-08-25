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
mod directory;
mod init;
mod keys;
mod lapse;
mod login;
mod mcp;
mod okf;
mod pack;
mod plugin;
mod prompt;
mod proposal;
mod recall;
mod reset;
mod scim;
mod scope;
mod session;
mod skill;
mod spool;
#[cfg(test)]
mod testing;
mod whoami;

use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use synveda_audit::{Actor, AuditAction, AuditEvent, Outcome};
use synveda_identity::Hs256Verifier;
use synveda_types::{
    CompositionConfig, DedupConfig, DedupMode, IdentityId, IdentityKind, IndexTier, InjectChannels,
    PackConfig, PromotionConfig, ProposalId, ProposalState, RedactionConfig, RedactionMode,
    RetentionConfig, ScanSeverity, ScopeId, SkillIndex, SkillScanConfig, TenantId, TenantStatus,
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
    /// the issuer, and starts the stack. The operator's identity, their own
    /// scope and their `administrator` grant at the tenant root all arrive
    /// on the first `synveda login`, from AUTH-2's provisioning transaction,
    /// chained under the operator's own subject. Workspaces, projects and
    /// org units are `POST /v1/workspaces` and `synveda scope create` after
    /// that.
    ///
    /// There is no path in here that writes a scope, an identity, a grant
    /// or a record behind the PDP's back — an installer runs once, as
    /// root-equivalent, before anybody is watching, which makes it the
    /// worst place in the product to keep a shortcut (seed §2.2).
    Init {
        /// Tenant slug to admit: lowercase, hyphenated. Also becomes the
        /// tenant root scope's slug when the first thing that needs a parent
        /// mints it.
        #[arg(long, default_value = "acme")]
        slug: String,
        /// Tenant display name; becomes the tenant root scope's name.
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
    /// The governed scope tree (CPR-7, ADR-0074 decision 5).
    ///
    /// Gateway calls under the bearer `synveda login` stored, like
    /// `proposal` and `recall`: creating a scope is a governed act whose
    /// `ScopeCreate` decision the PDP takes at the parent scope and whose
    /// event the gateway chains under your own identity.
    ///
    /// The tenant root is not creatable here — it is minted by the first
    /// thing that needs a parent (ADR-0071), so every `create` has one.
    /// There is no `delete`: retiring a scope is `--status archived`
    /// through the API, because a scope is what audit events, versions and
    /// grants name.
    #[command(subcommand)]
    Scope(ScopeCommand),
    /// Which identity is acting, and what it may do tenant-wide.
    ///
    /// The first question anybody asks a deployment they just logged into,
    /// and until CNSL-2 the answer was a `curl` (ADR-0058 decision 8).
    Whoami {
        /// Also probe the tenant plane: what this caller may do there.
        #[arg(long)]
        capabilities: bool,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// The directory plane's credentials (AUTH-4, ADR-0059 decision 13):
    /// the static bearer Entra and Okta authenticate `/scim/v2` with.
    ///
    /// A governed act over HTTP, not operator plumbing: issuing one is
    /// decided by the PDP at the tenant and chained, so a verb that wrote
    /// the row directly would answer a governed question with no decision
    /// in the trail.
    #[command(subcommand)]
    Scim(ScimCommand),
    /// The scheduled directory pull sync: what the last pass did, and the
    /// authorisation that releases its circuit breaker (AUTH-5, ADR-0060).
    #[command(subcommand)]
    Directory(DirectoryCommand),
    /// What is currently relaxed, and over what (AUTHZ-4's grants).
    ///
    /// The lapse machinery is this product's answer to "strict by default,
    /// relaxable by design" (seed §2.3), and until CNSL-2 there was no
    /// terminal in which to ask what was relaxed.
    #[command(subcommand)]
    Lapse(LapseCommand),
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
        /// The workspace this server's run belongs to (CPR-12, ADR-0078).
        ///
        /// Every write and every composition names the run it belongs to, and
        /// a run happens in a workspace. Omit it when you have one workspace
        /// and the server will use it; with more than one it asks, rather
        /// than writing a model's assertions into whichever sorted first.
        #[arg(long)]
        workspace: Option<String>,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else
        /// `default`.
        #[arg(long)]
        profile: Option<String>,
    },
    /// The Claude Code plugin (ADPT-1, ADR-0027): session-start injection
    /// and turn observation, installed into the harness that runs them.
    ///
    /// Separate from `mcp install` because Claude Code has its own CLI and
    /// its own plugin state, so this drives `claude plugin` rather than
    /// writing files another application owns.
    #[command(subcommand)]
    Plugin(PluginCommand),
    /// Database administration (dev bootstrap).
    #[command(subcommand)]
    Db(DbCommand),
    /// Destroy this deployment's database and build a fresh one at the
    /// current schema epoch (CPR-2, ADR-0069).
    ///
    /// Synveda is pre-1.0 and the context-platform redesign is a hard cut:
    /// there is no migration from the schema that came before it, so a
    /// database written before it is refused at startup rather than upgraded.
    /// This is what an operator runs next, and it is destruction rather than
    /// translation — every tenant, record and audit event in that database
    /// goes.
    ///
    /// It keeps everything that is not the database: `kms.key`, the compose
    /// profile, the console bundle, your stored logins, the Docker volumes,
    /// and the other databases on the same server — Temporal's two share the
    /// volume with ours, which is why this drops a database rather than a
    /// volume.
    Reset {
        /// What to reset. Required: `reset` names what it destroys rather
        /// than defaulting to everything there is.
        #[arg(long)]
        database: bool,
        /// Required. Without it nothing is destroyed and the command says
        /// what it would have done.
        #[arg(long)]
        force: bool,
    },
    /// The key-encryption key (TEN-4, ADR-0064). Everything else in the key
    /// plane is wrapped by this one, and it lives outside the database.
    #[command(subcommand)]
    Kms(KmsCommand),
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
    /// The immutable Skills catalogue (CPR-23, ADR-0085): import an Agent
    /// Skills-compatible bundle through VedaFlow, inspect exact versions,
    /// resolve project/personal bindings, and **install** an authorised
    /// version into a client's own skills directory.
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
    /// Open Knowledge Format v0.2 exchange (CPR-28, ADR-0087).
    ///
    /// Validation and inspection are local and use the exact pinned adapter.
    /// Import/export acts use the public project API; the gateway receives
    /// inert bytes, never a local path or permission to run Git/content.
    #[command(subcommand)]
    Okf(OkfCommand),
    /// The durable observation spool (CPR-12, ADR-0078).
    ///
    /// An agent client records what happened into a local spool before it
    /// tries to deliver it, so an unreachable gateway, a killed hook, a
    /// compaction or a reboot costs nothing. These are the commands for
    /// looking at that spool and pushing it.
    #[command(subcommand)]
    Session(SessionCommand),
    /// Query current governed Knowledge for a question (CPR-20, ADR-0084).
    ///
    /// Opens an ephemeral run and performs the public session-scoped Knowledge
    /// query under the bearer `synveda login` stored. The PDP decides each
    /// exact Knowledge revision and provenance descriptor.
    Recall {
        /// The question to answer.
        #[arg(long)]
        query: String,
        /// The workspace to compose in. Needed only when you can see more
        /// than one.
        #[arg(long)]
        workspace: Option<String>,
        /// Maximum current Knowledge results to return (1–100).
        #[arg(long)]
        limit: Option<u32>,
        /// Print the gateway's answer verbatim.
        #[arg(long)]
        json: bool,
        /// Skip the "reading as ..." line — for a harness piping the block
        /// straight into a session.
        #[arg(long)]
        quiet: bool,
        #[arg(long)]
        profile: Option<String>,
    },
}

#[derive(Subcommand)]
enum SessionCommand {
    /// Deliver every unacknowledged event now.
    ///
    /// The hooks do this on their own schedule — `SessionStart` retries the
    /// backlog, `Stop` delivers the turn, `SessionEnd` flushes within a
    /// bounded deadline. This is the same delivery, on demand, for when a
    /// gateway has been unreachable and you would rather not wait for the
    /// next session to start.
    Flush {
        /// The spool directory. Defaults to $XDG_STATE_HOME/synveda/spool.
        #[arg(long)]
        dir: Option<std::path::PathBuf>,
        /// Name each session as it goes.
        #[arg(long)]
        verbose: bool,
        /// Credential profile. Defaults to $SYNVEDA_PROFILE, else `default`.
        #[arg(long)]
        profile: Option<String>,
    },
    /// The spool itself: what is held and what has been delivered.
    #[command(subcommand)]
    Spool(SpoolCommand),
}

#[derive(Subcommand)]
enum SpoolCommand {
    /// What is held, per session, and since when.
    Status {
        /// The spool directory. Defaults to $XDG_STATE_HOME/synveda/spool.
        #[arg(long)]
        dir: Option<std::path::PathBuf>,
        /// Print the inventory as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Delete events this deployment has already answered for.
    ///
    /// Only acknowledged events are ever deleted, and `--acknowledged` is
    /// required rather than assumed: the one irreversible thing that can be
    /// done to an undelivered observation is delete it, so the command that
    /// reclaims disk says out loud what it is allowed to take. There is no
    /// flag that deletes undelivered events.
    Purge {
        /// Required. Delete only the events the gateway has acknowledged.
        #[arg(long)]
        acknowledged: bool,
        /// The spool directory. Defaults to $XDG_STATE_HOME/synveda/spool.
        #[arg(long)]
        dir: Option<std::path::PathBuf>,
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
    /// List stable Skill aggregates and their current immutable versions.
    List {
        /// Restrict the listing to this governing scope.
        #[arg(long)]
        scope: Option<ScopeId>,
        /// Print JSON.
        #[arg(long)]
        json: bool,
        /// Credential profile.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Inspect one exact immutable version by tenant-unique bundle name.
    Show {
        /// Agent Skills bundle name.
        name: String,
        /// Inspect this immutable version instead of the current one.
        #[arg(long)]
        version: Option<synveda_types::SkillVersionId>,
        /// Print JSON.
        #[arg(long)]
        json: bool,
        /// Suppress the connection banner.
        #[arg(long)]
        quiet: bool,
        /// Credential profile.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Install or update a complete Agent Skills-compatible directory.
    ///
    /// The mutation is a typed VedaFlow change. Content changes create an
    /// immutable version; the command never edits a version in place.
    Import {
        /// Directory holding SKILL.md and its text resources/scripts.
        dir: std::path::PathBuf,
        /// Scope governing the stable Skill aggregate.
        #[arg(long)]
        scope: ScopeId,
        /// Override the directory-derived bundle name.
        #[arg(long)]
        name: Option<String>,
        /// public | internal | confidential.
        #[arg(long)]
        sensitivity: Option<String>,
        /// Credential profile.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Materialise the exact version enabled at a project/principal scope.
    Install {
        /// Tenant-unique Agent Skills bundle name.
        name: String,
        /// Project or principal scope whose binding authorises exposure.
        #[arg(long)]
        scope: ScopeId,
        /// Supported client layout.
        #[arg(long, default_value = "claude-code")]
        client: String,
        /// Write under this directory instead of the client's default root.
        #[arg(long)]
        root: Option<std::path::PathBuf>,
        /// Print the receipt as JSON.
        #[arg(long)]
        json: bool,
        /// Credential profile.
        #[arg(long)]
        profile: Option<String>,
    },
    /// List exact immutable versions enabled by bindings at a scope.
    Available {
        /// Project or principal scope whose bindings resolve.
        #[arg(long)]
        scope: ScopeId,
        /// Print JSON.
        #[arg(long)]
        json: bool,
        /// Credential profile.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Reconcile a supported client root to one scope's enabled bindings.
    Sync {
        /// Project or principal scope whose bindings resolve.
        #[arg(long)]
        scope: ScopeId,
        /// Supported client layout.
        #[arg(long, default_value = "claude-code")]
        client: String,
        /// Write under this directory instead of the client's default root.
        #[arg(long)]
        root: Option<std::path::PathBuf>,
        /// Report changes without writing.
        #[arg(long)]
        dry_run: bool,
        /// Print JSON.
        #[arg(long)]
        json: bool,
        /// Credential profile.
        #[arg(long)]
        profile: Option<String>,
    },
}

#[derive(Subcommand)]
enum OkfCommand {
    /// Validate a local OKF v0.2 directory or archive without contacting a gateway.
    Validate {
        /// Directory, .zip, .tar, .tar.gz or .tgz bundle.
        path: std::path::PathBuf,
        /// Treat a directory as a checked-out Git tree at this explicit revision.
        #[arg(long)]
        source_revision: Option<String>,
        /// Print the complete inspection as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Inspect artifacts, exact types, extension metadata and proposed mappings locally.
    Inspect {
        /// Directory, .zip, .tar, .tar.gz or .tgz bundle.
        path: std::path::PathBuf,
        /// Treat a directory as a checked-out Git tree at this explicit revision.
        #[arg(long)]
        source_revision: Option<String>,
        /// Print the complete inspection as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Plan an import and optionally materialise reviewable candidates.
    Import {
        /// Directory, .zip, .tar, .tar.gz or .tgz bundle.
        path: std::path::PathBuf,
        /// Project receiving the immutable plan and candidate batch.
        #[arg(long)]
        project: synveda_types::ProjectId,
        /// Persist the immutable plan only; create no capture candidates.
        #[arg(long)]
        dry_run: bool,
        /// Treat a directory as a checked-out Git tree at this explicit revision.
        #[arg(long)]
        source_revision: Option<String>,
        /// Print the public API response as JSON.
        #[arg(long)]
        json: bool,
        /// Credential profile.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Export current visible project Knowledge as a new deterministic directory.
    Export {
        /// Project whose current visible Knowledge enters the bundle.
        #[arg(long)]
        project: synveda_types::ProjectId,
        /// New output directory. Existing paths are never overwritten.
        #[arg(long)]
        output: std::path::PathBuf,
        /// Export only these Knowledge items. Repeatable; empty means all visible current items.
        #[arg(long = "item")]
        item_ids: Vec<synveda_types::KnowledgeItemId>,
        /// Print the completed export summary as JSON.
        #[arg(long)]
        json: bool,
        /// Credential profile.
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
        /// Which authored-artifact channel, as its ref name (for example
        /// `prompt/published`).
        #[arg(long)]
        channel: String,
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
        /// why an artifact stopped being published.
        #[arg(long)]
        message: String,
        #[arg(long)]
        channel: String,
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
        channel: String,
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
        channel: String,
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
    /// Run an approved Knowledge proposal's typed effect. The gateway
    /// repeats PDP and revision checks before applying it (CPR-16).
    Apply {
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
    /// Remove our entry from a client's config (OPS-10, ADR-0067
    /// decision 3) — the exact mirror of `install`.
    ///
    /// It takes out the one key we own and writes every other byte back as
    /// it found it: your other MCP servers survive, and a hand-maintained
    /// JSONC file keeps its comments and its layout. That is the promise
    /// `install` makes in the other direction, and half a promise is not
    /// one.
    Uninstall {
        /// Which client to edit. Pass an unknown name to be told the ones
        /// this installation knows.
        #[arg(long)]
        client: String,
        /// Edit this file instead of the client's own.
        #[arg(long)]
        config: Option<std::path::PathBuf>,
        /// Report what would change and write nothing.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum PluginCommand {
    /// Install the plugin into a client, through that client's own
    /// installer.
    ///
    /// Claude Code reads plugins from *marketplaces* it has been told
    /// about, not from a directory dropped under `~/.claude/plugins/`. This
    /// adds the marketplace an installed release carries and installs the
    /// one plugin in it, by running `claude plugin` — so its state stays
    /// its own to manage.
    Install {
        /// Which client to install into. Pass an unknown name to be told
        /// the ones this release knows.
        #[arg(long, default_value = "claude-code")]
        client: String,
        /// A marketplace directory to install from, instead of the one an
        /// installed release carries. `scripts/package-plugin.sh` produces
        /// one from a checkout.
        #[arg(long)]
        from: Option<std::path::PathBuf>,
        /// Report the commands that would run and run neither.
        #[arg(long)]
        dry_run: bool,
        /// Reinstall over an existing `synveda@synveda`.
        #[arg(long)]
        force: bool,
        /// Claude Code's installation scope.
        #[arg(long, default_value = "user")]
        scope: String,
    },
    /// Remove the plugin from Claude Code (OPS-10, ADR-0067 decision 4).
    ///
    /// Drives `claude plugin uninstall` and then `marketplace remove`, in
    /// that order: Claude Code copies a plugin into a versioned cache it
    /// owns, so removing the marketplace alone leaves the plugin running.
    /// Verified against `claude plugin list` rather than the filesystem —
    /// removing and *unloading* are different events.
    Uninstall {
        /// Which client to remove it from.
        #[arg(long, default_value = "claude-code")]
        client: String,
        /// Report what would run and change nothing.
        #[arg(long)]
        dry_run: bool,
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
enum ScopeCommand {
    /// One level of the tenant's scope tree, or a scope's whole subtree.
    List {
        /// Anchor scope UUID. Defaults to the tenant root.
        #[arg(long)]
        under: Option<ScopeId>,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// One scope, with its path.
    Show {
        /// Scope UUID.
        id: ScopeId,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Create a scope under a parent you may write to.
    ///
    /// `kind` is one of the governed shapes: `org_unit`, `workspace`,
    /// `project` (the old org/division/department/team/user vocabulary is
    /// gone and fails here by name).
    Create {
        /// Parent scope UUID. Required — the tenant root is minted by the
        /// substrate, not created here.
        #[arg(long)]
        parent: ScopeId,
        /// Shape: org_unit, workspace, project.
        #[arg(long)]
        kind: String,
        /// Human-stable handle, unique among siblings, immutable.
        #[arg(long)]
        slug: String,
        /// Display name.
        #[arg(long)]
        name: String,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Move a scope — and its whole subtree — under a new parent.
    Move {
        /// The scope to move.
        id: ScopeId,
        /// The destination parent.
        #[arg(long)]
        parent: ScopeId,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Draw the tenant's whole scope tree.
    Tree {
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum DirectoryCommand {
    /// What the last pull pass did — and, when the circuit breaker
    /// refused, how many people it declined to seal.
    Status {
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Store this tenant's directory configuration, sealed under its own
    /// key (TEN-4, ADR-0064 decision 9).
    ///
    /// A tenant with one of these is pulled with it; a tenant without one
    /// falls back to the deployment's `SYNVEDA_OIDC_ISSUERS` entry. This is
    /// what lets one deployment pull two tenants from two directories, the
    /// limitation AUTH-5 shipped with.
    ///
    /// The whole configuration is sealed, not only the secret, so a
    /// credential and the host it is presented to cannot disagree.
    SetCredential {
        /// Tenant UUID.
        #[arg(long)]
        tenant: TenantId,
        /// A file holding the connector JSON, or `-` for stdin. A path
        /// rather than an argument, because an argument is in the shell
        /// history and in `ps`.
        #[arg(long)]
        config: std::path::PathBuf,
    },
    /// Destroy this tenant's stored directory configuration. The sweep then
    /// falls back to the deployment's, if it has one for this tenant.
    ClearCredential {
        /// Tenant UUID.
        #[arg(long)]
        tenant: TenantId,
    },
    /// Authorise the next complete pass to seal past the breaker.
    ///
    /// Bounded by `--ceiling`, and spent by the first pass that uses it: a
    /// pass proposing more than the ceiling refuses again rather than
    /// rounding down, and the authorisation does not survive into the next
    /// directory failure.
    AuthoriseSeals {
        /// The most this authorisation permits a pass to seal.
        #[arg(long)]
        ceiling: i32,
        /// Why. Recorded on the chain with your name against it.
        #[arg(long)]
        reason: String,
        /// How long it stands, in hours. Capped at 24; defaults to 2.
        #[arg(long)]
        hours: Option<f64>,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ScimCommand {
    /// Issue, list and revoke provisioning credentials.
    #[command(subcommand)]
    Token(ScimTokenCommand),
}

#[derive(Subcommand)]
enum ScimTokenCommand {
    /// Issue a credential and print it **once**.
    Issue {
        /// What an operator recognises it by when deciding to rotate.
        #[arg(long)]
        label: String,
        /// How long it lives, in days. Capped at 365; defaults to 90.
        #[arg(long)]
        days: Option<i64>,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Every credential this tenant has ever been issued, revoked and
    /// expired ones included — rotation is a decision about a history.
    List {
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Revoke one. A stamp rather than a delete: what it did stays
    /// answerable from the chain, named by this id.
    Revoke {
        /// The credential id, as `list` shows it.
        id: String,
        #[arg(long)]
        profile: Option<String>,
    },
}

#[derive(Subcommand)]
enum LapseCommand {
    /// Standing grants anywhere you may read, or one scope's history.
    ///
    /// Each grant is visible from *either* end — the scope that discloses
    /// and the scope that receives — so the steward of a granted team can
    /// list, and therefore revoke, what their own team holds (ADR-0058
    /// decision 7).
    List {
        /// Limit to grants over one target scope. Without it, every grant
        /// you may see, anywhere in the tenant.
        #[arg(long)]
        scope: Option<ScopeId>,
        /// Include grants that have expired or been revoked. Default with
        /// --scope (that form answers "who could read this in March"),
        /// off without it (that form answers "what is relaxed now").
        #[arg(long)]
        all: bool,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum TenantCommand {
    /// Admit a tenant; prints the created tenant as JSON.
    ///
    /// Provisions the tenant's encryption key in the same breath when a KEK
    /// is configured (TEN-4, ADR-0064). A deployment with no `SYNVEDA_KMS_KEY`
    /// still admits the tenant and says what is missing — the key plane is
    /// fail-closed, not fail-to-admit — and `tenant key provision` fills it
    /// in later.
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
    /// A tenant's encryption keys (TEN-4, ADR-0064).
    #[command(subcommand)]
    Key(TenantKeyCommand),
    /// Write a sealed archive of a tenant's records and audit chain.
    ///
    /// The AC's artefact: unreadable without that tenant's key. Sealed under
    /// a fresh per-archive key wrapped by the tenant's, so handing somebody
    /// an export does not hand them the key to the tenant's live secrets.
    ///
    /// This is not TEN-5's portable archive — there is no re-import, no
    /// assets and no destruction certificate here, and TEN-5 owns those.
    Export {
        /// Tenant UUID.
        #[arg(long)]
        tenant: TenantId,
        /// Where to write the archive.
        #[arg(long)]
        out: std::path::PathBuf,
    },
    /// Open a sealed archive, printing its contents.
    ExportOpen {
        /// The archive to open.
        #[arg(long)]
        archive: std::path::PathBuf,
    },
    /// Print an archive's cleartext header without opening it: whose it is,
    /// how big, which key generation. What a backup vault's index needs.
    ExportDescribe {
        /// The archive to describe.
        #[arg(long)]
        archive: std::path::PathBuf,
    },
}

#[derive(Subcommand)]
enum TenantKeyCommand {
    /// Mint a tenant's first data key, or report the one already there.
    Provision {
        #[arg(long)]
        tenant: TenantId,
    },
    /// Retire the current data key and mint the next generation.
    ///
    /// Re-seals nothing: payloads under the retired key keep opening under
    /// it and move forward when their rows are next written, so this
    /// returning does not mean every ciphertext is on the new key
    /// (ADR-0064 decision 6).
    Rotate {
        #[arg(long)]
        tenant: TenantId,
    },
    /// Which generation is current, and what sealed secrets the tenant holds.
    Status {
        #[arg(long)]
        tenant: TenantId,
    },
}

#[derive(Subcommand)]
enum KmsCommand {
    /// Mint a key-encryption key and print it as hex, and nothing else.
    ///
    /// Meant to be captured: `SYNVEDA_KMS_KEY=$(synveda kms keygen)`. Back it
    /// up — every key in the database is wrapped by it.
    Keygen,
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

/// Reads a connector configuration from a file, or from standard input for
/// `-`.
///
/// A path rather than an inline argument because the document holds a
/// credential, and an argument is in the shell history and visible in `ps`
/// while the command runs.
fn read_config_arg(path: &std::path::Path) -> Result<String, String> {
    if path.as_os_str() == "-" {
        use std::io::Read;
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|err| format!("read the configuration from stdin: {err}"))?;
        return Ok(text);
    }
    std::fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))
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
        Command::Scope(ScopeCommand::List {
            under,
            profile,
            json,
        }) => scope::list(&profile_name(profile), under.as_ref().copied(), json).await,
        Command::Scope(ScopeCommand::Show { id, profile, json }) => {
            scope::show(&profile_name(profile), id, json).await
        }
        Command::Scope(ScopeCommand::Create {
            parent,
            kind,
            slug,
            name,
            profile,
            json,
        }) => scope::create(&profile_name(profile), parent, &kind, &slug, &name, json).await,
        Command::Scope(ScopeCommand::Move {
            id,
            parent,
            profile,
            json,
        }) => scope::move_scope(&profile_name(profile), id, parent, json).await,
        Command::Scope(ScopeCommand::Tree { profile, json }) => {
            scope::tree(&profile_name(profile), json).await
        }

        Command::Whoami {
            capabilities,
            profile,
            json,
        } => whoami::show(&profile_name(profile), capabilities, json).await,
        Command::Directory(DirectoryCommand::Status { profile, json }) => {
            directory::status(&profile_name(profile), json).await
        }
        Command::Directory(DirectoryCommand::AuthoriseSeals {
            ceiling,
            reason,
            hours,
            profile,
            json,
        }) => {
            directory::authorise_seals(&profile_name(profile), ceiling, &reason, hours, json).await
        }
        Command::Scim(ScimCommand::Token(ScimTokenCommand::Issue {
            label,
            days,
            profile,
            json,
        })) => scim::issue(&profile_name(profile), &label, days, json).await,
        Command::Scim(ScimCommand::Token(ScimTokenCommand::List { profile, json })) => {
            scim::list(&profile_name(profile), json).await
        }
        Command::Scim(ScimCommand::Token(ScimTokenCommand::Revoke { id, profile })) => {
            scim::revoke(&profile_name(profile), &id).await
        }
        Command::Lapse(LapseCommand::List {
            scope,
            all,
            profile,
            json,
        }) => lapse::list(&profile_name(profile), scope, all, json).await,
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
            workspace,
            profile,
        } => mcp::serve(profile_name(profile), writes, workspace).await,
        Command::Mcp {
            workspace: _,
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
        Command::Mcp {
            command:
                Some(McpCommand::Uninstall {
                    client,
                    config,
                    dry_run,
                }),
            ..
        } => mcp::install::uninstall(&mcp::install::RemovePlan {
            client,
            config,
            dry_run,
        }),
        Command::Plugin(PluginCommand::Install {
            client,
            from,
            dry_run,
            force,
            scope,
        }) => plugin::install(&plugin::Plan {
            client,
            from,
            dry_run,
            force,
            scope,
        }),
        Command::Plugin(PluginCommand::Uninstall { client, dry_run }) => {
            plugin::uninstall(&plugin::RemovePlan { client, dry_run })
        }
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
            // Asked here as well as inside `migrate`, so the refusal reaches
            // a terminal as itself rather than wrapped in `storage:` — this
            // is the command whose whole job is to advance a schema, and the
            // answer "this one cannot be advanced, here is what to run" is
            // the answer (CPR-2, ADR-0069).
            synveda_store::epoch::preflight(&pool)
                .await
                .map_err(|refusal| refusal.to_string())?;
            synveda_store::migrate(&pool)
                .await
                .map_err(|err| err.to_string())?;
            eprintln!("migrations applied");
            Ok(())
        }
        Command::Reset { database, force } => reset::reset(reset::Plan { database, force }).await,
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
            let pool = connect_current_epoch().await?;
            let tenant = create_tenant(&pool, &slug, &name, status).await?;
            // The tenant's key, in the same command that admits it (TEN-4,
            // ADR-0064). Not in `create_tenant`'s transaction: wrapping a key
            // is a KMS call, and a network call inside the transaction that
            // admits a tenant is a transaction held open by somebody else's
            // outage. A failure here leaves an admitted tenant with no key,
            // which `tenant key provision` fixes and which the message says.
            let key = match keys::provision_quietly(&pool, tenant.id).await {
                Ok(version) => serde_json::json!({ "version": version }),
                Err(error) => {
                    eprintln!(
                        "tenant admitted, but its encryption key was not \
                         provisioned: {error}\nrun `synveda tenant key \
                         provision --tenant {}` once a KEK is configured",
                        tenant.id
                    );
                    serde_json::Value::Null
                }
            };
            let mut rendered = serde_json::to_value(&tenant).map_err(|err| err.to_string())?;
            if let Some(object) = rendered.as_object_mut() {
                object.insert("encryption_key".to_string(), key);
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&rendered).map_err(|err| err.to_string())?
            );
            Ok(())
        }
        Command::Kms(KmsCommand::Keygen) => keys::keygen(),
        Command::Tenant(TenantCommand::Key(TenantKeyCommand::Provision { tenant })) => {
            let pool = connect_current_epoch().await?;
            keys::provision(&pool, tenant).await
        }
        Command::Tenant(TenantCommand::Key(TenantKeyCommand::Rotate { tenant })) => {
            let pool = connect_current_epoch().await?;
            keys::rotate(&pool, tenant).await
        }
        Command::Tenant(TenantCommand::Key(TenantKeyCommand::Status { tenant })) => {
            let pool = connect_current_epoch().await?;
            keys::status(&pool, tenant).await
        }
        Command::Tenant(TenantCommand::Export { tenant, out }) => {
            let pool = connect_current_epoch().await?;
            keys::export(&pool, tenant, &out).await
        }
        Command::Tenant(TenantCommand::ExportOpen { archive }) => {
            let pool = connect_current_epoch().await?;
            keys::export_open(&pool, &archive).await
        }
        Command::Tenant(TenantCommand::ExportDescribe { archive }) => {
            keys::export_describe(&archive)
        }
        Command::Directory(DirectoryCommand::SetCredential { tenant, config }) => {
            let json = read_config_arg(&config)?;
            let pool = connect_current_epoch().await?;
            keys::set_directory_credential(&pool, tenant, &json).await
        }
        Command::Directory(DirectoryCommand::ClearCredential { tenant }) => {
            let pool = connect_current_epoch().await?;
            keys::clear_directory_credential(&pool, tenant).await
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
                        trace_retention: CompositionConfig::DEFAULT.trace_retention,
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
            let pool = connect_current_epoch().await?;
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
            let pool = connect_current_epoch().await?;
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
        Command::Service(ServiceCommand::Register {
            tenant,
            subject,
            scope,
            name,
        }) => {
            let pool = connect_current_epoch().await?;
            let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
                .await
                .map_err(|err| err.to_string())?;
            let anchor = synveda_store::scopes::get(&mut *tx, tenant, scope)
                .await
                .map_err(|err| err.to_string())?
                .filter(|node| node.tenant_id == tenant)
                .ok_or_else(|| format!("no scope {scope} in tenant {tenant}"))?;
            let identity_id = IdentityId::new();
            let display_name = name.as_deref().unwrap_or(&subject);
            // The agent's own scope: a principal-shaped scope under the
            // operator's anchor, so token confinement is tree position.
            let leaf = synveda_store::scopes::create(
                &mut tx,
                &synveda_store::scopes::NewScope {
                    id: ScopeId::new(),
                    tenant_id: tenant,
                    kind: synveda_types::scope::ScopeKind::Principal,
                    parent_scope_id: Some(anchor.id),
                    slug: synveda_store::scopes::principal_slug(&subject),
                    display_name: display_name.to_owned(),
                    attributes: serde_json::json!({}),
                    principal_id: Some(subject.clone()),
                    created_by: None,
                },
            )
            .await
            .map_err(|err| err.to_string())?;
            let identity = synveda_store::identities::create(
                &mut tx,
                identity_id,
                tenant,
                Some(&subject),
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
                    "anchor": {"slug": anchor.slug},
                }),
            )
            .await?;
            tx.commit().await.map_err(|err| err.to_string())?;
            eprintln!(
                "note: a running gateway caches entities out-of-process; restart it or use the API path"
            );
            println!(
                "{}",
                serde_json::to_string_pretty(&identity).map_err(|err| err.to_string())?
            );
            Ok(())
        }
        Command::Service(ServiceCommand::Remove { tenant, id }) => {
            let pool = connect_current_epoch().await?;
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
            let anchor_id = synveda_store::scopes::get(&mut *tx, tenant, identity.scope_id)
                .await
                .map_err(|err| err.to_string())?
                .and_then(|own| own.parent_scope_id);
            // Row first (its FK pins the leaf), then the leaf.
            synveda_store::identities::delete_service(&mut *tx, tenant, id)
                .await
                .map_err(|err| err.to_string())?;
            synveda_store::scopes::set_status(
                &mut *tx,
                tenant,
                identity.scope_id,
                synveda_types::scope::ScopeStatus::Archived,
            )
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
            let pool = connect_current_epoch().await?;
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
            let pool = connect_current_epoch().await?;
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
            let pool = connect_current_epoch().await?;
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
            ProposalCommand::Apply { id, profile } => {
                proposal::apply(&profile_name(profile), id).await
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
            SkillCommand::List {
                scope,
                json,
                profile,
            } => skill::list(&profile_name(profile), scope, json).await,
            SkillCommand::Show {
                name,
                version,
                json,
                quiet,
                profile,
            } => skill::show(&profile_name(profile), &name, version, json, quiet).await,
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
                scope,
                client,
                root,
                json,
                profile,
            } => {
                skill::install(
                    &profile_name(profile),
                    &name,
                    scope,
                    &client,
                    root.as_deref(),
                    json,
                )
                .await
            }
            SkillCommand::Available {
                scope,
                json,
                profile,
            } => skill::available(&profile_name(profile), scope, json).await,
            SkillCommand::Sync {
                scope,
                client,
                root,
                dry_run,
                json,
                profile,
            } => {
                skill::sync(
                    &profile_name(profile),
                    scope,
                    &client,
                    root.as_deref(),
                    dry_run,
                    json,
                )
                .await
            }
        },

        Command::Okf(command) => match command {
            OkfCommand::Validate {
                path,
                source_revision,
                json,
            } => okf::validate(&path, source_revision.as_deref(), json),
            OkfCommand::Inspect {
                path,
                source_revision,
                json,
            } => okf::inspect(&path, source_revision.as_deref(), json),
            OkfCommand::Import {
                path,
                project,
                dry_run,
                source_revision,
                json,
                profile,
            } => {
                okf::import(
                    &profile_name(profile),
                    &path,
                    project,
                    source_revision.as_deref(),
                    dry_run,
                    json,
                )
                .await
            }
            OkfCommand::Export {
                project,
                output,
                item_ids,
                json,
                profile,
            } => okf::export(&profile_name(profile), project, &output, &item_ids, json).await,
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
        Command::Session(command) => match command {
            SessionCommand::Flush {
                dir,
                verbose,
                profile,
            } => session::flush(&profile_name(profile), dir, verbose).await,
            SessionCommand::Spool(command) => match command {
                SpoolCommand::Status { dir, json } => session::status(dir, json),
                SpoolCommand::Purge { acknowledged, dir } => session::purge(dir, acknowledged),
            },
        },
        Command::Recall {
            query,
            workspace,
            limit,
            json,
            quiet,
            profile,
        } => {
            recall::recall(
                &profile_name(profile),
                recall::Ask {
                    query: &query,
                    workspace: workspace.as_deref(),
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
/// The break-glass audit seam, for the key plane's operator commands
/// (TEN-4, ADR-0064 decision 12 as amended).
///
/// `pub(crate)` so `keys.rs` chains its own acts rather than every one of
/// them being threaded back through `main`.
pub(crate) async fn record_break_glass(
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

/// [`connect`], then the schema epoch guard (CPR-2, ADR-0069).
///
/// Every store-level command goes through this. They open `DATABASE_URL`
/// directly and write with the owner role — which makes them the one family
/// of verbs that could quietly succeed against a database from before the
/// context-platform cut, writing new-model rows beside old-model ones with
/// nothing in the process to notice. The two that do not are the two that
/// cannot: `db migrate`, which creates the epoch, and `reset`, which is what
/// a refusal tells you to run.
async fn connect_current_epoch() -> Result<sqlx::PgPool, String> {
    let pool = connect().await?;
    synveda_store::epoch::verify(&pool)
        .await
        .map_err(|refusal| refusal.to_string())?;
    Ok(pool)
}

async fn connect() -> Result<sqlx::PgPool, String> {
    // `DATABASE_URL`, or the single-node profile's own Postgres — which is
    // the same default `synveda init` installs against, so the commands
    // INSTALL.md tells a new operator to run next (`audit tail`, `audit
    // verify`) work on a machine that has one deployment and no Makefile.
    //
    // The message this replaces named the Makefile, which is in a checkout
    // an installed operator does not have (OPS-8). Erroring on a missing
    // variable was right while a checkout was the only way to get here.
    let url = init::database_url();
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .map_err(|err| {
            format!(
                "connect to {url}: {err}\n\
                 (set DATABASE_URL to reach a database other than the one \
                 `synveda init` installs)"
            )
        })
}

#[cfg(test)]
mod hard_cut_tests {
    use super::*;

    #[test]
    fn removed_record_classification_command_is_not_an_alias() {
        let error = Cli::try_parse_from([
            "synveda",
            "proposal",
            "classify",
            "0198f000-0000-7000-8000-000000000001",
        ])
        .err()
        .expect("the removed command must fail parsing");
        assert!(
            error.to_string().contains("unrecognized subcommand"),
            "unexpected clap refusal: {error}"
        );
    }

    #[test]
    fn channel_history_requires_an_authored_artifact_ref() {
        let error = Cli::try_parse_from([
            "synveda",
            "channel",
            "history",
            "0198f000-0000-7000-8000-000000000001",
        ])
        .err()
        .expect("the removed memory default must not parse");
        assert!(error.to_string().contains("--channel"), "{error}");
    }

    #[test]
    fn okf_public_workflows_have_the_documented_command_shape() {
        let project = "0198f000-0000-7000-8000-000000000001";
        for args in [
            vec!["synveda", "okf", "validate", "bundle"],
            vec!["synveda", "okf", "inspect", "bundle"],
            vec![
                "synveda",
                "okf",
                "import",
                "bundle",
                "--project",
                project,
                "--dry-run",
            ],
            vec![
                "synveda",
                "okf",
                "export",
                "--project",
                project,
                "--output",
                "exported",
            ],
        ] {
            Cli::try_parse_from(args).expect("documented OKF command must parse");
        }
    }
}
