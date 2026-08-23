//! The durable observation spool (CPR-12, ADR-0078 decision 6).
//!
//! One file per session under `$XDG_STATE_HOME/synveda/spool/`, holding the
//! events a client has recorded and whether this deployment has them yet.
//!
//! # Two programs, one format
//!
//! The adapter writes these files and this CLI reads them. That is deliberate:
//! a hook runs for milliseconds and cannot own a retry schedule, so the thing
//! that retries has to be able to pick up where a hook left off — and a person
//! whose gateway was down for a day needs to be able to *see* what is held and
//! flush it by hand. Both halves read the same bytes.
//!
//! # What the old spool was, and why nothing reads it
//!
//! Until this feature the spool held a **cursor**: the uuid of the last
//! transcript entry a gateway 2xx had accepted, and nothing else. Everything
//! after it was re-derived by re-reading the harness's own transcript file.
//! That is at-least-once only while that file still exists and still contains
//! those entries — and a compaction rewrites it, a `/clear` truncates it, and
//! a deleted project takes it.
//!
//! So the old format is not migrated, not parsed and not consulted. There is
//! nothing in one to recover: it never held an event. Files carrying a
//! `spool_version` this build does not know are left alone and reported,
//! rather than guessed at.
//!
//! # The hash is SHA-256, and that is the one divergence
//!
//! Everything else in this product digests with BLAKE3. The writer here is
//! Node with no dependencies beyond its own types, and `node:crypto` has no
//! BLAKE3. This hash's job is detecting local corruption of a file on the
//! user's disk between the hook that wrote it and the flush that reads it; the
//! **authoritative** digest of an event is the server's BLAKE3 over the
//! canonical payload, computed on append, and nothing about that changes.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// The format version this build writes and reads.
///
/// A file carrying anything else is left where it is: a spool is somebody's
/// undelivered data, and a reader that guessed at an unknown layout could
/// silently drop it.
pub const SPOOL_VERSION: u32 = 1;

/// One spooled event.
///
/// Every field ADR-0078 decision 6 names as per-entry is here. The three
/// file-level ones — the format version, the installation and the session —
/// live in the header, because they are constant for the file's whole life and
/// repeating them per entry would be a chance for them to disagree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpoolEntry {
    /// The client's own id for this event — the idempotency unit the API keys
    /// on, and what makes a redelivered batch append nothing twice.
    pub client_event_id: String,
    /// The **client's** local order. Not the server's: the gateway assigns its
    /// own `sequence` on append and that one is authoritative for ordering a
    /// timeline. This one is what makes a bounded flush deterministic and what
    /// lets `spool status` name a range rather than a count.
    pub sequence: u64,
    /// One of `SessionEventType`'s names.
    pub event_type: String,
    /// When the client says it happened.
    pub occurred_at: DateTime<Utc>,
    /// The content.
    pub payload: serde_json::Value,
    /// SHA-256 over the canonical encoding of `payload`, hex.
    pub payload_hash: String,
    /// How many times delivery has been attempted.
    #[serde(default)]
    pub delivery_attempts: u32,
    /// When the last attempt was.
    #[serde(default)]
    pub last_attempt_at: Option<DateTime<Utc>>,
    /// Whether the gateway has resolved this event.
    ///
    /// True for every terminal answer, not only a successful store: `appended`,
    /// `duplicate`, `quarantined` and `denied` are all decisions this
    /// deployment has made, and re-sending any of them would produce the same
    /// answer forever.
    #[serde(default)]
    pub acknowledged: bool,
    /// What the gateway answered, once it has. `None` while pending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

impl SpoolEntry {
    /// Whether the stored hash still matches the stored payload.
    #[must_use]
    pub fn intact(&self) -> bool {
        payload_hash(&self.payload) == self.payload_hash
    }
}

/// One session's spool file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spool {
    /// The format's version.
    pub spool_version: u32,
    /// A stable id for this installation of the client — what tells two
    /// machines running the same client apart.
    pub client_installation_id: String,
    /// The agent client, as it names itself.
    pub client_name: String,
    /// The Synveda session these events belong to.
    ///
    /// `None` until one has been opened — which is the state a spool is in when
    /// the very first `SessionStart` could not reach the gateway. The events
    /// are still recorded; they simply have nowhere to go yet, and the next
    /// `SessionStart` opens the run and delivers them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The harness's own id for this run — what lets a stateless hook find the
    /// session it already opened instead of minting a second one.
    pub external_session_id: String,
    /// The workspace the run is in, needed to open the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// The project, when the run is against one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// The gateway this spool is bound to. A file written against one
    /// deployment must never be flushed into another.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_url: Option<String>,
    /// The last transcript entry the client turned into an entry here.
    ///
    /// A **recording** watermark, not a delivery one, and the difference is
    /// the whole point of this format. The old spool's cursor advanced only on
    /// a gateway 2xx, so everything after it had to be re-derived from the
    /// harness's transcript file — which a compaction rewrites. This advances
    /// as soon as an event is durable *here*, and delivery is tracked per
    /// entry instead.
    ///
    /// This side never sets it and must never drop it: a flush that wrote the
    /// file back without it would make the client re-record the whole
    /// transcript on its next hook.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_through: Option<String>,
    /// Whether the client has asked for this run to be closed.
    ///
    /// Set by the `SessionEnd` hook when its bounded flush could not drain the
    /// spool. Whoever finishes the delivery closes the run — which is why the
    /// two-phase close exists: the run sits in `ending`, still accepting the
    /// events it is waiting for, rather than closing over a backlog.
    #[serde(default)]
    pub close_requested: bool,
    /// Why the client stopped, carried to the close it could not perform.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_reason: Option<String>,
    /// When the file was created.
    pub created_at: DateTime<Utc>,
    /// When it was last written.
    pub updated_at: DateTime<Utc>,
    /// The events, in client order.
    #[serde(default)]
    pub entries: Vec<SpoolEntry>,
}

impl Spool {
    /// The entries the gateway has not resolved, in client order.
    pub fn pending(&self) -> impl Iterator<Item = &SpoolEntry> {
        self.entries.iter().filter(|entry| !entry.acknowledged)
    }

    /// How many entries are still pending.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.pending().count()
    }

    /// Whether every entry has been resolved.
    #[must_use]
    pub fn fully_acknowledged(&self) -> bool {
        self.entries.iter().all(|entry| entry.acknowledged)
    }

    /// Marks the entries the gateway resolved, by the client's own event id.
    ///
    /// Keyed by id rather than by position because the answer is per event: a
    /// batch that overlapped a previous one by three of ten comes back with
    /// three `duplicate` and seven `appended`, at their original positions, and
    /// a positional merge would mark the wrong rows the first time a denied
    /// event shortened the batch.
    pub fn acknowledge(&mut self, outcomes: &BTreeMap<String, String>, at: DateTime<Utc>) -> usize {
        let mut marked = 0;
        for entry in &mut self.entries {
            if entry.acknowledged {
                continue;
            }
            if let Some(outcome) = outcomes.get(&entry.client_event_id) {
                entry.acknowledged = true;
                entry.outcome = Some(outcome.clone());
                entry.last_attempt_at = Some(at);
                marked += 1;
            }
        }
        if marked > 0 {
            self.updated_at = at;
        }
        marked
    }

    /// Records that delivery was attempted for every pending entry, whether or
    /// not it succeeded.
    ///
    /// The count and the instant are what tell a person reading `spool status`
    /// the difference between "the gateway has been down for an hour" and
    /// "nothing has ever tried to send this".
    pub fn record_attempt(&mut self, at: DateTime<Utc>) {
        for entry in &mut self.entries {
            if !entry.acknowledged {
                entry.delivery_attempts = entry.delivery_attempts.saturating_add(1);
                entry.last_attempt_at = Some(at);
            }
        }
        self.updated_at = at;
    }

    /// Drops every acknowledged entry, returning how many went.
    ///
    /// The only deletion this plane performs, and it is why `purge` requires
    /// `--acknowledged` rather than defaulting to it: the one irreversible
    /// thing that can be done to an undelivered observation is delete it.
    pub fn purge_acknowledged(&mut self) -> usize {
        let before = self.entries.len();
        self.entries.retain(|entry| !entry.acknowledged);
        let removed = before - self.entries.len();
        if removed > 0 {
            self.updated_at = Utc::now();
        }
        removed
    }
}

/// SHA-256 over a payload's canonical encoding, hex.
///
/// Canonical through [`synveda_types::json::canonicalise`] for the reason the
/// server's own digest is: `cedar-policy-core` turns on
/// `serde_json/preserve_order` and Cargo unifies features across the workspace,
/// so an object iterates in whatever order it was parsed in. Without the sort a
/// payload re-read from disk could hash differently than it did when written,
/// and the integrity check would fire on healthy files.
#[must_use]
pub fn payload_hash(payload: &serde_json::Value) -> String {
    let canonical = synveda_types::json::canonicalise(payload).to_string();
    let digest = Sha256::digest(canonical.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `$XDG_STATE_HOME/synveda/spool`, else `~/.local/state/synveda/spool`.
///
/// The same resolution the adapter performs, and it must stay that way: two
/// programs that disagree about where the spool lives is a spool that silently
/// never drains.
#[must_use]
pub fn spool_dir() -> PathBuf {
    let base = match std::env::var("XDG_STATE_HOME") {
        // A relative XDG path is undefined behaviour per the spec; ignored
        // rather than resolved against whatever directory happens to be
        // current.
        Ok(value) if value.starts_with('/') => PathBuf::from(value),
        _ => match std::env::var("HOME") {
            Ok(home) => PathBuf::from(home).join(".local").join("state"),
            Err(_) => PathBuf::from(".local").join("state"),
        },
    };
    base.join("synveda").join("spool")
}

/// What a directory scan found.
pub struct Scan {
    /// The spools this build understands.
    pub spools: Vec<(PathBuf, Spool)>,
    /// Files that did not parse, or carried a `spool_version` this build does
    /// not know. Reported rather than deleted: somebody's undelivered data is
    /// not this command's to discard.
    pub unreadable: Vec<(PathBuf, String)>,
}

/// Reads every spool file in `dir`, newest last.
///
/// A missing directory is an empty scan and not an error: a machine that has
/// never run an agent has no spool, and that is the ordinary case for anybody
/// running `spool status` to find out what the command does.
pub fn scan(dir: &Path) -> Scan {
    let mut spools = Vec::new();
    let mut unreadable = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return Scan { spools, unreadable };
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    for path in paths {
        match read(&path) {
            Ok(spool) => spools.push((path, spool)),
            Err(message) => unreadable.push((path, message)),
        }
    }
    Scan { spools, unreadable }
}

/// Reads one spool file.
///
/// # Errors
///
/// The file is unreadable, is not JSON, or carries a `spool_version` this
/// build does not know.
pub fn read(path: &Path) -> Result<Spool, String> {
    let raw = fs::read_to_string(path).map_err(|err| format!("read: {err}"))?;
    let spool: Spool = serde_json::from_str(&raw).map_err(|err| format!("parse: {err}"))?;
    if spool.spool_version != SPOOL_VERSION {
        return Err(format!(
            "spool_version {} (this build reads {SPOOL_VERSION})",
            spool.spool_version
        ));
    }
    Ok(spool)
}

/// Writes one spool file atomically: a temporary in the same directory,
/// flushed and `fsync`ed, then renamed over the target.
///
/// Same directory so the rename is within one filesystem and therefore atomic.
/// `fsync` before the rename because a rename that lands before the data does
/// leaves a file whose name says it is complete and whose bytes are not — which
/// is exactly the failure this whole format exists to survive.
///
/// # Errors
///
/// The directory cannot be created, or the write, sync or rename fails.
pub fn write(path: &Path, spool: &Spool) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or_else(|| "spool path has no directory".to_owned())?;
    fs::create_dir_all(dir).map_err(|err| format!("create {}: {err}", dir.display()))?;
    let temporary = dir.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("spool"),
        std::process::id()
    ));
    let encoded = serde_json::to_vec(spool).map_err(|err| format!("encode: {err}"))?;
    {
        let mut file =
            fs::File::create(&temporary).map_err(|err| format!("create temporary: {err}"))?;
        file.write_all(&encoded)
            .map_err(|err| format!("write temporary: {err}"))?;
        file.sync_all()
            .map_err(|err| format!("sync temporary: {err}"))?;
    }
    fs::rename(&temporary, path).map_err(|err| {
        let _ = fs::remove_file(&temporary);
        format!("rename into place: {err}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, sequence: u64) -> SpoolEntry {
        let payload = serde_json::json!({"text": id});
        SpoolEntry {
            client_event_id: id.to_owned(),
            sequence,
            event_type: "message.user".to_owned(),
            occurred_at: Utc::now(),
            payload_hash: payload_hash(&payload),
            payload,
            delivery_attempts: 0,
            last_attempt_at: None,
            acknowledged: false,
            outcome: None,
        }
    }

    fn spool(entries: Vec<SpoolEntry>) -> Spool {
        Spool {
            spool_version: SPOOL_VERSION,
            client_installation_id: "install-1".to_owned(),
            client_name: "claude-code".to_owned(),
            session_id: Some("11111111-1111-1111-1111-111111111111".to_owned()),
            external_session_id: "harness-1".to_owned(),
            workspace_id: None,
            project_id: None,
            recorded_through: None,
            close_requested: false,
            end_reason: None,
            gateway_url: Some("http://127.0.0.1:8120".to_owned()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            entries,
        }
    }

    /// The hash is over the *canonical* encoding, so a payload whose keys came
    /// back in a different order still verifies. Without this, an entry
    /// round-tripped through any JSON library that preserves insertion order
    /// would read as corrupt.
    #[test]
    fn the_payload_hash_does_not_depend_on_key_order() {
        let a = serde_json::json!({"a": 1, "b": [2, 3]});
        let b = serde_json::json!({"b": [2, 3], "a": 1});
        assert_eq!(payload_hash(&a), payload_hash(&b));
        assert_eq!(payload_hash(&a).len(), 64);
        // Order *within* an array is content, not encoding.
        assert_ne!(
            payload_hash(&serde_json::json!({"b": [2, 3]})),
            payload_hash(&serde_json::json!({"b": [3, 2]}))
        );
    }

    #[test]
    fn a_tampered_payload_stops_being_intact() {
        let mut item = entry("e1", 1);
        assert!(item.intact());
        item.payload = serde_json::json!({"text": "something else"});
        assert!(!item.intact());
    }

    /// Acknowledgement is keyed by the client's own id, so a batch whose
    /// answers come back in a different order than it sent still marks the
    /// right rows.
    #[test]
    fn acknowledgement_is_keyed_by_event_id_and_not_by_position() {
        let mut spool = spool(vec![entry("e1", 1), entry("e2", 2), entry("e3", 3)]);
        let outcomes = BTreeMap::from([
            ("e3".to_owned(), "appended".to_owned()),
            ("e1".to_owned(), "duplicate".to_owned()),
        ]);
        assert_eq!(spool.acknowledge(&outcomes, Utc::now()), 2);
        assert_eq!(spool.pending_count(), 1);
        assert_eq!(spool.pending().next().unwrap().client_event_id, "e2");
        assert_eq!(spool.entries[0].outcome.as_deref(), Some("duplicate"));
    }

    /// Every terminal answer acknowledges, including the two that store
    /// nothing useful: re-sending a denied event produces the same denial
    /// forever, and a spool that retried it would never drain.
    #[test]
    fn a_denied_event_is_acknowledged_like_any_other() {
        let mut spool = spool(vec![entry("e1", 1)]);
        let outcomes = BTreeMap::from([("e1".to_owned(), "denied".to_owned())]);
        spool.acknowledge(&outcomes, Utc::now());
        assert!(spool.fully_acknowledged());
        assert_eq!(spool.purge_acknowledged(), 1);
        assert!(spool.entries.is_empty());
    }

    /// The whole point of `--acknowledged` being required: a purge must never
    /// be able to take something the gateway has not answered for.
    #[test]
    fn purging_never_takes_a_pending_entry() {
        let mut spool = spool(vec![entry("e1", 1), entry("e2", 2)]);
        let outcomes = BTreeMap::from([("e1".to_owned(), "appended".to_owned())]);
        spool.acknowledge(&outcomes, Utc::now());
        assert_eq!(spool.purge_acknowledged(), 1);
        assert_eq!(spool.entries.len(), 1);
        assert_eq!(spool.entries[0].client_event_id, "e2");
    }

    #[test]
    fn an_attempt_counts_only_against_what_is_still_pending() {
        let mut spool = spool(vec![entry("e1", 1), entry("e2", 2)]);
        let outcomes = BTreeMap::from([("e1".to_owned(), "appended".to_owned())]);
        spool.acknowledge(&outcomes, Utc::now());
        spool.record_attempt(Utc::now());
        spool.record_attempt(Utc::now());
        assert_eq!(spool.entries[0].delivery_attempts, 0);
        assert_eq!(spool.entries[1].delivery_attempts, 2);
    }

    #[test]
    fn a_spool_round_trips_through_an_atomic_write() {
        let dir = std::env::temp_dir().join(format!("synveda-spool-{}", std::process::id()));
        let path = dir.join("session.json");
        let original = spool(vec![entry("e1", 1)]);
        write(&path, &original).expect("write");
        let read_back = read(&path).expect("read");
        assert_eq!(read_back.entries.len(), 1);
        assert_eq!(read_back.external_session_id, "harness-1");
        assert!(read_back.entries[0].intact());
        // The temporary is gone: a directory littered with `.tmp` files is a
        // rename that did not happen.
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .expect("read dir")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temporary files left behind");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A file from a format this build does not know is refused by name rather
    /// than parsed optimistically. The old cursor-shaped spool is exactly this
    /// case, and ADR-0078 decision 6 says nothing reads it.
    #[test]
    fn an_unknown_spool_version_is_refused_rather_than_guessed_at() {
        let dir = std::env::temp_dir().join(format!("synveda-spool-v-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("create");
        let path = dir.join("old.json");
        fs::write(&path, br#"{"spool_version":99,"client_installation_id":"i","client_name":"c","external_session_id":"x","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","entries":[]}"#).expect("write");
        let error = read(&path).expect_err("an unknown version is refused");
        assert!(error.contains("spool_version 99"), "{error}");
        let scanned = scan(&dir);
        assert!(scanned.spools.is_empty());
        assert_eq!(scanned.unreadable.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    /// The old format had no `spool_version` at all. It must not parse as a
    /// version-1 spool with an empty entry list, which is what a `#[serde(default)]`
    /// on the version field would have produced — a silent "nothing to flush".
    #[test]
    fn the_previous_cursor_format_does_not_parse_as_this_one() {
        let dir = std::env::temp_dir().join(format!("synveda-spool-old-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("create");
        let path = dir.join("legacy.json");
        fs::write(
            &path,
            br#"{"session_id":"claude-code:abc","transcript_path":"/tmp/t.jsonl","cursor":"uuid-1","updated_at":"2026-01-01T00:00:00Z"}"#,
        )
        .expect("write");
        assert!(read(&path).is_err(), "the old cursor spool must not parse");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A missing spool directory is the ordinary state of a machine that has
    /// never run an agent, and must read as "nothing held" rather than as a
    /// failure.
    #[test]
    fn scanning_a_directory_that_does_not_exist_is_empty_and_not_an_error() {
        let scanned = scan(Path::new("/nonexistent/synveda/spool"));
        assert!(scanned.spools.is_empty());
        assert!(scanned.unreadable.is_empty());
    }
}
