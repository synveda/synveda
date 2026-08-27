//! `synveda session` — the durable spool's diagnostics (CPR-12, ADR-0078
//! decision 7).
//!
//! Three commands, and between them they answer the three questions somebody
//! has when they suspect their agent's memory is not arriving:
//!
//! ```text
//! synveda session spool status               what is held, and since when
//! synveda session flush                      send it now
//! synveda session spool purge --acknowledged take back the disk
//! ```
//!
//! # Why a CLI owns this at all
//!
//! A hook runs for milliseconds inside somebody else's process and cannot own
//! a retry schedule. So when a gateway has been unreachable for a day, the
//! backlog is on disk and the only thing that has run since is more hooks. A
//! person needs to be able to see it and push it, and neither of those is a
//! thing a hook can be asked to do on demand.
//!
//! # `purge` deletes only what the gateway has answered for
//!
//! `--acknowledged` is required rather than defaulted, and there is no
//! `--all`. The one irreversible thing this plane can do to an undelivered
//! observation is delete it, and a command that did that by default would
//! eventually do it by accident.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use crate::api::Api;
use crate::spool::{self, Spool};

/// The most events one flush sends in a single request.
///
/// The gateway's own `MAX_EVENT_BATCH`, restated. A spool holding a thousand
/// events is delivered as five requests rather than one refusal.
const MAX_BATCH: usize = 200;

/// What the gateway answers for one event of an append.
#[derive(Deserialize)]
struct AppendedEvent {
    outcome: String,
    client_event_id: String,
}

/// The append response, as much of it as a flush reads.
#[derive(Deserialize)]
struct AppendResponse {
    events: Vec<AppendedEvent>,
}

/// `synveda session flush` — deliver every unacknowledged event.
///
/// Walks the spool directory, sends each session's pending entries in client
/// order, and marks what the gateway resolved. A file it cannot deliver is
/// left exactly as it was, with its attempt count incremented, so the next
/// flush — or the next `SessionStart` — picks it up.
pub async fn flush(profile: &str, dir: Option<PathBuf>, verbose: bool) -> Result<(), String> {
    let dir = dir.unwrap_or_else(spool::spool_dir);
    let scanned = spool::scan(&dir);
    report_unreadable(&scanned);
    if scanned.spools.is_empty() {
        println!("Nothing spooled in {}.", dir.display());
        return Ok(());
    }

    let (api, _) = Api::connect(profile).await?;
    let mut sent = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;
    for (path, mut spool) in scanned.spools {
        if spool.pending_count() == 0 {
            continue;
        }
        if let Err(message) = pin_gateway(&mut spool.gateway_url, api.gateway()) {
            failed += 1;
            eprintln!("  {}: {message}", name(&path));
            continue;
        }
        // A run whose session was never opened has nowhere to deliver. The
        // adapter opens it at the next SessionStart, which is also the hook
        // that knows the workspace; a CLI flush guessing at one would open a
        // run in the wrong place.
        let Some(session_id) = spool.session_id.clone() else {
            skipped += 1;
            if verbose {
                println!(
                    "  {}: {} event(s) held, no session opened yet — the next SessionStart opens it",
                    name(&path),
                    spool.pending_count()
                );
            }
            continue;
        };

        match deliver(&api, &session_id, &mut spool).await {
            Ok(marked) => {
                sent += marked;
                if verbose {
                    println!("  {}: delivered {marked} event(s)", name(&path));
                }
                // The other half of the two-phase close (ADR-0076): a
                // `SessionEnd` whose bounded flush ran out of time leaves the
                // run in `ending` and asks whoever drains the spool to finish
                // it. That is this, and it must happen only once the backlog
                // is actually gone — closing over undelivered events is the
                // thing `ending` exists to avoid.
                if spool.close_requested && spool.fully_acknowledged() {
                    match close_run(&api, &session_id, spool.end_reason.as_deref()).await {
                        Ok(()) => {
                            spool.close_requested = false;
                            if verbose {
                                println!("  {}: closed the run", name(&path));
                            }
                        }
                        Err(message) => {
                            eprintln!(
                                "  {}: delivered, but could not close: {message}",
                                name(&path)
                            );
                        }
                    }
                }
            }
            Err(message) => {
                failed += 1;
                spool.record_attempt(Utc::now());
                eprintln!("  {}: {message}", name(&path));
            }
        }
        // Written whatever happened: an attempt that failed is a fact worth
        // keeping, and an acknowledgement that is not persisted is an event
        // this deployment holds and the spool will send again.
        spool::write(&path, &spool)?;
    }

    if failed > 0 {
        println!("Delivered {sent} event(s); {failed} session(s) could not be reached.");
        return Err(format!("{failed} session(s) could not be delivered"));
    }
    let held: String = if skipped > 0 {
        format!(" {skipped} session(s) have no run opened yet.")
    } else {
        String::new()
    };
    println!("Delivered {sent} event(s).{held}");
    Ok(())
}

/// Binds a pre-session spool once and refuses to reinterpret a stored run id
/// at another deployment.
///
/// The URL is deliberately absent from the error: a malformed profile must
/// not turn user-info or another credential-bearing URL into diagnostics.
fn pin_gateway(bound: &mut Option<String>, current: &str) -> Result<(), &'static str> {
    match bound {
        Some(existing) if existing != current => {
            Err("spool belongs to a different gateway; held without sending or rebinding")
        }
        Some(_) => Ok(()),
        None => {
            *bound = Some(current.to_owned());
            Ok(())
        }
    }
}

/// Sends one session's pending entries, in batches, marking what came back.
async fn deliver(api: &Api, session_id: &str, spool: &mut Spool) -> Result<usize, String> {
    let path = format!("/v1/sessions/{session_id}/events");
    let mut marked = 0usize;
    loop {
        let batch: Vec<_> = spool.pending().take(MAX_BATCH).cloned().collect();
        if batch.is_empty() {
            break;
        }
        // Integrity is checked before the wire, not after: a payload whose
        // hash no longer matches is a corrupted local file, and sending it
        // would write that corruption into the ledger.
        if let Some(bad) = batch.iter().find(|entry| !entry.intact()) {
            return Err(format!(
                "event {} failed its payload hash; the spool file is corrupt and was not sent",
                bad.client_event_id
            ));
        }
        let body = json!({
            "events": batch
                .iter()
                .map(|entry| json!({
                    "event_type": entry.event_type,
                    "client_event_id": entry.client_event_id,
                    "occurred_at": entry.occurred_at,
                    "payload": entry.payload,
                }))
                .collect::<Vec<_>>(),
        });
        spool.record_attempt(Utc::now());
        let response: AppendResponse = api.post_as(&path, Some(body)).await?;
        let outcomes: BTreeMap<String, String> = response
            .events
            .into_iter()
            .map(|event| (event.client_event_id, event.outcome))
            .collect();
        let just_marked = spool.acknowledge(&outcomes, Utc::now());
        // Nothing moved: the gateway answered without resolving anything this
        // spool sent. Continuing would loop forever on the same batch.
        if just_marked == 0 {
            return Err(
                "the gateway acknowledged none of the batch; nothing was marked delivered"
                    .to_owned(),
            );
        }
        marked += just_marked;
    }
    Ok(marked)
}

/// Closes a run whose events have all landed.
///
/// A conflict is not an error here: the run may already be closed, by an
/// earlier flush or by a hook that got there first, and a close is idempotent
/// in intent even though the API refuses a second transition.
async fn close_run(api: &Api, session_id: &str, reason: Option<&str>) -> Result<(), String> {
    let mut body = serde_json::Map::new();
    body.insert("status".to_owned(), json!("ended"));
    if let Some(reason) = reason {
        body.insert("end_reason".to_owned(), json!(reason));
    }
    match api
        .post(
            &format!("/v1/sessions/{session_id}/end"),
            Some(serde_json::Value::Object(body)),
        )
        .await
    {
        Ok(_) => Ok(()),
        Err(message) if message.contains("closed") || message.contains("conflict") => Ok(()),
        Err(message) => Err(message),
    }
}

/// `synveda session spool status` — what is held, per session.
pub fn status(dir: Option<PathBuf>, as_json: bool) -> Result<(), String> {
    let dir = dir.unwrap_or_else(spool::spool_dir);
    let scanned = spool::scan(&dir);

    if as_json {
        let rows: Vec<_> = scanned
            .spools
            .iter()
            .map(|(path, spool)| {
                json!({
                    "file": path.to_string_lossy(),
                    "client_name": spool.client_name,
                    "client_installation_id": spool.client_installation_id,
                    "session_id": spool.session_id,
                    "external_session_id": spool.external_session_id,
                    "events": spool.entries.len(),
                    "pending": spool.pending_count(),
                    "acknowledged": spool.entries.len() - spool.pending_count(),
                    "corrupt": spool.entries.iter().filter(|e| !e.intact()).count(),
                    "attempts": spool.pending().map(|e| e.delivery_attempts).max().unwrap_or(0),
                    "oldest_pending": spool.pending().map(|e| e.occurred_at).min(),
                    "updated_at": spool.updated_at,
                })
            })
            .collect();
        let unreadable: Vec<_> = scanned
            .unreadable
            .iter()
            .map(|(path, reason)| json!({"file": path.to_string_lossy(), "reason": reason}))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "directory": dir.to_string_lossy(),
                "spools": rows,
                "unreadable": unreadable,
            }))
            .map_err(|err| err.to_string())?
        );
        return Ok(());
    }

    report_unreadable(&scanned);
    if scanned.spools.is_empty() {
        println!("Nothing spooled in {}.", dir.display());
        return Ok(());
    }

    let mut pending_total = 0usize;
    let mut corrupt_total = 0usize;
    for (path, spool) in &scanned.spools {
        let pending = spool.pending_count();
        pending_total += pending;
        let corrupt = spool.entries.iter().filter(|entry| !entry.intact()).count();
        corrupt_total += corrupt;
        let run = spool
            .session_id
            .as_deref()
            .unwrap_or("(no run opened yet — the next SessionStart opens it)");
        println!("{}", name(path));
        println!(
            "  client   {} ({})",
            spool.client_name, spool.client_installation_id
        );
        println!("  run      {run}");
        println!("  harness  {}", spool.external_session_id);
        println!(
            "  events   {} total, {} pending, {} acknowledged{}",
            spool.entries.len(),
            pending,
            spool.entries.len() - pending,
            if spool.fully_acknowledged() {
                " — fully delivered"
            } else {
                ""
            }
        );
        if pending > 0 {
            // The range rather than a count: "events 7 through 19 are
            // unacknowledged" is something a person can act on.
            let first = spool.pending().map(|entry| entry.sequence).min();
            let last = spool.pending().map(|entry| entry.sequence).max();
            if let (Some(first), Some(last)) = (first, last) {
                println!("  waiting  sequence {first}–{last}");
            }
            let attempts = spool
                .pending()
                .map(|entry| entry.delivery_attempts)
                .max()
                .unwrap_or(0);
            match spool
                .pending()
                .filter_map(|entry| entry.last_attempt_at)
                .max()
            {
                Some(at) => println!("  attempts {attempts}, last at {at}"),
                None => println!("  attempts none yet"),
            }
        }
        if corrupt > 0 {
            println!(
                "  CORRUPT  {corrupt} event(s) failed their payload hash and will not be sent"
            );
        }
        println!();
    }
    println!(
        "{} session(s), {pending_total} event(s) waiting to be delivered.",
        scanned.spools.len()
    );
    if corrupt_total > 0 {
        println!("{corrupt_total} event(s) are corrupt on disk and are held rather than sent.");
    }
    if pending_total > 0 {
        println!("Run `synveda session flush` to deliver them now.");
    }
    Ok(())
}

/// `synveda session spool purge --acknowledged` — drop what the gateway has.
///
/// `acknowledged` is a required flag rather than a default (ADR-0078
/// decision 7). It reads as a tautology on the command line and it is not one:
/// it is the difference between a command that can only reclaim disk and a
/// command that can destroy an observation nobody has delivered yet.
pub fn purge(dir: Option<PathBuf>, acknowledged: bool) -> Result<(), String> {
    if !acknowledged {
        return Err(
            "`synveda session spool purge` deletes only acknowledged events, and says so: \
             pass --acknowledged. There is no flag that deletes undelivered ones."
                .to_owned(),
        );
    }
    let dir = dir.unwrap_or_else(spool::spool_dir);
    let scanned = spool::scan(&dir);
    report_unreadable(&scanned);
    if scanned.spools.is_empty() {
        println!("Nothing spooled in {}.", dir.display());
        return Ok(());
    }

    let mut removed_events = 0usize;
    let mut removed_files = 0usize;
    let mut kept = 0usize;
    for (path, mut spool) in scanned.spools {
        let removed = spool.purge_acknowledged();
        removed_events += removed;
        if spool.entries.is_empty() {
            // Nothing left to deliver and nothing left to read: the file is
            // the last thing holding the directory open.
            std::fs::remove_file(&path)
                .map_err(|err| format!("remove {}: {err}", path.display()))?;
            removed_files += 1;
            continue;
        }
        kept += spool.pending_count();
        if removed > 0 {
            spool::write(&path, &spool)?;
        }
    }
    println!("Removed {removed_events} acknowledged event(s) and {removed_files} empty file(s).");
    if kept > 0 {
        println!("{kept} event(s) are still waiting to be delivered and were kept.");
    }
    Ok(())
}

/// Files this build could not read, named rather than silently skipped.
///
/// A spool it cannot parse may be somebody's undelivered transcript, and a
/// command that said nothing about it would look like a command that found
/// nothing.
fn report_unreadable(scanned: &spool::Scan) {
    for (path, reason) in &scanned.unreadable {
        eprintln!("  {}: unreadable ({reason}); left in place", name(path));
    }
}

fn name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spool_is_pinned_to_one_gateway() {
        let mut bound = None;
        pin_gateway(&mut bound, "https://one.example").unwrap();
        pin_gateway(&mut bound, "https://one.example").unwrap();
        assert!(pin_gateway(&mut bound, "https://two.example").is_err());
        assert_eq!(bound.as_deref(), Some("https://one.example"));
    }

    /// The guard that makes `--acknowledged` mean something. Without the flag
    /// the command refuses and touches nothing.
    #[test]
    fn purge_without_the_flag_refuses_and_says_why() {
        let error = purge(Some(PathBuf::from("/nonexistent")), false)
            .expect_err("purge must require the flag");
        assert!(error.contains("--acknowledged"), "{error}");
        assert!(
            error.contains("no flag that deletes undelivered"),
            "the refusal should close the door it is asked about: {error}"
        );
    }
}
