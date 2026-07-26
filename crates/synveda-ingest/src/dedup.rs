//! The dedup judge (MEM-5, ADR-0039 decisions 4–6): given one extraction
//! candidate and the neighbours the nominator surfaced, decide whether the
//! candidate restates an existing record, contradicts one, or is simply new.
//!
//! The nominator is `synveda_store::dedup` — it owns the LSH encoding
//! because it owns the columns. This module owns the *decision*, which is
//! policy: its thresholds ride the effective policy pack, and its verdict
//! decides whether a fact stops composing.
//!
//! # The judge is tuned to refuse
//!
//! A missed update leaves a stale fact composing beside a fresh one, which is
//! what the product did before this feature. A wrong supersession removes a
//! true fact from every future inject. The second is worse, so the
//! contradiction rule is a conjunction of three conditions, each of which can
//! only reject:
//!
//! 1. the two frames overlap by at least the pack's threshold — "these are
//!    about the same thing";
//! 2. they share their leading frame word — a crude subject proxy, and the
//!    conjunct that separates "we deploy on Tuesdays / Thursdays" from
//!    "deploys go through make deploy / tests go through make test";
//! 3. something actually changed — a differing frame word or a differing
//!    value.
//!
//! It will miss real updates: a subject named last, a passive voice, a
//! re-worded subject, a language whose word order is not English's. That is
//! deliberate, it is unmeasured until EVAL-2, and the model-backed judge
//! ADR-0039 decision 6 describes is where the recall comes from.

use chrono::{DateTime, Utc};
use synveda_store::dedup::{ContentTokens, jaccard};
use synveda_types::{DedupConfig, RecordId};

/// Counter: dedup decisions, labelled `outcome = insert | merge |
/// supersede | superseded_on_arrival | refused_published`.
///
/// `refused_published` is the one that earns its place: it counts the
/// contradictions the pipeline found against *reviewed* material and
/// declined to act on, which is how a tenant learns that somebody should
/// open a proposal (ADR-0039 decision 9).
pub const DEDUP_DECISIONS_TOTAL: &str = "synveda_dedup_decisions_total";

/// Histogram: neighbours nominated per candidate, labelled `leg = lexical |
/// dense`.
pub const DEDUP_CANDIDATES: &str = "synveda_dedup_candidates";

/// Histogram: seconds spent nominating and judging one candidate.
pub const DEDUP_SECONDS: &str = "synveda_dedup_seconds";

/// Why a pair was decided the way it was — short, machine-readable, and
/// stored on the edge row and in the audit payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The two contents are the same statement, character for character
    /// after trimming.
    Identical,
    /// Similar enough on either signal to be the same statement restated,
    /// and not a contradiction.
    NearDuplicate,
    /// Same subject, changed assertion.
    Contradiction,
}

impl Reason {
    /// The stored form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Reason::Identical => "identical",
            Reason::NearDuplicate => "near-duplicate",
            Reason::Contradiction => "contradiction",
        }
    }
}

/// One decided pair: which record, why, and on what evidence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pairing {
    /// The neighbour.
    pub record_id: RecordId,
    /// The verdict class.
    pub reason: Reason,
    /// Exact Jaccard over the two full token sets.
    pub jaccard: f64,
    /// Cosine similarity, when the dense leg nominated this neighbour under
    /// a comparable model.
    pub cosine: Option<f64>,
}

/// What the pipeline should do with one candidate.
///
/// Invariant: `merge_into` and the other two are mutually exclusive — an
/// absorbed candidate writes no record, so it can neither close a window nor
/// have one closed on it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Judgement {
    /// `Some` → the candidate restates this record and does not insert; the
    /// survivor is reinforced instead.
    pub merge_into: Option<Pairing>,
    /// Records this candidate contradicts and therefore closes, at its own
    /// `valid_from`.
    pub closes: Vec<Pairing>,
    /// The *newer* record that contradicts this one: the candidate arrived
    /// late and inserts with its window already shut at that record's
    /// `valid_from` (ADR-0039 decision 8).
    pub closed_by: Option<(Pairing, DateTime<Utc>)>,
    /// Contradictions found against published material and declined
    /// (ADR-0039 decision 9). Counted and audited, never acted on.
    pub refused_published: Vec<Pairing>,
}

impl Judgement {
    /// Whether the candidate becomes a record.
    #[must_use]
    pub fn inserts(&self) -> bool {
        self.merge_into.is_none()
    }

    /// The valid-time end the candidate's own record should carry, if it
    /// arrived after the fact that replaced it.
    #[must_use]
    pub fn valid_to(&self) -> Option<DateTime<Utc>> {
        self.closed_by.map(|(_, closed_at)| closed_at)
    }
}

/// One extraction candidate, as the judge sees it.
#[derive(Debug, Clone)]
pub struct Incoming<'a> {
    /// The final, rescanned content — exactly what will be persisted.
    pub content: &'a str,
    /// Its three token views.
    pub tokens: &'a ContentTokens,
    /// When the observed thing held in the world.
    pub valid_from: DateTime<Utc>,
}

/// One nominated neighbour, hydrated and scored by the caller.
#[derive(Debug, Clone)]
pub struct Nominee {
    /// The neighbour's record id.
    pub record_id: RecordId,
    /// Its persisted content.
    pub content: String,
    /// Its token views.
    pub tokens: ContentTokens,
    /// Its valid-from — which of the two statements is the newer assertion.
    pub valid_from: DateTime<Utc>,
    /// Its transaction time, for the merge-target tiebreak.
    pub tx_from: DateTime<Utc>,
    /// Cosine similarity to the candidate, when the dense leg nominated it
    /// under a comparable model.
    pub cosine: Option<f64>,
    /// Whether the neighbour's scope publishes it. Published material is off
    /// limits to supersession — reviewed content leaves the trust boundary
    /// through a proposal or a rollback, never through somebody's session.
    pub published: bool,
}

/// Judges one candidate against its nominated neighbours.
///
/// Deterministic: neighbours are considered in a total order (newest
/// assertion first, then newest transaction time, then id), the checks run in
/// ADR-0039 decision 4's fixed order — identical, contradiction,
/// near-duplicate — and nothing here reads a clock or a map's iteration
/// order.
#[must_use]
pub fn judge(config: &DedupConfig, incoming: &Incoming<'_>, nominees: &[Nominee]) -> Judgement {
    let mut judgement = Judgement::default();
    if !config.mode.merges() {
        return judgement;
    }
    let mut ordered: Vec<&Nominee> = nominees.iter().collect();
    // Published first, then the newest assertion: the merge target should be
    // the record a reader would actually have seen, which is seed §4.4's own
    // resolution order restricted to one (scope, owner, class) group — where
    // scope position and kind are constant, so only channel and recency are
    // left to say anything.
    ordered.sort_by(|a, b| {
        b.published
            .cmp(&a.published)
            .then_with(|| b.valid_from.cmp(&a.valid_from))
            .then_with(|| b.tx_from.cmp(&a.tx_from))
            .then_with(|| a.record_id.cmp(&b.record_id))
    });

    let mut near_duplicate: Option<Pairing> = None;
    for nominee in ordered {
        let score = jaccard(&incoming.tokens.all, &nominee.tokens.all);
        let pairing = |reason| Pairing {
            record_id: nominee.record_id,
            reason,
            jaccard: score,
            cosine: nominee.cosine,
        };

        // 1. The same statement, character for character. Certain, and
        //    decided before anything else can reinterpret it.
        if incoming.content.trim() == nominee.content.trim() {
            judgement.merge_into = Some(pairing(Reason::Identical));
            judgement.closes.clear();
            judgement.closed_by = None;
            return judgement;
        }

        // 2. A contradiction, asked *before* the near-duplicate thresholds:
        //    a long statement with one value changed is both, and testing
        //    similarity first would merge exactly the updates this feature
        //    exists to catch (ADR-0039 decision 4).
        if config.mode.supersedes() && contradicts(config, incoming.tokens, &nominee.tokens) {
            let pairing = pairing(Reason::Contradiction);
            if nominee.published {
                // The governance boundary. Counted, audited, and left alone.
                judgement.refused_published.push(pairing);
            } else if nominee.valid_from < incoming.valid_from {
                judgement.closes.push(pairing);
            } else if nominee.valid_from > incoming.valid_from {
                // The candidate observed something that held *earlier* than
                // the record contradicting it: it lands already closed
                // rather than being dropped. The nearest such record wins,
                // so the candidate's window ends where the next assertion
                // began.
                let closes_at = nominee.valid_from;
                match judgement.closed_by {
                    Some((_, existing)) if existing <= closes_at => {}
                    _ => judgement.closed_by = Some((pairing, closes_at)),
                }
            }
            // Equal valid-from is nobody's update: both statements claim the
            // same instant, so neither can be said to replace the other, and
            // composition's conflict rules resolve the block.
            continue;
        }

        // 3. Not a contradiction, and similar enough on either signal to be
        //    the same statement restated. Kept rather than returned, so a
        //    contradiction found on a later neighbour still takes precedence.
        if near_duplicate.is_none()
            && (score >= config.near_dup_jaccard()
                || nominee
                    .cosine
                    .is_some_and(|cosine| cosine >= config.near_dup_cosine()))
        {
            near_duplicate = Some(pairing(Reason::NearDuplicate));
        }
    }

    // A candidate that contradicts something is new information, whatever
    // else it resembles: merging it would discard the update.
    if judgement.closes.is_empty() && judgement.closed_by.is_none() {
        judgement.merge_into = near_duplicate;
    }
    judgement
}

/// The contradiction rule (ADR-0039 decision 5): three conjuncts, each of
/// which can only refuse.
fn contradicts(config: &DedupConfig, incoming: &ContentTokens, neighbour: &ContentTokens) -> bool {
    // A statement with no content words has no subject to share. Refuse
    // rather than guess: "it is at 10" against "it is at 11" is not
    // something this judge can responsibly close a window over.
    let (Some(lead), Some(other_lead)) = (incoming.frame.first(), neighbour.frame.first()) else {
        return false;
    };
    if lead != other_lead {
        return false;
    }
    let shared = incoming
        .frame
        .iter()
        .filter(|token| neighbour.frame.contains(token))
        .count();
    let smaller = incoming.frame.len().min(neighbour.frame.len());
    if smaller == 0 {
        return false;
    }
    // The overlap *coefficient*, not Jaccard: an update is routinely longer
    // than the statement it replaces ("the stand-up is at 09:30" → "the
    // stand-up moved to 10:15 from this week"), and Jaccard charges for the
    // added words twice.
    let overlap = shared as f64 / smaller as f64;
    if overlap < config.conflict_frame_overlap() {
        return false;
    }
    // Something has to have changed, or this is a restatement and the
    // near-duplicate band owns it.
    incoming.frame != neighbour.frame || incoming.values != neighbour.values
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use synveda_store::dedup::tokenise;

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0).unwrap()
    }

    fn id(byte: u8) -> RecordId {
        RecordId::from_uuid(uuid::Uuid::from_bytes([byte; 16]))
    }

    fn nominee(byte: u8, content: &str, valid_from: DateTime<Utc>) -> Nominee {
        Nominee {
            record_id: id(byte),
            content: content.to_owned(),
            tokens: tokenise(content),
            valid_from,
            tx_from: valid_from,
            cosine: None,
            published: false,
        }
    }

    fn verdict(content: &str, valid_from: DateTime<Utc>, nominees: &[Nominee]) -> Judgement {
        let tokens = tokenise(content);
        judge(
            &DedupConfig::DEFAULT,
            &Incoming {
                content,
                tokens: &tokens,
                valid_from,
            },
            nominees,
        )
    }

    /// The AC's own shape: a fact, then the same fact with its value
    /// changed. The older record's window closes and the newer one inserts.
    #[test]
    fn a_changed_value_on_the_same_subject_supersedes() {
        let existing = nominee(1, "We deploy on Tuesdays.", at(100));
        let judgement = verdict("We deploy on Thursdays.", at(200), &[existing]);
        assert!(judgement.inserts(), "the update is new information");
        assert_eq!(judgement.closes.len(), 1);
        assert_eq!(judgement.closes[0].record_id, id(1));
        assert_eq!(judgement.closes[0].reason, Reason::Contradiction);
        assert!(judgement.closed_by.is_none());
    }

    /// The false positive the leading-frame-word conjunct exists for: same
    /// grammar, different subject, two true facts.
    #[test]
    fn a_changed_subject_in_the_same_frame_is_not_a_contradiction() {
        let existing = nominee(1, "Deploys go through make deploy.", at(100));
        let judgement = verdict("Tests go through make test.", at(200), &[existing]);
        assert!(judgement.closes.is_empty(), "both statements are true");
        assert!(
            judgement.merge_into.is_none(),
            "and they are not duplicates"
        );
    }

    /// Two facts about one subject that do not exclude each other: the
    /// overlap conjunct refuses.
    #[test]
    fn two_facts_about_one_subject_both_survive() {
        let existing = nominee(1, "The API rate limit is 100/s.", at(100));
        let judgement = verdict("The API returns 429 on overload.", at(200), &[existing]);
        assert!(judgement.closes.is_empty());
        assert!(judgement.merge_into.is_none());
    }

    /// An update with no numbers in it at all — the value conjunct is not
    /// the only way something can change.
    #[test]
    fn a_changed_word_supersedes_without_any_value_token() {
        let existing = nominee(1, "I prefer dark roast coffee.", at(100));
        let judgement = verdict("I now prefer light roast coffee.", at(200), &[existing]);
        assert_eq!(judgement.closes.len(), 1, "{judgement:?}");
        assert_eq!(judgement.closes[0].record_id, id(1));
    }

    #[test]
    fn an_identical_restatement_merges_and_closes_nothing() {
        let existing = nominee(1, "The runbook lives in docs/deploy.md.", at(100));
        let judgement = verdict(
            "  The runbook lives in docs/deploy.md.  ",
            at(200),
            &[existing],
        );
        let merged = judgement.merge_into.expect("identical content merges");
        assert_eq!(merged.record_id, id(1));
        assert_eq!(merged.reason, Reason::Identical);
        assert!((merged.jaccard - 1.0).abs() < 1e-9);
        assert!(judgement.closes.is_empty());
    }

    /// A restatement that is *not* character-identical but is well above the
    /// Jaccard threshold, and asserts nothing new.
    #[test]
    fn a_near_restatement_merges() {
        let existing = nominee(
            1,
            "The incident review is owned by the platform team.",
            at(100),
        );
        let judgement = verdict(
            "The incident review is owned by the platform team",
            at(200),
            &[existing],
        );
        let merged = judgement.merge_into.expect("a restatement merges");
        assert_eq!(merged.reason, Reason::NearDuplicate);
    }

    /// The semantic leg on its own: a full re-wording shares almost no
    /// tokens, so only cosine can call it a restatement.
    #[test]
    fn the_dense_leg_alone_can_call_a_paraphrase_a_duplicate() {
        let mut existing = nominee(1, "Deployment happens every Tuesday.", at(100));
        existing.cosine = Some(0.99);
        let judgement = verdict(
            "We ship code at the start of each week.",
            at(200),
            &[existing],
        );
        let merged = judgement.merge_into.expect("cosine reaches the band");
        assert_eq!(merged.reason, Reason::NearDuplicate);
        assert_eq!(merged.cosine, Some(0.99));
        assert!(merged.jaccard < 0.3, "and the lexical leg would not have");
    }

    /// ADR-0039 decision 8: never ADD-only cuts both ways. A candidate that
    /// observed an earlier state lands with its window already shut.
    #[test]
    fn a_late_arriving_older_fact_inserts_already_closed() {
        let newer = nominee(1, "We deploy on Thursdays.", at(300));
        let judgement = verdict("We deploy on Tuesdays.", at(100), &[newer]);
        assert!(
            judgement.inserts(),
            "a late fact is recorded, never dropped"
        );
        assert_eq!(judgement.valid_to(), Some(at(300)));
        assert!(judgement.closes.is_empty());
        let (pairing, _) = judgement.closed_by.expect("closed by the newer record");
        assert_eq!(pairing.record_id, id(1));
    }

    /// Two stale variants, both contradicted by one new statement: the
    /// Graphiti pattern closes every invalidated assertion, not the nearest.
    #[test]
    fn one_statement_closes_every_record_it_contradicts() {
        let judgement = verdict(
            "We deploy on Thursdays.",
            at(300),
            &[
                nominee(1, "We deploy on Tuesdays.", at(100)),
                nominee(2, "We deploy on Wednesdays.", at(200)),
            ],
        );
        let mut closed: Vec<RecordId> = judgement.closes.iter().map(|p| p.record_id).collect();
        closed.sort();
        assert_eq!(closed, vec![id(1), id(2)]);
    }

    /// The governance boundary: reviewed material is left exactly as it is,
    /// and the refusal is data rather than silence.
    #[test]
    fn a_contradiction_against_published_material_is_refused_and_counted() {
        let mut published = nominee(1, "We deploy on Tuesdays.", at(100));
        published.published = true;
        let judgement = verdict("We deploy on Thursdays.", at(200), &[published]);
        assert!(judgement.closes.is_empty(), "the pipeline may not close it");
        assert_eq!(judgement.refused_published.len(), 1);
        assert!(judgement.inserts(), "the new fact is still recorded");
    }

    /// Same instant, different assertion: neither replaces the other, and
    /// CTX-2's conflict rules resolve the block instead.
    #[test]
    fn a_tie_on_valid_time_supersedes_nobody() {
        let existing = nominee(1, "We deploy on Tuesdays.", at(100));
        let judgement = verdict("We deploy on Thursdays.", at(100), &[existing]);
        assert!(judgement.closes.is_empty());
        assert!(judgement.closed_by.is_none());
        assert!(judgement.merge_into.is_none());
    }

    /// A contradiction outranks a near-duplicate found on another neighbour:
    /// merging would discard the update.
    #[test]
    fn a_contradiction_anywhere_beats_a_near_duplicate_elsewhere() {
        let mut paraphrase = nominee(2, "Something else entirely.", at(50));
        paraphrase.cosine = Some(0.99);
        let judgement = verdict(
            "We deploy on Thursdays.",
            at(300),
            &[nominee(1, "We deploy on Tuesdays.", at(100)), paraphrase],
        );
        assert!(judgement.merge_into.is_none(), "{judgement:?}");
        assert_eq!(judgement.closes.len(), 1);
    }

    #[test]
    fn the_modes_bound_what_the_judge_will_say() {
        let existing = nominee(1, "We deploy on Tuesdays.", at(100));
        let tokens = tokenise("We deploy on Thursdays.");
        let incoming = Incoming {
            content: "We deploy on Thursdays.",
            tokens: &tokens,
            valid_from: at(200),
        };
        let merge_only = DedupConfig {
            mode: synveda_types::DedupMode::Merge,
            ..DedupConfig::DEFAULT
        };
        let judgement = judge(&merge_only, &incoming, std::slice::from_ref(&existing));
        assert!(judgement.closes.is_empty(), "merge mode closes no windows");

        let off = DedupConfig {
            mode: synveda_types::DedupMode::Off,
            ..DedupConfig::DEFAULT
        };
        let judgement = judge(&off, &incoming, &[existing]);
        assert_eq!(judgement, Judgement::default(), "off does nothing at all");
    }

    /// The merge target is the record a reader would have seen: published
    /// first, then the newest assertion.
    #[test]
    fn a_merge_lands_on_the_published_copy_when_there_is_one() {
        let mut published = nominee(1, "The runbook lives in docs/deploy.md.", at(100));
        published.published = true;
        let newer = nominee(2, "The runbook lives in docs/deploy.md.", at(500));
        let judgement = verdict(
            "The runbook lives in docs/deploy.md.",
            at(900),
            &[newer, published],
        );
        assert_eq!(
            judgement.merge_into.expect("merges").record_id,
            id(1),
            "reviewed material is the survivor a reader actually sees"
        );
    }
}
