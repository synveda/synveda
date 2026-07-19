//! The redaction scanner (MEM-2, ADR-0021): PII patterns and
//! gitleaks-derived secret rules, applied to observe payloads *before*
//! anything persists (seed §6).
//!
//! Rules are data — id, category, regex, optional redaction group,
//! optional validator — compiled once into a [`regex::RegexSet`]
//! prefilter plus per-rule regexes. Detection walks every JSON string
//! value (object keys are not scanned; structure is preserved), replaces
//! matched spans with `[REDACTED:<rule-id>]`, and reports findings as
//! rule id + category + count. The matched text itself appears in no
//! output of this module: not in findings, not in errors, not in traces —
//! that discipline is what makes the MEM-2 AC structural.
//!
//! Validators run in code on the candidate match (Luhn for cards, mod-97
//! for IBANs, Shannon entropy for keyword-anchored generic secrets — the
//! gitleaks keyword + regex + entropy discipline), so "16 digits" alone
//! is not a card and prose after `token:` is not a secret. The tech
//! plan's regex+ML split (§1.2) is honoured as a seam: a future ML/NER
//! pass joins behind [`Ruleset`] without moving the enforcement point.

use std::sync::LazyLock;

use regex::{Regex, RegexSet};
use serde::Serialize;
use synveda_types::{RedactionConfig, RedactionMode};

/// A finding's category — the axis pack configuration keys on
/// (ADR-0021 decision 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FindingCategory {
    /// A credential: key, token, password, private key.
    Secret,
    /// Personally identifiable information.
    Pii,
}

impl FindingCategory {
    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            FindingCategory::Secret => "secret",
            FindingCategory::Pii => "pii",
        }
    }
}

/// One rule's aggregated findings within one payload. Carries the rule
/// id and span count only — never the matched text (ADR-0021 decision 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// The rule that matched, e.g. `aws-access-key-id`.
    pub rule: &'static str,
    /// The rule's category.
    pub category: FindingCategory,
    /// How many spans the rule redacted across the payload.
    pub count: usize,
}

/// A scanned payload: the redacted value plus what was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanOutcome {
    /// The payload with every validated match replaced by
    /// `[REDACTED:<rule-id>]`. Identical to the input when nothing
    /// matched.
    pub payload: serde_json::Value,
    /// Aggregated findings, in ruleset order. Empty means clean.
    pub findings: Vec<Finding>,
}

impl ScanOutcome {
    /// The disposition the pack's config assigns this outcome
    /// (ADR-0021 decision 4): the strictest mode among the categories
    /// that triggered, `None` when the payload is clean. Redaction has
    /// already happened either way — the mode decides flow only.
    #[must_use]
    pub fn disposition(&self, config: &RedactionConfig) -> Option<RedactionMode> {
        self.findings
            .iter()
            .map(|finding| match finding.category {
                FindingCategory::Secret => config.secrets,
                FindingCategory::Pii => config.pii,
            })
            .max()
    }
}

/// Scans one observe payload: walks its string values, redacts every
/// validated match in place, and reports findings. The only allocation
/// on a clean payload is the moved-in value itself.
///
/// Synchronous CPU work: callers on an async runtime with large batches
/// wrap it in `spawn_blocking` (ADR-0021 decision 1). The span records
/// counts only — never content.
#[must_use]
#[tracing::instrument(name = "ingest.redaction.scan", skip_all, fields(redaction.findings))]
pub fn scan(payload: serde_json::Value) -> ScanOutcome {
    let ruleset = builtin();
    let mut payload = payload;
    let mut counts = vec![0usize; ruleset.rules.len()];
    walk(&mut payload, ruleset, &mut counts);
    let findings = ruleset
        .rules
        .iter()
        .zip(&counts)
        .filter(|(_, count)| **count > 0)
        .map(|(rule, count)| Finding {
            rule: rule.id,
            category: rule.category,
            count: *count,
        })
        .collect::<Vec<_>>();
    tracing::Span::current().record(
        "redaction.findings",
        findings
            .iter()
            .map(|finding: &Finding| finding.count)
            .sum::<usize>(),
    );
    ScanOutcome { payload, findings }
}

fn walk(value: &mut serde_json::Value, ruleset: &Ruleset, counts: &mut [usize]) {
    match value {
        serde_json::Value::String(text) => {
            if let Some(redacted) = ruleset.redact(text, counts) {
                *text = redacted;
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                walk(item, ruleset, counts);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values_mut() {
                walk(item, ruleset, counts);
            }
        }
        _ => {}
    }
}

/// A candidate validator: receives the would-be-redacted span, returns
/// whether it is a real finding.
type Validator = fn(&str) -> bool;

struct Rule {
    id: &'static str,
    category: FindingCategory,
    regex: Regex,
    /// The capture group to redact; 0 redacts the whole match. Group
    /// redaction keeps surrounding structure readable (`api_key =
    /// [REDACTED:generic-api-key]`, a URL keeping its host).
    group: usize,
    validator: Option<Validator>,
}

/// The compiled builtin ruleset. Compilation happens once per process;
/// a pattern that fails to compile is a defect in this file, caught by
/// this crate's tests before it can ship.
struct Ruleset {
    rules: Vec<Rule>,
    prefilter: RegexSet,
}

impl Ruleset {
    /// Redacts every validated match in `text`; returns the rebuilt
    /// string, or `None` when nothing matched. Increments `counts` per
    /// rule for accepted spans.
    fn redact(&self, text: &str, counts: &mut [usize]) -> Option<String> {
        let matched = self.prefilter.matches(text);
        if !matched.matched_any() {
            return None;
        }
        // Candidate spans from every matched rule, validated in code.
        let mut spans: Vec<(usize, usize, usize)> = Vec::new();
        for index in matched.iter() {
            let rule = &self.rules[index];
            for captures in rule.regex.captures_iter(text) {
                let Some(span) = captures.get(rule.group) else {
                    continue;
                };
                if rule
                    .validator
                    .is_none_or(|validator| validator(span.as_str()))
                {
                    spans.push((span.start(), span.end(), index));
                }
            }
        }
        if spans.is_empty() {
            return None;
        }
        // Overlaps resolve deterministically: earliest start wins, the
        // longer span on a tie — so a token inside a larger finding
        // (a JWT inside a pasted header) redacts once, as the larger.
        spans.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        let mut rebuilt = String::with_capacity(text.len());
        let mut cursor = 0usize;
        for (start, end, rule) in spans {
            if start < cursor {
                continue;
            }
            rebuilt.push_str(&text[cursor..start]);
            rebuilt.push_str("[REDACTED:");
            rebuilt.push_str(self.rules[rule].id);
            rebuilt.push(']');
            counts[rule] += 1;
            cursor = end;
        }
        rebuilt.push_str(&text[cursor..]);
        Some(rebuilt)
    }
}

fn builtin() -> &'static Ruleset {
    static RULESET: LazyLock<Ruleset> = LazyLock::new(compile_builtin);
    &RULESET
}

/// The rule table. Secret grammars are gitleaks-derived (MIT); PII is
/// deliberately conservative (validators over broad grammars). Larger/
/// more specific spans come first only for readability — overlap
/// resolution is positional, not ordinal.
fn compile_builtin() -> Ruleset {
    use FindingCategory::{Pii, Secret};
    let table: &[(&str, FindingCategory, &str, usize, Option<Validator>)] = &[
        (
            "private-key",
            Secret,
            r"-----BEGIN [A-Z ]*PRIVATE KEY(?: BLOCK)?-----[\s\S]*?(?:-----END [A-Z ]*PRIVATE KEY(?: BLOCK)?-----|\z)",
            0,
            None,
        ),
        (
            "aws-access-key-id",
            Secret,
            r"\b(?:AKIA|ASIA|ABIA|ACCA|A3T[A-Z0-9])[A-Z0-9]{16}\b",
            0,
            None,
        ),
        (
            "aws-secret-access-key",
            Secret,
            r#"(?i)aws[a-z0-9_ .:=\-]{0,25}?["']?([A-Za-z0-9/+=]{40})["']?"#,
            1,
            Some(high_entropy),
        ),
        (
            "github-token",
            Secret,
            r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36,255}\b",
            0,
            None,
        ),
        (
            "github-fine-grained-pat",
            Secret,
            r"\bgithub_pat_[0-9A-Za-z_]{82}\b",
            0,
            None,
        ),
        (
            "gitlab-pat",
            Secret,
            r"\bglpat-[0-9A-Za-z_\-]{20,64}\b",
            0,
            None,
        ),
        (
            "slack-token",
            Secret,
            r"\bxox[baprs]-[0-9A-Za-z\-]{10,72}",
            0,
            None,
        ),
        (
            "stripe-key",
            Secret,
            r"\b[rs]k_(?:live|test)_[0-9a-zA-Z]{10,99}\b",
            0,
            None,
        ),
        (
            "google-api-key",
            Secret,
            r"\bAIza[0-9A-Za-z_\-]{35}\b",
            0,
            None,
        ),
        (
            "openai-api-key",
            Secret,
            r"\bsk-[A-Za-z0-9_\-]*T3BlbkFJ[A-Za-z0-9_\-]{10,}",
            0,
            None,
        ),
        (
            "anthropic-api-key",
            Secret,
            r"\bsk-ant-[A-Za-z0-9_\-]{20,120}",
            0,
            None,
        ),
        (
            "jwt",
            Secret,
            r"\beyJ[A-Za-z0-9_\-]{8,}\.eyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{16,}",
            0,
            None,
        ),
        (
            "url-credentials",
            Secret,
            r"[a-zA-Z][a-zA-Z0-9+.\-]*://[^/\s:@]{1,64}:([^@\s/]{3,64})@",
            1,
            None,
        ),
        (
            "generic-api-key",
            Secret,
            r#"(?i)\b(?:api[_\-]?key|apikey|secret|token|passwd|password|credential|auth[_\-]?token|access[_\-]?key)\b[\s"':=]{1,10}["']?([A-Za-z0-9+/=_\-.]{16,80})["']?"#,
            1,
            Some(generic_secret_valid),
        ),
        (
            "payment-card",
            Pii,
            r"\b(?:\d[ \-]?){12,18}\d\b",
            0,
            Some(luhn_valid),
        ),
        (
            "iban",
            Pii,
            r"\b[A-Z]{2}\d{2}[A-Z0-9]{10,30}\b",
            0,
            Some(iban_valid),
        ),
        ("us-ssn", Pii, r"\b\d{3}-\d{2}-\d{4}\b", 0, Some(ssn_valid)),
        (
            "email",
            Pii,
            r"\b[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,63}\b",
            0,
            None,
        ),
        (
            "phone",
            Pii,
            r"(?:\+|\b00)[1-9]\d{0,2}[ .\-]?(?:\(\d{1,4}\)[ .\-]?)?\d{2,4}(?:[ .\-]?\d{2,4}){1,4}\b",
            0,
            Some(phone_valid),
        ),
    ];
    let rules: Vec<Rule> = table
        .iter()
        .map(|(id, category, pattern, group, validator)| Rule {
            id,
            category: *category,
            regex: Regex::new(pattern)
                .unwrap_or_else(|err| panic!("builtin rule {id} does not compile: {err}")),
            group: *group,
            validator: *validator,
        })
        .collect();
    let prefilter = RegexSet::new(table.iter().map(|(_, _, pattern, _, _)| *pattern))
        .expect("builtin ruleset compiles as a set");
    Ruleset { rules, prefilter }
}

// ── Validators ───────────────────────────────────────────────────────────────

/// Shannon entropy in bits per character — the gitleaks gate for
/// keyword-anchored generic matches. 3.5 admits random keys (hex ≈ 4,
/// base64 ≈ 5+) and rejects prose and identifiers (English ≈ 3).
fn high_entropy(candidate: &str) -> bool {
    let bytes = candidate.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let mut counts = [0usize; 256];
    for byte in bytes {
        counts[*byte as usize] += 1;
    }
    let len = bytes.len() as f64;
    let entropy: f64 = counts
        .iter()
        .filter(|count| **count > 0)
        .map(|count| {
            let p = *count as f64 / len;
            -p * p.log2()
        })
        .sum();
    entropy >= 3.5
}

/// The generic-rule gate: entropy alone is not enough — long snake_case
/// English ("temporarily_unavailable") clears 3.5 bits/char — so a
/// candidate must also mix digits and letters, the way minted keys do
/// and prose does not.
fn generic_secret_valid(candidate: &str) -> bool {
    candidate
        .chars()
        .any(|character| character.is_ascii_digit())
        && candidate
            .chars()
            .any(|character| character.is_ascii_alphabetic())
        && high_entropy(candidate)
}

/// Luhn check over the digits of a card candidate; separators stripped,
/// length constrained to real card grammars.
fn luhn_valid(candidate: &str) -> bool {
    let digits: Vec<u32> = candidate
        .chars()
        .filter_map(|character| character.to_digit(10))
        .collect();
    if !(13..=19).contains(&digits.len()) {
        return false;
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(position, digit)| {
            if position % 2 == 1 {
                let doubled = digit * 2;
                if doubled > 9 { doubled - 9 } else { doubled }
            } else {
                *digit
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

/// IBAN mod-97 (ISO 13616): rotate the first four characters to the end,
/// map letters to 10..35, and the number must be ≡ 1 (mod 97).
fn iban_valid(candidate: &str) -> bool {
    let rotated = candidate.chars().skip(4).chain(candidate.chars().take(4));
    let mut remainder: u64 = 0;
    for character in rotated {
        let value = match character {
            '0'..='9' => character as u64 - '0' as u64,
            'A'..='Z' => character as u64 - 'A' as u64 + 10,
            _ => return false,
        };
        remainder = if value < 10 {
            (remainder * 10 + value) % 97
        } else {
            (remainder * 100 + value) % 97
        };
    }
    remainder == 1
}

/// US SSN structural rules: area 001–899 excluding 666, group and serial
/// non-zero.
fn ssn_valid(candidate: &str) -> bool {
    let mut parts = candidate.split('-');
    let (Some(area), Some(group), Some(serial)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let (Ok(area), Ok(group), Ok(serial)) = (
        area.parse::<u16>(),
        group.parse::<u16>(),
        serial.parse::<u16>(),
    ) else {
        return false;
    };
    (1..=899).contains(&area) && area != 666 && group != 0 && serial != 0
}

/// E.164 bounds for an international-format candidate: 8–15 digits.
fn phone_valid(candidate: &str) -> bool {
    let digits = candidate.chars().filter(char::is_ascii_digit).count();
    (8..=15).contains(&digits)
}
