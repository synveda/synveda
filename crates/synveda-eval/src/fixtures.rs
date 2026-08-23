//! The labelled extraction corpus (EVAL-2, ADR-0046 decision 7).
//!
//! One corpus, two readers: this one measures the product path over HTTP,
//! and `crates/synveda-ingest/tests/extraction_precision.rs` measures the
//! extractor function with no stack at all. Both deserialize the **full**
//! format with `deny_unknown_fields`, so a field added for one reader
//! cannot be silently ignored by the other. It is a data dependency, not
//! a crate dependency — this crate still depends on no Synveda crate
//! (ADR-0028 decision 1).
//!
//! The validation below runs in `synveda-eval check`, which needs no
//! database and no gateway. A mislabelled fixture — an expected token
//! absent from its own source, bait present in it — would move a gated
//! number forever and silently, and in *both* readers, because they read
//! the same files.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

/// The record classes the corpus may label (seed §4.2). Declared here
/// rather than imported for the same reason every other wire type is.
pub const CLASSES: [&str; 6] = [
    "decision",
    "entity",
    "episode",
    "fact",
    "preference",
    "procedure",
];

/// The session event types that carry memory (CPR-12, ADR-0078 decision 2).
/// A fixture whose event type is outside this list would be appended,
/// ordered and auditable and would enqueue no extraction work at all — so
/// it would score zero and read as a quality collapse rather than as the
/// corpus error it is. Declared here rather than imported for the same
/// reason [`CLASSES`] is.
pub const EVENT_TYPES: [&str; 7] = [
    "message.user",
    "message.assistant",
    "tool.invoked",
    "tool.result",
    "file.changed",
    "command.executed",
    "memory.asserted",
];

/// One group file: one eval actor's worth of fixtures. The partition is
/// load-bearing — records land at the caller's home scope (ADR-0020), so
/// one actor per group is what keeps each group's corpus its own
/// (ADR-0046 decision 2).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub group: String,
    /// The actor to seed and sweep as. Must name an actor the environment
    /// carries.
    pub actor: String,
    /// Why this group exists, for whoever adds to it next.
    pub note: String,
    pub fixtures: Vec<Fixture>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    pub name: String,
    /// Why this fixture is interesting, or what it is expected to miss and
    /// why. It rides into the report, where an expected miss is otherwise
    /// indistinguishable from a regression.
    #[serde(default)]
    pub note: String,
    pub input: FixtureInput,
    pub expected: Vec<Expected>,
    /// Phrases a hallucinating extractor would plausibly produce from this
    /// transcript and which the transcript does not support (ADR-0046
    /// decision 6).
    #[serde(default)]
    pub must_not_extract: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureInput {
    /// A session event type, not the old observe `kind`: the vocabulary
    /// changed with the plane (CPR-12, ADR-0078 decision 2).
    pub event_type: String,
    pub session_id: String,
    pub occurred_at: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expected {
    pub class: String,
    /// A distinctive term any faithful summary keeps. Absent means "any
    /// record of this class counts", which is weaker and rarely what a
    /// fixture wants.
    #[serde(default)]
    pub content_contains: Option<String>,
}

impl Fixture {
    /// Every string value in the payload, joined — the same walk the
    /// extractor and the redaction scanner take. Used by the guards.
    #[must_use]
    pub fn source_text(&self) -> String {
        fn collect<'a>(value: &'a serde_json::Value, into: &mut Vec<&'a str>) {
            match value {
                serde_json::Value::String(text) => into.push(text),
                serde_json::Value::Array(items) => {
                    items.iter().for_each(|item| collect(item, into));
                }
                serde_json::Value::Object(map) => {
                    map.values().for_each(|item| collect(item, into));
                }
                _ => {}
            }
        }
        let mut parts = Vec::new();
        collect(&self.input.payload, &mut parts);
        parts.join(" ")
    }

    /// Does a served record satisfy this expectation? Class equality plus
    /// case-insensitive containment — MEM-3's predicate, unchanged, so the
    /// two readers cannot disagree about what a match is.
    #[must_use]
    pub fn matches(expected: &Expected, class: &str, content: &str) -> bool {
        expected.class == class
            && expected
                .content_contains
                .as_deref()
                .is_none_or(|token| content.to_lowercase().contains(&token.to_lowercase()))
    }
}

/// Every `*.json` group in a directory, in filename order so two runs
/// report in the same order. Validated as a whole: the guards are
/// corpus-wide, not per file.
pub fn load_corpus(dir: &Path) -> Result<Vec<Group>, String> {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .map_err(|err| format!("read the corpus {}: {err}", dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err(format!("{} holds no fixture groups", dir.display()));
    }

    let mut groups = Vec::with_capacity(paths.len());
    for path in paths {
        let raw = std::fs::read_to_string(&path)
            .map_err(|err| format!("read {}: {err}", path.display()))?;
        let group: Group = serde_json::from_str(&raw)
            .map_err(|err| format!("{} is not a valid fixture group: {err}", path.display()))?;
        groups.push(group);
    }
    validate(&groups)?;
    Ok(groups)
}

/// The four guards the corpus README states, plus the vocabulary checks
/// serde cannot make.
fn validate(groups: &[Group]) -> Result<(), String> {
    let mut names: BTreeMap<&str, &str> = BTreeMap::new();
    let mut sessions: BTreeMap<&str, &str> = BTreeMap::new();

    for group in groups {
        if group.fixtures.is_empty() {
            return Err(format!("group `{}` holds no fixtures", group.group));
        }
        for fixture in &group.fixtures {
            let at = |what: &str| format!("{}/{}: {what}", group.group, fixture.name);

            if let Some(previous) = names.insert(&fixture.name, &group.group) {
                return Err(format!(
                    "fixture name `{}` is used by both group `{previous}` and group `{}`",
                    fixture.name, group.group
                ));
            }
            // The harness attributes a served record back to its fixture
            // through provenance; a collision merges two fixtures' results.
            if let Some(previous) = sessions.insert(&fixture.input.session_id, &fixture.name) {
                return Err(format!(
                    "session id `{}` is used by both `{previous}` and `{}`",
                    fixture.input.session_id, fixture.name
                ));
            }
            if !EVENT_TYPES.contains(&fixture.input.event_type.as_str()) {
                return Err(at(&format!(
                    "event type `{}` is not one of {EVENT_TYPES:?} — a type outside \
                     this list carries no memory, so the fixture would score zero \
                     without anything having gone wrong",
                    fixture.input.event_type
                )));
            }
            if chrono::DateTime::parse_from_rfc3339(&fixture.input.occurred_at).is_err() {
                return Err(at(&format!(
                    "occurred_at `{}` is not an RFC 3339 instant",
                    fixture.input.occurred_at
                )));
            }

            let source = fixture.source_text().to_lowercase();
            for expected in &fixture.expected {
                if !CLASSES.contains(&expected.class.as_str()) {
                    return Err(at(&format!(
                        "class `{}` is not one of {CLASSES:?}",
                        expected.class
                    )));
                }
                if let Some(token) = &expected.content_contains
                    && !source.contains(&token.to_lowercase())
                {
                    return Err(at(&format!(
                        "expected token {token:?} is absent from its own source — a mislabelled \
                         fixture, not a real miss"
                    )));
                }
            }
            for bait in &fixture.must_not_extract {
                if source.contains(&bait.to_lowercase()) {
                    return Err(at(&format!(
                        "bait {bait:?} appears in its own source, so it can never be a \
                         hallucination"
                    )));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(fixtures: &str) -> Result<Vec<Group>, String> {
        let raw = format!(r#"{{"group":"g","actor":"a","note":"n","fixtures":[{fixtures}]}}"#);
        let parsed: Group = serde_json::from_str(&raw).map_err(|err| err.to_string())?;
        let groups = vec![parsed];
        validate(&groups)?;
        Ok(groups)
    }

    const CLEAN: &str = r#"{
        "name": "f1",
        "input": {"event_type": "message.user", "session_id": "s1",
                  "occurred_at": "2026-07-20T10:00:00Z",
                  "payload": {"text": "We chose Cedar over OpenFGA."}},
        "expected": [{"class": "decision", "content_contains": "Cedar"}]
    }"#;

    #[test]
    fn a_group_round_trips_with_its_defaults() {
        let groups = group(CLEAN).expect("parses");
        let fixture = &groups[0].fixtures[0];
        assert!(fixture.note.is_empty());
        assert!(fixture.must_not_extract.is_empty());
        assert_eq!(fixture.source_text(), "We chose Cedar over OpenFGA.");
    }

    #[test]
    fn an_unknown_field_is_refused_rather_than_ignored() {
        // The failure this guards is the one that makes two readers report
        // the same wrong number: a field one of them understands and the
        // other silently drops.
        let json = CLEAN.replace(r#""expected""#, r#""expcted""#);
        let err = group(&json).expect_err("unknown field must not parse");
        assert!(err.contains("expcted"), "unhelpful error: {err}");
    }

    #[test]
    fn an_expected_token_absent_from_its_source_is_refused() {
        let json = CLEAN.replace("\"Cedar\"", "\"OpenFGA was chosen\"");
        let err = group(&json).expect_err("mislabelled fixture must not validate");
        assert!(err.contains("absent from its own source"), "{err}");
    }

    #[test]
    fn bait_present_in_its_source_is_refused() {
        // Bait the transcript actually says is not bait: a faithful
        // extractor reproduces it, and the axis measures copying.
        let json = CLEAN.replace(
            r#""expected""#,
            r#""must_not_extract": ["chose Cedar"], "expected""#,
        );
        let err = group(&json).expect_err("non-bait must not validate");
        assert!(err.contains("can never be a hallucination"), "{err}");
    }

    #[test]
    fn a_duplicate_session_id_is_refused() {
        let second = CLEAN.replace("\"f1\"", "\"f2\"");
        let err = group(&format!("{CLEAN},{second}")).expect_err("collision must not validate");
        assert!(err.contains("session id `s1`"), "{err}");
    }

    #[test]
    fn a_duplicate_fixture_name_is_refused() {
        let second = CLEAN.replace("\"s1\"", "\"s2\"");
        let err = group(&format!("{CLEAN},{second}")).expect_err("duplicate must not validate");
        assert!(err.contains("fixture name `f1`"), "{err}");
    }

    #[test]
    fn the_vocabularies_are_closed() {
        let bad_class = CLEAN.replace("\"decision\"", "\"opinion\"");
        assert!(
            group(&bad_class).is_err(),
            "unknown class must not validate"
        );
        let bad_type = CLEAN.replace("\"message.user\"", "\"thought\"");
        assert!(
            group(&bad_type).is_err(),
            "unknown event type must not validate"
        );
        // And a type the *session plane* accepts but which carries no
        // memory is refused here too: it would append cleanly, enqueue
        // nothing, and score as a quality collapse (ADR-0078 decision 2).
        let no_memory = CLEAN.replace("\"message.user\"", "\"session.started\"");
        assert!(
            group(&no_memory).is_err(),
            "a type that carries no memory must not validate"
        );
        let bad_instant = CLEAN.replace("\"2026-07-20T10:00:00Z\"", "\"last Tuesday\"");
        assert!(
            group(&bad_instant).is_err(),
            "bad instant must not validate"
        );
    }

    #[test]
    fn matching_is_class_equality_plus_case_insensitive_containment() {
        let expected = Expected {
            class: "decision".to_owned(),
            content_contains: Some("Cedar".to_owned()),
        };
        assert!(Fixture::matches(&expected, "decision", "we chose cedar"));
        assert!(!Fixture::matches(&expected, "fact", "we chose Cedar"));
        assert!(!Fixture::matches(&expected, "decision", "we chose OpenFGA"));

        // No token means any record of the class counts.
        let loose = Expected {
            class: "fact".to_owned(),
            content_contains: None,
        };
        assert!(Fixture::matches(&loose, "fact", "anything at all"));
    }
}
