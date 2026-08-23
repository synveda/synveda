//! The security corpus (EVAL-5, ADR-0048 decision 12).
//!
//! A fourth file kind, for ADR-0046 decision 10's reason applied a third
//! time: a boundary declaration and a Q&A question are half inert in each
//! other. What this format says that no other one can is **who must not
//! see what** — and it says it exhaustively, because the guard that
//! matters here is not "did a field get ignored" but "did a pair get
//! left out". An undeclared (record, reader) pair is an unmeasured
//! boundary, and a security suite that skips one silently is the failure
//! mode it exists to prevent (decision 5).
//!
//! Note what this format deliberately does *not* carry: which boundary
//! separates a record from a reader. That is derived per pair from facts
//! the run already holds — the reader's tenant and the record's installed
//! tier — because a corpus author who mis-declared it would move a gated
//! count into the wrong axis, and a derived answer cannot be mis-declared.
//!
//! Every struct here refuses unknown fields, for EVAL-1's reason.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;

use crate::fixtures::EVENT_TYPES;

/// The observe kinds MEM-1 accepts.
/// The tiers a classify proposal may install. `public` and `internal` are
/// absent on purpose: the pipeline already floors at `internal`
/// (ADR-0022) and a proposal that installed a tier below the working one
/// would be a declassification this corpus has no reason to model.
const TIERS: [&str; 2] = ["confidential", "restricted"];

/// A word long enough to be worth generating a variant from. Matches the
/// AUTHZ-5 leak suite's own generator, which this one scales
/// (ADR-0038 decision 19).
const MIN_VARIANT_WORD: usize = 4;

/// One corpus file: material with declared boundaries, and the variant
/// budget the generated half is capped at.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Corpus {
    pub corpus: String,
    /// Why this corpus exists, for whoever adds to it next.
    pub note: String,
    /// Every reader this corpus makes claims about. A record must place
    /// each of them in exactly one of `readable_by` and `forbidden_to`.
    pub readers: Vec<String>,
    pub material: Vec<Material>,
    /// How many distinct generated variants this corpus is worth asking.
    /// The run's own budget narrows it (`--security-variants`), never
    /// widens it: a corpus knows how much material it has and the
    /// combinatorial tail past that is permutations of the same words.
    pub variants: usize,
}

/// One record: how it is planted, where it ends up, what tier it carries,
/// and who may and may not see it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    pub key: String,
    /// Who writes it. Its home leaf is where the record lives, forever —
    /// a promotion publishes a channel that *names* it there (ADR-0034
    /// decision 3).
    pub actor: String,
    pub session_id: String,
    /// A session event type that carries memory (CPR-12, ADR-0078 decision 2).
    /// Defaults to `message.user`, which is what an unlabelled line of a
    /// transcript is.
    #[serde(default = "default_event_type")]
    pub event_type: String,
    pub text: String,
    /// The distinctive phrase the containment predicate looks for. The
    /// second of the two graders (decision 6) — identity says "this record
    /// was served", this says "these bytes were rendered", and a block
    /// where they disagree is its own defect.
    pub marker: String,
    /// The tier to install, through a classify proposal the author opens
    /// at their own home scope. Absent leaves whatever the pipeline
    /// produced, which is `internal` (ADR-0022 clamps below it).
    #[serde(default)]
    pub classify: Option<String>,
    /// The hierarchy node to climb to, named as the environment names it.
    /// Absent leaves the material on its author's leaf, where only the
    /// author composes it.
    #[serde(default)]
    pub promote_to: Option<String>,
    /// Readers this record must reach. The positive control: without it a
    /// run of zeros is indistinguishable from an empty corpus (decision 4).
    #[serde(default)]
    pub readable_by: Vec<String>,
    /// Readers no surface may disclose it to, under any phrasing.
    pub forbidden_to: Vec<String>,
    /// What this record's content attempts to forge, when it is a
    /// structural probe rather than ordinary material (decision 9). Rides
    /// into the report; the assertion is the same for every record.
    #[serde(default)]
    pub forges: Option<String>,
    /// Why this record is in the corpus, for a reader of the report.
    #[serde(default)]
    pub note: String,
}

fn default_event_type() -> String {
    "message.user".to_owned()
}

impl Material {
    /// Whether this record ends up above the working tier, which is what
    /// makes a boundary a *sensitivity* one rather than a scope one.
    #[must_use]
    pub fn is_classified(&self) -> bool {
        self.classify.is_some()
    }
}

/// One generated query, and whether it is part of the hand-written core
/// every reader is asked or of the combinatorial tail that rotates.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Variant {
    pub query: String,
    pub core: bool,
}

/// Every query a reader might plausibly ask this corpus back with.
///
/// Two classes, and the split is what makes a bounded run honest. The
/// **core** is the material itself — each whole line, each significant
/// word, and the upper-cased forms — and every reader is asked all of it
/// on every path, because these are the phrasings a corpus author chose
/// and they are the sharpest probes in the set. The **tail** is every
/// ordered pair of distinct significant words drawn from anywhere in the
/// corpus, which is where a leak under a phrasing nobody tried would live:
/// half of a forbidden line beside half of a permitted one is exactly the
/// query a fused ranker is worst at.
///
/// Deterministic and combinatorial rather than randomised, so the same
/// corpus generates the same list in the same order on every run — a gate
/// that samples randomly fails randomly, and a leak found at variant N has
/// to be reproducible by re-running the first N.
#[must_use]
pub fn variants(corpus: &Corpus, cap: usize) -> Vec<Variant> {
    let mut core: BTreeSet<String> = BTreeSet::new();
    let mut words: BTreeSet<String> = BTreeSet::new();
    for record in &corpus.material {
        core.insert(record.marker.clone());
        core.insert(record.marker.to_uppercase());
        for word in significant_words(&record.marker) {
            core.insert(word.to_uppercase());
            core.insert(word);
        }
        // The tail's vocabulary is the whole seeded text, not just the
        // marker: the marker is the phrase a *grader* looks for, and the
        // words a reader would actually type include the rest of the
        // sentence it came out of.
        words.extend(significant_words(&record.text));
    }
    // Sorted, so the core's order does not depend on which record
    // contributed a word first.
    let mut out: Vec<Variant> = core
        .into_iter()
        .map(|query| Variant { query, core: true })
        .collect();
    out.sort();
    out.dedup();

    let words: Vec<&String> = words.iter().collect();
    let mut tail: Vec<Variant> = Vec::new();
    for left in &words {
        for right in &words {
            if left == right {
                continue;
            }
            tail.push(Variant {
                query: format!("{left} {right}"),
                core: false,
            });
        }
    }
    let budget = cap.min(corpus.variants);
    tail.truncate(budget.saturating_sub(out.len()));
    out.extend(tail);
    out
}

/// The deterministic slice a bounded run asks (decision 13): **every core
/// variant**, plus an evenly spread selection of the tail that fills the
/// remaining budget exactly.
///
/// Not the first N. The core is the designed set and dropping it to save
/// time would drop the sharpest probes first; the tail is where thinning
/// belongs, because taking its head would cluster on whichever word sorts
/// first and leave most of the corpus's vocabulary unasked.
///
/// Spread rather than strided, and that is not a detail: `every k-th` with
/// `k = ceil(tail / room)` collapses to half the budget the moment `k`
/// rounds up to 2 — a nightly asking for ten thousand variants would
/// quietly ask five and a half thousand, which is exactly the "passes by
/// measuring less" failure the gated floors exist to catch, arriving from
/// inside the harness. This selection returns `room` items on the nose, so
/// `security_variants` equals the budget and a floor can be the budget.
#[must_use]
pub fn slice(all: Vec<Variant>, budget: usize) -> Vec<Variant> {
    let core = all.iter().filter(|variant| variant.core).count();
    if all.len() <= budget {
        return all;
    }
    let room = budget.saturating_sub(core);
    if room == 0 {
        return all.into_iter().filter(|variant| variant.core).collect();
    }
    let tail = all.len() - core;
    let mut position = 0_usize;
    all.into_iter()
        .filter(|variant| {
            if variant.core {
                return true;
            }
            // Bresenham: take this one when the running quota crosses an
            // integer boundary. Exactly `room` of `tail`, evenly spread,
            // and the same items every run.
            let take = (position * room) / tail != ((position + 1) * room) / tail;
            position += 1;
            take
        })
        .collect()
}

/// Words long enough to carry meaning, lower-cased, in order of
/// appearance and deduplicated by the caller's set.
fn significant_words(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| word.len() >= MIN_VARIANT_WORD)
        .map(str::to_lowercase)
        .collect()
}

/// Every `*.json` corpus in a directory, in filename order so two runs
/// report in the same order.
pub fn load_corpora(dir: &Path) -> Result<Vec<Corpus>, String> {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .map_err(|err| format!("read the security corpus {}: {err}", dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err(format!("{} holds no security corpora", dir.display()));
    }

    let mut corpora = Vec::with_capacity(paths.len());
    for path in paths {
        let raw = std::fs::read_to_string(&path)
            .map_err(|err| format!("read {}: {err}", path.display()))?;
        let corpus: Corpus = serde_json::from_str(&raw)
            .map_err(|err| format!("{} is not a valid security corpus: {err}", path.display()))?;
        corpora.push(corpus);
    }
    validate(&corpora)?;
    Ok(corpora)
}

/// The checks serde cannot make, all of which run in `synveda-eval check`
/// with no database and no gateway.
///
/// The first one is the reason this function exists at all: **every
/// (record, reader) pair is declared exactly once**. A pair left out of
/// both lists is a boundary nothing asserts, and it would not show up
/// anywhere — the leak counts would still read zero and the report would
/// still look complete.
fn validate(corpora: &[Corpus]) -> Result<(), String> {
    let mut keys: BTreeMap<&str, &str> = BTreeMap::new();
    let mut sessions: BTreeMap<&str, &str> = BTreeMap::new();
    let mut names: BTreeSet<&str> = BTreeSet::new();

    for corpus in corpora {
        if !names.insert(corpus.corpus.as_str()) {
            return Err(format!("two corpora are both named `{}`", corpus.corpus));
        }
        if corpus.readers.is_empty() {
            return Err(format!(
                "corpus `{}` names no readers, so it makes no claim about anybody",
                corpus.corpus
            ));
        }
        if corpus.material.is_empty() {
            return Err(format!("corpus `{}` plants nothing", corpus.corpus));
        }
        if corpus.variants == 0 {
            return Err(format!(
                "corpus `{}` asks for zero variants; a leak suite whose denominator is zero is a \
                 gate that cannot fail",
                corpus.corpus
            ));
        }
        let readers: BTreeSet<&str> = corpus.readers.iter().map(String::as_str).collect();
        if readers.len() != corpus.readers.len() {
            return Err(format!("corpus `{}` names a reader twice", corpus.corpus));
        }

        for record in &corpus.material {
            let at = |what: &str| format!("{}/{}: {what}", corpus.corpus, record.key);
            if let Some(previous) = keys.insert(&record.key, &corpus.corpus) {
                return Err(format!(
                    "material key `{}` is used by both corpus `{previous}` and corpus `{}`",
                    record.key, corpus.corpus
                ));
            }
            sessions.insert(&record.session_id, &corpus.corpus);
            if !EVENT_TYPES.contains(&record.event_type.as_str()) {
                return Err(at(&format!(
                    "event type `{}` is not one of {EVENT_TYPES:?}",
                    record.event_type
                )));
            }
            if let Some(tier) = &record.classify
                && !TIERS.contains(&tier.as_str())
            {
                return Err(at(&format!("classify `{tier}` is not one of {TIERS:?}")));
            }
            if record.marker.trim().is_empty() {
                return Err(at(
                    "a record with no marker cannot be graded by containment, which is half of \
                     what makes a leak visible",
                ));
            }
            // The marker has to be in the text it claims to mark, or the
            // containment predicate is looking for something no block
            // could ever carry — EVAL-2's "an expected token absent from
            // its own source", at the boundary where being wrong is a
            // gate that cannot fail.
            if !record.text.contains(&record.marker) {
                return Err(at(&format!(
                    "marker {:?} does not appear in this record's own text, so the containment \
                     half of the grader would look for bytes the product could never render",
                    record.marker
                )));
            }
            if record.forbidden_to.is_empty() {
                return Err(at(
                    "a record forbidden to nobody measures no boundary; this is a security \
                     corpus, and a control that is readable by everyone belongs in the Q&A one",
                ));
            }

            // The exhaustiveness guard (decision 5).
            let readable: BTreeSet<&str> = record.readable_by.iter().map(String::as_str).collect();
            let forbidden: BTreeSet<&str> =
                record.forbidden_to.iter().map(String::as_str).collect();
            for who in readable.union(&forbidden) {
                if !readers.contains(who) {
                    return Err(at(&format!(
                        "names `{who}`, which is not one of this corpus's readers"
                    )));
                }
            }
            let both: Vec<&&str> = readable.intersection(&forbidden).collect();
            if !both.is_empty() {
                return Err(at(&format!(
                    "declares {both:?} both readable and forbidden, and a boundary that says two \
                     things asserts neither"
                )));
            }
            let undeclared: Vec<&&str> = readers
                .iter()
                .filter(|who| !readable.contains(*who) && !forbidden.contains(*who))
                .collect();
            if !undeclared.is_empty() {
                return Err(at(&format!(
                    "says nothing about {undeclared:?}; every (record, reader) pair is a boundary \
                     this suite either asserts or leaves unmeasured, and an unmeasured one still \
                     reports zero leaks"
                )));
            }

            // `promote_to` is a climb, and a climb only makes sense
            // towards readers the record is supposed to reach.
            if record.promote_to.is_some() && record.readable_by.is_empty() {
                return Err(at(
                    "climbs to a scope and is readable by nobody, so the promotion is either \
                     pointless or the boundaries are wrong",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLEAN: &str = r#"{
        "corpus": "c1",
        "note": "n",
        "readers": ["sec-owner", "sec-mate", "sec-neighbour"],
        "variants": 100,
        "material": [
            {"key": "vault", "actor": "sec-owner", "session_id": "s-vault",
             "text": "The vault ceremony needs two custodians and the offline shard.",
             "marker": "two custodians and the offline shard",
             "classify": "restricted",
             "readable_by": [],
             "forbidden_to": ["sec-owner", "sec-mate", "sec-neighbour"]},
            {"key": "rota", "actor": "sec-owner", "session_id": "s-rota",
             "event_type": "message.assistant",
             "text": "The incident bridge rota is maintained by the platform leads.",
             "marker": "incident bridge rota",
             "promote_to": "vault",
             "readable_by": ["sec-owner", "sec-mate"],
             "forbidden_to": ["sec-neighbour"]}
        ]
    }"#;

    fn parse(json: &str) -> Result<Vec<Corpus>, String> {
        let corpus: Corpus = serde_json::from_str(json).map_err(|err| err.to_string())?;
        let corpora = vec![corpus];
        validate(&corpora)?;
        Ok(corpora)
    }

    #[test]
    fn a_corpus_round_trips_with_its_defaults() {
        let corpora = parse(CLEAN).expect("parses");
        let corpus = &corpora[0];
        assert_eq!(corpus.material[0].event_type, "message.user");
        assert!(corpus.material[0].is_classified());
        assert!(!corpus.material[1].is_classified());
        assert!(corpus.material[0].forges.is_none());
    }

    #[test]
    fn an_unknown_field_is_refused_rather_than_ignored() {
        let json = CLEAN.replace(r#""forbidden_to""#, r#""forbiden_to""#);
        let err = parse(&json).expect_err("unknown field must not parse");
        assert!(err.contains("forbiden_to"), "unhelpful error: {err}");
    }

    /// The guard this whole format exists for (ADR-0048 decision 5). An
    /// undeclared pair reports zero leaks and looks exactly like a pair
    /// that held, which is the one way a security suite can be green and
    /// wrong at the same time.
    #[test]
    fn a_reader_no_record_declares_is_a_parse_error() {
        let json = CLEAN.replace(
            r#""forbidden_to": ["sec-neighbour"]"#,
            r#""forbidden_to": []"#,
        );
        let err = parse(&json).expect_err("an undeclared pair must not validate");
        assert!(
            err.contains("measures no boundary"),
            "unhelpful error: {err}"
        );

        let widened = CLEAN.replace(
            r#""readers": ["sec-owner", "sec-mate", "sec-neighbour"]"#,
            r#""readers": ["sec-owner", "sec-mate", "sec-neighbour", "xt-reader"]"#,
        );
        let err = parse(&widened).expect_err("a reader nothing declares must not validate");
        assert!(err.contains("says nothing about"), "unhelpful error: {err}");
        assert!(err.contains("xt-reader"), "{err}");
    }

    #[test]
    fn a_pair_declared_twice_is_a_parse_error() {
        let json = CLEAN.replace(
            r#""readable_by": ["sec-owner", "sec-mate"]"#,
            r#""readable_by": ["sec-owner", "sec-mate", "sec-neighbour"]"#,
        );
        let err = parse(&json).expect_err("a contradiction must not validate");
        assert!(err.contains("asserts neither"), "unhelpful error: {err}");
    }

    /// EVAL-2's "an expected token absent from its own source", at the
    /// boundary where being wrong makes a gate that cannot fail.
    #[test]
    fn a_marker_absent_from_its_own_text_is_refused() {
        let json = CLEAN.replace(
            r#""marker": "incident bridge rota""#,
            r#""marker": "incident bridge roster""#,
        );
        let err = parse(&json).expect_err("a dangling marker must not validate");
        assert!(err.contains("does not appear"), "unhelpful error: {err}");
    }

    #[test]
    fn the_vocabularies_are_closed() {
        assert!(
            parse(&CLEAN.replace(
                r#""event_type": "message.assistant""#,
                r#""event_type": "thought""#
            ))
            .is_err(),
            "unknown kind must not validate"
        );
        assert!(
            parse(&CLEAN.replace(r#""classify": "restricted""#, r#""classify": "secret""#))
                .is_err(),
            "unknown tier must not validate"
        );
        assert!(
            parse(&CLEAN.replace(r#""variants": 100"#, r#""variants": 0"#)).is_err(),
            "a zero denominator must not validate"
        );
    }

    /// Core first and every reader asked all of it; the tail is
    /// combinatorial and deterministic, so the same corpus generates the
    /// same list in the same order every run.
    #[test]
    fn variants_are_deterministic_and_the_core_comes_from_the_material() {
        let corpus = &parse(CLEAN).expect("parses")[0];
        let first = variants(corpus, 10_000);
        let second = variants(corpus, 10_000);
        assert_eq!(first, second, "same corpus, same list, same order");

        let queries: Vec<&str> = first.iter().map(|v| v.query.as_str()).collect();
        assert!(
            queries.contains(&"incident bridge rota"),
            "the whole marker"
        );
        assert!(queries.contains(&"custodians"), "each significant word");
        assert!(queries.contains(&"CUSTODIANS"), "and its upper-cased form");
        assert!(
            !queries.contains(&"two"),
            "words too short to carry meaning"
        );
        assert!(
            first.iter().any(|v| !v.core && v.query.contains(' ')),
            "the combinatorial tail is pairs"
        );
        // The tail's vocabulary is the whole seeded text and not only the
        // marker: `ceremony` is in the first record's sentence, is not in
        // its marker, and is exactly the sort of word a reader would type.
        assert!(
            first
                .iter()
                .any(|v| !v.core && v.query.split(' ').any(|word| word == "ceremony")),
            "the tail draws on the text"
        );
        assert!(
            !queries.contains(&"ceremony"),
            "…and the core stays the marker's own words"
        );

        // The corpus's own budget is a ceiling the run cannot widen.
        assert!(variants(corpus, 10_000).len() <= corpus.variants);
    }

    /// A bounded run keeps every core variant and strides the tail
    /// (decision 13): taking the head would cluster on whichever word
    /// sorts first and leave most of the vocabulary unasked.
    #[test]
    fn a_slice_keeps_the_core_whole_and_strides_the_tail() {
        let corpus = &parse(CLEAN).expect("parses")[0];
        let all = variants(corpus, 10_000);
        let core = all.iter().filter(|v| v.core).count();
        assert!(core > 0 && core < all.len(), "the fixture has both classes");

        let cut = slice(all.clone(), core + 4);
        assert_eq!(
            cut.iter().filter(|v| v.core).count(),
            core,
            "every core variant survives a slice"
        );
        // Exactly the budget, not approximately it: a floor on
        // `security_variants` is only worth committing if the slice hits
        // the number it was asked for (ADR-0048 decision 3).
        assert_eq!(cut.len(), core + 4, "the budget is filled exactly");
        assert!(cut.len() < all.len());

        // The rounding case that a stride would have got wrong: asking
        // for a shade under half the tail must not return a quarter of it.
        let tail = all.len() - core;
        let half = slice(all.clone(), core + tail / 2 - 1);
        assert_eq!(half.len(), core + tail / 2 - 1);

        // Striding, not truncating: the last tail variant of the full list
        // is not automatically the first thing dropped.
        let tail: Vec<&str> = cut
            .iter()
            .filter(|v| !v.core)
            .map(|v| v.query.as_str())
            .collect();
        let head: Vec<&str> = all
            .iter()
            .filter(|v| !v.core)
            .take(tail.len())
            .map(|v| v.query.as_str())
            .collect();
        assert_ne!(tail, head, "a strided slice is not the tail's head");

        // A budget at or above the full list is the full list.
        assert_eq!(slice(all.clone(), all.len()).len(), all.len());
    }
}
