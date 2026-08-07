//! The judge measured before it measures (EVAL-3, ADR-0061 decision 4).
//!
//! ADR-0046 option 6 deferred the model-backed judge and gave a reason
//! that was never cost: "a judge whose own precision nobody has measured
//! should not be the thing that decides whether the product regressed."
//! This is that objection discharged — a labelled set of (question,
//! reference, candidate, human label) rows, the configured judge run over
//! it, and its agreement reported as a number beside the disagreements
//! that produced it.
//!
//! Two things this deliberately does not do. It does not gate: decision 5
//! keeps the model-judged tier off the merge path and off the nightly,
//! and a judge measurement that failed a build would be that gate through
//! a side door. And it does not average the errored calls away — a pair
//! the judge could not grade is counted and named, because an agreement
//! rate whose denominator the run chose is a rate that improves by
//! measuring less (decision 7).
//!
//! The starter set under `evals/fixtures/judge/` is **not** the corpus
//! decision 4 names. That corpus is EVAL-2's unmatched-record list plus
//! LongMemEval's own references, and neither is here yet; what is here is
//! the format they arrive in, seeded with rows that separate the two
//! judges. `note` on each set says so, and the report repeats it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::judge::{AnyJudge, Judge, JudgeInput, Tally};

/// One labelled file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabelSet {
    pub set: String,
    /// Where these labels came from and what they are good for. Read by
    /// whoever quotes the agreement rate this set produces.
    pub note: String,
    pub pairs: Vec<LabelledPair>,
}

/// One human-labelled claim of equivalence.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabelledPair {
    pub name: String,
    pub question: String,
    /// The corpus's own answer.
    pub reference: String,
    /// What was produced, and what the judge grades.
    pub candidate: String,
    /// The human verdict this pair exists to test a judge against.
    pub label: bool,
    /// Why this pair is interesting — usually which judge is expected to
    /// miss it. A disagreement with a stated reason is a known limit; one
    /// without is a finding.
    #[serde(default)]
    pub note: String,
}

impl LabelledPair {
    #[must_use]
    pub fn input(&self) -> JudgeInput<'_> {
        JudgeInput {
            question: &self.question,
            reference: &self.reference,
            candidate: &self.candidate,
        }
    }
}

/// Every `*.json` labelled set in a directory, in filename order so two
/// runs report in the same order. Validated as a whole: the guards are
/// corpus-wide.
pub fn load_sets(dir: &Path) -> Result<Vec<LabelSet>, String> {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .map_err(|err| format!("read the labelled sets {}: {err}", dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err(format!("{} holds no labelled sets", dir.display()));
    }

    let mut sets = Vec::with_capacity(paths.len());
    for path in paths {
        let raw = std::fs::read_to_string(&path)
            .map_err(|err| format!("read {}: {err}", path.display()))?;
        let set: LabelSet = serde_json::from_str(&raw)
            .map_err(|err| format!("{} is not a valid labelled set: {err}", path.display()))?;
        sets.push(set);
    }
    validate(&sets)?;
    Ok(sets)
}

/// The checks serde cannot make, run with no stack so a set that would
/// have measured nothing fails in `synveda-eval check` rather than in the
/// number someone published from it.
fn validate(sets: &[LabelSet]) -> Result<(), String> {
    let mut set_names: BTreeSet<&str> = BTreeSet::new();
    let mut pair_names: BTreeMap<&str, &str> = BTreeMap::new();

    for set in sets {
        if !set_names.insert(set.set.as_str()) {
            return Err(format!("two labelled sets are both named `{}`", set.set));
        }
        if set.note.trim().is_empty() {
            return Err(format!(
                "labelled set `{}` says nothing about where its labels came from; an agreement \
                 rate is only quotable next to the set that produced it",
                set.set
            ));
        }
        if set.pairs.is_empty() {
            return Err(format!("labelled set `{}` labels nothing", set.set));
        }

        for pair in &set.pairs {
            let at = |what: &str| format!("{}/{}: {what}", set.set, pair.name);
            if let Some(previous) = pair_names.insert(&pair.name, &set.set) {
                return Err(format!(
                    "pair name `{}` is used by both set `{previous}` and set `{}`",
                    pair.name, set.set
                ));
            }
            for (field, value) in [
                ("question", &pair.question),
                ("reference", &pair.reference),
                ("candidate", &pair.candidate),
            ] {
                if value.trim().is_empty() {
                    return Err(at(&format!(
                        "{field} is blank, and a judge cannot be measured on a pair that says \
                         nothing"
                    )));
                }
            }
        }

        // A set whose labels are all one value cannot tell a judge from a
        // constant: a judge that answered `true` to everything would score
        // 1.0 on an all-true set, and the number would be indistinguishable
        // from a judge that read the pairs.
        let accepted = set.pairs.iter().filter(|pair| pair.label).count();
        if accepted == 0 || accepted == set.pairs.len() {
            return Err(format!(
                "labelled set `{}` labels every pair {}; a judge that answered {} to everything \
                 would score perfect agreement on it, so it measures nothing",
                set.set,
                if accepted == 0 { "false" } else { "true" },
                if accepted == 0 { "false" } else { "true" },
            ));
        }
    }
    Ok(())
}

/// What one labelled set says about one judge.
#[derive(Debug, Serialize)]
pub struct Agreement {
    pub set: String,
    pub note: String,
    pub method: String,
    /// Every model (or ruleset) version that produced a verdict in this
    /// run. Plural because a long run can straddle a model rollout, and a
    /// rate quoted against "the model" would then name the wrong one.
    pub model_versions: Vec<String>,
    pub pairs: usize,
    /// Pairs that produced a verdict. The agreement denominator, and
    /// deliberately not `pairs`.
    pub graded: usize,
    pub agreed: usize,
    /// The judge said correct where the human said wrong. These are the
    /// expensive ones for a benchmark: they inflate a published score.
    pub false_accepts: Vec<String>,
    /// The judge said wrong where the human said correct. These deflate
    /// it, and they are where the lexical rubric spends its misses.
    pub false_rejects: Vec<String>,
    /// Pairs the judge could not grade at all, with the reason. Reported
    /// rather than dropped: an unreachable model and a disagreeing one
    /// mean opposite things.
    pub ungraded: Vec<String>,
}

/// Runs one labelled set through the configured judge.
///
/// A pair that fails to grade does not fail the run — the remaining pairs
/// still say something, and a single refusal or timeout ending a
/// measurement would make the number depend on the weather.
pub async fn measure(judge: &AnyJudge, set: &LabelSet, tally: &mut Tally) -> Agreement {
    let mut outcome = Agreement {
        set: set.set.clone(),
        note: set.note.clone(),
        method: judge.method().to_owned(),
        model_versions: Vec::new(),
        pairs: set.pairs.len(),
        graded: 0,
        agreed: 0,
        false_accepts: Vec::new(),
        false_rejects: Vec::new(),
        ungraded: Vec::new(),
    };

    for pair in &set.pairs {
        match tally.grade(judge, &pair.input()).await {
            Ok(verdict) => {
                outcome.graded += 1;
                if !outcome
                    .model_versions
                    .iter()
                    .any(|seen| seen == &verdict.model_version)
                {
                    outcome.model_versions.push(verdict.model_version.clone());
                }
                if verdict.correct == pair.label {
                    outcome.agreed += 1;
                } else {
                    let row = format!(
                        "{}: judged {}, labelled {} — {}{}",
                        pair.name,
                        verdict.correct,
                        pair.label,
                        verdict.rationale,
                        if pair.note.is_empty() {
                            String::new()
                        } else {
                            format!(" (expected: {})", pair.note)
                        }
                    );
                    if verdict.correct {
                        outcome.false_accepts.push(row);
                    } else {
                        outcome.false_rejects.push(row);
                    }
                }
            }
            Err(err) => outcome.ungraded.push(format!("{}: {err}", pair.name)),
        }
    }
    outcome
}

/// The judge's own axes, reduced over every set.
///
/// Reported, never gated (decision 5). They exist so a published QA score
/// can be quoted next to the agreement rate of the judge that produced
/// it — decision 4's rule that no claim may be tighter than its judge.
pub fn metrics(agreements: &[Agreement]) -> BTreeMap<String, f64> {
    let mut metrics = BTreeMap::new();
    if agreements.is_empty() {
        return metrics;
    }

    let mut graded = 0usize;
    let mut agreed = 0usize;
    let mut ungraded = 0usize;
    let mut false_accepts = 0usize;
    let mut false_rejects = 0usize;
    for agreement in agreements {
        graded += agreement.graded;
        agreed += agreement.agreed;
        ungraded += agreement.ungraded.len();
        false_accepts += agreement.false_accepts.len();
        false_rejects += agreement.false_rejects.len();
        if agreement.graded > 0 {
            metrics.insert(
                format!("judge_agreement_{}", agreement.set),
                round(agreement.agreed as f64 / agreement.graded as f64),
            );
        }
    }

    if graded > 0 {
        metrics.insert(
            "judge_agreement".to_owned(),
            round(agreed as f64 / graded as f64),
        );
        metrics.insert(
            "judge_false_accept_rate".to_owned(),
            round(false_accepts as f64 / graded as f64),
        );
        metrics.insert(
            "judge_false_reject_rate".to_owned(),
            round(false_rejects as f64 / graded as f64),
        );
    }
    // Always present, including at zero: the count is what makes the
    // agreement denominator readable, and an absent axis would let a run
    // that graded half its pairs read like one that graded all of them.
    metrics.insert("judge_ungraded".to_owned(), ungraded as f64);
    metrics
}

/// One judged run, as a file someone can keep next to the score it
/// qualifies. Decision 4's rule needs both halves in one place: a
/// published QA accuracy and the agreement of the judge that produced it.
#[derive(Debug, Serialize)]
pub struct JudgeReport {
    pub method: String,
    pub started_at: String,
    pub sets: Vec<Agreement>,
    pub tally: Tally,
    pub metrics: BTreeMap<String, f64>,
}

/// The stderr summary. Leads with the disagreements rather than the rate:
/// the rate is one number and the rows are what anyone does anything
/// with — reversal trigger (a) turns them into the next feature's input.
#[must_use]
pub fn summarise(report: &JudgeReport) -> String {
    let mut out = format!(
        "\njudge: {} against {} set(s)\n",
        report.method,
        report.sets.len()
    );
    for agreement in &report.sets {
        out.push_str(&format!(
            "  {} — {}/{} agreed of {} pair(s), models: {}\n",
            agreement.set,
            agreement.agreed,
            agreement.graded,
            agreement.pairs,
            if agreement.model_versions.is_empty() {
                "none".to_owned()
            } else {
                agreement.model_versions.join(", ")
            }
        ));
        for row in &agreement.false_accepts {
            out.push_str(&format!("      accepted a wrong answer: {row}\n"));
        }
        for row in &agreement.false_rejects {
            out.push_str(&format!("      rejected a right answer: {row}\n"));
        }
        for row in &agreement.ungraded {
            out.push_str(&format!("      ungraded: {row}\n"));
        }
    }
    out.push_str("\n  axis                       measured\n");
    for (metric, value) in &report.metrics {
        out.push_str(&format!("  {metric:<26} {value:>8.3}\n"));
    }
    out.push_str(&format!(
        "\n  {} call(s) in {:.3}s{}\n",
        report.tally.total(),
        report.tally.seconds,
        report
            .tally
            .calls
            .iter()
            .map(|(outcome, count)| format!(", {count} {outcome}"))
            .collect::<String>()
    ));
    out.push_str(
        "  This measures the judge, not the product, and gates nothing (ADR-0061 decision 5).\n",
    );
    out
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::judge::LexicalJudge;

    const CLEAN: &str = r#"{
        "set": "starter",
        "note": "hand-written, not the corpus decision 4 names",
        "pairs": [
            {"name": "p-same", "question": "when did the lease end",
             "reference": "March", "candidate": "Your lease ended in March.",
             "label": true},
            {"name": "p-wrong", "question": "when did the lease end",
             "reference": "March", "candidate": "I have no record of that.",
             "label": false}
        ]
    }"#;

    fn parse(json: &str) -> Result<Vec<LabelSet>, String> {
        let set: LabelSet = serde_json::from_str(json).map_err(|err| err.to_string())?;
        let sets = vec![set];
        validate(&sets)?;
        Ok(sets)
    }

    #[test]
    fn a_set_round_trips_with_its_defaults() {
        let sets = parse(CLEAN).expect("parses");
        assert_eq!(sets[0].pairs.len(), 2);
        assert_eq!(sets[0].pairs[0].note, "");
    }

    #[test]
    fn an_unknown_field_is_refused_rather_than_ignored() {
        let json = CLEAN.replace(r#""label": true"#, r#""lable": true"#);
        let err = parse(&json).expect_err("unknown field must not parse");
        assert!(err.contains("lable"), "unhelpful error: {err}");
    }

    /// The guard that keeps an agreement rate meaningful.
    #[test]
    fn a_set_labelled_all_one_way_is_refused() {
        let json = CLEAN.replace(r#""label": false"#, r#""label": true"#);
        let err = parse(&json).expect_err("a constant set must not validate");
        assert!(err.contains("measures nothing"), "unhelpful error: {err}");
    }

    #[test]
    fn a_blank_field_is_refused() {
        let json = CLEAN.replace(
            r#""reference": "March", "candidate": "Your"#,
            r#""reference": "  ", "candidate": "Your"#,
        );
        let err = parse(&json).expect_err("a blank reference must not validate");
        assert!(err.contains("says nothing"), "unhelpful error: {err}");
    }

    #[tokio::test]
    async fn agreement_counts_the_two_disagreements_apart_and_never_averages_an_error_in() {
        let set: LabelSet = serde_json::from_str(
            r#"{
                "set": "s", "note": "n",
                "pairs": [
                    {"name": "agree-yes", "question": "q", "reference": "March",
                     "candidate": "It was March.", "label": true},
                    {"name": "agree-no", "question": "q", "reference": "March",
                     "candidate": "No idea.", "label": false},
                    {"name": "false-reject", "question": "q", "reference": "three weeks",
                     "candidate": "About 21 days.", "label": true,
                     "note": "the rubric cannot see the paraphrase"},
                    {"name": "false-accept", "question": "q", "reference": "March",
                     "candidate": "Not March, and not any month you named.", "label": false}
                ]
            }"#,
        )
        .expect("parses");

        let judge = AnyJudge::Lexical(LexicalJudge::new());
        let mut tally = Tally::default();
        let agreement = measure(&judge, &set, &mut tally).await;

        assert_eq!(agreement.pairs, 4);
        assert_eq!(agreement.graded, 4);
        assert_eq!(agreement.agreed, 2);
        assert_eq!(agreement.false_rejects.len(), 1);
        assert!(
            agreement.false_rejects[0].contains("cannot see the paraphrase"),
            "the pair's note explains a known miss: {:?}",
            agreement.false_rejects
        );
        // Bare containment cannot see a negation, which is the rubric's
        // other blind spot and the one that inflates a score.
        assert_eq!(agreement.false_accepts.len(), 1);
        assert_eq!(agreement.model_versions, vec!["rubric@1".to_owned()]);
        assert_eq!(tally.total(), 4);

        let metrics = metrics(&[agreement]);
        assert_eq!(metrics.get("judge_agreement"), Some(&0.5));
        assert_eq!(metrics.get("judge_agreement_s"), Some(&0.5));
        assert_eq!(metrics.get("judge_false_accept_rate"), Some(&0.25));
        assert_eq!(metrics.get("judge_false_reject_rate"), Some(&0.25));
        assert_eq!(metrics.get("judge_ungraded"), Some(&0.0));
    }

    #[tokio::test]
    async fn an_ungraded_pair_leaves_the_denominator_and_is_reported() {
        // An empty reference is the one thing the rubric refuses to grade.
        // It stands in here for the model judge's refusals and timeouts:
        // the run continues, the count is published, and the agreement
        // rate is over what was actually graded.
        let set = LabelSet {
            set: "s".to_owned(),
            note: "n".to_owned(),
            pairs: vec![
                LabelledPair {
                    name: "gradeable".to_owned(),
                    question: "q".to_owned(),
                    reference: "March".to_owned(),
                    candidate: "It was March.".to_owned(),
                    label: true,
                    note: String::new(),
                },
                LabelledPair {
                    name: "ungradeable".to_owned(),
                    question: "q".to_owned(),
                    reference: "...".to_owned(),
                    candidate: "anything".to_owned(),
                    label: true,
                    note: String::new(),
                },
            ],
        };
        let mut tally = Tally::default();
        let agreement = measure(&AnyJudge::Lexical(LexicalJudge::new()), &set, &mut tally).await;

        assert_eq!(agreement.pairs, 2);
        assert_eq!(agreement.graded, 1);
        assert_eq!(agreement.agreed, 1);
        assert_eq!(agreement.ungraded.len(), 1);
        let metrics = metrics(&[agreement]);
        // 1.0 over what was graded, with the dropped pair visible beside
        // it rather than folded into the rate.
        assert_eq!(metrics.get("judge_agreement"), Some(&1.0));
        assert_eq!(metrics.get("judge_ungraded"), Some(&1.0));
    }
}
