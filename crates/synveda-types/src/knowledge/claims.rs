//! Optional bounded claim declarations in immutable metadata (ADR-0142).
//! The declaration is evidence to compare, not proof of truth or authority.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{ConflictClassification, KnowledgeRevisionContent};
use crate::{Error, KnowledgeItemId, Result};

const KEY: &str = "synveda_claims";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Projection {
    schema_version: u8,
    claims: Vec<Claim>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claim {
    subject: Subject,
    predicate: String,
    assertion_status: AssertionStatus,
    applicability: BTreeMap<String, String>,
    #[serde(default)]
    unit: Option<String>,
    constraint: Constraint,
    evidence: Evidence,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Subject {
    Named { key: String },
    Entity { item_id: KnowledgeItemId },
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AssertionStatus {
    Asserted,
    Proposed,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    start_byte: u32,
    end_byte: u32,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Atom {
    Decimal { coefficient: i64, scale: u8 },
    Text { value: String },
    Boolean { value: bool },
    SemverRelease { major: u32, minor: u32, patch: u32 },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operator", rename_all = "snake_case", deny_unknown_fields)]
enum Constraint {
    Eq {
        value: Atom,
    },
    Ne {
        value: Atom,
    },
    Lt {
        value: Atom,
    },
    Lte {
        value: Atom,
    },
    Gt {
        value: Atom,
    },
    Gte {
        value: Atom,
    },
    Between {
        lower: Atom,
        upper: Atom,
        lower_inclusive: bool,
        upper_inclusive: bool,
    },
}

fn invalid() -> Error {
    Error::Invalid { message: "synveda_claims must be a bounded v1 claim projection with typed values, applicability and exact UTF-8 body evidence".to_owned() }
}

fn bounded(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn projection(content: &KnowledgeRevisionContent) -> Result<Option<Projection>> {
    let Some(value) = content.metadata.get(KEY) else {
        return Ok(None);
    };
    let parsed: Projection = serde_json::from_value(value.clone()).map_err(|_| invalid())?;
    if parsed.schema_version != 1 || !(1..=8).contains(&parsed.claims.len()) {
        return Err(invalid());
    }
    for claim in &parsed.claims {
        if matches!(&claim.subject, Subject::Named { key } if !bounded(key, 128))
            || !bounded(&claim.predicate, 64)
            || claim.applicability.len() > 8
            || claim
                .applicability
                .iter()
                .any(|(key, value)| !bounded(key, 64) || !bounded(value, 128))
            || claim.unit.as_ref().is_some_and(|unit| !bounded(unit, 32))
            || claim.evidence.start_byte >= claim.evidence.end_byte
            || content
                .body_markdown
                .get(claim.evidence.start_byte as usize..claim.evidence.end_byte as usize)
                .is_none_or(|text| text.trim().is_empty())
        {
            return Err(invalid());
        }
        validate_constraint(&claim.constraint, claim.unit.as_deref())?;
    }
    Ok(Some(parsed))
}

pub(super) fn validate(content: &KnowledgeRevisionContent) -> Result<()> {
    projection(content).map(|_| ())
}

fn validate_atom(atom: &Atom, unit: Option<&str>, ordered: bool) -> Result<()> {
    let valid = match atom {
        Atom::Decimal { scale, .. } => *scale <= 9 && unit.is_some(),
        Atom::Text { value } => !ordered && unit.is_none() && bounded(value, 256),
        Atom::Boolean { .. } => !ordered && unit.is_none(),
        Atom::SemverRelease { .. } => unit.is_none(),
    };
    if valid { Ok(()) } else { Err(invalid()) }
}

fn validate_constraint(constraint: &Constraint, unit: Option<&str>) -> Result<()> {
    match constraint {
        Constraint::Eq { value } | Constraint::Ne { value } => validate_atom(value, unit, false),
        Constraint::Lt { value }
        | Constraint::Lte { value }
        | Constraint::Gt { value }
        | Constraint::Gte { value } => validate_atom(value, unit, true),
        Constraint::Between {
            lower,
            upper,
            lower_inclusive,
            upper_inclusive,
        } => {
            validate_atom(lower, unit, true)?;
            validate_atom(upper, unit, true)?;
            let lower = normalized(lower, unit).ok_or_else(invalid)?;
            let upper = normalized(upper, unit).ok_or_else(invalid)?;
            if !same_axis(&lower, &upper)
                || lower > upper
                || (lower == upper && !(*lower_inclusive && *upper_inclusive))
            {
                return Err(invalid());
            }
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Normalized {
    Decimal(Dimension, i128),
    Text(String),
    Boolean(bool),
    Version([u32; 3]),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Dimension {
    Time,
    Bytes,
    Count,
    Other(String),
}

fn normalized(atom: &Atom, unit: Option<&str>) -> Option<Normalized> {
    Some(match atom {
        Atom::Decimal { coefficient, scale } => {
            let power = 9_u8.checked_sub(*scale)?;
            let (axis, factor) = match unit? {
                "ns" => (Dimension::Time, 1_i128),
                "us" => (Dimension::Time, 1000),
                "ms" => (Dimension::Time, 1_000_000),
                "s" => (Dimension::Time, 1_000_000_000),
                "byte" | "bytes" => (Dimension::Bytes, 1),
                "KiB" => (Dimension::Bytes, 1024),
                "MiB" => (Dimension::Bytes, 1_048_576),
                "count" | "attempt" => (Dimension::Count, 1),
                other => (Dimension::Other(other.to_owned()), 1),
            };
            let amount = i128::from(*coefficient)
                .checked_mul(10_i128.pow(u32::from(power)))?
                .checked_mul(factor)?;
            Normalized::Decimal(axis, amount)
        }
        Atom::Text { value } => Normalized::Text(value.clone()),
        Atom::Boolean { value } => Normalized::Boolean(*value),
        Atom::SemverRelease {
            major,
            minor,
            patch,
        } => Normalized::Version([*major, *minor, *patch]),
    })
}

fn same_axis(left: &Normalized, right: &Normalized) -> bool {
    match (left, right) {
        (Normalized::Decimal(left, _), Normalized::Decimal(right, _)) => left == right,
        (Normalized::Text(_), Normalized::Text(_))
        | (Normalized::Boolean(_), Normalized::Boolean(_))
        | (Normalized::Version(_), Normalized::Version(_)) => true,
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Bound {
    value: Normalized,
    inclusive: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum ValueSet {
    Interval {
        lower: Option<Bound>,
        upper: Option<Bound>,
    },
    Except(Normalized),
}

fn value_set(constraint: &Constraint, unit: Option<&str>) -> Option<ValueSet> {
    let bound =
        |atom: &Atom, inclusive| normalized(atom, unit).map(|value| Bound { value, inclusive });
    Some(match constraint {
        Constraint::Eq { value } => ValueSet::Interval {
            lower: Some(bound(value, true)?),
            upper: Some(bound(value, true)?),
        },
        Constraint::Ne { value } => ValueSet::Except(normalized(value, unit)?),
        Constraint::Lt { value } => ValueSet::Interval {
            lower: None,
            upper: Some(bound(value, false)?),
        },
        Constraint::Lte { value } => ValueSet::Interval {
            lower: None,
            upper: Some(bound(value, true)?),
        },
        Constraint::Gt { value } => ValueSet::Interval {
            lower: Some(bound(value, false)?),
            upper: None,
        },
        Constraint::Gte { value } => ValueSet::Interval {
            lower: Some(bound(value, true)?),
            upper: None,
        },
        Constraint::Between {
            lower,
            upper,
            lower_inclusive,
            upper_inclusive,
        } => ValueSet::Interval {
            lower: Some(bound(lower, *lower_inclusive)?),
            upper: Some(bound(upper, *upper_inclusive)?),
        },
    })
}

#[derive(PartialEq, Eq)]
enum Relation {
    Equal,
    Disjoint,
    Review,
}

fn before(upper: &Option<Bound>, lower: &Option<Bound>) -> bool {
    upper
        .as_ref()
        .zip(lower.as_ref())
        .is_some_and(|(upper, lower)| {
            upper.value < lower.value
                || (upper.value == lower.value && !(upper.inclusive && lower.inclusive))
        })
}

fn point(set: &ValueSet) -> Option<&Normalized> {
    if let ValueSet::Interval {
        lower: Some(lower),
        upper: Some(upper),
    } = set
        && lower == upper
        && lower.inclusive
    {
        return Some(&lower.value);
    }
    None
}

fn representative(set: &ValueSet) -> Option<&Normalized> {
    match set {
        ValueSet::Interval { lower, upper } => {
            lower.as_ref().or(upper.as_ref()).map(|bound| &bound.value)
        }
        ValueSet::Except(value) => Some(value),
    }
}

fn relation(left: &Claim, right: &Claim) -> Relation {
    let Some(left) = value_set(&left.constraint, left.unit.as_deref()) else {
        return Relation::Review;
    };
    let Some(right) = value_set(&right.constraint, right.unit.as_deref()) else {
        return Relation::Review;
    };
    if [&left, &right].iter().any(|set| {
        matches!(
            representative(set),
            Some(Normalized::Decimal(Dimension::Other(_), _))
        )
    }) {
        return Relation::Review;
    }
    if !representative(&left)
        .zip(representative(&right))
        .is_some_and(|(left, right)| same_axis(left, right))
    {
        return Relation::Review;
    }
    if left == right {
        return Relation::Equal;
    }
    let disjoint = match (&left, &right) {
        (
            ValueSet::Interval {
                lower: ll,
                upper: lu,
            },
            ValueSet::Interval {
                lower: rl,
                upper: ru,
            },
        ) => before(lu, rl) || before(ru, ll),
        (ValueSet::Except(value), other) | (other, ValueSet::Except(value)) => {
            point(other) == Some(value)
        }
    };
    if disjoint {
        Relation::Disjoint
    } else {
        Relation::Review
    }
}

fn applicable(left: &Claim, right: &Claim) -> bool {
    left.subject == right.subject
        && left.predicate == right.predicate
        && left.applicability.iter().all(|(key, value)| {
            right
                .applicability
                .get(key)
                .is_none_or(|other| other == value)
        })
}

fn covers_body(projection: &Projection, content: &KnowledgeRevisionContent) -> bool {
    let mut spans: Vec<_> = projection
        .claims
        .iter()
        .map(|claim| {
            (
                claim.evidence.start_byte as usize,
                claim.evidence.end_byte as usize,
            )
        })
        .collect();
    spans.sort_unstable();
    let mut end = 0;
    for (start, until) in spans {
        if start > end && !content.body_markdown[end..start].trim().is_empty() {
            return false;
        }
        end = end.max(until);
    }
    content.body_markdown[end..].trim().is_empty()
}

fn other_content(content: &KnowledgeRevisionContent) -> KnowledgeRevisionContent {
    let mut content = content.clone();
    if let Some(object) = content.metadata.as_object_mut() {
        object.remove(KEY);
    }
    content
}

pub(super) enum Comparison {
    Fallback,
    Separate,
    Matched(ConflictClassification, &'static str),
}

pub(super) fn compare(
    proposed: &KnowledgeRevisionContent,
    existing: &KnowledgeRevisionContent,
    at: DateTime<Utc>,
) -> Comparison {
    let (Ok(Some(left)), Ok(Some(right))) = (projection(proposed), projection(existing)) else {
        return Comparison::Fallback;
    };
    if proposed
        .valid_to
        .is_some_and(|end| end <= existing.valid_from)
        || existing
            .valid_to
            .is_some_and(|end| end <= proposed.valid_from)
    {
        return Comparison::Separate;
    }
    let mut matched_left = vec![false; left.claims.len()];
    let mut matched_right = vec![false; right.claims.len()];
    let mut contradiction = false;
    let mut uncertain = false;
    for (li, proposed_claim) in left.claims.iter().enumerate() {
        for (ri, existing_claim) in right.claims.iter().enumerate() {
            if !applicable(proposed_claim, existing_claim) {
                continue;
            }
            matched_left[li] = true;
            matched_right[ri] = true;
            if proposed_claim.assertion_status != AssertionStatus::Asserted
                || existing_claim.assertion_status != AssertionStatus::Asserted
            {
                uncertain = true;
                continue;
            }
            match relation(proposed_claim, existing_claim) {
                Relation::Disjoint => contradiction = true,
                Relation::Review => uncertain = true,
                Relation::Equal => {
                    uncertain |= proposed_claim.applicability != existing_claim.applicability
                }
            }
        }
    }
    if !matched_left.iter().any(|matched| *matched) {
        return if covers_body(&left, proposed) && covers_body(&right, existing) {
            Comparison::Separate
        } else {
            Comparison::Fallback
        };
    }
    if proposed.valid_from > at
        && existing.valid_from <= at
        && existing.valid_to.is_none_or(|end| at < end)
    {
        return Comparison::Matched(ConflictClassification::Transition, "claim_future_validity");
    }
    if proposed.valid_from > at
        || proposed.valid_to.is_some_and(|end| end <= at)
        || existing.valid_from > at
        || existing.valid_to.is_some_and(|end| end <= at)
    {
        uncertain = true;
    }
    if contradiction && !uncertain {
        return Comparison::Matched(
            ConflictClassification::Contradiction,
            "claim_values_disjoint",
        );
    }
    let agreement = !uncertain
        && !contradiction
        && matched_left.iter().all(|matched| *matched)
        && matched_right.iter().all(|matched| *matched)
        && proposed.valid_from == existing.valid_from
        && proposed.valid_to == existing.valid_to
        && covers_body(&left, proposed)
        && covers_body(&right, existing)
        && other_content(proposed) == other_content(existing);
    if agreement {
        Comparison::Matched(ConflictClassification::Support, "claim_agreement")
    } else {
        Comparison::Matched(
            ConflictClassification::Supersession,
            "claim_comparison_requires_review",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Sensitivity;
    use serde_json::{Value, json};

    fn content(body: &str, value: Value) -> KnowledgeRevisionContent {
        KnowledgeRevisionContent {
            title: "Source-backed claim".to_owned(),
            body_markdown: body.to_owned(),
            summary: body.to_owned(),
            tags: Vec::new(),
            sensitivity: Sensitivity::Internal,
            confidence_permille: 900,
            valid_from: DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
            valid_to: None,
            stale_after: None,
            verification_metadata: json!({}),
            metadata: json!({KEY: {"schema_version": 1, "claims": [{
                "subject": {"kind": "named", "key": "retry-worker"}, "predicate": "retry_cap",
                "assertion_status": "asserted", "applicability": {"environment": "production"},
                "constraint": {"operator": "eq", "value": value},
                "evidence": {"start_byte": 0, "end_byte": body.len()},
            }]}}),
        }
    }

    fn decimal(body: &str, coefficient: i64, unit: &str) -> KnowledgeRevisionContent {
        let mut content = content(
            body,
            json!({"kind": "decimal", "coefficient": coefficient, "scale": 0}),
        );
        content.metadata[KEY]["claims"][0]["unit"] = json!(unit);
        content
    }

    fn declaration(content: &mut KnowledgeRevisionContent) -> &mut Value {
        &mut content.metadata[KEY]["claims"][0]
    }

    fn class(
        proposed: &KnowledgeRevisionContent,
        existing: &KnowledgeRevisionContent,
    ) -> Option<ConflictClassification> {
        super::super::validate_knowledge_revision_content(proposed).unwrap();
        super::super::validate_knowledge_revision_content(existing).unwrap();
        super::super::classify_knowledge_match(
            super::super::KnowledgeType::Procedure,
            proposed,
            &super::super::knowledge_revision_content_hash(proposed),
            super::super::KnowledgeType::Procedure,
            existing,
            &super::super::knowledge_revision_content_hash(existing),
            DateTime::from_timestamp(1_800_000_000, 0).unwrap(),
        )
        .map(|matched| matched.classification)
    }

    #[test]
    fn values_versions_units_identifiers_and_comparators_retain_their_propositions() {
        let three = decimal("Retry at most 3 times.", 3, "count");
        let five = decimal("Retry at most 5 times.", 5, "count");
        assert_eq!(
            class(&five, &three),
            Some(ConflictClassification::Contradiction)
        );
        let old = content(
            "Release 1.9.0",
            json!({"kind": "semver_release", "major": 1, "minor": 9, "patch": 0}),
        );
        let new = content(
            "Release 1.10.0",
            json!({"kind": "semver_release", "major": 1, "minor": 10, "patch": 0}),
        );
        assert_eq!(
            class(&new, &old),
            Some(ConflictClassification::Contradiction)
        );
        assert_eq!(
            class(
                &decimal("Timeout 10 ms", 10, "ms"),
                &decimal("Timeout 10 s", 10, "s")
            ),
            Some(ConflictClassification::Contradiction)
        );
        let hundred = decimal("Cap exactly 100.", 100, "count");
        let mut below = decimal("Cap less than 100.", 100, "count");
        declaration(&mut below)["constraint"]["operator"] = json!("lt");
        assert_eq!(
            class(&hundred, &below),
            Some(ConflictClassification::Contradiction)
        );
        declaration(&mut below)["constraint"]["operator"] = json!("lte");
        assert_eq!(
            class(&hundred, &below),
            Some(ConflictClassification::Supersession)
        );
        for (left, right) in [("A", "a"), ("X", "Y")] {
            assert_eq!(
                class(
                    &content(left, json!({"kind":"text", "value":left})),
                    &content(right, json!({"kind":"text", "value":right}))
                ),
                Some(ConflictClassification::Contradiction)
            );
        }
        let positive = content("TLS is required.", json!({"kind":"boolean", "value":true}));
        let negative = content(
            "TLS is not required.",
            json!({"kind":"boolean", "value":false}),
        );
        assert_eq!(
            class(&positive, &negative),
            Some(ConflictClassification::Contradiction)
        );
        assert_eq!(three.body_markdown, "Retry at most 3 times.");
        assert_eq!(five.body_markdown, "Retry at most 5 times.");
    }

    #[test]
    fn applicability_predicates_status_and_time_determine_review_not_lexical_proximity() {
        let production = decimal("Retry cap 3 in production.", 3, "count");
        let mut development = decimal("Retry cap 5 in development.", 5, "count");
        declaration(&mut development)["applicability"]["environment"] = json!("development");
        assert_eq!(class(&development, &production), None);
        let mut deployment = production.clone();
        declaration(&mut deployment)["predicate"] = json!("deployed_version");
        let mut minimum = production.clone();
        declaration(&mut minimum)["predicate"] = json!("minimum_supported_version");
        assert_eq!(class(&deployment, &minimum), None);
        let mut proposal = production.clone();
        declaration(&mut proposal)["assertion_status"] = json!("proposed");
        assert_eq!(
            class(&proposal, &production),
            Some(ConflictClassification::Supersession)
        );
        let mut future = decimal("Retry cap 5 after rollout.", 5, "count");
        future.valid_from = DateTime::from_timestamp(1_900_000_000, 0).unwrap();
        assert_eq!(
            class(&future, &production),
            Some(ConflictClassification::Transition)
        );
        let mut history = production.clone();
        history.valid_to = Some(future.valid_from);
        assert_eq!(
            class(&future, &history),
            None,
            "half-open intervals do not overlap"
        );
    }

    #[test]
    fn equivalent_values_cannot_auto_support_changed_or_unparsed_evidence() {
        let existing = decimal("Keep the timeout at 10 ms.", 10, "ms");
        let mut equivalent = existing.clone();
        declaration(&mut equivalent)["unit"] = json!("s");
        declaration(&mut equivalent)["constraint"]["value"] =
            json!({"kind":"decimal", "coefficient":1, "scale":2});
        assert_eq!(
            class(&equivalent, &existing),
            Some(ConflictClassification::Support)
        );
        equivalent.summary = "Do not keep that timeout.".to_owned();
        assert_eq!(
            class(&equivalent, &existing),
            Some(ConflictClassification::Supersession)
        );
        let mut partial = existing.clone();
        declaration(&mut partial)["evidence"]["end_byte"] = json!(4);
        assert_eq!(
            class(&partial, &existing),
            Some(ConflictClassification::Supersession)
        );
        let unknown = decimal("Timeout uses 10 ticks.", 10, "time");
        assert_eq!(
            class(&unknown, &existing),
            Some(ConflictClassification::Supersession)
        );
        assert_ne!(
            super::super::knowledge_revision_content_hash(&equivalent),
            super::super::knowledge_revision_content_hash(&existing)
        );
    }

    #[test]
    fn malformed_bounds_and_fabricated_status_fail_at_the_revision_boundary() {
        let valid = decimal("π retry cap 3.", 3, "count");
        let mut invalid_cases = Vec::new();
        let mut bad = valid.clone();
        declaration(&mut bad)["assertion_status"] = json!("approved");
        invalid_cases.push(bad);
        let mut bad = valid.clone();
        declaration(&mut bad)["evidence"]["end_byte"] = json!(1);
        invalid_cases.push(bad);
        let mut bad = valid.clone();
        declaration(&mut bad)["constraint"]["value"]["scale"] = json!(10);
        invalid_cases.push(bad);
        let mut bad = valid.clone();
        declaration(&mut bad)["extra"] = json!(true);
        invalid_cases.push(bad);
        let mut bad = valid.clone();
        bad.metadata[KEY]["schema_version"] = json!(2);
        invalid_cases.push(bad);
        let mut bad = valid.clone();
        bad.metadata[KEY]["claims"] = json!(vec![declaration(&mut valid.clone()).clone(); 9]);
        invalid_cases.push(bad);
        let mut bad = valid.clone();
        declaration(&mut bad)
            .as_object_mut()
            .unwrap()
            .remove("unit");
        invalid_cases.push(bad);
        let mut bad = valid.clone();
        declaration(&mut bad)["constraint"] = json!({"operator":"between",
            "lower":{"kind":"decimal","coefficient":3,"scale":0}, "upper":{"kind":"decimal","coefficient":3,"scale":0},
            "lower_inclusive":true, "upper_inclusive":false});
        invalid_cases.push(bad);
        let mut bad = content("Flag", json!({"kind":"boolean","value":true}));
        declaration(&mut bad)["constraint"]["operator"] = json!("gt");
        invalid_cases.push(bad);
        let mut bad = content(
            "Version",
            json!({"kind":"semver_release","major":1,"minor":10,"patch":0,"prerelease":"rc1"}),
        );
        declaration(&mut bad)["predicate"] = json!("minimum_supported_version");
        invalid_cases.push(bad);
        for invalid in invalid_cases {
            assert!(super::super::validate_knowledge_revision_content(&invalid).is_err());
        }
    }
}
