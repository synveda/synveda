//! Prompt vocabulary (seed §4.3, tech plan §2.3; PRMT-1, ADR-0049).
//!
//! A prompt is the first asset a human *writes* rather than one the
//! pipeline derives, and the two things that makes new are both here: the
//! name a consumer puts in its source code, and the variable schema that
//! decides whether a template can be rendered at all.
//!
//! # What this module decides and what it does not
//!
//! It describes a template and bounds it. It never authorises (that is
//! `PromptRead`/`PromptWrite` at the seam the caller crossed), it never
//! stores (that is `synveda_store::prompts`), and it never addresses (that
//! is `synveda_vedaflow::PromptAsset`, which hashes the canonical form).
//!
//! What it does own is the **substitution rule**, in one implementation
//! (ADR-0049 decision 12): a schema returned beside a template and checked
//! by nobody is a document, so [`PromptTemplate::validate`] refuses a
//! template whose placeholders and declared variables disagree, and
//! [`PromptTemplate::render`] refuses a missing required value and an
//! undeclared one rather than substituting an empty string.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// The most segments a prompt name may have: `a/b/c/d`.
///
/// A bound rather than a taste: the name is a channel tree's entry name
/// and a curator file's glob subject (ADR-0032), and a registry whose
/// paths nest without limit is a filesystem nobody reviews.
pub const MAX_NAME_SEGMENTS: usize = 4;

/// The longest one segment may be.
pub const MAX_SEGMENT_CHARS: usize = 64;

/// The longest a whole name may be — well under
/// `vedaflow_tree_entries.name`'s 255 and `vedaflow_refs.name`'s 200, so a
/// name that parses can always be stored.
pub const MAX_NAME_CHARS: usize = 128;

/// The longest template a prompt may carry.
///
/// `synveda_vedaflow::MAX_OBJECT_BYTES` is the hard bound; this one is the
/// product's, and it is smaller for the same reason `MAX_CHANNEL_MEMBERS`
/// is: a prompt is something two people read before it reaches a fleet.
pub const MAX_TEMPLATE_CHARS: usize = 32_768;

/// The longest a description may be — one line in a listing.
pub const MAX_DESCRIPTION_CHARS: usize = 512;

/// The most variables one template may declare.
pub const MAX_VARIABLES: usize = 64;

/// The longest a variable's default may be.
pub const MAX_DEFAULT_CHARS: usize = 4_096;

/// A prompt's name: the identifier a consumer writes in its source, and
/// the entry name its channel tree carries (ADR-0049 decision 3).
///
/// Path-shaped — `support/triage-reply` — which is ADR-0031's reserved "a
/// path for the authored asset types" and the first real paths ADR-0032's
/// curator glob was written to accept.
///
/// Lower-case by construction. A name is an identifier a person types from
/// memory, and two names differing only in case would be two prompts that
/// look like one; refusing at parse is cheaper than a support ticket.
///
/// **Deliberately not a uuid.** The identifier goes in a consumer's source
/// code, and the *same* name at a nearer scope is how a team overrides the
/// org's version (decision 8) — a unique id cannot express that.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct PromptName(String);

impl PromptName {
    /// The name as stored, hashed, and rendered.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Its segments, in order.
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }
}

impl fmt::Display for PromptName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for PromptName {
    type Err = Error;

    fn from_str(name: &str) -> Result<Self> {
        let invalid = |detail: &str| {
            Err(Error::Invalid {
                message: format!("prompt name {name:?} {detail}"),
            })
        };
        if name.is_empty() {
            return invalid("is empty");
        }
        if name.chars().count() > MAX_NAME_CHARS {
            return invalid(&format!("is longer than {MAX_NAME_CHARS} characters"));
        }
        let segments: Vec<&str> = name.split('/').collect();
        if segments.len() > MAX_NAME_SEGMENTS {
            return invalid(&format!("has more than {MAX_NAME_SEGMENTS} segments"));
        }
        for segment in &segments {
            if segment.is_empty() {
                return invalid("has an empty segment");
            }
            if segment.chars().count() > MAX_SEGMENT_CHARS {
                return invalid(&format!(
                    "has a segment longer than {MAX_SEGMENT_CHARS} characters"
                ));
            }
            let mut chars = segment.chars();
            let first = chars.next().expect("a non-empty segment has a first char");
            if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
                return invalid("has a segment not starting with a lower-case letter or digit");
            }
            if let Some(bad) = chars.find(|c| !matches!(c, 'a'..='z' | '0'..='9' | '-' | '_' | '.'))
            {
                return invalid(&format!(
                    "contains {bad:?}; segments are lower-case letters, digits, '-', '_' and '.'"
                ));
            }
            // `..` is refused outright rather than normalised: a name is a
            // key, never a filesystem path to walk, and the two readings
            // must not be allowed to look alike.
            if segment.contains("..") {
                return invalid("contains '..'");
            }
        }
        Ok(PromptName(name.to_owned()))
    }
}

impl<'de> Deserialize<'de> for PromptName {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

/// One declared variable of a template.
///
/// **A variable is required exactly when it has no default** (ADR-0049
/// decision 12). One rule rather than a `required` flag beside a default,
/// which would make two of its four combinations meaningless — a required
/// variable with a default nobody can reach, and an optional one without a
/// default that renders as silence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptVariable {
    /// The placeholder's name: `{{ subject }}` declares `subject`.
    pub name: String,
    /// What a caller should put here. Optional, and read by a person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The value used when a caller supplies none. Its presence is what
    /// makes the variable optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

impl PromptVariable {
    /// A required variable — one with no default.
    #[must_use]
    pub fn required(name: impl Into<String>) -> Self {
        PromptVariable {
            name: name.into(),
            description: None,
            default: None,
        }
    }

    /// Whether a render must be given a value for this variable.
    #[must_use]
    pub const fn is_required(&self) -> bool {
        self.default.is_none()
    }
}

/// A prompt template and the schema that makes it renderable.
///
/// The content half of the asset: what a scope publishes and what a
/// consumer is served. The governed fields around it — scope, owner,
/// sensitivity — live on the stored draft and inside the object address
/// (`synveda_vedaflow::PromptAsset`), because those are facts about the
/// asset rather than parts of the text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptTemplate {
    /// Its identifier.
    pub name: PromptName,
    /// One line, read in a listing and at review.
    pub description: String,
    /// The text, with `{{ name }}` placeholders.
    pub template: String,
    /// Every placeholder the template uses, declared.
    pub variables: Vec<PromptVariable>,
}

impl PromptTemplate {
    /// Refuses a template that could not be rendered, or could be rendered
    /// with a value nobody declared (ADR-0049 decisions 12 and 13).
    ///
    /// Four rules, each naming its offender:
    ///
    /// - every `{{` opens a placeholder and must close with `}}` on a
    ///   declared name — the strict reading, because the lenient one ships
    ///   `{{ user name }}` to a fleet as literal text;
    /// - every declared variable is used by at least one placeholder — a
    ///   declaration nothing reads is configuration every consumer fills
    ///   in for nothing;
    /// - variable names are unique and identifier-shaped; and
    /// - the sizes are bounded.
    pub fn validate(&self) -> Result<()> {
        let invalid = |message: String| Err(Error::Invalid { message });
        let chars = self.template.chars().count();
        if chars == 0 || chars > MAX_TEMPLATE_CHARS {
            return invalid(format!(
                "a prompt template must be 1..={MAX_TEMPLATE_CHARS} characters"
            ));
        }
        if self.description.chars().count() > MAX_DESCRIPTION_CHARS {
            return invalid(format!(
                "a prompt description must be at most {MAX_DESCRIPTION_CHARS} characters"
            ));
        }
        if self.variables.len() > MAX_VARIABLES {
            return invalid(format!(
                "a prompt may declare at most {MAX_VARIABLES} variables"
            ));
        }
        let mut declared: Vec<&str> = Vec::with_capacity(self.variables.len());
        for variable in &self.variables {
            validate_variable_name(&variable.name)?;
            if declared.contains(&variable.name.as_str()) {
                return invalid(format!(
                    "variable {:?} is declared twice; a second declaration \
                     would silently win",
                    variable.name
                ));
            }
            if let Some(default) = &variable.default
                && default.chars().count() > MAX_DEFAULT_CHARS
            {
                return invalid(format!(
                    "variable {:?}'s default is longer than {MAX_DEFAULT_CHARS} characters",
                    variable.name
                ));
            }
            if let Some(description) = &variable.description
                && description.chars().count() > MAX_DESCRIPTION_CHARS
            {
                return invalid(format!(
                    "variable {:?}'s description is longer than \
                     {MAX_DESCRIPTION_CHARS} characters",
                    variable.name
                ));
            }
            declared.push(&variable.name);
        }

        let used = self.placeholders()?;
        for placeholder in &used {
            if !declared.contains(placeholder) {
                return invalid(format!(
                    "the template uses {{{{ {placeholder} }}}}, which no variable \
                     declares; a consumer cannot supply what the schema does not name"
                ));
            }
        }
        for name in &declared {
            if !used.contains(name) {
                return invalid(format!(
                    "variable {name:?} is declared but the template never uses it; \
                     every consumer would fill it in for nothing"
                ));
            }
        }
        Ok(())
    }

    /// The placeholder names the template uses, in first-use order.
    ///
    /// Every `{{` must close on an identifier: an unclosed or malformed
    /// one is an error here rather than literal text downstream
    /// (decision 13).
    pub fn placeholders(&self) -> Result<Vec<&str>> {
        let mut found: Vec<&str> = Vec::new();
        let bytes = self.template.as_bytes();
        let mut index = 0_usize;
        while let Some(offset) = self.template[index..].find("{{") {
            let open = index + offset;
            let after = open + 2;
            let Some(close_offset) = self.template[after..].find("}}") else {
                return Err(Error::Invalid {
                    message: format!(
                        "the template has a {{{{ at byte {open} that never closes; \
                         every {{{{ opens a placeholder (ADR-0049 decision 13)"
                    ),
                });
            };
            let close = after + close_offset;
            let inner = self.template[after..close].trim();
            validate_variable_name(inner).map_err(|_| Error::Invalid {
                message: format!(
                    "{{{{{}}}}} at byte {open} is not a placeholder; every {{{{ opens \
                     one, and its name is lower-case letters, digits and '_' \
                     (ADR-0049 decision 13)",
                    &self.template[after..close]
                ),
            })?;
            if !found.contains(&inner) {
                found.push(inner);
            }
            index = close + 2;
            debug_assert!(index <= bytes.len());
        }
        Ok(found)
    }

    /// Substitutes `values` into the template.
    ///
    /// Refuses rather than guesses: a required variable with no value is an
    /// error naming it, and a value the schema does not declare is an error
    /// too — a caller passing `subjct` has made a mistake that an ignored
    /// key would hide until someone read the model's output.
    ///
    /// Values are substituted literally and never re-scanned, so a value
    /// containing `{{ other }}` is text rather than a second round.
    pub fn render(&self, values: &BTreeMap<String, String>) -> Result<String> {
        for key in values.keys() {
            if !self.variables.iter().any(|variable| &variable.name == key) {
                return Err(Error::Invalid {
                    message: format!(
                        "{} declares no variable {key:?}; the values a render \
                         supplies must be ones the schema names",
                        self.name
                    ),
                });
            }
        }
        let mut out = String::with_capacity(self.template.len());
        let mut index = 0_usize;
        while let Some(offset) = self.template[index..].find("{{") {
            let open = index + offset;
            out.push_str(&self.template[index..open]);
            let after = open + 2;
            let Some(close_offset) = self.template[after..].find("}}") else {
                return Err(Error::Invalid {
                    message: format!("{} has a {{{{ that never closes", self.name),
                });
            };
            let close = after + close_offset;
            let name = self.template[after..close].trim();
            let variable = self
                .variables
                .iter()
                .find(|variable| variable.name == name)
                .ok_or_else(|| Error::Invalid {
                    message: format!("{} uses undeclared {{{{ {name} }}}}", self.name),
                })?;
            let value = match (values.get(name), &variable.default) {
                (Some(value), _) => value.as_str(),
                (None, Some(default)) => default.as_str(),
                (None, None) => {
                    return Err(Error::Invalid {
                        message: format!(
                            "{} requires a value for {{{{ {name} }}}}, which has no default",
                            self.name
                        ),
                    });
                }
            };
            out.push_str(value);
            index = close + 2;
        }
        out.push_str(&self.template[index..]);
        Ok(out)
    }
}

/// A variable name: lower snake, identifier-shaped, so a placeholder can
/// never be mistaken for prose.
fn validate_variable_name(name: &str) -> Result<()> {
    let invalid = |detail: &str| {
        Err(Error::Invalid {
            message: format!("variable name {name:?} {detail}"),
        })
    };
    if name.is_empty() {
        return invalid("is empty");
    }
    if name.chars().count() > MAX_SEGMENT_CHARS {
        return invalid(&format!("is longer than {MAX_SEGMENT_CHARS} characters"));
    }
    let mut chars = name.chars();
    let first = chars.next().expect("a non-empty name has a first char");
    if !first.is_ascii_lowercase() {
        return invalid("does not start with a lower-case letter");
    }
    if let Some(bad) = chars.find(|c| !matches!(c, 'a'..='z' | '0'..='9' | '_')) {
        return invalid(&format!(
            "contains {bad:?}; names are lower-case letters, digits and '_'"
        ));
    }
    Ok(())
}

/// Which version of a prompt a caller is asking for (ADR-0049 decision 2).
///
/// Deliberately **not** [`crate::Channel`]. `published` is a channel;
/// `draft` is a row, because a set channel cannot express withdrawal
/// (ADR-0032 decision 2) and an author replacing a draft is exactly the
/// withdrawal it cannot express. Spelling that as a channel on the wire
/// would name a ref no scope has.
///
/// There is no `Default` on the type — the *route* defaults to `published`,
/// which is a decision about a surface rather than about the vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PromptChannel {
    /// The authored working copy at one named scope. Never walked to:
    /// unreviewed content reaches a caller who asked for that scope's
    /// unreviewed content by name, and nobody else (decision 15).
    Draft,
    /// The reviewed version its scope stands behind — what a consumer gets
    /// when it asks for nothing in particular.
    Published,
}

impl PromptChannel {
    /// Both values.
    pub const ALL: [PromptChannel; 2] = [PromptChannel::Draft, PromptChannel::Published];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            PromptChannel::Draft => "draft",
            PromptChannel::Published => "published",
        }
    }
}

impl fmt::Display for PromptChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PromptChannel {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        PromptChannel::ALL
            .into_iter()
            .find(|channel| channel.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown prompt channel: {s:?}"),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(text: &str, variables: &[&str]) -> PromptTemplate {
        PromptTemplate {
            name: "support/triage".parse().unwrap(),
            description: "triage reply".to_owned(),
            template: text.to_owned(),
            variables: variables
                .iter()
                .map(|name| PromptVariable::required(*name))
                .collect(),
        }
    }

    #[test]
    fn names_are_paths_and_round_trip() {
        for name in [
            "house-style",
            "support/triage-reply",
            "a/b/c/d",
            "eng/api.v2/review",
            "p0",
        ] {
            let parsed: PromptName = name.parse().unwrap();
            assert_eq!(parsed.as_str(), name);
            assert_eq!(parsed.to_string(), name);
        }
        assert_eq!(
            "support/triage-reply"
                .parse::<PromptName>()
                .unwrap()
                .segments()
                .collect::<Vec<_>>(),
            vec!["support", "triage-reply"]
        );
    }

    /// A name is a key a person types, not a path to walk and not a
    /// display string: the refusals are what keep two names that look
    /// alike from being two prompts.
    #[test]
    fn names_that_are_not_identifiers_are_refused() {
        for name in [
            "",
            "/leading",
            "trailing/",
            "double//slash",
            "House-Style",
            "with space",
            "a/b/c/d/e",
            "../escape",
            "-leading-dash",
            "unicode-é",
        ] {
            assert!(
                name.parse::<PromptName>().is_err(),
                "{name:?} must not parse as a prompt name"
            );
        }
        let long = "x".repeat(MAX_SEGMENT_CHARS + 1);
        assert!(long.parse::<PromptName>().is_err());
    }

    /// Every name that parses fits the columns it is stored in — the tree
    /// entry (255) and, through a curator glob, a ref name (200).
    #[test]
    fn every_parseable_name_fits_the_schema() {
        // The segment bound alone would allow 4×64+3 = 259 characters, so
        // the whole-name bound is the one doing the work.
        let widest = std::iter::repeat_n("x".repeat(MAX_SEGMENT_CHARS), MAX_NAME_SEGMENTS)
            .collect::<Vec<_>>()
            .join("/");
        assert!(widest.parse::<PromptName>().is_err());

        let longest = format!(
            "{}/{}",
            "x".repeat(MAX_SEGMENT_CHARS),
            "x".repeat(MAX_NAME_CHARS - MAX_SEGMENT_CHARS - 1)
        );
        let parsed: PromptName = longest.parse().expect("the longest name still parses");
        assert_eq!(parsed.as_str().len(), MAX_NAME_CHARS);
        assert!(parsed.as_str().len() <= 200, "fits vedaflow_refs.name");
        assert!(parsed.as_str().len() <= 255, "fits a tree entry name");
    }

    #[test]
    fn placeholders_are_found_in_first_use_order() {
        let prompt = template(
            "Hi {{ name }}, about {{topic}} — {{ name }}",
            &["name", "topic"],
        );
        assert_eq!(prompt.placeholders().unwrap(), vec!["name", "topic"]);
        prompt.validate().unwrap();
    }

    /// Decision 13: the strict reading. The lenient one ships a typo to a
    /// fleet as literal text.
    #[test]
    fn every_open_brace_pair_must_close_on_a_declared_name() {
        let unclosed = template("Hi {{ name", &["name"]);
        let err = unclosed.placeholders().unwrap_err();
        assert!(
            format!("{err}").contains("never closes"),
            "unexpected: {err}"
        );

        let prose = template("send {{ user name }} the note", &["user"]);
        let err = prose.validate().unwrap_err();
        assert!(
            format!("{err}").contains("is not a placeholder"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn a_placeholder_no_variable_declares_is_refused() {
        let err = template("Hi {{ name }} of {{ team }}", &["name"])
            .validate()
            .unwrap_err();
        assert!(format!("{err}").contains("no variable declares"), "{err}");
    }

    #[test]
    fn a_variable_the_template_never_uses_is_refused() {
        let err = template("Hi {{ name }}", &["name", "team"])
            .validate()
            .unwrap_err();
        assert!(
            format!("{err}").contains("the template never uses it"),
            "{err}"
        );
    }

    #[test]
    fn a_variable_declared_twice_is_refused() {
        let mut prompt = template("Hi {{ name }}", &["name"]);
        prompt.variables.push(PromptVariable::required("name"));
        let err = prompt.validate().unwrap_err();
        assert!(format!("{err}").contains("declared twice"), "{err}");
    }

    #[test]
    fn rendering_substitutes_every_occurrence() {
        let prompt = template("Hi {{name}}, {{name}} — re {{ topic }}", &["name", "topic"]);
        let values = BTreeMap::from([
            ("name".to_owned(), "Bea".to_owned()),
            ("topic".to_owned(), "the outage".to_owned()),
        ]);
        assert_eq!(
            prompt.render(&values).unwrap(),
            "Hi Bea, Bea — re the outage"
        );
    }

    /// The two refusals that make the schema load-bearing rather than
    /// decorative (decision 12).
    #[test]
    fn rendering_refuses_a_missing_required_value_and_an_undeclared_one() {
        let prompt = template("Hi {{ name }}", &["name"]);
        let err = prompt.render(&BTreeMap::new()).unwrap_err();
        assert!(format!("{err}").contains("requires a value"), "{err}");

        let err = prompt
            .render(&BTreeMap::from([
                ("name".to_owned(), "Bea".to_owned()),
                ("nmae".to_owned(), "Bea".to_owned()),
            ]))
            .unwrap_err();
        assert!(format!("{err}").contains("declares no variable"), "{err}");
    }

    #[test]
    fn a_default_makes_a_variable_optional_and_nothing_else_does() {
        let mut prompt = template("Tone: {{ tone }}", &["tone"]);
        assert!(prompt.variables[0].is_required());
        assert!(prompt.render(&BTreeMap::new()).is_err());

        prompt.variables[0].default = Some("neutral".to_owned());
        assert!(!prompt.variables[0].is_required());
        assert_eq!(prompt.render(&BTreeMap::new()).unwrap(), "Tone: neutral");
        assert_eq!(
            prompt
                .render(&BTreeMap::from([("tone".to_owned(), "warm".to_owned())]))
                .unwrap(),
            "Tone: warm"
        );
    }

    /// A value is text. Re-scanning it would let a caller's input reach the
    /// substitution rule, which is the prompt-injection shape EVAL-5 gates
    /// against one layer up.
    #[test]
    fn a_substituted_value_is_never_re_scanned() {
        let prompt = template("{{ body }}", &["body"]);
        let values = BTreeMap::from([("body".to_owned(), "literal {{ body }}".to_owned())]);
        assert_eq!(prompt.render(&values).unwrap(), "literal {{ body }}");
    }

    #[test]
    fn prompt_channels_round_trip_and_are_not_channels() {
        for channel in PromptChannel::ALL {
            assert_eq!(
                channel.to_string().parse::<PromptChannel>().unwrap(),
                channel
            );
            assert_eq!(
                serde_json::to_string(&channel).unwrap(),
                format!("\"{}\"", channel.as_str())
            );
        }
        // The two vocabularies overlap on one word and differ on the rest,
        // which is the reason they are two types (decision 2).
        assert!("derived".parse::<PromptChannel>().is_err());
        assert!("staged".parse::<PromptChannel>().is_err());
        assert!("draft".parse::<crate::Channel>().is_err());
    }
}
