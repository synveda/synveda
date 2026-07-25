//! MEM-2 scanner tests (ADR-0021): every builtin rule detects and
//! redacts its seeded fixture, validators reject look-alikes, the JSON
//! walk preserves structure, and no output of the scanner ever contains
//! the matched text.
//!
//! Fixture values are the well-known documentation examples (AWS's
//! `AKIAIOSFODNN7EXAMPLE`, the Luhn-valid test PAN `4111…`), never real
//! credentials.

use serde_json::{Value, json};
use synveda_ingest::{FindingCategory, scan};
use synveda_types::{RedactionConfig, RedactionMode};

/// Scans a one-string payload and returns (redacted text, findings).
fn scan_text(text: &str) -> (String, Vec<(String, usize)>) {
    let outcome = scan(json!({ "text": text }));
    let redacted = outcome.payload["text"]
        .as_str()
        .expect("string survives as a string")
        .to_owned();
    let findings = outcome
        .findings
        .iter()
        .map(|finding| (finding.rule.to_owned(), finding.count))
        .collect();
    (redacted, findings)
}

fn assert_redacts(rule: &str, text: &str, secret_span: &str) {
    let (redacted, findings) = scan_text(text);
    assert!(
        !redacted.contains(secret_span),
        "{rule}: the matched text must not survive: {redacted:?}"
    );
    assert!(
        redacted.contains(&format!("[REDACTED:{rule}]")),
        "{rule}: the placeholder must name the rule: {redacted:?}"
    );
    assert!(
        findings.iter().any(|(id, count)| id == rule && *count >= 1),
        "{rule}: the finding must be reported: {findings:?}"
    );
}

// ── Secret rules ─────────────────────────────────────────────────────────────

#[test]
fn secret_rules_detect_their_grammars() {
    assert_redacts(
        "aws-access-key-id",
        "creds: AKIAIOSFODNN7EXAMPLE ok",
        "AKIAIOSFODNN7EXAMPLE",
    );
    assert_redacts(
        "aws-secret-access-key",
        r#"aws_secret_access_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY""#,
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
    );
    assert_redacts(
        "github-token",
        "export GH=ghp_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789",
        "ghp_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789",
    );
    assert_redacts(
        "github-fine-grained-pat",
        &format!("token github_pat_{}", "a1B2".repeat(20) + "cd"),
        "github_pat_",
    );
    assert_redacts(
        "gitlab-pat",
        &format!("glpat-{}", "AbCd1234EfGh5678IjKl"),
        &format!("glpat-{}", "AbCd1234EfGh5678IjKl"),
    );
    assert_redacts(
        "slack-token",
        &format!("hook uses xoxb-{}", "123456789012-AbCdEfGhIjKlMnOp"),
        &format!("xoxb-{}", "123456789012-AbCdEfGhIjKlMnOp"),
    );
    assert_redacts(
        "stripe-key",
        "sk_live_AbCdEf123456GhIjKl",
        "sk_live_AbCdEf123456GhIjKl",
    );
    assert_redacts(
        "google-api-key",
        "key=AIzaSyA1bC2dE3fG4hI5jK6lM7nO8pQ9rS0tU1v",
        "AIzaSyA1bC2dE3fG4hI5jK6lM7nO8pQ9rS0tU1v",
    );
    assert_redacts(
        "openai-api-key",
        &format!(
            "sk-proj-{}{}",
            "AbCdEfGhIjKlMnOpQrStT3Blbk", "FJUvWxYz0123456789Ab"
        ),
        "T3BlbkFJ",
    );
    assert_redacts(
        "anthropic-api-key",
        "ANTHROPIC_API_KEY=sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWxYz012345",
        "sk-ant-api03",
    );
    assert_redacts(
        "jwt",
        "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJhbGljZSJ9.dGVzdHNpZ25hdHVyZXZhbHVl",
        "eyJhbGciOiJIUzI1NiJ9",
    );
    assert_redacts(
        "url-credentials",
        "postgres://synveda:sup3r-s3cret-pw@localhost:5432/db",
        "sup3r-s3cret-pw",
    );
    assert_redacts(
        "generic-api-key",
        "api_key: q7GX2mV9pLw4tZkR8nHj5cYd",
        "q7GX2mV9pLw4tZkR8nHj5cYd",
    );
}

#[test]
fn private_key_blocks_redact_including_unterminated() {
    let block = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA\n-----END RSA PRIVATE KEY-----";
    assert_redacts(
        "private-key",
        &format!("cfg:\n{block}\ndone"),
        "MIIEowIBAAKCAQEA",
    );
    // A truncated paste must still redact to the end of the string
    // rather than leak the tail.
    let (redacted, _) = scan_text("-----BEGIN PRIVATE KEY-----\nMIIEvAIBADANBg trailing");
    assert!(!redacted.contains("MIIEvAIBADANBg"), "{redacted:?}");
}

#[test]
fn url_credentials_keep_the_url_shape() {
    let (redacted, _) = scan_text("postgres://synveda:sup3r-s3cret-pw@localhost:5432/db");
    assert_eq!(
        redacted, "postgres://synveda:[REDACTED:url-credentials]@localhost:5432/db",
        "group redaction must preserve scheme, user, and host"
    );
}

#[test]
fn generic_api_key_requires_entropy_and_mixed_classes() {
    // Keyword present, but the value is prose: the gate (entropy +
    // digits-and-letters) must hold, or every sentence about tokens
    // gets mangled.
    for prose in [
        "the token: temporarily_unavailable today",
        "password requirements_document v2 attached",
        "secret: 1234567890123456 is the order number",
    ] {
        let (redacted, findings) = scan_text(prose);
        assert_eq!(redacted, prose);
        assert!(findings.is_empty(), "{prose:?} → {findings:?}");
    }
}

// ── PII rules ────────────────────────────────────────────────────────────────

#[test]
fn pii_rules_detect_their_grammars() {
    assert_redacts(
        "email",
        "contact alice.smith@example.com please",
        "alice.smith@example.com",
    );
    assert_redacts(
        "payment-card",
        "card 4111 1111 1111 1111 exp 12/28",
        "4111 1111 1111 1111",
    );
    assert_redacts("us-ssn", "ssn 536-90-4399 on file", "536-90-4399");
    // GB82 WEST 1234 5698 7654 32 is the ISO 13616 example, compacted.
    assert_redacts(
        "iban",
        "pay GB82WEST12345698765432 now",
        "GB82WEST12345698765432",
    );
    assert_redacts("phone", "call +1 415 555 2671 tomorrow", "+1 415 555 2671");
}

#[test]
fn pii_validators_reject_look_alikes() {
    // 16 digits failing Luhn: an order id, not a card.
    let (redacted, findings) = scan_text("order 4111 1111 1111 1112 shipped");
    assert_eq!(redacted, "order 4111 1111 1111 1112 shipped");
    assert!(findings.is_empty(), "{findings:?}");
    // An IBAN-shaped string with a wrong checksum.
    let (_, findings) = scan_text("ref GB00WEST12345698765432");
    assert!(findings.is_empty(), "{findings:?}");
    // 666 area SSNs are never issued.
    let (_, findings) = scan_text("id 666-12-3456");
    assert!(findings.is_empty(), "{findings:?}");
    // A + followed by too many digits is not E.164.
    let (_, findings) = scan_text("checksum +12345678901234567890");
    assert!(findings.is_empty(), "{findings:?}");
}

// ── Walk & structure ─────────────────────────────────────────────────────────

#[test]
fn clean_payloads_come_back_identical() {
    let payload = json!({
        "text": "refactored the tenant resolver; tests green",
        "nested": {"list": [1, 2, {"note": "plain prose"}], "flag": true},
    });
    let outcome = scan(payload.clone());
    assert_eq!(outcome.payload, payload);
    assert!(outcome.findings.is_empty());
}

#[test]
fn nested_strings_are_scanned_and_structure_preserved() {
    let outcome = scan(json!({
        "turns": [
            {"role": "user", "content": "my key is AKIAIOSFODNN7EXAMPLE"},
            {"role": "assistant", "content": ["ok", "email bob@example.org noted"]},
        ],
        "count": 2,
    }));
    let turns = outcome.payload["turns"].as_array().expect("array survives");
    assert_eq!(
        turns[0]["content"],
        "my key is [REDACTED:aws-access-key-id]"
    );
    assert_eq!(turns[1]["content"][1], "email [REDACTED:email] noted");
    assert_eq!(outcome.payload["count"], 2);
    assert_eq!(outcome.findings.len(), 2);
}

#[test]
fn multiple_hits_aggregate_per_rule() {
    let outcome = scan(json!({
        "a": "AKIAIOSFODNN7EXAMPLE and AKIAIOSFODNN7EXAMPLF",
        "b": "also alice@example.com",
    }));
    let aws = outcome
        .findings
        .iter()
        .find(|finding| finding.rule == "aws-access-key-id")
        .expect("aws finding");
    assert_eq!(aws.count, 2);
    assert_eq!(aws.category, FindingCategory::Secret);
}

#[test]
fn overlapping_candidates_redact_once_deterministically() {
    // The JWT is also plausible generic-api-key material after
    // "token:"; the span merge must produce exactly one placeholder.
    let (redacted, _) =
        scan_text("token: eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJhbGljZSJ9.dGVzdHNpZ25hdHVyZXZhbHVl");
    assert_eq!(redacted.matches("[REDACTED:").count(), 1, "{redacted:?}");
}

#[test]
fn scanner_output_never_contains_the_matched_text() {
    // The AC's core discipline, asserted at the unit level: serialise
    // everything the scanner returns and sweep for the seeds.
    let seeds = [
        "AKIAIOSFODNN7EXAMPLE",
        "ghp_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789",
        "4111 1111 1111 1111",
        "alice.smith@example.com",
    ];
    let outcome = scan(json!({
        "text": seeds.join(" and "),
    }));
    let everything = format!(
        "{} {}",
        serde_json::to_string(&outcome.payload).expect("payload serialises"),
        serde_json::to_string(&outcome.findings).expect("findings serialise"),
    );
    for seed in seeds {
        let compact = seed.replace(' ', "");
        assert!(
            !everything.contains(seed) && !everything.contains(&compact),
            "seed {seed:?} leaked into scanner output: {everything}"
        );
    }
}

// ── Disposition ──────────────────────────────────────────────────────────────

#[test]
fn disposition_is_the_strictest_triggered_mode() {
    let clean = scan(json!({"text": "nothing here"}));
    assert_eq!(clean.disposition(&RedactionConfig::STRICT), None);

    let pii_only = scan(json!({"text": "mail alice@example.com"}));
    assert_eq!(
        pii_only.disposition(&RedactionConfig::STRICT),
        Some(RedactionMode::Redact),
        "strict PII redacts on ingest (seed §6)"
    );

    let both = scan(json!({"text": "AKIAIOSFODNN7EXAMPLE for alice@example.com"}));
    assert_eq!(
        both.disposition(&RedactionConfig::STRICT),
        Some(RedactionMode::Quarantine),
        "the secret's quarantine outranks the PII's redact"
    );
    assert_eq!(
        both.disposition(&RedactionConfig::REDACT_ALL),
        Some(RedactionMode::Redact)
    );
    let deny_secrets = RedactionConfig {
        secrets: RedactionMode::Deny,
        pii: RedactionMode::Redact,
    };
    assert_eq!(both.disposition(&deny_secrets), Some(RedactionMode::Deny));
}

// ── Robustness ───────────────────────────────────────────────────────────────

#[test]
fn deeply_nested_payloads_do_not_overflow() {
    // serde_json's default recursion limit (128) bounds parsed depth;
    // the walk must survive a value built at that bound.
    let mut value = Value::String("AKIAIOSFODNN7EXAMPLE".to_owned());
    for _ in 0..127 {
        value = json!([value]);
    }
    let outcome = scan(value);
    assert_eq!(outcome.findings.len(), 1);
}
