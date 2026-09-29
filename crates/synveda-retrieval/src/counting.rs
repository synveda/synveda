//! Counts the rendered Synveda text contribution (CTX-8, ADR-0118).
//!
//! The `o200k_base` data ships inside the already locked MIT-licensed
//! `tiktoken-rs` crate. A caller may identify that exact encoding; otherwise
//! its count is a reference estimate, not a provider or billable usage claim.

use std::fmt;
use std::sync::{Arc, OnceLock};

use synveda_types::{Error, Result};
use tiktoken_rs::CoreBPE;

/// Independent resource ceiling for the complete Synveda text contribution.
pub const MAX_RENDERED_CONTEXT_BYTES: usize = 2_000_000;

/// Meaning of a local rendered-text count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountMethod {
    /// The historical four-characters-per-token comparison path.
    LegacyEstimate,
    /// `o200k_base` is used as a reference, with no receiving-model claim.
    ReferenceEstimate,
    /// The caller explicitly identified `o200k_base` as its text encoding.
    ExactO200kBase,
}

impl CountMethod {
    /// Stable public description of count certainty.
    #[must_use]
    pub const fn certainty(self) -> &'static str {
        match self {
            Self::LegacyEstimate | Self::ReferenceEstimate => "estimated",
            Self::ExactO200kBase => "exact_encoding",
        }
    }

    /// Exact encoding or reference encoding used for the count.
    #[must_use]
    pub const fn encoding(self) -> Option<&'static str> {
        match self {
            Self::LegacyEstimate => None,
            Self::ReferenceEstimate | Self::ExactO200kBase => Some("o200k_base"),
        }
    }
}

/// One locally initialized counter, reusable across composition families.
#[derive(Clone)]
pub struct TokenCounter {
    method: CountMethod,
    encoder: Option<Arc<CoreBPE>>,
}

impl fmt::Debug for TokenCounter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TokenCounter")
            .field("method", &self.method)
            .finish_non_exhaustive()
    }
}

impl TokenCounter {
    /// Reproduces the pre-optimisation comparison count.
    #[must_use]
    pub const fn legacy() -> Self {
        Self {
            method: CountMethod::LegacyEstimate,
            encoder: None,
        }
    }

    /// Counts `o200k_base` text locally. `exact` states whether the caller
    /// explicitly identified that encoding, not whether its host prompt is
    /// fully accounted for.
    pub fn o200k(exact: bool) -> Result<Self> {
        static ENCODER: OnceLock<std::result::Result<Arc<CoreBPE>, String>> = OnceLock::new();
        let encoder = ENCODER.get_or_init(|| {
            tiktoken_rs::o200k_base()
                .map(Arc::new)
                .map_err(|error| error.to_string())
        });
        match encoder {
            Ok(encoder) => Ok(Self {
                method: if exact {
                    CountMethod::ExactO200kBase
                } else {
                    CountMethod::ReferenceEstimate
                },
                encoder: Some(Arc::clone(encoder)),
            }),
            Err(error) => Err(Error::Internal {
                message: format!("load the local o200k_base tokenizer: {error}"),
            }),
        }
    }

    /// Describes the count retained with a ContextRun.
    #[must_use]
    pub const fn method(&self) -> CountMethod {
        self.method
    }

    /// Counts the supplied exact string, with no per-component summation.
    /// It excludes provider role framing, history, tools outside this block
    /// and output reservation.
    #[must_use]
    pub fn count(&self, text: &str) -> u32 {
        match &self.encoder {
            Some(encoder) => u32::try_from(encoder.encode_ordinary(text).len()).unwrap_or(u32::MAX),
            None => u32::try_from(text.chars().count().div_ceil(4)).unwrap_or(u32::MAX),
        }
    }

    /// Rejects excessively large input independently of the token estimate.
    pub fn check_bytes(&self, text: &str) -> Result<()> {
        if text.len() > MAX_RENDERED_CONTEXT_BYTES {
            return Err(Error::Invalid {
                message: format!(
                    "rendered context exceeds the {}-byte resource limit",
                    MAX_RENDERED_CONTEXT_BYTES
                ),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_encoding_is_distinct_from_an_unknown_model_estimate() {
        let exact = TokenCounter::o200k(true).expect("embedded tokenizer");
        let reference = TokenCounter::o200k(false).expect("embedded tokenizer");
        for text in [
            "Hello world",
            "fn main() {\n    println!(\"✅\");\n}",
            "東京とالعربية",
        ] {
            assert_eq!(exact.count(text), reference.count(text));
        }
        assert_eq!(exact.method().certainty(), "exact_encoding");
        assert_eq!(reference.method().certainty(), "estimated");
        assert_eq!(reference.method().encoding(), Some("o200k_base"));
        assert_eq!(TokenCounter::legacy().method().encoding(), None);
    }

    #[test]
    fn concatenated_representation_is_counted_as_one_string() {
        let counter = TokenCounter::o200k(true).expect("embedded tokenizer");
        let rendered = "{\"body_markdown\":\"a\\nb\"}";
        assert_eq!(
            counter.count(rendered),
            counter.count(&format!("{}{}", "{\"body_", "markdown\":\"a\\nb\"}"))
        );
        assert!(counter.count("東京とالعربية") > 0);
    }

    #[test]
    fn byte_ceiling_is_independent() {
        let counter = TokenCounter::legacy();
        assert!(
            counter
                .check_bytes(&"x".repeat(MAX_RENDERED_CONTEXT_BYTES))
                .is_ok()
        );
        assert!(
            counter
                .check_bytes(&"x".repeat(MAX_RENDERED_CONTEXT_BYTES + 1))
                .is_err()
        );
    }
}
