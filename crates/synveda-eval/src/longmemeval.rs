//! LongMemEval, read in the format its authors publish (EVAL-3, ADR-0061
//! decision 2).
//!
//! The fourth corpus kind, and the first one this repo did not write. That
//! difference decides almost everything here.
//!
//! **It is read, never converted.** ADR-0047 trigger (f) asked that a
//! benchmark corpus arrive in EVAL-4's Q&A format or that the reason it
//! cannot be recorded; decision 2 records it, and the consequence is this
//! module. A conversion step would be a place where a corpus quietly
//! became ours — and decision 2 names editing an external corpus as "the
//! one thing that would invalidate the score". So the field names below
//! are LongMemEval's, spelled exactly as it spells them, and nothing in
//! this harness rewrites them.
//!
//! **Its guards are integrity checks, not authoring discipline.** EVAL-4's
//! `needs` rules are instructions to whoever writes the next question, and
//! an external corpus cannot be expected to satisfy them. What *can* be
//! demanded is internal consistency: that the three haystack arrays line
//! up, that every evidence session named is a session that exists, that
//! the abstention marker and the evidence list agree. When one of those
//! fails the answer is never to edit the corpus — it is that this corpus
//! cannot be scored until upstream is asked about it.
//!
//! **It is not vendored, and the score names its digest instead.** A
//! 500-instance haystack is hundreds of megabytes; `longmemeval_s` alone
//! is far past what belongs in a git history. So the licence and the
//! attribution live in `evals/fixtures/longmemeval/` and the data is
//! fetched, which leaves a published number with no bytes to point at.
//! The digest is that gap closed: every run records the BLAKE3 of the file
//! it read, and decision 11's published row carries it. A benchmark score
//! whose corpus cannot be identified is a benchmark score nobody can
//! reproduce, including us.
//!
//! One caveat, recorded here because it is the kind that is worse when
//! discovered later: the schema below is transcribed from LongMemEval's
//! published format, and nothing has parsed the real file yet — the same
//! standing AUTH-4's vendor corpus has in CLAUDE.md. If upstream spells a
//! field differently the loader refuses the corpus loudly and one line
//! fixes it, which is the failure this shape was chosen to have.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The six question types LongMemEval publishes. Closed, because a type
/// this harness did not know would drop out of the per-category reduction
/// while still counting in the total — a score that silently stopped
/// covering one of the abilities it claims to measure.
pub const QUESTION_TYPES: [&str; 6] = [
    "knowledge-update",
    "multi-session",
    "single-session-assistant",
    "single-session-preference",
    "single-session-user",
    "temporal-reasoning",
];

/// The two speakers in a haystack session. Closed for a duller reason
/// than the types: every turn becomes an observed event, and a role this
/// harness cannot render is material that would be seeded as something
/// else.
pub const ROLES: [&str; 2] = ["assistant", "user"];

/// LongMemEval's own marker for an abstention instance — a question whose
/// answer is nowhere in the haystack, and whose correct behaviour is to
/// say so. Their eval splits on this suffix and excludes those instances
/// from retrieval scoring; decision 5 keeps both halves of that, and the
/// guard below asserts the suffix and the evidence list agree rather than
/// trusting either one alone.
pub const ABSTENTION_SUFFIX: &str = "_abs";

/// Where the fetched corpus is expected to be. The directory is committed
/// — licence, attribution and the instructions for fetching — and the data
/// file inside it is not.
pub const DEFAULT_PATH: &str = "evals/fixtures/longmemeval/longmemeval_s.json";

/// How many instances a routine run measures (decision 7).
///
/// Provisional, and deliberately small: one instance is ~40 sessions of
/// chat seeded one event at a time through `/v1` with the whole extraction
/// pipeline behind it, and nobody has measured that throughput yet. The
/// number to raise it to is the one the first live run reports, not the
/// one that sounds thorough — and raising it is a one-line change whose
/// diff should carry the measurement that justified it.
pub const DEFAULT_INSTANCES: usize = 10;

/// One benchmark instance, in LongMemEval's published field names.
///
/// `deny_unknown_fields` on somebody else's corpus is a deliberate call
/// and the opposite of the forgiving one. The usual argument for
/// tolerance — upstream may add a field and we should not break — is
/// exactly backwards for a corpus whose numbers get published: a field we
/// ignored is a field the score was computed without, and finding that out
/// after publication is the failure decision 1 already caught once, in a
/// different register. The failure is loud, immediate, and one line to
/// fix.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instance {
    pub question_id: String,
    pub question_type: String,
    pub question: String,
    pub answer: String,
    /// When the question is asked. Load-bearing rather than decorative:
    /// it is what makes the temporal-reasoning questions answerable, and
    /// a run that dropped it would be grading "when" against no now.
    pub question_date: String,
    pub haystack_dates: Vec<String>,
    pub haystack_session_ids: Vec<String>,
    pub haystack_sessions: Vec<Vec<Turn>>,
    /// The evidence sessions — the deterministic tier's entire expectation
    /// (decision 5). Defaulted rather than required because an abstention
    /// instance has no evidence to name, and an absent list and an empty
    /// one say the same thing there.
    #[serde(default)]
    pub answer_session_ids: Vec<String>,
}

/// One turn of one haystack session.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Turn {
    pub role: String,
    pub content: String,
    /// Upstream's own annotation on the turns that carry the answer. Only
    /// present on evidence sessions, and only in some of the released
    /// variants — so the guard that reads it is vacuous when it is absent
    /// and sharp when it is there.
    #[serde(default)]
    pub has_answer: Option<bool>,
}

impl Instance {
    /// Whether this is one of the 30 abstention instances. Read from the
    /// id because that is what upstream's own eval splits on; `validate`
    /// asserts the evidence list agrees, so this is never the only thing
    /// deciding it.
    #[must_use]
    pub fn is_abstention(&self) -> bool {
        self.question_id.ends_with(ABSTENTION_SUFFIX)
    }

    /// The haystack, joined. `validate` has already established that the
    /// three arrays are the same length, so this zip drops nothing.
    pub fn sessions(&self) -> impl Iterator<Item = Session<'_>> {
        self.haystack_session_ids
            .iter()
            .zip(&self.haystack_dates)
            .zip(&self.haystack_sessions)
            .map(|((session_id, date), turns)| Session {
                session_id,
                date,
                turns,
            })
    }

    /// How many turns this instance would seed. The unit the run's cost is
    /// actually measured in, and the one decision 7's slice exists to
    /// bound.
    #[must_use]
    pub fn turns(&self) -> usize {
        self.haystack_sessions.iter().map(Vec::len).sum()
    }
}

/// One session of the haystack, with the three arrays already joined.
pub struct Session<'a> {
    pub session_id: &'a str,
    pub date: &'a str,
    pub turns: &'a [Turn],
}

/// Upstream's timestamp, `2023/05/20 (Sat) 02:29`, read as UTC.
///
/// Every one of them is parsed by `validate` rather than by the runner,
/// and that placement is the point. A haystack date becomes an observed
/// event's `occurred_at`, and a corpus whose timestamps this harness
/// cannot render is a corpus that would fail halfway through seeding its
/// four-hundredth event — after the tenant, the actor and everything
/// before it. The weekday in the middle is redundant with the date and is
/// parsed rather than skipped, because a corpus where the two disagree is
/// one to ask upstream about.
pub fn parse_date(raw: &str) -> Result<chrono::DateTime<chrono::Utc>, String> {
    chrono::NaiveDateTime::parse_from_str(raw.trim(), "%Y/%m/%d (%a) %H:%M")
        .map(|naive| naive.and_utc())
        .map_err(|err| {
            format!("`{raw}` is not a LongMemEval timestamp (`%Y/%m/%d (%a) %H:%M`): {err}")
        })
}

/// A corpus file, and the bytes it was.
#[derive(Debug)]
pub struct Corpus {
    /// The file name, not the path: a published row should say which
    /// variant was measured (`longmemeval_s` and `longmemeval_oracle` are
    /// very different claims), and a path says where somebody's laptop
    /// kept it.
    pub file: String,
    /// BLAKE3 of the file, hex. What makes the score reproducible when the
    /// corpus cannot be committed.
    pub digest: String,
    pub bytes: u64,
    pub instances: Vec<Instance>,
}

/// Reads and validates a corpus file.
///
/// The whole file is parsed even when a run will only measure a slice of
/// it, and that is on purpose twice over: the guards are corpus-wide, and
/// a digest over bytes nobody parsed would attest to a file rather than to
/// a corpus. It costs memory proportional to the file — `longmemeval_s`
/// is the large case and it is a few gigabytes of parsed structs. If that
/// becomes the thing stopping anyone from running it, reversal trigger (f)
/// is already written for the shape of fix it needs.
pub fn load(path: &Path) -> Result<Corpus, String> {
    let raw = std::fs::read(path).map_err(|err| {
        format!(
            "read the LongMemEval corpus {}: {err} — it is fetched rather than committed; see \
             evals/fixtures/longmemeval/NOTICE.md",
            path.display()
        )
    })?;
    let digest = blake3::hash(&raw).to_hex().to_string();
    let bytes = raw.len() as u64;
    let instances: Vec<Instance> = serde_json::from_slice(&raw)
        .map_err(|err| format!("{} is not a LongMemEval corpus: {err}", path.display()))?;
    let corpus = Corpus {
        file: path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        ),
        digest,
        bytes,
        instances,
    };
    validate(&corpus)?;
    Ok(corpus)
}

/// The same, for a stack that may not have fetched it. `check` runs in CI,
/// where the corpus is absent by construction — and an absent corpus must
/// read as "nothing was validated" out loud rather than as a pass
/// (decision 7).
pub fn load_if_present(path: &Path) -> Result<Option<Corpus>, String> {
    if !path.exists() {
        return Ok(None);
    }
    load(path).map(Some)
}

/// The checks serde cannot make.
///
/// Every one of them is a property grading depends on, and none of them is
/// a matter of taste about how the corpus was written. A failure here is a
/// question for upstream, never a licence to edit the file: the message
/// says so, because the tempting fix is the one that would invalidate the
/// score.
fn validate(corpus: &Corpus) -> Result<(), String> {
    if corpus.instances.is_empty() {
        return Err(format!("{} holds no instances", corpus.file));
    }
    let mut ids: BTreeSet<&str> = BTreeSet::new();

    for instance in &corpus.instances {
        let at = |what: &str| {
            format!(
                "{}/{}: {what} — this is upstream's corpus; report it rather than editing the \
                 file, because a corpus edited until it validates is a corpus whose score means \
                 nothing",
                corpus.file, instance.question_id
            )
        };
        if !ids.insert(instance.question_id.as_str()) {
            return Err(at("two instances share this question id"));
        }
        if !QUESTION_TYPES.contains(&instance.question_type.as_str()) {
            return Err(at(&format!(
                "question type `{}` is not one of {QUESTION_TYPES:?}, so it would count in the \
                 total and in no category",
                instance.question_type
            )));
        }
        if instance.question.trim().is_empty() || instance.answer.trim().is_empty() {
            return Err(at(
                "an instance with no question or no answer grades nothing",
            ));
        }
        // The "now" the temporal questions are asked at. A run that
        // dropped it would be grading "when" against no now.
        parse_date(&instance.question_date).map_err(|err| at(&format!("question_date: {err}")))?;

        // The three haystack arrays are one table written as three
        // columns. Unequal lengths mean the session/date/turns join is a
        // guess, and every number downstream would be computed over the
        // wrong rows.
        let (dates, sessions, turns) = (
            instance.haystack_dates.len(),
            instance.haystack_session_ids.len(),
            instance.haystack_sessions.len(),
        );
        if dates != sessions || sessions != turns {
            return Err(at(&format!(
                "the haystack arrays disagree: {sessions} session id(s), {dates} date(s), {turns} \
                 session(s) of turns"
            )));
        }
        if sessions == 0 {
            return Err(at(
                "an instance with no haystack has nothing to retrieve from",
            ));
        }

        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for session_id in &instance.haystack_session_ids {
            if !seen.insert(session_id.as_str()) {
                return Err(at(&format!(
                    "session id `{session_id}` appears twice in one haystack, so \"did the block \
                     bind it\" has two answers"
                )));
            }
        }
        for session in instance.sessions() {
            if session.turns.is_empty() {
                return Err(at(&format!(
                    "session `{}` holds no turns, so seeding it would plant nothing",
                    session.session_id
                )));
            }
            parse_date(session.date)
                .map_err(|err| at(&format!("session `{}`: {err}", session.session_id)))?;
            for turn in session.turns {
                if !ROLES.contains(&turn.role.as_str()) {
                    return Err(at(&format!(
                        "session `{}` has a turn by `{}`, which is not one of {ROLES:?}",
                        session.session_id, turn.role
                    )));
                }
                if turn.content.trim().is_empty() {
                    return Err(at(&format!(
                        "session `{}` has an empty turn by `{}`, which would be seeded as an \
                         event carrying nothing and shrink the haystack silently",
                        session.session_id, turn.role
                    )));
                }
            }
        }

        // The deterministic tier's expectation has to be reachable. An
        // evidence session outside the haystack would score every run zero
        // on that instance forever, for a corpus reason.
        for evidence in &instance.answer_session_ids {
            if !seen.contains(evidence.as_str()) {
                return Err(at(&format!(
                    "evidence session `{evidence}` is not in this instance's haystack, so no run \
                     could ever bind it"
                )));
            }
        }

        // The abstention marker and the evidence list are two statements
        // of one fact, and the retrieval denominator depends on which is
        // true. Trusting either alone is how 30 instances quietly enter or
        // leave a published rate.
        match (instance.is_abstention(), instance.answer_session_ids.len()) {
            (true, count) if count > 0 => {
                return Err(at(&format!(
                    "the id marks this as an abstention instance but it names {count} evidence \
                     session(s); the retrieval tier excludes abstention instances and cannot do \
                     both"
                )));
            }
            (false, 0) => {
                return Err(at(
                    "no evidence session is named and the id does not mark this as an abstention \
                     instance, so the retrieval tier would score it zero for having nothing to \
                     find",
                ));
            }
            _ => {}
        }

        // Upstream's per-turn annotation, checked in the one direction
        // that can corrupt a denominator: a session holding an answer must
        // be named as evidence. The other direction is left alone because
        // the annotation is sparse in some released variants, and demanding
        // it would fail a corpus for being less annotated rather than for
        // being inconsistent.
        for session in instance.sessions() {
            let marked = session
                .turns
                .iter()
                .any(|turn| turn.has_answer.unwrap_or(false));
            if marked
                && !instance
                    .answer_session_ids
                    .iter()
                    .any(|id| id == session.session_id)
            {
                return Err(at(&format!(
                    "session `{}` has a turn marked `has_answer` but is not named in \
                     answer_session_ids, so upstream's two annotations disagree about where the \
                     evidence is",
                    session.session_id
                )));
            }
        }
    }
    Ok(())
}

/// What a run measured, stated rather than implied (decision 7).
///
/// EVAL-5's rule travels with its shape: a suite that bounds its coverage
/// says what it bounded. Every field here rides into the report and into
/// the published row, because a slice that is not named reads as the whole
/// corpus to everyone who did not run it.
#[derive(Debug, Clone, Serialize)]
pub struct Slice {
    pub file: String,
    pub digest: String,
    pub corpus_bytes: u64,
    /// Every instance in the file.
    pub corpus_instances: usize,
    /// The instances this run measures.
    pub instances: usize,
    /// How they were chosen, in words.
    pub rule: String,
    /// How many of `instances` are abstention instances. They are measured
    /// at the QA tier and excluded from the retrieval tier, which is
    /// LongMemEval's own convention — and stating the count is what keeps
    /// that exclusion from being a silent one (decision 5).
    pub abstention_instances: usize,
    /// Per question type, so a slice that missed a category is visible
    /// before the run rather than after.
    pub types: BTreeMap<String, usize>,
    /// Chat turns across the slice — the unit the seeding cost is actually
    /// paid in.
    pub turns: usize,
}

/// Picks the instances a run measures.
///
/// Deterministic and stratified, and the second half is why this is not
/// simply "the first N". Sorting by `(question_type, question_id)` and
/// then taking evenly-spread indices gives every category a share of the
/// slice proportional to its share of the corpus. A contiguous slice of
/// arbitrary ids would leave whole abilities at zero instances while still
/// reporting a per-category axis for them — a published number over an
/// empty denominator, which is the failure this file spends most of its
/// guards avoiding.
#[must_use]
pub fn slice(corpus: &Corpus, count: usize) -> (Vec<&Instance>, Slice) {
    let mut ordered: Vec<&Instance> = corpus.instances.iter().collect();
    ordered.sort_by(|left, right| {
        (&left.question_type, &left.question_id).cmp(&(&right.question_type, &right.question_id))
    });

    let total = ordered.len();
    let taken = count.min(total);
    let picked: Vec<&Instance> = if taken == total {
        ordered
    } else {
        (0..taken)
            .map(|index| ordered[index * total / taken])
            .collect()
    };

    let mut types: BTreeMap<String, usize> = BTreeMap::new();
    for instance in &picked {
        *types.entry(instance.question_type.clone()).or_default() += 1;
    }
    let rule = if taken == total {
        format!("all {total} instance(s)")
    } else {
        format!("{taken} of {total} instance(s), evenly spread over (question_type, question_id)")
    };
    let summary = Slice {
        file: corpus.file.clone(),
        digest: corpus.digest.clone(),
        corpus_bytes: corpus.bytes,
        corpus_instances: total,
        instances: picked.len(),
        rule,
        abstention_instances: picked
            .iter()
            .filter(|instance| instance.is_abstention())
            .count(),
        types,
        turns: picked.iter().map(|instance| instance.turns()).sum(),
    };
    (picked, summary)
}

impl Slice {
    /// The slice as a person reads it, for the line `check` and every run
    /// print before anything expensive happens.
    #[must_use]
    pub fn describe(&self) -> String {
        let types: Vec<String> = self
            .types
            .iter()
            .map(|(question_type, count)| format!("{question_type} {count}"))
            .collect();
        format!(
            "{} ({:.1} MiB, blake3 {}): {} — {} turn(s), {} abstention instance(s) excluded from \
             the retrieval tier; {}",
            self.file,
            self.corpus_bytes as f64 / (1024.0 * 1024.0),
            self.digest.chars().take(12).collect::<String>(),
            self.rule,
            self.turns,
            self.abstention_instances,
            types.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two instances in upstream's shape: one ordinary, one abstention.
    const CLEAN: &str = r#"[
        {
            "question_id": "aa11",
            "question_type": "multi-session",
            "question": "how long did the move take",
            "answer": "three weeks",
            "question_date": "2023/05/20 (Sat) 02:29",
            "haystack_dates": ["2023/04/01 (Sat) 10:00", "2023/04/09 (Sun) 18:12"],
            "haystack_session_ids": ["s-one", "s-two"],
            "haystack_sessions": [
                [{"role": "user", "content": "we start packing tomorrow"}],
                [{"role": "user", "content": "keys handed over today", "has_answer": true},
                 {"role": "assistant", "content": "three weeks door to door"}]
            ],
            "answer_session_ids": ["s-two"]
        },
        {
            "question_id": "bb22_abs",
            "question_type": "single-session-user",
            "question": "what did the surveyor charge",
            "answer": "never mentioned",
            "question_date": "2023/05/20 (Sat) 02:31",
            "haystack_dates": ["2023/04/01 (Sat) 10:00"],
            "haystack_session_ids": ["s-three"],
            "haystack_sessions": [[{"role": "user", "content": "we start packing tomorrow"}]],
            "answer_session_ids": []
        }
    ]"#;

    fn parse(json: &str) -> Result<Corpus, String> {
        let instances: Vec<Instance> = serde_json::from_str(json).map_err(|err| err.to_string())?;
        let corpus = Corpus {
            file: "longmemeval_test.json".to_owned(),
            digest: "b3-test".to_owned(),
            bytes: json.len() as u64,
            instances,
        };
        validate(&corpus)?;
        Ok(corpus)
    }

    #[test]
    fn a_corpus_round_trips_in_upstreams_own_field_names() {
        let corpus = parse(CLEAN).expect("parses");
        assert_eq!(corpus.instances.len(), 2);
        let first = &corpus.instances[0];
        assert!(!first.is_abstention());
        assert_eq!(first.turns(), 3);
        assert_eq!(first.sessions().count(), 2);
        let second = first.sessions().nth(1).expect("two sessions");
        assert_eq!(second.session_id, "s-two");
        assert_eq!(second.date, "2023/04/09 (Sun) 18:12");
        assert!(corpus.instances[1].is_abstention());
    }

    #[test]
    fn an_unknown_field_is_refused_rather_than_ignored() {
        // The call the module doc argues for: a field upstream added and we
        // ignored is a field the published score was computed without.
        let json = CLEAN.replace(r#""question_date""#, r#""asked_on""#);
        let err = parse(&json).expect_err("an unknown field must not parse");
        assert!(err.contains("asked_on"), "unhelpful error: {err}");
    }

    #[test]
    fn haystack_arrays_that_disagree_are_refused() {
        let json = CLEAN.replace(
            r#""haystack_session_ids": ["s-one", "s-two"]"#,
            r#""haystack_session_ids": ["s-one"]"#,
        );
        let err = parse(&json).expect_err("a ragged haystack must not validate");
        assert!(err.contains("disagree"), "unhelpful error: {err}");
    }

    /// The guard the retrieval tier rests on: an expectation outside the
    /// haystack would score zero forever for a corpus reason.
    #[test]
    fn an_evidence_session_outside_the_haystack_is_refused() {
        let json = CLEAN.replace(
            r#""answer_session_ids": ["s-two"]"#,
            r#""answer_session_ids": ["s-ghost"]"#,
        );
        let err = parse(&json).expect_err("dangling evidence must not validate");
        assert!(err.contains("s-ghost"), "unhelpful error: {err}");
        assert!(err.contains("report it rather than editing"), "{err}");
    }

    /// Both halves, because the retrieval denominator depends on which of
    /// upstream's two statements is true.
    #[test]
    fn the_abstention_marker_and_the_evidence_list_must_agree() {
        let evidence_on_abstention = CLEAN.replace(
            r#""answer_session_ids": []"#,
            r#""answer_session_ids": ["s-three"]"#,
        );
        let err = parse(&evidence_on_abstention).expect_err("an abstention with evidence");
        assert!(err.contains("abstention"), "unhelpful error: {err}");

        let no_evidence = CLEAN.replace(
            r#""answer_session_ids": ["s-two"]"#,
            r#""answer_session_ids": []"#,
        );
        let err = parse(&no_evidence).expect_err("a graded instance with no evidence");
        assert!(err.contains("nothing to find"), "unhelpful error: {err}");
    }

    /// Upstream's two annotations, cross-checked in the direction that can
    /// move a denominator.
    #[test]
    fn a_marked_turn_outside_the_evidence_sessions_is_refused() {
        let json = CLEAN.replace(
            r#"[{"role": "user", "content": "we start packing tomorrow"}],
                [{"role": "user", "content": "keys handed over today", "has_answer": true},"#,
            r#"[{"role": "user", "content": "we start packing tomorrow", "has_answer": true}],
                [{"role": "user", "content": "keys handed over today"},"#,
        );
        let err = parse(&json).expect_err("a marked turn outside the evidence");
        assert!(err.contains("s-one"), "unhelpful error: {err}");
        assert!(
            err.contains("disagree about where the evidence is"),
            "{err}"
        );
    }

    /// Parsed at validation rather than at seed time, so a corpus this
    /// harness cannot render fails before a tenant exists rather than
    /// after four hundred events.
    #[test]
    fn timestamps_are_parsed_and_a_corpus_with_one_we_cannot_render_is_refused() {
        let asked = parse_date("2023/05/20 (Sat) 02:29").expect("upstream's format");
        assert_eq!(asked.to_rfc3339(), "2023-05-20T02:29:00+00:00");

        let err = parse_date("2023-05-20T02:29:00Z").expect_err("RFC3339 is not their format");
        assert!(err.contains("not a LongMemEval timestamp"), "{err}");

        // The weekday is redundant with the date, and parsing it rather
        // than skipping it means a corpus where the two disagree is
        // refused instead of silently read. 2023/04/01 was a Saturday.
        assert!(
            parse_date("2023/04/01 (Mon) 10:00").is_err(),
            "a weekday that contradicts its own date must not parse"
        );

        let json = CLEAN.replace(r#""2023/05/20 (Sat) 02:29""#, r#""20 May 2023 02:29""#);
        let err = parse(&json).expect_err("an unparseable question date");
        assert!(err.contains("question_date"), "unhelpful error: {err}");

        let json = CLEAN.replace(r#""2023/04/09 (Sun) 18:12""#, r#""2023/04/09 18:12""#);
        let err = parse(&json).expect_err("an unparseable session date");
        assert!(err.contains("s-two"), "unhelpful error: {err}");
    }

    #[test]
    fn an_empty_turn_is_refused_rather_than_seeded() {
        let json = CLEAN.replace(r#""we start packing tomorrow""#, r#""   ""#);
        let err = parse(&json).expect_err("an empty turn must not validate");
        assert!(err.contains("shrink the haystack silently"), "{err}");
    }

    #[test]
    fn the_vocabularies_are_closed() {
        let err = parse(&CLEAN.replace(r#""multi-session""#, r#""multi-hop""#))
            .expect_err("an unknown question type must not validate");
        assert!(err.contains("multi-hop"), "{err}");

        let err = parse(&CLEAN.replace(r#""role": "assistant""#, r#""role": "system""#))
            .expect_err("an unknown role must not validate");
        assert!(err.contains("system"), "{err}");
    }

    #[test]
    fn a_duplicate_question_id_is_refused() {
        let json = CLEAN.replace(r#""bb22_abs""#, r#""aa11""#);
        let err = parse(&json).expect_err("a duplicate id must not validate");
        assert!(err.contains("share this question id"), "{err}");
    }

    /// A slice must cover every category the corpus has, or the per-category
    /// axes it publishes are rates over nothing.
    #[test]
    fn a_slice_is_stratified_deterministic_and_declares_itself() {
        let mut instances = Vec::new();
        for question_type in QUESTION_TYPES {
            for index in 0..10 {
                let abstention = index == 0;
                let json = format!(
                    r#"{{"question_id": "{question_type}-{index:02}{suffix}",
                         "question_type": "{question_type}",
                         "question": "q", "answer": "a",
                         "question_date": "2023/05/20 (Sat) 02:29",
                         "haystack_dates": ["2023/04/01 (Sat) 10:00"],
                         "haystack_session_ids": ["s-{question_type}-{index}"],
                         "haystack_sessions": [[{{"role": "user", "content": "c"}}]],
                         "answer_session_ids": [{evidence}]}}"#,
                    suffix = if abstention { ABSTENTION_SUFFIX } else { "" },
                    evidence = if abstention {
                        String::new()
                    } else {
                        format!(r#""s-{question_type}-{index}""#)
                    },
                );
                instances.push(serde_json::from_str::<Instance>(&json).expect("parses"));
            }
        }
        let corpus = Corpus {
            file: "longmemeval_s.json".to_owned(),
            digest: "b3-test".to_owned(),
            bytes: 1024 * 1024 * 7,
            instances,
        };
        validate(&corpus).expect("valid");

        let (picked, summary) = slice(&corpus, 12);
        assert_eq!(picked.len(), 12);
        assert_eq!(summary.corpus_instances, 60);
        assert_eq!(
            summary.types.len(),
            QUESTION_TYPES.len(),
            "a slice that missed a category: {:?}",
            summary.types
        );
        for count in summary.types.values() {
            assert_eq!(*count, 2, "unstratified: {:?}", summary.types);
        }
        assert!(summary.rule.contains("12 of 60"), "{}", summary.rule);
        assert!(
            summary.describe().contains("blake3 b3-test"),
            "{}",
            summary.describe()
        );

        // Deterministic: the same corpus and the same count pick the same
        // instances, which is what lets two runs be compared at all.
        let (again, _) = slice(&corpus, 12);
        let ids = |chosen: &[&Instance]| -> Vec<String> {
            chosen.iter().map(|i| i.question_id.clone()).collect()
        };
        assert_eq!(ids(&picked), ids(&again));
    }

    #[test]
    fn a_slice_wider_than_the_corpus_is_the_whole_corpus_and_says_so() {
        let corpus = parse(CLEAN).expect("parses");
        let (picked, summary) = slice(&corpus, 500);
        assert_eq!(picked.len(), 2);
        assert_eq!(summary.instances, 2);
        assert_eq!(summary.abstention_instances, 1);
        assert_eq!(summary.turns, 4);
        assert!(summary.rule.contains("all 2"), "{}", summary.rule);
    }

    #[test]
    fn an_absent_corpus_reads_as_absent_rather_than_as_empty() {
        let missing = Path::new("evals/fixtures/longmemeval/nothing-is-here.json");
        assert!(
            load_if_present(missing)
                .expect("absence is not an error")
                .is_none()
        );
    }
}
