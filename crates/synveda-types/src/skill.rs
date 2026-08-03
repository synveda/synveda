//! Skill vocabulary (seed §4.3, tech plan §2.3; SKIL-1, ADR-0051).
//!
//! A skill is the third asset a human *writes* rather than one the pipeline
//! derives, and the first whose **format belongs to somebody else**. A
//! memory record, a prompt template and a pack document are shapes this
//! product chose and only this product reads; a skill's bytes are read by
//! clients this product does not ship, against the agentskills.io open
//! standard, and the acceptance criterion is a third party's loader
//! accepting them unmodified.
//!
//! Everything new lives here because of that inversion:
//!
//! - [`SkillName`] carries the **spec's** grammar, which is stricter than
//!   the product's own (no `_`, no `.`, one segment) — reusing
//!   [`crate::PromptName`]'s would admit at the first step what a client
//!   refuses at the last (ADR-0051 decision 6).
//! - [`SkillFilePath`] is validated against **filesystems** rather than
//!   taste, because a bundle becomes files on somebody's laptop and nothing
//!   in this product has ever done that (decision 7).
//! - [`Frontmatter`] is a **strict subset** of YAML rather than YAML,
//!   because the reviewed meaning and the loaded meaning must be the same
//!   meaning, and two parsers reading one document differently is exactly
//!   how that stops being true — silently (decision 4).
//!
//! # What this module decides and what it does not
//!
//! It describes a bundle and bounds it. It never authorises (that is
//! `SkillRead`/`SkillWrite` at the seam the caller crossed), it never
//! stores (that is `synveda_store::skills`), it never addresses (that is
//! `synveda_vedaflow::SkillAsset`), and it never writes a file (that is the
//! CLI — seed §2.6, the harness is a guest).

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// The file every skill must carry, spelled exactly.
///
/// A bundle without one is not a skill under the open spec, so a surface
/// that accepted one would ship an artefact no client will load
/// (ADR-0051 decision 3).
pub const SKILL_MANIFEST: &str = "SKILL.md";

/// The longest a skill name may be — the open spec's own bound, and also
/// the longest directory name an install creates.
pub const MAX_SKILL_NAME_CHARS: usize = 64;

/// The longest a `description` may be.
///
/// The spec's progressive disclosure loads it at every session (~80 tokens),
/// so this is generous rather than roomy: a description that needs more than
/// this is a `SKILL.md` body.
pub const MAX_SKILL_DESCRIPTION_CHARS: usize = 1_024;

/// The most files one bundle may hold.
pub const MAX_SKILL_FILES: usize = 64;

/// The longest one bundled path may be.
pub const MAX_SKILL_PATH_CHARS: usize = 128;

/// The most segments one bundled path may have: `references/api/v2/read.md`.
pub const MAX_SKILL_PATH_SEGMENTS: usize = 4;

/// The longest one path segment may be — under every filesystem's own 255.
pub const MAX_SKILL_PATH_SEGMENT_CHARS: usize = 64;

/// The longest one bundled file may be.
pub const MAX_SKILL_FILE_CHARS: usize = 65_536;

/// The longest a whole bundle may be, across every file.
///
/// `synveda_vedaflow::MAX_OBJECT_BYTES` bounds one object; this bounds the
/// thing a person reviews, and it is the reason an install is fast.
pub const MAX_SKILL_BUNDLE_CHARS: usize = 262_144;

/// The longest one frontmatter value may be.
pub const MAX_FRONTMATTER_VALUE_CHARS: usize = 4_096;

/// The most entries `allowed-tools` or `metadata` may carry.
pub const MAX_FRONTMATTER_ENTRIES: usize = 64;

/// Windows device names, which that platform resolves *before* looking at
/// the directory — so a file called `aux.py` is not a file there.
const RESERVED_STEMS: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// A skill's name: the identifier a consumer writes, the entry prefix its
/// channel tree carries, **and the directory an install creates**
/// (ADR-0051 decision 6).
///
/// One segment, lower-case ASCII letters and digits with `-` inside — the
/// agentskills.io grammar, which is deliberately stricter than
/// [`crate::PromptName`]'s and [`crate::ContextPackName`]'s: those allow `_`
/// and `.`, and a name this product accepted but a client refused would fail
/// the one criterion this feature exists to meet.
///
/// **Deliberately not a uuid**, for [`crate::PromptName`]'s reason and one
/// more: because the name is the installed directory name, a team's
/// `code-review` overriding the org's is a *filesystem* fact — a client's
/// skills root is flat, so only one of them can exist there at all, and
/// which one is ADR-0049 decision 8's nearest-first walk.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct SkillName(String);

impl SkillName {
    /// The name as stored, hashed, rendered, and written to disk.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SkillName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for SkillName {
    type Err = Error;

    fn from_str(name: &str) -> Result<Self> {
        let invalid = |detail: &str| {
            Err(Error::Invalid {
                message: format!(
                    "skill name {name:?} {detail}; the agentskills.io grammar is one segment \
                     of lower-case letters, digits and '-', at most \
                     {MAX_SKILL_NAME_CHARS} characters (ADR-0051 decision 6)"
                ),
            })
        };
        if name.is_empty() {
            return invalid("is empty");
        }
        if name.chars().count() > MAX_SKILL_NAME_CHARS {
            return invalid("is too long");
        }
        let mut chars = name.chars();
        let first = chars.next().expect("a non-empty name has a first char");
        if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
            return invalid("does not start with a lower-case letter or digit");
        }
        if let Some(bad) = chars.find(|c| !matches!(c, 'a'..='z' | '0'..='9' | '-')) {
            return invalid(&format!("contains {bad:?}"));
        }
        if name.ends_with('-') {
            return invalid("ends with '-'");
        }
        Ok(SkillName(name.to_owned()))
    }
}

impl<'de> Deserialize<'de> for SkillName {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

/// One file's path inside a bundle: `SKILL.md`, `scripts/check.py`,
/// `references/api.md`.
///
/// Validated against **filesystems**, not against taste (ADR-0051
/// decision 7). Every rule below is a way a materialisation could write
/// somewhere it was not asked to, or write two governed objects into one
/// file:
///
/// - no `.` or `..` segment, no absolute form, no backslash and no colon —
///   escape and drive-letter shapes;
/// - no control characters, and **ASCII only**: macOS normalises filenames
///   to NFD, so a name authored as NFC would come back off the disk as
///   different bytes and a re-hash would fail against the commit that
///   published it;
/// - no trailing dot or space in a segment — Windows strips both, silently
///   turning two paths into one;
/// - no reserved device stem (`con`, `nul`, `com1`…), which Windows
///   resolves before it looks at the directory.
///
/// Case folding is a *bundle* property rather than a path one, so it lives
/// in [`SkillBundle::validate`]: two paths are each fine and the pair is
/// not.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct SkillFilePath(String);

impl SkillFilePath {
    /// The path as stored, hashed, and joined onto a client's skills root.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Its segments, in order.
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }

    /// Whether this is the bundle's `SKILL.md`.
    #[must_use]
    pub fn is_manifest(&self) -> bool {
        self.0 == SKILL_MANIFEST
    }

    /// The key two paths collide on when a case-folding filesystem writes
    /// them (ADR-0051 decision 7).
    #[must_use]
    pub fn fold_key(&self) -> String {
        self.0.to_ascii_lowercase()
    }
}

impl fmt::Display for SkillFilePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for SkillFilePath {
    type Err = Error;

    fn from_str(path: &str) -> Result<Self> {
        let invalid = |detail: &str| {
            Err(Error::Invalid {
                message: format!("bundled path {path:?} {detail} (ADR-0051 decision 7)"),
            })
        };
        if path.is_empty() {
            return invalid("is empty");
        }
        if path.chars().count() > MAX_SKILL_PATH_CHARS {
            return invalid(&format!("is longer than {MAX_SKILL_PATH_CHARS} characters"));
        }
        let segments: Vec<&str> = path.split('/').collect();
        if segments.len() > MAX_SKILL_PATH_SEGMENTS {
            return invalid(&format!("has more than {MAX_SKILL_PATH_SEGMENTS} segments"));
        }
        for segment in &segments {
            if segment.is_empty() {
                return invalid("has an empty segment; it is relative and never absolute");
            }
            if segment.chars().count() > MAX_SKILL_PATH_SEGMENT_CHARS {
                return invalid(&format!(
                    "has a segment longer than {MAX_SKILL_PATH_SEGMENT_CHARS} characters"
                ));
            }
            if *segment == "." || *segment == ".." {
                return invalid("has a '.' or '..' segment, which walks out of the bundle");
            }
            if let Some(bad) = segment
                .chars()
                .find(|c| !c.is_ascii() || c.is_ascii_control())
            {
                // ASCII-only is a normalisation rule, not xenophobia about
                // filenames: macOS stores NFD, so a non-ASCII name would not
                // read back as the bytes that were published.
                return invalid(&format!(
                    "contains {bad:?}; bundled paths are printable ASCII, because a \
                     case-folding, unicode-normalising filesystem cannot return the bytes \
                     a commit published"
                ));
            }
            if segment.contains('\\') || segment.contains(':') {
                return invalid("contains '\\' or ':', which name a path on Windows");
            }
            if segment.ends_with('.') || segment.ends_with(' ') || segment.starts_with(' ') {
                return invalid(
                    "has a segment ending in '.' or padded with spaces; Windows strips both, which merges two paths into one file",
                );
            }
            let stem = segment.split('.').next().unwrap_or(segment);
            if RESERVED_STEMS.contains(&stem.to_ascii_lowercase().as_str()) {
                return invalid(&format!(
                    "has the reserved device stem {stem:?}, which Windows resolves before it \
                     looks at the directory"
                ));
            }
        }
        Ok(SkillFilePath(path.to_owned()))
    }
}

impl<'de> Deserialize<'de> for SkillFilePath {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

/// One file of a bundle: what an author uploads, what a reviewer reads, and
/// what an install writes byte for byte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillFile {
    /// Its path within the bundle.
    pub path: SkillFilePath,
    /// Its bytes, as text. A bundle is reviewed content, so it is UTF-8
    /// with no NUL — the cheapest honest test for "somebody uploaded a
    /// binary" (ADR-0051 option 9).
    pub content: String,
}

impl SkillFile {
    /// Refuses a file that could not be stored, reviewed or written.
    pub fn validate(&self) -> Result<()> {
        let chars = self.content.chars().count();
        if chars > MAX_SKILL_FILE_CHARS {
            return Err(Error::Invalid {
                message: format!(
                    "file {} is {chars} characters, over the {MAX_SKILL_FILE_CHARS} one \
                     bundled file may hold",
                    self.path
                ),
            });
        }
        if self.content.contains('\0') {
            return Err(Error::Invalid {
                message: format!(
                    "file {} contains a NUL byte, so it is not reviewable text; a skill \
                     bundle carries no binaries (ADR-0051 option 9)",
                    self.path
                ),
            });
        }
        Ok(())
    }
}

/// A whole bundle: the unit a client loads and the unit a reviewer approves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillBundle {
    /// The skill's name — the directory an install creates, and the name
    /// `SKILL.md`'s own frontmatter must agree with.
    pub name: SkillName,
    /// Its files, `SKILL.md` among them.
    pub files: Vec<SkillFile>,
}

impl SkillBundle {
    /// The bundle's `SKILL.md`, if it has one.
    #[must_use]
    pub fn manifest(&self) -> Option<&SkillFile> {
        self.files.iter().find(|file| file.path.is_manifest())
    }

    /// Refuses a bundle no client would load, and one a filesystem would
    /// mangle.
    ///
    /// The spec's own rules are checked here rather than left to a client
    /// (ADR-0051 decision 5): a validation this product skips is a refusal
    /// somebody else delivers to a user who has already published.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] naming the offending file, path or key.
    pub fn validate(&self) -> Result<Frontmatter> {
        let invalid = |message: String| Err(Error::Invalid { message });
        if self.files.len() > MAX_SKILL_FILES {
            return invalid(format!(
                "a skill bundle holds at most {MAX_SKILL_FILES} files, and this one names {}",
                self.files.len()
            ));
        }
        let mut total = 0_usize;
        let mut folded: BTreeMap<String, &SkillFilePath> = BTreeMap::new();
        for file in &self.files {
            file.validate()?;
            total += file.content.chars().count();
            if let Some(previous) = folded.insert(file.path.fold_key(), &file.path) {
                if previous == &file.path {
                    return invalid(format!("file {} is named twice", file.path));
                }
                // The one that is not obvious. Both paths are legal, both
                // are distinct objects, and a case-folding filesystem writes
                // them into one file whose contents are whichever the
                // installer happened to write last.
                return invalid(format!(
                    "files {previous} and {} differ only in case, so a case-folding \
                     filesystem (macOS, Windows) would write them into one file and keep \
                     whichever was installed last (ADR-0051 decision 7)",
                    file.path
                ));
            }
        }
        if total > MAX_SKILL_BUNDLE_CHARS {
            return invalid(format!(
                "the bundle is {total} characters, over the {MAX_SKILL_BUNDLE_CHARS} a skill \
                 may hold; split it into two skills"
            ));
        }
        let manifest = self.manifest().ok_or_else(|| Error::Invalid {
            message: format!(
                "the bundle has no {SKILL_MANIFEST}; without one it is not a skill under the \
                 open spec and no client will load it (ADR-0051 decision 3)"
            ),
        })?;
        let frontmatter = Frontmatter::parse(&manifest.content)?;
        if frontmatter.name != self.name.as_str() {
            return invalid(format!(
                "{SKILL_MANIFEST} declares name {:?} but the skill is {:?}; the open spec \
                 requires the frontmatter name to match the directory, which is also this \
                 registry's key (ADR-0051 decision 5)",
                frontmatter.name,
                self.name.as_str()
            ));
        }
        Ok(frontmatter)
    }
}
/// The keys the open standard itself defines.
///
/// Closed on purpose, and on this workspace's own precedent —
/// `deny_unknown_fields` on every wire format since EVAL-1. An unknown key
/// is not merely unread: it may change a client's behaviour (`allowed-tools`
/// is the obvious one) in a way no reviewer's tooling rendered.
const SPEC_KEYS: [&str; 7] = [
    "name",
    "description",
    "license",
    "version",
    "allowed-tools",
    "metadata",
    "user-invocable",
];

/// Keys real clients put in frontmatter that the spec does not define and
/// this product does not interpret — accepted, kept, and rendered at review
/// as [`Frontmatter::extra`], never silently dropped.
///
/// This list is ADR-0051 reversal trigger (a) discharged rather than
/// theorised: `tests/skill_corpus.rs` read 37 installed bundles on its first
/// run and the subset refused five of them, three for keys it had never
/// heard of. Widening is a one-line change **with a bundle to point at**,
/// which is the whole discipline — the alternative was a general YAML parser
/// (ADR-0051 option 4), and that is still refused.
const CLIENT_KEYS: [&str; 3] = ["tools", "argument-hint", "disable-model-invocation"];

/// A `SKILL.md`'s YAML frontmatter, as the strict subset reads it
/// (ADR-0051 decision 4).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Frontmatter {
    /// The skill's name. Required, and must equal the bundle's.
    pub name: String,
    /// What the skill is for. Required — it is what a client loads at every
    /// session, and what SKIL-4 will advertise.
    pub description: String,
    /// Its licence, if declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Its version, if declared. The product's version is the commit; this
    /// is the author's own label and is never interpreted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The tools the skill declares it needs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    /// Whether a user may invoke it directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_invocable: Option<bool>,
    /// The spec's extension slot, flattened to strings.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
    /// [`CLIENT_KEYS`] as they were written: parsed, so the document is
    /// unambiguous, and kept, so a review can show what a client will act
    /// on.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, String>,
}

/// One key's value as the block accumulates it, before it is matched onto a
/// field.
#[derive(Default)]
struct Pending {
    /// The text after `key:` on its own line, plus every folded
    /// continuation line beneath it.
    scalar: Vec<String>,
    /// `- item` lines.
    items: Vec<String>,
    /// `nested: value` lines.
    map: BTreeMap<String, String>,
}

impl Frontmatter {
    /// Parses a `SKILL.md`'s frontmatter, refusing everything the subset
    /// cannot represent.
    ///
    /// The refusal is safe in a way a permissive parser is not: because the
    /// bytes ship verbatim, **a construct this refuses is a construct nobody
    /// can author**, so no document exists in the product whose meaning two
    /// parsers could read differently.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] naming the line and what about it was refused.
    pub fn parse(content: &str) -> Result<Frontmatter> {
        let mut lines = content.split_inclusive('\n').enumerate();
        let invalid = |line: usize, detail: String| -> Error {
            Error::Invalid {
                message: format!("{SKILL_MANIFEST} frontmatter, line {line}: {detail}"),
            }
        };

        match lines.next() {
            Some((_, first)) if trim_line(first) == "---" => {}
            _ => {
                return Err(Error::Invalid {
                    message: format!(
                        "{SKILL_MANIFEST} does not open with a `---` frontmatter block; the \
                         open spec requires one carrying `name` and `description`"
                    ),
                });
            }
        }

        let mut fields: BTreeMap<String, Pending> = BTreeMap::new();
        let mut order: Vec<String> = Vec::new();
        let mut current: Option<String> = None;
        // Whether a blank or comment line has been skipped since the last
        // content line. It matters for exactly one shape: a folded scalar,
        // where YAML turns a blank line into a newline rather than a space.
        let mut broke = false;
        let mut closed = false;
        for (index, raw) in lines {
            let number = index + 1;
            let line = trim_line(raw);
            if line == "---" {
                closed = true;
                break;
            }
            if line.contains('\t') {
                return Err(invalid(
                    number,
                    "contains a tab; YAML indentation is spaces, and a tab means different \
                     things to different parsers"
                        .to_owned(),
                ));
            }
            let trimmed = line.trim_start_matches(' ');
            if trimmed.is_empty() || trimmed.starts_with('#') {
                broke = true;
                continue;
            }
            if trimmed.trim_end() != trimmed {
                return Err(invalid(
                    number,
                    "has trailing whitespace, which changes an unquoted value invisibly".to_owned(),
                ));
            }
            let indent = line.len() - trimmed.len();

            if indent == 0 {
                if trimmed.starts_with("- ") {
                    return Err(invalid(
                        number,
                        "starts a sequence at the top level; frontmatter is a mapping".to_owned(),
                    ));
                }
                let (key, rest) = trimmed
                    .split_once(':')
                    .ok_or_else(|| invalid(number, format!("is not `key: value`: {trimmed:?}")))?;
                let key = key.trim_end();
                if key == "<<" {
                    return Err(invalid(
                        number,
                        "is a merge key, whose precedence is implementation-defined".to_owned(),
                    ));
                }
                if !SPEC_KEYS.contains(&key) && !CLIENT_KEYS.contains(&key) {
                    return Err(invalid(
                        number,
                        format!(
                            "declares {key:?}, which the subset does not know. It reads {} \
                             from the open spec and {} from clients; anything else belongs \
                             under `metadata:`, or widens this list in a commit that names \
                             the bundle it was found in (ADR-0051 decision 4)",
                            SPEC_KEYS.join(", "),
                            CLIENT_KEYS.join(", "),
                        ),
                    ));
                }
                if fields.contains_key(key) {
                    return Err(invalid(
                        number,
                        format!(
                            "declares {key:?} a second time; YAML parsers disagree about \
                             which one wins"
                        ),
                    ));
                }
                let mut pending = Pending::default();
                let rest = rest.trim_start_matches(' ');
                if !rest.is_empty() {
                    pending.scalar.push(rest.to_owned());
                }
                fields.insert(key.to_owned(), pending);
                order.push(key.to_owned());
                current = Some(key.to_owned());
                broke = false;
                continue;
            }

            // Indented: one level of block under the key above it. Which
            // shape it may take is decided by the key rather than by the
            // line, because the vocabulary is closed and each key's type is
            // the spec's — which removes the one real ambiguity, a folded
            // scalar whose text happens to contain a colon.
            let key = current
                .clone()
                .ok_or_else(|| invalid(number, "is indented under nothing".to_owned()))?;
            let pending = fields
                .get_mut(&key)
                .expect("the current key was inserted when it was read");
            match key.as_str() {
                "metadata" => {
                    let (nested, rest) = trimmed.split_once(':').ok_or_else(|| {
                        invalid(number, format!("is not `key: value` under {key:?}"))
                    })?;
                    let nested = nested.trim_end().to_owned();
                    let rest = rest.trim_start_matches(' ');
                    if rest.is_empty() {
                        return Err(invalid(
                            number,
                            format!("nests a second block under {key:?}; the subset is one deep"),
                        ));
                    }
                    if pending
                        .map
                        .insert(
                            nested.clone(),
                            scalar(rest).map_err(|why| invalid(number, why))?,
                        )
                        .is_some()
                    {
                        return Err(invalid(
                            number,
                            format!("declares {nested:?} a second time"),
                        ));
                    }
                }
                "allowed-tools" => {
                    let item = trimmed.strip_prefix("- ").ok_or_else(|| {
                        invalid(
                            number,
                            format!("is not a `- item` under {key:?}, which is a sequence"),
                        )
                    })?;
                    pending
                        .items
                        .push(scalar(item).map_err(|why| invalid(number, why))?);
                }
                _ => {
                    if trimmed.starts_with("- ") {
                        return Err(invalid(
                            number,
                            format!("is a sequence item under {key:?}, which is a scalar"),
                        ));
                    }
                    if broke {
                        return Err(invalid(
                            number,
                            format!(
                                "continues {key:?} across a blank line; YAML folds that to a \
                                 newline where an ordinary break folds to a space, and the \
                                 two readings are a place parsers differ"
                            ),
                        ));
                    }
                    // A folded scalar: `description:` with its text on the
                    // following indented lines, or a quoted value continued
                    // across them. Real bundles are written both ways — the
                    // corpus test found one on its first run.
                    pending.scalar.push(trimmed.to_owned());
                }
            }
            broke = false;
        }
        if !closed {
            return Err(Error::Invalid {
                message: format!("{SKILL_MANIFEST}'s frontmatter block is never closed with `---`"),
            });
        }

        let mut out = Frontmatter::default();
        for key in order {
            let pending = fields.remove(&key).expect("every ordered key was inserted");
            let folded = pending.scalar.join(" ");
            let shape = |detail: &str| Error::Invalid {
                message: format!("{SKILL_MANIFEST} frontmatter: {key:?} {detail}"),
            };
            match key.as_str() {
                "allowed-tools" => {
                    out.allowed_tools = if pending.items.is_empty() {
                        if folded.is_empty() {
                            Vec::new()
                        } else {
                            flow_sequence(&folded)?
                        }
                    } else {
                        if !folded.is_empty() {
                            return Err(shape("has both an inline value and a block"));
                        }
                        pending.items
                    };
                }
                "metadata" => {
                    if !folded.is_empty() {
                        return Err(shape(
                            "is a mapping written as an indented block, never an inline value",
                        ));
                    }
                    out.metadata = pending.map;
                }
                scalar_key => {
                    if !pending.items.is_empty() || !pending.map.is_empty() {
                        return Err(shape("is a scalar, and carries a block"));
                    }
                    let value = scalar(&folded).map_err(|why| shape(&why))?;
                    match scalar_key {
                        "name" => out.name = value,
                        "description" => out.description = value,
                        "license" => out.license = Some(value),
                        "version" => out.version = Some(value),
                        "user-invocable" => {
                            out.user_invocable = Some(match value.as_str() {
                                "true" => true,
                                "false" => false,
                                other => {
                                    return Err(shape(&format!(
                                        "is {other:?}, which is neither `true` nor `false`"
                                    )));
                                }
                            });
                        }
                        other => {
                            out.extra.insert(other.to_owned(), value);
                        }
                    }
                }
            }
        }
        out.validate()?;
        Ok(out)
    }

    /// Refuses frontmatter no client would accept.
    fn validate(&self) -> Result<()> {
        let invalid = |detail: &str| {
            Err(Error::Invalid {
                message: format!("{SKILL_MANIFEST} frontmatter: {detail}"),
            })
        };
        if self.name.is_empty() {
            return invalid(
                "declares no `name`. The open spec requires one, and it must match the \
                 directory (ADR-0051 decision 5)",
            );
        }
        // The name inside the artefact obeys the same grammar as the name
        // outside it, or the pair could agree on something no client loads.
        self.name.parse::<SkillName>()?;
        if self.description.trim().is_empty() {
            return invalid(
                "declares no `description`. It is what a client loads at every session and \
                 what a reader chooses the skill by, so an empty one ships a skill nothing \
                 will reach for (ADR-0051 decision 5)",
            );
        }
        if self.description.chars().count() > MAX_SKILL_DESCRIPTION_CHARS {
            return invalid(&format!(
                "`description` is longer than {MAX_SKILL_DESCRIPTION_CHARS} characters"
            ));
        }
        if self.allowed_tools.len() > MAX_FRONTMATTER_ENTRIES
            || self.metadata.len() > MAX_FRONTMATTER_ENTRIES
        {
            return invalid(&format!(
                "declares more than {MAX_FRONTMATTER_ENTRIES} entries under one key"
            ));
        }
        Ok(())
    }
}

/// One line, without its terminator.
fn trim_line(line: &str) -> &str {
    line.trim_end_matches('\n').trim_end_matches('\r')
}

/// A scalar, in the three forms the subset allows.
///
/// Everything else is refused *by shape*: an anchor, an alias, a tag, a
/// block scalar and a plain value carrying `: ` are each a place two YAML
/// parsers can differ, and each is a construct nobody can author because
/// the bytes ship verbatim.
fn scalar(raw: &str) -> std::result::Result<String, String> {
    let raw = raw.trim_end();
    if raw.len() > MAX_FRONTMATTER_VALUE_CHARS {
        return Err(format!(
            "is longer than {MAX_FRONTMATTER_VALUE_CHARS} characters"
        ));
    }
    if let Some(rest) = raw.strip_prefix('\'') {
        let body = rest
            .strip_suffix('\'')
            .ok_or("opens a single quote it never closes")?;
        if body.contains('\'') && !body.contains("''") {
            return Err("is single-quoted around an unescaped quote".to_owned());
        }
        return Ok(body.replace("''", "'"));
    }
    if let Some(rest) = raw.strip_prefix('"') {
        let body = rest
            .strip_suffix('"')
            .ok_or("opens a double quote it never closes")?;
        return unescape(body);
    }
    match raw.chars().next() {
        Some('&') => Err("is an anchor; the subset has no aliases to resolve it with".to_owned()),
        Some('*') => {
            Err("is an alias, which resolves against state a reviewer cannot see".to_owned())
        }
        Some('!') => {
            Err("is a tag, whose meaning is the parser's rather than the document's".to_owned())
        }
        Some('|' | '>') => Err(
            "is a block scalar; its chomping and indentation rules differ \
                                between parsers, so quote the value instead"
                .to_owned(),
        ),
        Some('[') => Ok(raw.to_owned()),
        Some('{') => Err(
            "is a flow mapping; the subset takes `metadata:` as an indented \
                          block instead"
                .to_owned(),
        ),
        _ => {
            if raw.contains(": ") || raw.ends_with(':') {
                return Err(
                    "is unquoted and contains ':', which YAML reads as a nested mapping; \
                     quote it"
                        .to_owned(),
                );
            }
            if raw.contains(" #") {
                return Err(
                    "is unquoted and contains ' #', which YAML reads as a comment; quote it"
                        .to_owned(),
                );
            }
            Ok(raw.to_owned())
        }
    }
}

/// The escapes a double-quoted value may carry. Anything else is refused
/// rather than passed through, because a backslash that means one thing here
/// and another there is the whole hazard.
fn unescape(body: &str) -> std::result::Result<String, String> {
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some(other) => return Err(format!("carries the unknown escape \\{other}")),
            None => return Err("ends with a lone backslash".to_owned()),
        }
    }
    Ok(out)
}

/// `[a, b, c]` — the one flow form the subset takes, because real skills
/// write `allowed-tools` that way.
fn flow_sequence(raw: &str) -> Result<Vec<String>> {
    let Some(body) = raw
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    else {
        // A single unquoted tool name is a one-element sequence, which is
        // how a scalar reaches here at all.
        return Ok(vec![raw.to_owned()]);
    };
    let body = body.trim();
    if body.is_empty() {
        return Ok(Vec::new());
    }
    body.split(',')
        .map(|item| {
            scalar(item.trim()).map_err(|why| Error::Invalid {
                message: format!("{SKILL_MANIFEST} frontmatter: an `allowed-tools` entry {why}"),
            })
        })
        .collect()
}

/// Which version of a skill a caller is asking for (ADR-0051 decision 1, on
/// ADR-0049 decision 2's shape).
///
/// Deliberately **not** [`crate::Channel`], for [`crate::PromptChannel`]'s
/// reason: `published` is a channel; `draft` is a row, because a set channel
/// cannot express withdrawal and an author replacing a draft is exactly
/// that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SkillChannel {
    /// The authored working copy at one named scope.
    Draft,
    /// The reviewed version its scope stands behind — and the only one an
    /// install will write to a client's disk.
    Published,
}

impl SkillChannel {
    /// Both values.
    pub const ALL: [SkillChannel; 2] = [SkillChannel::Draft, SkillChannel::Published];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            SkillChannel::Draft => "draft",
            SkillChannel::Published => "published",
        }
    }
}

impl fmt::Display for SkillChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SkillChannel {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        SkillChannel::ALL
            .into_iter()
            .find(|channel| channel.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown skill channel: {s:?}"),
            })
    }
}

/// One entry of a scope's `skill/published` tree: `skill/path`
/// (ADR-0051 decision 2).
///
/// [`crate::DocumentPath`]'s shape, and it parses back unambiguously for its
/// reason: a skill name cannot contain `/`, so the split is at the first one
/// and everything after it is the file.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SkillPath {
    /// The bundle.
    pub skill: SkillName,
    /// The file inside it.
    pub file: SkillFilePath,
}

impl SkillPath {
    /// The pair as one path.
    #[must_use]
    pub fn new(skill: SkillName, file: SkillFilePath) -> Self {
        SkillPath { skill, file }
    }
}

impl fmt::Display for SkillPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.skill, self.file)
    }
}

impl FromStr for SkillPath {
    type Err = Error;

    fn from_str(path: &str) -> Result<Self> {
        let (skill, file) = path.split_once('/').ok_or_else(|| Error::Invalid {
            message: format!(
                "skill path {path:?} names no file; a skill channel entry is `skill/path` \
                 (ADR-0051 decision 2)"
            ),
        })?;
        Ok(SkillPath {
            skill: skill.parse()?,
            file: file.parse()?,
        })
    }
}

impl Serialize for SkillPath {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for SkillPath {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(name: &str) -> String {
        format!("---\nname: {name}\ndescription: Does a thing.\n---\n\n# Body\n")
    }

    fn bundle(name: &str) -> SkillBundle {
        SkillBundle {
            name: name.parse().unwrap(),
            files: vec![SkillFile {
                path: SKILL_MANIFEST.parse().unwrap(),
                content: manifest(name),
            }],
        }
    }

    /// The spec's grammar, which is stricter than the product's own
    /// (ADR-0051 decision 6). Every rejected name below parses fine as a
    /// prompt or a pack name, which is exactly the point.
    #[test]
    fn a_skill_name_is_the_specs_grammar_not_the_products() {
        for name in ["code-review", "pdf", "a", "x9", "deploy-to-prod"] {
            assert_eq!(name.parse::<SkillName>().unwrap().as_str(), name);
        }
        for name in [
            "",
            "Code-Review",
            "code_review",
            "code.review",
            "code/review",
            "-leading",
            "trailing-",
            "with space",
            "café",
        ] {
            assert!(name.parse::<SkillName>().is_err(), "{name:?} was accepted");
        }
        // The three that a prompt name takes and a skill name does not.
        for name in ["code_review", "code.review", "code/review"] {
            assert!(
                name.parse::<crate::PromptName>().is_ok(),
                "{name:?} should still be a legal prompt name"
            );
        }
        assert!(
            "x".repeat(MAX_SKILL_NAME_CHARS + 1)
                .parse::<SkillName>()
                .is_err()
        );
    }

    /// Decision 7, one rule at a time. Every one of these is a way a
    /// materialisation writes somewhere it was not asked to.
    #[test]
    fn a_bundled_path_is_validated_against_filesystems() {
        for path in [
            "SKILL.md",
            "scripts/check.py",
            "references/api/v2/read.md",
            "Makefile",
            "a-b_c.d",
        ] {
            assert_eq!(path.parse::<SkillFilePath>().unwrap().as_str(), path);
        }
        for (path, why) in [
            ("", "empty"),
            ("/etc/passwd", "absolute"),
            ("../escape.md", "parent"),
            ("scripts/../../out.md", "parent inside"),
            ("./here.md", "dot segment"),
            ("a//b", "empty segment"),
            ("c:\\windows", "drive letter"),
            ("scripts\\check.py", "backslash"),
            ("trailing.", "trailing dot"),
            ("trailing ", "trailing space"),
            (" leading", "leading space"),
            ("con.py", "reserved stem"),
            ("NUL", "reserved stem upper"),
            ("lpt9.txt", "reserved stem"),
            ("café.md", "non-ascii"),
            ("a/b/c/d/e", "too deep"),
        ] {
            assert!(path.parse::<SkillFilePath>().is_err(), "{why}: {path:?}");
        }
        assert!(
            format!("{}.md", "x".repeat(MAX_SKILL_PATH_SEGMENT_CHARS))
                .parse::<SkillFilePath>()
                .is_err()
        );
    }

    /// The one that is not obvious: both paths are legal, both are distinct
    /// objects, and one file survives the install.
    #[test]
    fn two_paths_differing_only_in_case_are_refused_as_a_pair() {
        let mut b = bundle("demo");
        b.files.push(SkillFile {
            path: "scripts/Run.py".parse().unwrap(),
            content: "print(1)".to_owned(),
        });
        b.files.push(SkillFile {
            path: "scripts/run.py".parse().unwrap(),
            content: "print(2)".to_owned(),
        });
        let err = b.validate().unwrap_err();
        assert!(format!("{err}").contains("differ only in case"), "{err}");

        // …and the exact same path twice says something different.
        let mut b = bundle("demo");
        b.files.push(SkillFile {
            path: "scripts/run.py".parse().unwrap(),
            content: "print(1)".to_owned(),
        });
        b.files.push(SkillFile {
            path: "scripts/run.py".parse().unwrap(),
            content: "print(2)".to_owned(),
        });
        let err = b.validate().unwrap_err();
        assert!(format!("{err}").contains("named twice"), "{err}");
    }

    #[test]
    fn a_bundle_without_a_skill_md_is_not_a_skill() {
        let b = SkillBundle {
            name: "demo".parse().unwrap(),
            files: vec![SkillFile {
                path: "reference.md".parse().unwrap(),
                content: "text".to_owned(),
            }],
        };
        let err = b.validate().unwrap_err();
        assert!(format!("{err}").contains("no SKILL.md"), "{err}");
    }

    /// The spec's own rule, enforced where it can still be fixed
    /// (decision 5): a client refuses this, so this product must.
    #[test]
    fn the_frontmatter_name_must_equal_the_skills_name() {
        let mut b = bundle("demo");
        b.files[0].content = manifest("something-else");
        let err = b.validate().unwrap_err();
        assert!(format!("{err}").contains("but the skill is"), "{err}");
    }

    #[test]
    fn a_valid_bundle_yields_its_frontmatter() {
        let front = bundle("demo").validate().unwrap();
        assert_eq!(front.name, "demo");
        assert_eq!(front.description, "Does a thing.");
        assert!(front.allowed_tools.is_empty());
    }

    #[test]
    fn the_subset_reads_the_shapes_real_skills_use() {
        let front = Frontmatter::parse(
            "---\n\
             name: demo\n\
             description: \"Reviews code: carefully.\"\n\
             license: Apache-2.0\n\
             user-invocable: true\n\
             allowed-tools:\n\
             \x20 - Read\n\
             \x20 - Bash(ls *)\n\
             metadata:\n\
             \x20 short-description: Review code\n\
             ---\n\
             # Body\n",
        )
        .unwrap();
        assert_eq!(front.name, "demo");
        assert_eq!(front.description, "Reviews code: carefully.");
        assert_eq!(front.license.as_deref(), Some("Apache-2.0"));
        assert_eq!(front.user_invocable, Some(true));
        assert_eq!(front.allowed_tools, vec!["Read", "Bash(ls *)"]);
        assert_eq!(front.metadata["short-description"], "Review code");

        // The flow form, which `allowed-tools` is often written in.
        let front =
            Frontmatter::parse("---\nname: d\ndescription: x\nallowed-tools: [Read, Write]\n---\n")
                .unwrap();
        assert_eq!(front.allowed_tools, vec!["Read", "Write"]);
    }

    /// The two shapes `tests/skill_corpus.rs` found on its first run against
    /// 37 installed bundles: a `description` folded across lines, and keys
    /// clients define that the spec does not. Both are widenings taken
    /// deliberately, with a bundle to point at (ADR-0051 reversal trigger a).
    #[test]
    fn the_subset_reads_the_shapes_a_real_corpus_turned_up() {
        // `math-olympiad`: the value starts on the *next* line and the
        // quoted scalar spans several, folding to single spaces.
        let front = Frontmatter::parse(
            "---\n\
             name: math-olympiad\n\
             description:\n\
             \x20 \"Solve competition problems with adversarial\n\
             \x20 verification. Activates on 'prove this'.\"\n\
             version: 0.1.0\n\
             ---\n",
        )
        .unwrap();
        assert_eq!(
            front.description,
            "Solve competition problems with adversarial verification. Activates on 'prove this'."
        );
        assert_eq!(front.version.as_deref(), Some("0.1.0"));

        // `claude-md-improver` and `example-command`: keys the spec does not
        // define. Kept rather than dropped, so a review can render what a
        // client will act on.
        let front = Frontmatter::parse(
            "---\n\
             name: demo\n\
             description: x\n\
             tools: Read, Glob, Grep\n\
             argument-hint: <arg> [optional]\n\
             disable-model-invocation: true\n\
             ---\n",
        )
        .unwrap();
        assert_eq!(front.extra["tools"], "Read, Glob, Grep");
        assert_eq!(front.extra["argument-hint"], "<arg> [optional]");
        assert_eq!(front.extra["disable-model-invocation"], "true");
        assert!(front.metadata.is_empty(), "a client key is not metadata");
    }

    /// A fold across a blank line is a newline in YAML and a space
    /// everywhere this parser looks, so it is refused rather than guessed.
    #[test]
    fn a_value_folded_across_a_blank_line_is_refused() {
        let err =
            Frontmatter::parse("---\nname: d\ndescription:\n  first part\n\n  second part\n---\n")
                .unwrap_err();
        assert!(format!("{err}").contains("across a blank line"), "{err}");
    }

    /// Decision 4, construct by construct. Each of these is a place two
    /// parsers can read one document differently.
    #[test]
    fn the_subset_refuses_every_construct_two_parsers_could_differ_on() {
        let head = "---\nname: demo\n";
        for (body, why) in [
            ("description: &anchor text\n---\n", "anchor"),
            ("description: *alias\n---\n", "alias"),
            ("description: !!str text\n---\n", "tag"),
            ("description: |\n  block\n---\n", "block scalar"),
            ("description: text\ndescription: other\n---\n", "duplicate"),
            ("description: a: b\n---\n", "unquoted colon"),
            ("description: text # note\n---\n", "unquoted comment"),
            ("description: text\n<<: base\n---\n", "merge key"),
            ("description: text\n\tindented: x\n---\n", "tab"),
            ("description: text\nunknown-key: x\n---\n", "unknown key"),
            ("description: text\n", "unclosed"),
            ("- item\n---\n", "top-level sequence"),
            ("description: {a: b}\n---\n", "flow mapping"),
            ("metadata:\n  nested:\n    deeper: x\n---\n", "two levels"),
            ("description: \"bad \\q escape\"\n---\n", "unknown escape"),
        ] {
            let doc = format!("{head}{body}");
            let result = Frontmatter::parse(&doc);
            assert!(result.is_err(), "{why} was accepted: {doc:?}");
        }
        // A `%YAML` directive is refused by the same rule that requires the
        // block to open the file: there is nowhere to put one.
        assert!(Frontmatter::parse("%YAML 1.2\n---\nname: d\ndescription: x\n---\n").is_err());
        // And the control: the shape all of those are variations on parses.
        assert!(Frontmatter::parse("---\nname: demo\ndescription: text\n---\n").is_ok());
    }

    #[test]
    fn frontmatter_must_open_the_file_and_carry_both_required_keys() {
        assert!(Frontmatter::parse("# No frontmatter\n").is_err());
        assert!(Frontmatter::parse("\n---\nname: d\ndescription: x\n---\n").is_err());
        let err = Frontmatter::parse("---\ndescription: x\n---\n").unwrap_err();
        assert!(format!("{err}").contains("no `name`"), "{err}");
        let err = Frontmatter::parse("---\nname: d\ndescription: \"  \"\n---\n").unwrap_err();
        assert!(format!("{err}").contains("no `description`"), "{err}");
        // The name inside the artefact obeys the same grammar as the one
        // outside it.
        assert!(Frontmatter::parse("---\nname: Bad_Name\ndescription: x\n---\n").is_err());
    }

    #[test]
    fn a_bundle_is_bounded_and_says_which_bound_it_broke() {
        let mut b = bundle("demo");
        b.files.push(SkillFile {
            path: "big.md".parse().unwrap(),
            content: "x".repeat(MAX_SKILL_FILE_CHARS + 1),
        });
        assert!(format!("{}", b.validate().unwrap_err()).contains("over the"));

        let mut b = bundle("demo");
        b.files.push(SkillFile {
            path: "bin.dat".parse().unwrap(),
            content: "head\0tail".to_owned(),
        });
        assert!(format!("{}", b.validate().unwrap_err()).contains("NUL"));

        let mut b = bundle("demo");
        for n in 0..MAX_SKILL_FILES {
            b.files.push(SkillFile {
                path: format!("f{n}.md").parse().unwrap(),
                content: "x".to_owned(),
            });
        }
        assert!(format!("{}", b.validate().unwrap_err()).contains("at most"));
    }

    #[test]
    fn a_skill_path_round_trips_through_its_rendered_form() {
        let path: SkillPath = "code-review/scripts/check.py".parse().unwrap();
        assert_eq!(path.skill.as_str(), "code-review");
        assert_eq!(path.file.as_str(), "scripts/check.py");
        assert_eq!(path.to_string(), "code-review/scripts/check.py");
        assert_eq!(path.to_string().parse::<SkillPath>().unwrap(), path);
        assert!("code-review".parse::<SkillPath>().is_err());
    }

    /// Every path that parses fits the columns it is stored in — the tree
    /// entry (255) and a curator glob's ref name (200).
    #[test]
    fn every_parseable_path_fits_the_schema() {
        let file = format!(
            "{}/{}",
            "y".repeat(MAX_SKILL_PATH_SEGMENT_CHARS),
            "z".repeat(MAX_SKILL_PATH_CHARS - MAX_SKILL_PATH_SEGMENT_CHARS - 1)
        );
        let widest = SkillPath::new(
            "x".repeat(MAX_SKILL_NAME_CHARS).parse().unwrap(),
            file.parse().expect("the longest bundled path parses"),
        )
        .to_string();
        assert_eq!(
            widest.chars().count(),
            MAX_SKILL_NAME_CHARS + 1 + MAX_SKILL_PATH_CHARS
        );
        assert!(widest.len() <= 200, "fits vedaflow_refs.name");
        assert!(widest.len() <= 255, "fits a tree entry name");
        assert_eq!(widest.parse::<SkillPath>().unwrap().to_string(), widest);
    }

    #[test]
    fn skill_channels_round_trip_and_are_not_channels() {
        for channel in SkillChannel::ALL {
            assert_eq!(
                channel.to_string().parse::<SkillChannel>().unwrap(),
                channel
            );
            assert_eq!(
                serde_json::to_string(&channel).unwrap(),
                format!("\"{}\"", channel.as_str())
            );
        }
        assert!("derived".parse::<SkillChannel>().is_err());
        assert!("draft".parse::<crate::Channel>().is_err());
    }
}
