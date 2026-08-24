//! `synveda channel` — rollback and pinning from a terminal (FLOW-7,
//! ADR-0036 decision 12).
//!
//! HTTP-only, on FLOW-6's precedent and for its reason. A rewind is an act
//! whose authority is `ChannelRollback` at a scope, whose actor is an
//! identity with roles, and whose event the gateway chains; a CLI that
//! moved the ref itself would have to invent an identity for `updated_by`
//! and would leave no decision in the trail — for the act with the largest
//! blast radius in the product. So this module opens no database
//! connection, and these verbs take no `--database-url`.
//!
//! Four verbs and one rule between them. `status` and `history` show what
//! a scope's channels hold and the states they have held; `rollback`
//! installs one of those states; `pin` and `unpin` hold what the channel
//! serves without moving where it points. The history is the menu a
//! rewind is chosen from, and the gateway accepts exactly what the history
//! lists (ADR-0036 decision 11), so an operator never has to guess whether
//! a commit is a legal target.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use synveda_types::{IdentityId, ScopeId};

use crate::api::{Api, Origin};

// ── The wire shapes (`crates/synveda-gateway/src/channels.rs`) ─────────

#[derive(Deserialize)]
struct ChannelsResponse {
    channels: Vec<ChannelView>,
}

#[derive(Deserialize)]
struct ChannelView {
    name: String,
    commit: String,
    entries: usize,
    updated_at: DateTime<Utc>,
    updated_by: IdentityId,
    #[serde(default)]
    pin: Option<PinView>,
}

#[derive(Deserialize)]
struct PinView {
    commit: String,
    pinned_at: DateTime<Utc>,
    pinned_by: IdentityId,
}

#[derive(Deserialize)]
struct HistoryResponse {
    channel: String,
    head: String,
    #[serde(default)]
    pin: Option<PinView>,
    history: Vec<HistoryEntry>,
}

#[derive(Deserialize)]
struct HistoryEntry {
    commit: String,
    #[serde(default)]
    merge_parents: Vec<String>,
    message: String,
    committed_at: DateTime<Utc>,
    members: usize,
    head: bool,
    served: bool,
}

#[derive(Deserialize)]
struct RollbackResponse {
    channel: String,
    from: String,
    to: String,
    members: usize,
    removed: Vec<String>,
    restored: Vec<RestoredMember>,
}

#[derive(Deserialize)]
struct RestoredMember {
    /// The authored member path.
    member: String,
}

#[derive(Deserialize)]
struct PinResponse {
    channel: String,
    commit: String,
    #[serde(default)]
    previous: Option<String>,
    head: String,
}

#[derive(Deserialize)]
struct UnpinResponse {
    channel: String,
    #[serde(default)]
    released: Option<String>,
    head: String,
}

// ── The verbs ──────────────────────────────────────────────────────────

/// `synveda channel status <scope>` — what stands at a scope.
pub async fn status(profile: &str, scope: ScopeId, json_out: bool) -> Result<(), String> {
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    let path = format!("/v1/channels/{scope}");
    if json_out {
        println!("{}", api.get(&path).await?);
        return Ok(());
    }

    let response: ChannelsResponse = api.get_as(&path).await?;
    if response.channels.is_empty() {
        println!("no channels at {scope} — nothing has been committed here");
        return Ok(());
    }
    for channel in &response.channels {
        println!(
            "{:<18} {}  {} entries  moved {} by {}",
            channel.name,
            short(&channel.commit),
            channel.entries,
            channel.updated_at.format("%Y-%m-%d %H:%M"),
            channel.updated_by,
        );
        // The line that keeps a curator from concluding their publication
        // vanished: the ref moved, the readers did not.
        if let Some(pin) = &channel.pin {
            println!(
                "{:<18} └─ PINNED at {} since {} by {} — readers compose this, not the head",
                "",
                short(&pin.commit),
                pin.pinned_at.format("%Y-%m-%d %H:%M"),
                pin.pinned_by,
            );
        }
    }
    Ok(())
}

/// `synveda channel history <scope>` — the states a channel has held.
pub async fn history(
    profile: &str,
    scope: ScopeId,
    channel: String,
    limit: Option<u32>,
    json_out: bool,
) -> Result<(), String> {
    let selector = query(&channel, limit)?;
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);
    let path = format!("/v1/channels/{scope}/history{selector}");
    if json_out {
        println!("{}", api.get(&path).await?);
        return Ok(());
    }

    let response: HistoryResponse = api.get_as(&path).await?;
    println!("{} at {scope}", response.channel);
    if let Some(pin) = &response.pin {
        println!(
            "  pinned at {} since {} — readers are held there until it is released",
            short(&pin.commit),
            pin.pinned_at.format("%Y-%m-%d %H:%M"),
        );
    }
    println!();
    for entry in &response.history {
        // Newest first, and each line says what a rewind to it would mean.
        // `head` is where the channel already is; `served` is what readers
        // actually compose, which differs only under a pin.
        let marker = match (entry.head, entry.served) {
            (true, true) => "* head, served",
            (true, false) => "* head",
            (false, true) => "  served (pinned)",
            (false, false) => "",
        };
        println!(
            "{}  {} members  {}  {}{}",
            short(&entry.commit),
            entry.members,
            entry.committed_at.format("%Y-%m-%d %H:%M"),
            entry.message.lines().next().unwrap_or_default(),
            if marker.is_empty() {
                String::new()
            } else {
                format!("   [{}]", marker.trim())
            },
        );
        if !entry.merge_parents.is_empty() {
            println!(
                "          via proposal commit {} (not itself a rollback target)",
                short(&entry.merge_parents[0]),
            );
        }
    }
    println!();
    println!(
        "rewind with: synveda channel rollback {scope} --channel {} --from {} --to <commit> --message <why>",
        response.channel,
        short(&response.head),
    );
    Ok(())
}

/// `synveda channel rollback <scope> --from --to --message`.
pub async fn rollback(
    profile: &str,
    scope: ScopeId,
    from: String,
    to: String,
    message: String,
    channel: String,
    json_out: bool,
) -> Result<(), String> {
    let mut body = json!({
        "from_commit": from,
        "to_commit": to,
        "message": message,
    });
    with_channel(&mut body, &channel)?;
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);

    let path = format!("/v1/channels/{scope}/rollback");
    if json_out {
        println!("{}", api.post(&path, Some(body)).await?);
        return Ok(());
    }

    let response: RollbackResponse = api.post_as(&path, Some(body)).await?;
    println!(
        "{} at {scope}: {} → {}",
        response.channel,
        short(&response.from),
        short(&response.to),
    );
    println!("  {} members published here now", response.members);
    for member in &response.removed {
        println!("  - {member} is no longer published material");
    }
    for member in &response.restored {
        println!("  ~ {} is published at an earlier version", member.member);
    }
    // The FLOW-7 sentence, said out loud: nothing else has to happen.
    println!("  every session that starts from now composes the state above");
    Ok(())
}

/// `synveda channel pin <scope> --commit --reason`.
pub async fn pin(
    profile: &str,
    scope: ScopeId,
    commit: String,
    reason: String,
    channel: String,
    json_out: bool,
) -> Result<(), String> {
    let mut body = json!({ "commit": commit, "reason": reason });
    with_channel(&mut body, &channel)?;
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);

    let path = format!("/v1/channels/{scope}/pin");
    if json_out {
        println!("{}", api.post(&path, Some(body)).await?);
        return Ok(());
    }

    let response: PinResponse = api.post_as(&path, Some(body)).await?;
    match &response.previous {
        Some(previous) => println!(
            "{} at {scope}: readers moved from {} to {}",
            response.channel,
            short(previous),
            short(&response.commit),
        ),
        None => println!(
            "{} at {scope}: readers held at {}",
            response.channel,
            short(&response.commit),
        ),
    }
    if response.head != response.commit {
        println!(
            "  the channel itself points at {} — publications keep landing there",
            short(&response.head),
        );
    }
    Ok(())
}

/// `synveda channel unpin <scope> --reason`.
pub async fn unpin(
    profile: &str,
    scope: ScopeId,
    reason: String,
    channel: String,
    json_out: bool,
) -> Result<(), String> {
    let mut body = json!({ "reason": reason });
    with_channel(&mut body, &channel)?;
    let (api, origin) = Api::connect(profile).await?;
    announce(&api, &origin);

    let path = format!("/v1/channels/{scope}/unpin");
    if json_out {
        println!("{}", api.post(&path, Some(body)).await?);
        return Ok(());
    }

    let response: UnpinResponse = api.post_as(&path, Some(body)).await?;
    match &response.released {
        Some(released) => println!(
            "{} at {scope}: released {} — readers catch up to {} on their next session",
            response.channel,
            short(released),
            short(&response.head),
        ),
        None => println!(
            "{} at {scope} was not pinned; it serves {}",
            response.channel,
            short(&response.head),
        ),
    }
    Ok(())
}

// ── Rendering ──────────────────────────────────────────────────────────

/// Splits an authored-artifact ref such as `skill/published` into the two
/// fields the routes take.
///
/// One flag rather than two, because a channel is written as one name
/// everywhere else in the product — in refs, in the audit payloads, and in
/// what `status` prints — and an operator who has just read one off the
/// screen should be able to type it back.
fn with_channel(body: &mut serde_json::Value, name: &str) -> Result<(), String> {
    let (asset, channel) = split_channel(name)?;
    body["asset"] = json!(asset);
    body["channel"] = json!(channel);
    Ok(())
}

/// The same two fields as a query string.
fn query(name: &str, limit: Option<u32>) -> Result<String, String> {
    let (asset, channel) = split_channel(name)?;
    let mut parts = vec![format!("asset={asset}"), format!("channel={channel}")];
    if let Some(limit) = limit {
        parts.push(format!("limit={limit}"));
    }
    Ok(format!("?{}", parts.join("&")))
}

fn split_channel(name: &str) -> Result<(&str, &str), String> {
    let (asset, channel) = name.split_once('/').ok_or_else(|| {
        format!("--channel takes an asset/channel name such as skill/published, got {name:?}")
    })?;
    if !matches!(asset, "prompt" | "context-pack" | "skill") || channel.is_empty() {
        return Err(format!(
            "--channel names a public authored-artifact ref: prompt, context-pack or skill; got {name:?}"
        ));
    }
    Ok((asset, channel))
}

/// A commit, abbreviated the way git does. Full hashes are in `--json`;
/// twelve characters is what fits beside a message and is what an operator
/// copies back into `--to`, which the gateway accepts because a commit is
/// named by its full hex there.
fn short(commit: &str) -> String {
    commit.chars().take(12).collect()
}

/// Says which identity is about to act, once, on stderr — the same line
/// `synveda proposal` prints, and for the same reason: a rewind reaches
/// every agent under the scope, and "as whom" is not something to guess.
fn announce(api: &Api, origin: &Origin) {
    match origin {
        Origin::Profile(name) => eprintln!(
            "synveda: {} at {} (profile `{name}`)",
            api.subject,
            api.gateway()
        ),
        Origin::Environment => eprintln!("synveda: acting with SYNVEDA_TOKEN at {}", api.gateway()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_channel_name_splits_into_the_two_fields_the_route_takes() {
        let mut body = json!({ "message": "why" });
        with_channel(&mut body, "skill/published").expect("split");
        assert_eq!(body["asset"], "skill");
        assert_eq!(body["channel"], "published");
        assert_eq!(body["message"], "why");
    }

    #[test]
    fn the_removed_memory_channel_is_refused() {
        let mut body = json!({ "message": "why" });
        let error = with_channel(&mut body, "memory/published")
            .expect_err("the raw-record channel is not public");
        assert!(error.contains("authored-artifact"), "{error}");
    }

    #[test]
    fn a_name_without_a_slash_is_refused_with_the_shape_it_wanted() {
        let mut body = json!({});
        let error = with_channel(&mut body, "published")
            .expect_err("a bare channel name is not a ref name");
        assert!(error.contains("skill/published"), "{error}");
    }

    #[test]
    fn the_query_string_carries_only_what_was_asked_for() {
        assert_eq!(
            query("context-pack/staged", Some(5)).expect("query"),
            "?asset=context-pack&channel=staged&limit=5"
        );
        assert_eq!(
            query("prompt/published", None).expect("query"),
            "?asset=prompt&channel=published"
        );
        assert!(query("published", None).is_err());
    }

    #[test]
    fn commits_abbreviate_to_a_readable_prefix() {
        let commit = "0123456789abcdef0123456789abcdef";
        assert_eq!(short(commit), "0123456789ab");
        assert_eq!(short("abc"), "abc");
    }
}
