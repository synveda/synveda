//! GRPH-2's first acceptance clause (ADR-0044): **entity dedup precision
//! on a fixture set**.
//!
//! The measurement is pairwise, which is the only scoring that says
//! anything useful about a resolver: for every unordered pair of mentions
//! in the labelled set, the resolver either puts them on the same vertex or
//! it does not, and ground truth either agrees or it does not. Precision is
//! the share of the merges it made that were right — the number the AC
//! names, and the one ADR-0044 decision 3 chose the algorithm for. Recall
//! is reported beside it and deliberately not asserted: the fixture set
//! carries "PostgreSQL", "International Business Machines" and "Jorg
//! Muller" precisely so the recall the resolver gives up is visible rather
//! than hidden.
//!
//! No database and no network: [`resolve`] is a pure function, and the
//! vertex it converges on is the schema's unique constraint. That the two
//! agree — the same key really does reach the same row — is the DB-backed
//! half of the AC, in `crates/synveda-gateway/tests/graph_linking.rs`.

use std::collections::BTreeMap;

use serde::Deserialize;
use synveda_ingest::linking::{
    MENTION_EXACT_PERMILLE, MENTION_NORMALISED_PERMILLE, Resolution, resolve,
};

/// The provisional precision target. Provisional in the sense ADR-0022
/// decision 8 uses the word: EVAL-2 owns the real one, on a corpus larger
/// than a fixture file. The fixture set contains one deliberate false
/// merge (`Paris`), so a perfect score here would mean the set had been
/// made flattering rather than the resolver made better.
const PROVISIONAL_TARGET: f64 = 0.95;

const FIXTURES: &str = include_str!("fixtures/graph/mentions.json");

#[derive(Deserialize)]
struct Fixtures {
    entities: Vec<Group>,
    refused: Vec<String>,
}

#[derive(Deserialize)]
struct Group {
    entity: String,
    mentions: Vec<String>,
}

fn fixtures() -> Fixtures {
    serde_json::from_str(FIXTURES).expect("labelled mention set parses")
}

/// One mention, its ground-truth entity, and what the resolver made of it.
struct Scored {
    entity: String,
    mention: String,
    key: Option<String>,
}

fn score(groups: &[Group]) -> Vec<Scored> {
    groups
        .iter()
        .flat_map(|group| {
            group.mentions.iter().map(|mention| Scored {
                entity: group.entity.clone(),
                mention: mention.clone(),
                key: resolve(mention).map(|resolution| resolution.key),
            })
        })
        .collect()
}

/// Pairwise counts over the whole set.
#[derive(Default)]
struct Report {
    /// Merged and right.
    true_positive: usize,
    /// Merged and wrong — two entities on one vertex.
    false_positive: usize,
    /// Not merged and wrong — one entity split across vertices.
    false_negative: usize,
    /// Mentions the resolver declined, which merge with nothing.
    refused: usize,
}

impl Report {
    fn precision(&self) -> f64 {
        let merged = self.true_positive + self.false_positive;
        if merged == 0 {
            return 0.0;
        }
        self.true_positive as f64 / merged as f64
    }

    fn recall(&self) -> f64 {
        let same = self.true_positive + self.false_negative;
        if same == 0 {
            return 0.0;
        }
        self.true_positive as f64 / same as f64
    }

    fn print(&self, scored: &[Scored]) {
        println!("entity resolution over {} mentions:", scored.len());
        println!(
            "  merged {} pairs, {} of them correct; {} same-entity pairs left unmerged",
            self.true_positive + self.false_positive,
            self.true_positive,
            self.false_negative
        );
        println!(
            "  precision {:.3}; recall (informational) {:.3}; refused {}",
            self.precision(),
            self.recall(),
            self.refused
        );
        // The clusters themselves, so a failing run says *what* merged
        // rather than only by how much the number moved.
        let mut clusters: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for item in scored {
            let key = item.key.as_deref().unwrap_or("<refused>");
            clusters.entry(key).or_default().push(&item.entity);
        }
        for (key, mut entities) in clusters {
            entities.sort_unstable();
            entities.dedup();
            if entities.len() > 1 {
                println!("  !! {key:?} holds {entities:?}");
            }
        }
    }
}

fn measure(scored: &[Scored]) -> Report {
    let mut report = Report {
        refused: scored.iter().filter(|item| item.key.is_none()).count(),
        ..Report::default()
    };
    for (index, left) in scored.iter().enumerate() {
        for right in &scored[index + 1..] {
            let same_entity = left.entity == right.entity;
            let merged = match (&left.key, &right.key) {
                (Some(left), Some(right)) => left == right,
                _ => false,
            };
            match (merged, same_entity) {
                (true, true) => report.true_positive += 1,
                (true, false) => report.false_positive += 1,
                (false, true) => report.false_negative += 1,
                (false, false) => {}
            }
        }
    }
    report
}

/// The AC: pairwise precision on the labelled set meets the provisional
/// target.
#[test]
fn entity_resolution_precision_meets_provisional_target() {
    let fixtures = fixtures();
    let scored = score(&fixtures.entities);
    let report = measure(&scored);
    report.print(&scored);
    assert!(
        report.precision() >= PROVISIONAL_TARGET,
        "pairwise precision {:.3} under the provisional target {PROVISIONAL_TARGET}",
        report.precision()
    );
}

/// The one wrong merge in the set is the one the fixtures document, and it
/// is a genuine ambiguity rather than an over-eager rule. If a *rule* ever
/// starts merging entities, this test fails before the precision target
/// does — a threshold has slack, an enumerated failure does not.
#[test]
fn the_only_false_merge_is_the_shared_surface_form() {
    let fixtures = fixtures();
    let scored = score(&fixtures.entities);
    let mut wrong: Vec<(&str, &str, &str)> = Vec::new();
    for (index, left) in scored.iter().enumerate() {
        for right in &scored[index + 1..] {
            if left.entity != right.entity && left.key.is_some() && left.key == right.key {
                wrong.push((
                    left.key.as_deref().unwrap_or_default(),
                    &left.mention,
                    &right.mention,
                ));
            }
        }
    }
    assert_eq!(
        wrong,
        vec![("paris", "Paris", "Paris")],
        "the set's only wrong merge is two different things sharing one name"
    );
}

/// Mentions the resolver must decline: a redaction placeholder never
/// becomes a graph identity (ADR-0044 decision 9), and neither does a
/// pronoun or an empty string the schema would refuse anyway.
#[test]
fn the_refusals_are_refused() {
    for mention in fixtures().refused {
        assert!(
            resolve(&mention).is_none(),
            "{mention:?} must not resolve to a vertex key"
        );
    }
}

/// Every resolution carries the tier ADR-0044 decision 4 promises, and a
/// mention that needed a word removed never claims the exact one — the
/// property GRPH-3's ranking will lean on.
#[test]
fn the_confidence_tier_reports_what_normalisation_did() {
    let exact: Vec<Resolution> = ["Ada Lovelace", "ada lovelace", "Postgres.", "(Postgres)"]
        .iter()
        .map(|mention| resolve(mention).expect("resolves"))
        .collect();
    for resolution in &exact {
        assert_eq!(
            resolution.confidence_permille, MENTION_EXACT_PERMILLE,
            "{:?} discarded nothing and must claim the exact tier",
            resolution.label
        );
    }
    let normalised: Vec<Resolution> = ["Ada Lovelace's", "The Bank of England", "North Star Ltd"]
        .iter()
        .map(|mention| resolve(mention).expect("resolves"))
        .collect();
    for resolution in &normalised {
        assert_eq!(
            resolution.confidence_permille, MENTION_NORMALISED_PERMILLE,
            "{:?} removed a word and must not claim the exact tier",
            resolution.label
        );
    }
}
