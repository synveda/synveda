//! The reader measured against its own probes (EVAL-3, ADR-0061
//! decision 6), the way `agreement` measures the judge against its
//! labelled set.
//!
//! **What this is not.** The blocks here come from a file, not from
//! `/v1/inject`. That makes every number below a property of the reader
//! and the judge and nothing else — it is *not* a QA accuracy, not a
//! benchmark score, and not a measurement of Synveda, because Synveda did
//! not compose the block. The axes are prefixed `probe_` rather than
//! `qa_` so no reduction, baseline or report can quietly mistake one for
//! the other. When the LongMemEval corpus lands, the same reader and the
//! same judge run over blocks the product actually served, and *those*
//! numbers are decision 5's model-judged tier.
//!
//! **Why measure it separately at all.** For ADR-0046 option 6's reason,
//! one seam over: a reader whose behaviour nobody has characterised
//! should not be the thing standing between the block and the score. A
//! reader that invents when the block is thin, or abstains when it is
//! merely terse, moves a published figure in a direction that looks
//! exactly like a retrieval regression. Probes are how that is told apart
//! before it matters.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::judge::{self, AnyJudge, Judge, JudgeInput};
use crate::reader::{self, AnyReader, Reader, ReaderInput};

/// A word long enough to carry meaning for the support guard.
const MIN_CONTENT_WORD: usize = 4;

/// How a correct answer relates to the block's text (decision 6, and
/// EVAL-4 decision 5's `needs` pattern one seam over).
///
/// `quoted` means the answer is in the block to be found, and the support
/// guard below holds the author to that. `derived` means it has to be
/// worked out — two dates into a duration, a later entry over an earlier
/// one — so it legitimately shares no word with the block, and the guard
/// must not apply. Without the distinction the guard would forbid exactly
/// the probes that tell a reader from a selector.
pub const ANSWER_FORMS: [&str; 2] = ["quoted", "derived"];

/// One probe file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeSet {
    pub set: String,
    /// Where these blocks came from and what they are good for.
    pub note: String,
    pub probes: Vec<Probe>,
}

/// One question, one block, and what a correct reading of it says.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    pub name: String,
    pub question: String,
    /// The block, in the renderer's own vocabulary — `## path (kind)`
    /// headings, `- [class] text` entries, and the legend and watermark
    /// that follow. Written the way the product writes them, because a
    /// reader tested against a tidied block is a reader untested against
    /// the furniture it will actually be handed.
    pub block: String,
    /// What a correct answer says. For an abstention probe, what a
    /// correct decline says.
    pub reference: String,
    /// `quoted` (the default) or `derived` — see [`ANSWER_FORMS`].
    #[serde(default = "default_answer_form")]
    pub answer_form: String,
    /// Whether the block deliberately fails to support an answer.
    #[serde(default)]
    pub expect_abstention: bool,
    /// Why this probe is interesting, or which reader is expected to miss
    /// it. A miss with a stated reason is a known limit; one without is a
    /// finding.
    #[serde(default)]
    pub note: String,
}

fn default_answer_form() -> String {
    "quoted".to_owned()
}

/// Every `*.json` probe set in a directory, in filename order.
pub fn load_sets(dir: &Path) -> Result<Vec<ProbeSet>, String> {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .map_err(|err| format!("read the reader probes {}: {err}", dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err(format!("{} holds no reader probes", dir.display()));
    }

    let mut sets = Vec::with_capacity(paths.len());
    for path in paths {
        let raw = std::fs::read_to_string(&path)
            .map_err(|err| format!("read {}: {err}", path.display()))?;
        let set: ProbeSet = serde_json::from_str(&raw)
            .map_err(|err| format!("{} is not a valid probe set: {err}", path.display()))?;
        sets.push(set);
    }
    validate(&sets)?;
    Ok(sets)
}

/// The checks serde cannot make, run with no stack.
fn validate(sets: &[ProbeSet]) -> Result<(), String> {
    let mut set_names: BTreeSet<&str> = BTreeSet::new();
    let mut probe_names: BTreeMap<&str, &str> = BTreeMap::new();

    for set in sets {
        if !set_names.insert(set.set.as_str()) {
            return Err(format!("two probe sets are both named `{}`", set.set));
        }
        if set.note.trim().is_empty() {
            return Err(format!(
                "probe set `{}` says nothing about where its blocks came from",
                set.set
            ));
        }
        if set.probes.is_empty() {
            return Err(format!("probe set `{}` probes nothing", set.set));
        }

        for probe in &set.probes {
            let at = |what: &str| format!("{}/{}: {what}", set.set, probe.name);
            if let Some(previous) = probe_names.insert(&probe.name, &set.set) {
                return Err(format!(
                    "probe name `{}` is used by both set `{previous}` and set `{}`",
                    probe.name, set.set
                ));
            }
            for (field, value) in [
                ("question", &probe.question),
                ("block", &probe.block),
                ("reference", &probe.reference),
            ] {
                if value.trim().is_empty() {
                    return Err(at(&format!("{field} is blank")));
                }
            }

            if !ANSWER_FORMS.contains(&probe.answer_form.as_str()) {
                return Err(at(&format!(
                    "answer_form `{}` is not one of {ANSWER_FORMS:?}",
                    probe.answer_form
                )));
            }

            // EVAL-4 rules 4 and 5, one seam over, and split the same way
            // by a declaration the author makes.
            //
            // A `quoted` probe claims its answer is in the block; if the
            // two share no content word, nothing could have produced it
            // and the probe would read in the report as a reader that
            // cannot read. A `derived` probe claims the opposite — the
            // answer has to be worked out — so it is exempt, and pays for
            // the exemption with a `note` saying what the derivation is.
            // An unexplained exemption is how the guard would be escaped
            // by everyone in six months.
            //
            // There is no mirror-image guard for abstention probes:
            // their reference is a decline, so it shares nothing with any
            // block by construction and a containment check would pass
            // for all of them without looking. Whether an abstention
            // block truly supports nothing is the author's claim, and
            // `note` is where they defend it.
            let derived = probe.answer_form == "derived";
            if derived && probe.note.trim().is_empty() {
                return Err(at(
                    "declared `derived`, which exempts it from the support guard, but says \
                     nothing about how the answer follows from the block — an exemption nobody \
                     has to justify is one every probe will claim",
                ));
            }
            if !probe.expect_abstention && !derived {
                let shared = content_words(&probe.block)
                    .intersection(&content_words(&probe.reference))
                    .count();
                if shared == 0 {
                    return Err(at(
                        "declared `quoted` but the block shares no content word with the \
                         reference answer, so no reader could produce that answer from it — this \
                         probe would fail for a corpus reason and read as a reader that cannot \
                         read. Mark it `derived` with a note, mark it `expect_abstention`, or \
                         give the block the material.",
                    ));
                }
            }
        }

        // A set of nothing but abstention probes measures a reader that
        // says "I don't know" to everything as perfect.
        if set.probes.iter().all(|probe| probe.expect_abstention) {
            return Err(format!(
                "probe set `{}` expects an abstention from every probe; a reader that abstained \
                 always would score perfectly on it, so it measures no reading at all",
                set.set
            ));
        }
    }
    Ok(())
}

/// Content words of a string, lowercased, short words dropped.
fn content_words(text: &str) -> BTreeSet<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|word| word.len() >= MIN_CONTENT_WORD)
        .collect()
}

/// What one probe set says about one reader (graded by one judge).
#[derive(Debug, Serialize)]
pub struct Reading {
    pub set: String,
    pub note: String,
    pub reader: String,
    pub judge: String,
    /// Every reader model (or ruleset) version that produced an answer.
    pub reader_versions: Vec<String>,
    /// Every judge model (or ruleset) version that produced a verdict.
    pub judge_versions: Vec<String>,
    pub probes: usize,
    /// Probes that expected an answer and got one the judge graded.
    pub answerable_graded: usize,
    /// Of those, the ones the judge called correct.
    pub answered_correctly: usize,
    /// Probes whose block supports nothing.
    pub abstention_probes: usize,
    /// Of those, the ones the reader declined rather than invented.
    pub abstained: usize,
    /// A reader that answered where the block supports nothing. The
    /// expensive failure: seed §4.4's invented context, and the one a QA
    /// accuracy alone would hide.
    pub inventions: Vec<String>,
    /// A reader that declined where the block does support an answer.
    pub over_abstentions: Vec<String>,
    /// Answers the judge called wrong, with its rationale.
    pub misreadings: Vec<String>,
    /// Probes the reader could not answer at all, with the reason.
    pub unread: Vec<String>,
    /// Answers the judge could not grade, with the reason.
    pub ungraded: Vec<String>,
    /// Every answer the reader produced, whatever the outcome.
    ///
    /// The lists above are the finding; this is the evidence, and a
    /// benchmark artefact that kept only the former would be a score with
    /// nothing behind it. It also closes the hole a clean run opens: at
    /// 7/7 every list above is empty, so without this the report of a
    /// perfect run records nothing the reader actually said, and a reader
    /// right for the wrong reasons is indistinguishable from one that
    /// read the block.
    pub transcript: Vec<Read>,
}

/// One probe's answer and, when a judge was asked, its verdict.
#[derive(Debug, Serialize)]
pub struct Read {
    pub probe: String,
    pub answer: String,
    pub abstained: bool,
    /// Absent for an abstention probe: those are decided by what the
    /// reader did, without a judge (EVAL-1 decision 4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correct: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

/// Runs one probe set: reader first, then the judge over what it said.
///
/// A probe that fails at either stage does not fail the run — the rest
/// still say something, and one refusal ending a measurement would make
/// the number depend on the weather.
pub async fn measure(
    reader: &AnyReader,
    judge: &AnyJudge,
    set: &ProbeSet,
    reader_tally: &mut reader::Tally,
    judge_tally: &mut judge::Tally,
) -> Reading {
    let mut outcome = Reading {
        set: set.set.clone(),
        note: set.note.clone(),
        reader: reader.method().to_owned(),
        judge: judge.method().to_owned(),
        reader_versions: Vec::new(),
        judge_versions: Vec::new(),
        probes: set.probes.len(),
        answerable_graded: 0,
        answered_correctly: 0,
        abstention_probes: set
            .probes
            .iter()
            .filter(|probe| probe.expect_abstention)
            .count(),
        abstained: 0,
        inventions: Vec::new(),
        over_abstentions: Vec::new(),
        misreadings: Vec::new(),
        unread: Vec::new(),
        ungraded: Vec::new(),
        transcript: Vec::new(),
    };

    for probe in &set.probes {
        let answer = match reader_tally
            .read(
                reader,
                &ReaderInput {
                    question: &probe.question,
                    block: &probe.block,
                },
            )
            .await
        {
            Ok(answer) => answer,
            Err(err) => {
                outcome.unread.push(format!("{}: {err}", probe.name));
                continue;
            }
        };
        remember(&mut outcome.reader_versions, &answer.model_version);

        // The abstention axis first, because it is decided by what the
        // reader *did* rather than by what a judge thinks of it — EVAL-1
        // decision 4's point, and the reason `abstained` is a field
        // rather than a phrase to be recognised in prose.
        if probe.expect_abstention {
            if answer.abstained {
                outcome.abstained += 1;
            } else {
                outcome.inventions.push(row(probe, &answer.text));
            }
            outcome.transcript.push(Read {
                probe: probe.name.clone(),
                answer: answer.text,
                abstained: answer.abstained,
                correct: None,
                rationale: None,
            });
            continue;
        }
        if answer.abstained {
            outcome.over_abstentions.push(row(probe, &answer.text));
            outcome.transcript.push(Read {
                probe: probe.name.clone(),
                answer: answer.text,
                abstained: true,
                correct: None,
                rationale: None,
            });
            continue;
        }

        match judge_tally
            .grade(
                judge,
                &JudgeInput {
                    question: &probe.question,
                    reference: &probe.reference,
                    candidate: &answer.text,
                },
            )
            .await
        {
            Ok(verdict) => {
                remember(&mut outcome.judge_versions, &verdict.model_version);
                outcome.answerable_graded += 1;
                outcome.transcript.push(Read {
                    probe: probe.name.clone(),
                    answer: answer.text.clone(),
                    abstained: false,
                    correct: Some(verdict.correct),
                    rationale: Some(verdict.rationale.clone()),
                });
                if verdict.correct {
                    outcome.answered_correctly += 1;
                } else {
                    outcome.misreadings.push(format!(
                        "{}: read {:?} — {}{}",
                        probe.name,
                        truncate(&answer.text),
                        verdict.rationale,
                        suffix(probe)
                    ));
                }
            }
            Err(err) => {
                outcome.ungraded.push(format!("{}: {err}", probe.name));
                outcome.transcript.push(Read {
                    probe: probe.name.clone(),
                    answer: answer.text,
                    abstained: false,
                    correct: None,
                    rationale: None,
                });
            }
        }
    }
    outcome
}

fn remember(seen: &mut Vec<String>, version: &str) {
    if !seen.iter().any(|known| known == version) {
        seen.push(version.to_owned());
    }
}

fn row(probe: &Probe, text: &str) -> String {
    format!("{}: read {:?}{}", probe.name, truncate(text), suffix(probe))
}

fn suffix(probe: &Probe) -> String {
    if probe.note.is_empty() {
        String::new()
    } else {
        format!(" (expected: {})", probe.note)
    }
}

fn truncate(text: &str) -> String {
    text.chars().take(96).collect()
}

/// The reader's axes, reduced over every set.
///
/// `probe_`, never `qa_`: these blocks came from a file. The prefix is
/// the guard against a future reduction folding them into the product's
/// numbers, where they would read as a QA accuracy Synveda earned.
pub fn metrics(readings: &[Reading]) -> BTreeMap<String, f64> {
    let mut metrics = BTreeMap::new();
    if readings.is_empty() {
        return metrics;
    }

    let mut graded = 0usize;
    let mut correct = 0usize;
    let mut abstention_probes = 0usize;
    let mut abstained = 0usize;
    let mut inventions = 0usize;
    let mut over_abstentions = 0usize;
    let mut unread = 0usize;
    let mut ungraded = 0usize;
    for reading in readings {
        graded += reading.answerable_graded;
        correct += reading.answered_correctly;
        abstention_probes += reading.abstention_probes;
        abstained += reading.abstained;
        inventions += reading.inventions.len();
        over_abstentions += reading.over_abstentions.len();
        unread += reading.unread.len();
        ungraded += reading.ungraded.len();
        if reading.answerable_graded > 0 {
            metrics.insert(
                format!("probe_answer_rate_{}", reading.set),
                round(reading.answered_correctly as f64 / reading.answerable_graded as f64),
            );
        }
    }

    if graded > 0 {
        metrics.insert(
            "probe_answer_rate".to_owned(),
            round(correct as f64 / graded as f64),
        );
    }
    // Absent rather than zero when nothing asked: a 0.0 here would read
    // as "the reader invented every time" when what happened is that no
    // probe tested it — the `hallucination_rate` convention.
    if abstention_probes > 0 {
        metrics.insert(
            "probe_abstention_rate".to_owned(),
            round(abstained as f64 / abstention_probes as f64),
        );
        metrics.insert(
            "probe_invention_rate".to_owned(),
            round(inventions as f64 / abstention_probes as f64),
        );
    }
    // Always present, including at zero — an over-abstention is a reader
    // silently declining work the block supported, and an absent axis
    // would let a run that did it read like one that did not.
    metrics.insert("probe_over_abstentions".to_owned(), over_abstentions as f64);
    metrics.insert("probe_unread".to_owned(), unread as f64);
    metrics.insert("probe_ungraded".to_owned(), ungraded as f64);
    metrics
}

/// One probe run, as a file someone can keep.
#[derive(Debug, Serialize)]
pub struct ReadingReport {
    pub reader: String,
    pub judge: String,
    /// The bias declaration ADR-0061 option 7 owes when both seams are
    /// model-backed. Absent when there is none to make.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub independence: Option<String>,
    pub started_at: String,
    pub sets: Vec<Reading>,
    pub reader_tally: reader::Tally,
    pub judge_tally: judge::Tally,
    pub metrics: BTreeMap<String, f64>,
}

/// The stderr summary. Leads with what the reader got wrong, and ends by
/// saying what the numbers are not.
#[must_use]
pub fn summarise(report: &ReadingReport) -> String {
    let mut out = format!(
        "\nread: {} answering, {} grading, {} set(s)\n",
        report.reader,
        report.judge,
        report.sets.len()
    );
    if let Some(note) = &report.independence {
        out.push_str(&format!("  note: {note}\n"));
    }
    for reading in &report.sets {
        out.push_str(&format!(
            "  {} — {}/{} answered correctly, {}/{} abstained where they should, of {} probe(s)\n",
            reading.set,
            reading.answered_correctly,
            reading.answerable_graded,
            reading.abstained,
            reading.abstention_probes,
            reading.probes
        ));
        for row in &reading.inventions {
            out.push_str(&format!(
                "      invented where the block held nothing: {row}\n"
            ));
        }
        for row in &reading.over_abstentions {
            out.push_str(&format!("      declined a supported question: {row}\n"));
        }
        for row in &reading.misreadings {
            out.push_str(&format!("      misread: {row}\n"));
        }
        for row in &reading.unread {
            out.push_str(&format!("      unread: {row}\n"));
        }
        for row in &reading.ungraded {
            out.push_str(&format!("      ungraded: {row}\n"));
        }
    }
    out.push_str("\n  axis                       measured\n");
    for (metric, value) in &report.metrics {
        out.push_str(&format!("  {metric:<26} {value:>8.3}\n"));
    }
    out.push_str(&format!(
        "\n  reader: {} call(s) in {:.3}s | judge: {} call(s) in {:.3}s\n",
        report.reader_tally.total(),
        report.reader_tally.seconds,
        report.judge_tally.total(),
        report.judge_tally.seconds
    ));
    out.push_str(&crate::agreement::tokens_line(
        "  reader",
        &report.reader_tally.tokens,
    ));
    out.push_str(&crate::agreement::tokens_line(
        "  judge",
        &report.judge_tally.tokens,
    ));
    out.push_str(
        "  These blocks came from a file, not from /v1/inject: this measures the reader and the\n  \
         judge, not Synveda, and it is not a QA accuracy (ADR-0061 decision 6).\n",
    );
    out
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use crate::judge::LexicalJudge;
    use crate::reader::ExtractiveReader;

    use super::*;

    // `r###`, not `r#`: a block's own scope heading puts a literal `"##`
    // inside the string, which is the terminator for both shorter forms.
    const CLEAN: &str = r###"{
        "set": "starter",
        "note": "hand-written blocks, not blocks the product served",
        "probes": [
            {"name": "p-answer", "question": "how long did the renovation take",
             "block": "## alice (user)\n- [episode] The renovation ran three weeks.\n",
             "reference": "three weeks"},
            {"name": "p-abstain", "question": "who supplies the beans",
             "block": "## alice (user)\n- [episode] The renovation ran three weeks.\n",
             "reference": "the block holds no answer",
             "expect_abstention": true}
        ]
    }"###;

    fn parse(json: &str) -> Result<Vec<ProbeSet>, String> {
        let set: ProbeSet = serde_json::from_str(json).map_err(|err| err.to_string())?;
        let sets = vec![set];
        validate(&sets)?;
        Ok(sets)
    }

    #[test]
    fn a_set_round_trips_with_its_defaults() {
        let sets = parse(CLEAN).expect("parses");
        assert_eq!(sets[0].probes.len(), 2);
        assert!(!sets[0].probes[0].expect_abstention);
        assert!(sets[0].probes[1].expect_abstention);
    }

    #[test]
    fn an_unknown_field_is_refused_rather_than_ignored() {
        let json = CLEAN.replace(r#""expect_abstention""#, r#""expect_abstension""#);
        let err = parse(&json).expect_err("unknown field must not parse");
        assert!(err.contains("expect_abstension"), "unhelpful error: {err}");
    }

    /// EVAL-4's `needs` discipline, one seam over: a probe nothing could
    /// answer would read in the report as a reader that cannot read.
    #[test]
    fn a_quoted_probe_whose_block_cannot_support_its_reference_is_refused() {
        let json = CLEAN.replace(
            r#""reference": "three weeks""#,
            r#""reference": "nine furlongs""#,
        );
        let err = parse(&json).expect_err("an unsupported probe must not validate");
        assert!(
            err.contains("no reader could produce"),
            "unhelpful error: {err}"
        );
    }

    /// The exemption that makes synthesis probes writable — and the price
    /// of it. Without the note requirement, `derived` is a word every
    /// probe would learn to carry.
    #[test]
    fn a_derived_probe_is_exempt_from_the_support_guard_but_must_say_why() {
        let unexplained = CLEAN.replace(
            r#""reference": "three weeks""#,
            r#""reference": "nine furlongs", "answer_form": "derived""#,
        );
        let err = parse(&unexplained).expect_err("an unexplained exemption must not validate");
        assert!(
            err.contains("nobody has to justify"),
            "unhelpful error: {err}"
        );

        let explained = unexplained.replace(
            r#""answer_form": "derived""#,
            r#""answer_form": "derived", "note": "eight furlongs to the mile, and the block gives miles""#,
        );
        assert!(
            parse(&explained).is_ok(),
            "an explained derivation parses: {:?}",
            parse(&explained)
        );
    }

    #[test]
    fn the_answer_form_vocabulary_is_closed() {
        let json = CLEAN.replace(
            r#""reference": "three weeks""#,
            r#""reference": "three weeks", "answer_form": "vibes""#,
        );
        let err = parse(&json).expect_err("unknown answer_form must not validate");
        assert!(err.contains("quoted"), "unhelpful error: {err}");
    }

    #[test]
    fn a_set_of_nothing_but_abstentions_is_refused() {
        let json = CLEAN.replace(
            r#""reference": "three weeks""#,
            r#""reference": "the block holds no answer", "expect_abstention": true"#,
        );
        let err = parse(&json).expect_err("a set of only abstentions measures nothing");
        assert!(err.contains("no reading at all"), "unhelpful error: {err}");
    }

    #[tokio::test]
    async fn the_two_failure_modes_are_counted_apart() {
        let set: ProbeSet = serde_json::from_str(CLEAN).expect("parses");
        let reader = AnyReader::Extractive(ExtractiveReader::new());
        let judge = AnyJudge::Lexical(LexicalJudge::new());
        let mut reader_tally = reader::Tally::default();
        let mut judge_tally = judge::Tally::default();

        let reading = measure(&reader, &judge, &set, &mut reader_tally, &mut judge_tally).await;

        assert_eq!(reading.probes, 2);
        assert_eq!(reading.answerable_graded, 1);
        assert_eq!(reading.answered_correctly, 1);
        assert_eq!(reading.abstention_probes, 1);
        assert_eq!(reading.abstained, 1);
        assert!(reading.inventions.is_empty());
        assert_eq!(reading.reader_versions, vec!["selection@1".to_owned()]);
        assert_eq!(reading.judge_versions, vec!["rubric@1".to_owned()]);
        // The reader was called twice, the judge only once: an abstention
        // probe is decided without a judge, which is what makes the
        // abstention axis independent of one.
        assert_eq!(reader_tally.total(), 2);
        assert_eq!(judge_tally.total(), 1);
        assert_eq!(reader_tally.calls.get("abstained"), Some(&1));
        assert_eq!(reader_tally.calls.get("answered"), Some(&1));

        let metrics = metrics(&[reading]);
        assert_eq!(metrics.get("probe_answer_rate"), Some(&1.0));
        assert_eq!(metrics.get("probe_abstention_rate"), Some(&1.0));
        assert_eq!(metrics.get("probe_invention_rate"), Some(&0.0));
        assert_eq!(metrics.get("probe_over_abstentions"), Some(&0.0));
        // Nothing is named `qa_*`: these blocks came from a file.
        assert!(metrics.keys().all(|axis| axis.starts_with("probe_")));
    }

    #[tokio::test]
    async fn an_invention_is_named_and_never_folded_into_the_answer_rate() {
        // A block that mentions beans but answers nothing about supply.
        // The extractive reader matches on the word and returns the line,
        // which is exactly the invention this axis exists to catch.
        let set: ProbeSet = serde_json::from_str(
            r#"{
                "set": "s", "note": "n",
                "probes": [
                    {"name": "answerable", "question": "how long did the renovation take",
                     "block": "- [episode] The renovation ran three weeks.",
                     "reference": "three weeks"},
                    {"name": "invents", "question": "which vendor supplies the beans",
                     "block": "- [fact] The beans are stored in the third cupboard.",
                     "reference": "the block holds no answer",
                     "expect_abstention": true,
                     "note": "selection matches on `beans` and returns an unrelated line"}
                ]
            }"#,
        )
        .expect("parses");

        let mut reader_tally = reader::Tally::default();
        let mut judge_tally = judge::Tally::default();
        let reading = measure(
            &AnyReader::Extractive(ExtractiveReader::new()),
            &AnyJudge::Lexical(LexicalJudge::new()),
            &set,
            &mut reader_tally,
            &mut judge_tally,
        )
        .await;

        assert_eq!(reading.inventions.len(), 1);
        assert!(
            reading.inventions[0].contains("selection matches on"),
            "{:?}",
            reading.inventions
        );
        assert_eq!(reading.abstained, 0);
        // The answer rate is untouched by it — an invention is its own
        // failure, not a wrong answer, and averaging the two would let a
        // reader trade honesty for accuracy.
        assert_eq!(reading.answered_correctly, 1);
        assert_eq!(reading.answerable_graded, 1);
        let metrics = metrics(&[reading]);
        assert_eq!(metrics.get("probe_answer_rate"), Some(&1.0));
        assert_eq!(metrics.get("probe_invention_rate"), Some(&1.0));
        assert_eq!(metrics.get("probe_abstention_rate"), Some(&0.0));
    }
}
