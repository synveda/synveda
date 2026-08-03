//! Context-pack vocabulary (seed §4.3, tech plan §2.3; PRMT-2, ADR-0050).
//!
//! A context pack is the second asset a human *writes* rather than one the
//! pipeline derives, and the first whose content has to enter the corpus
//! the read path ranks. Two things that makes new live here: the
//! **document path** a scope's pack channel names, and the **chunker**
//! that decides which pieces of a document become records.
//!
//! # What this module decides and what it does not
//!
//! It describes a pack, bounds it, and cuts a document into chunks. It
//! never authorises (that is `ContextPackRead`/`ContextPackWrite` at the
//! seam the caller crossed), it never stores (that is
//! `synveda_store::packs`), and it never addresses (that is
//! `synveda_vedaflow::ContextPackAsset`, which hashes the canonical form).
//!
//! # Why the chunker is here, and why it is structural
//!
//! Chunk boundaries decide content addresses: a chunk row carries the
//! address of the document it was cut from, and the same document
//! re-authored must produce the same chunks or every publication would
//! re-embed a bundle nobody edited (ADR-0050 decision 4). So [`chunk`] is
//! a pure function of the text — no clock, no model, no network. ADR-0050
//! option 9 rejected a semantic splitter outright for exactly this: the
//! same bytes would chunk differently on different days.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// The longest a pack name may be. One segment: a pack is a bundle's
/// identifier, and the nesting belongs to the documents inside it.
pub const MAX_PACK_NAME_CHARS: usize = 64;

/// The most segments a document name may have: `runbooks/payments/oncall.md`.
pub const MAX_DOCUMENT_NAME_SEGMENTS: usize = 3;

/// The longest a document name may be.
///
/// With [`MAX_PACK_NAME_CHARS`] this bounds the tree entry name
/// `pack/document` at 193 characters — inside `vedaflow_tree_entries.name`
/// (255) and inside `vedaflow_refs.name` (200), which is what ADR-0032's
/// curator globs match against.
pub const MAX_DOCUMENT_NAME_CHARS: usize = 128;

/// The longest one name segment may be.
pub const MAX_PACK_SEGMENT_CHARS: usize = 64;

/// The longest a pack description may be — one line in a listing.
pub const MAX_PACK_DESCRIPTION_CHARS: usize = 512;

/// The longest a document title may be. Rendered into the index line, so
/// it is bounded well below the line's own width.
pub const MAX_DOCUMENT_TITLE_CHARS: usize = 160;

/// The longest one document may be.
///
/// `synveda_vedaflow::MAX_OBJECT_BYTES` is the hard bound; this one is the
/// product's, and it is the number that decides how slow the slowest write
/// in the product can get (ADR-0050 consequences).
pub const MAX_DOCUMENT_CHARS: usize = 262_144;

/// The most documents one pack may hold.
pub const MAX_PACK_DOCUMENTS: usize = 64;

/// The most chunks one document may be cut into.
///
/// The bound the ADR's own consequences ask for: authoring scans, chunks
/// and embeds in one request, and every chunk is a row, a vector and a
/// signature. A document that would exceed this is refused at authoring
/// naming the number, rather than discovered as a timeout.
pub const MAX_DOCUMENT_CHUNKS: usize = 512;

/// The character budget one chunk aims for.
///
/// A constant rather than a knob, and deliberately: it is an input to
/// every chunk's content and therefore to the document address a
/// publication binds. Making it configurable would mean two scopes
/// chunking identical bytes differently, which is the nondeterminism
/// ADR-0050 option 9 refused a model-driven splitter for.
pub const CHUNK_CHARS: usize = 1_200;

/// A context pack's name: the bundle's identifier, and the first segment
/// of every entry its channel tree carries (ADR-0050 decision 3).
///
/// One segment, lower-case, by construction. **Deliberately not a uuid**,
/// for [`crate::PromptName`]'s reason: the same name at a nearer scope is
/// how a team overrides the org's bundle, and a unique id cannot express
/// that.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct ContextPackName(String);

impl ContextPackName {
    /// The name as stored, hashed, and rendered.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContextPackName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for ContextPackName {
    type Err = Error;

    fn from_str(name: &str) -> Result<Self> {
        validate_name(name, "context pack name", 1, MAX_PACK_NAME_CHARS)?;
        Ok(ContextPackName(name.to_owned()))
    }
}

impl<'de> Deserialize<'de> for ContextPackName {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

/// A document's name within its pack: path-shaped, so a bundle can carry
/// `runbooks/payments.md` rather than flattening a real directory into one
/// segment.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct DocumentName(String);

impl DocumentName {
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

impl fmt::Display for DocumentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for DocumentName {
    type Err = Error;

    fn from_str(name: &str) -> Result<Self> {
        validate_name(
            name,
            "document name",
            MAX_DOCUMENT_NAME_SEGMENTS,
            MAX_DOCUMENT_NAME_CHARS,
        )?;
        Ok(DocumentName(name.to_owned()))
    }
}

impl<'de> Deserialize<'de> for DocumentName {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

/// One entry of a scope's `context-pack/published` tree: `pack/document`
/// (ADR-0050 decision 3).
///
/// ADR-0031's reserved "a path for the authored asset types", and the
/// first *bundle* ADR-0032's curator glob gets to glob over — `payments/*`
/// is a rule about one pack, which is a thing a curator file could not
/// express while prompts were the only paths.
///
/// It parses unambiguously because the pack name is a single segment: the
/// split is at the first `/`, and everything after it is the document.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocumentPath {
    /// The bundle.
    pub pack: ContextPackName,
    /// The document inside it.
    pub document: DocumentName,
}

impl DocumentPath {
    /// The pair as one path.
    #[must_use]
    pub fn new(pack: ContextPackName, document: DocumentName) -> Self {
        DocumentPath { pack, document }
    }
}

impl fmt::Display for DocumentPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.pack, self.document)
    }
}

impl FromStr for DocumentPath {
    type Err = Error;

    fn from_str(path: &str) -> Result<Self> {
        let (pack, document) = path.split_once('/').ok_or_else(|| Error::Invalid {
            message: format!(
                "document path {path:?} names no document; a pack channel entry is \
                 `pack/document` (ADR-0050 decision 3)"
            ),
        })?;
        Ok(DocumentPath {
            pack: pack.parse()?,
            document: document.parse()?,
        })
    }
}

impl Serialize for DocumentPath {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for DocumentPath {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

/// One document of a pack: what an author uploads and what a reviewer
/// reads.
///
/// The governed fields around it — the scope and the tier — live on the
/// stored row and inside the object address
/// (`synveda_vedaflow::ContextPackAsset`), for [`crate::PromptTemplate`]'s
/// reason: those are facts about the asset rather than parts of the text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackDocument {
    /// Its name within the pack.
    pub name: DocumentName,
    /// One line, read in a listing, at review, and in the index tier's
    /// rendered entry (ADR-0050 decision 10).
    pub title: String,
    /// The text.
    pub content: String,
}

impl PackDocument {
    /// Refuses a document that could not be stored, chunked or rendered.
    ///
    /// Bounds only: a document is prose, and there is no placeholder
    /// grammar to disagree with the way a prompt's schema can. What is
    /// **not** checked here is the scanner (`ingest::redaction`) — that
    /// runs at authoring, where the effective pack's redaction config is
    /// known (ADR-0050 decision 11).
    pub fn validate(&self) -> Result<()> {
        let invalid = |message: String| Err(Error::Invalid { message });
        let chars = self.content.chars().count();
        if chars == 0 || chars > MAX_DOCUMENT_CHARS {
            return invalid(format!(
                "document {} must be 1..={MAX_DOCUMENT_CHARS} characters, and is {chars}",
                self.name
            ));
        }
        if self.title.chars().count() > MAX_DOCUMENT_TITLE_CHARS {
            return invalid(format!(
                "document {}'s title is longer than {MAX_DOCUMENT_TITLE_CHARS} characters",
                self.name
            ));
        }
        // A title is one line because the index tier renders it into one
        // (ADR-0048 decision 9's rule, applied where the text enters rather
        // than only where it is shown).
        if self.title.contains(['\n', '\r']) {
            return invalid(format!(
                "document {}'s title spans lines; it is rendered into one index entry",
                self.name
            ));
        }
        let chunks = chunk(&self.content).len();
        if chunks > MAX_DOCUMENT_CHUNKS {
            return invalid(format!(
                "document {} cuts into {chunks} chunks, over the {MAX_DOCUMENT_CHUNKS} \
                 one document may hold; split it into two documents",
                self.name
            ));
        }
        Ok(())
    }
}

/// One piece of a document, as [`chunk`] cut it — the unit that becomes a
/// pinned record (ADR-0050 decision 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentChunk {
    /// Its position in the document, from zero. Part of the chunk row's
    /// identity and the order the index tier names them in.
    pub ordinal: u32,
    /// The nearest enclosing heading, when the document has one — the
    /// `§ heading` half of ADR-0050 decision 10's index line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading: Option<String>,
    /// The text of this piece.
    pub content: String,
}

/// Cuts a document into chunks: structural, deterministic, and a pure
/// function of the text (ADR-0050 decision 4).
///
/// Three rules, in order:
///
/// 1. **A heading always starts a chunk.** A Markdown ATX heading (`#` to
///    `######` followed by space) closes whatever was accumulating and
///    becomes the section every chunk under it is labelled with. A chunk
///    that spanned two headings would make `pack/document § heading` a
///    guess.
/// 2. **Paragraphs pack up to [`CHUNK_CHARS`], and are never split
///    across chunks** while one fits at all — blank-line-separated blocks
///    are the structure prose already has.
/// 3. **A paragraph longer than the budget is hard-split** at the last
///    whitespace before the limit, or at the character limit when it has
///    none (a minified blob, a base64 payload). Deterministic either way.
///
/// Whitespace-only pieces are dropped: a chunk of blank lines is a record
/// that would rank against real material and say nothing.
#[must_use]
pub fn chunk(content: &str) -> Vec<DocumentChunk> {
    let mut chunks: Vec<DocumentChunk> = Vec::new();
    let mut heading: Option<String> = None;
    let mut buffer = String::new();

    // Flushes what has accumulated under the current heading.
    let flush = |chunks: &mut Vec<DocumentChunk>, buffer: &mut String, heading: &Option<String>| {
        let text = buffer.trim();
        if !text.is_empty() {
            chunks.push(DocumentChunk {
                ordinal: u32::try_from(chunks.len()).unwrap_or(u32::MAX),
                heading: heading.clone(),
                content: text.to_owned(),
            });
        }
        buffer.clear();
    };

    for block in blocks(content) {
        match block {
            Block::Heading(text) => {
                flush(&mut chunks, &mut buffer, &heading);
                heading = Some(text.to_owned());
            }
            Block::Paragraph(text) => {
                for piece in split_paragraph(text) {
                    // Rule 2: a paragraph joins the buffer while the result
                    // still fits, and starts a new chunk when it does not.
                    let separated = buffer.chars().count() + 2 + piece.chars().count();
                    if !buffer.is_empty() && separated > CHUNK_CHARS {
                        flush(&mut chunks, &mut buffer, &heading);
                    }
                    if !buffer.is_empty() {
                        buffer.push_str("\n\n");
                    }
                    buffer.push_str(piece);
                }
            }
        }
    }
    flush(&mut chunks, &mut buffer, &heading);
    chunks
}

/// A document's structure as the chunker reads it.
enum Block<'a> {
    /// An ATX heading line, with its `#`s and surrounding space stripped.
    Heading(&'a str),
    /// A run of non-blank, non-heading lines.
    Paragraph(&'a str),
}

/// Splits a document into headings and blank-line-separated paragraphs,
/// borrowing throughout — the source is the chunk content, so a
/// paragraph's bytes are exactly the document's.
fn blocks(content: &str) -> Vec<Block<'_>> {
    let mut out: Vec<Block<'_>> = Vec::new();
    let mut start: Option<usize> = None;
    let mut end = 0_usize;
    let mut offset = 0_usize;

    for line in content.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if let Some(text) = atx_heading(trimmed) {
            close_paragraph(content, &mut out, &mut start, end);
            out.push(Block::Heading(text));
        } else if trimmed.trim().is_empty() {
            close_paragraph(content, &mut out, &mut start, end);
        } else {
            if start.is_none() {
                start = Some(offset);
            }
            end = offset + trimmed.len();
        }
        offset += line.len();
    }
    close_paragraph(content, &mut out, &mut start, end);
    out
}

/// Closes the paragraph that has been accumulating, if any.
fn close_paragraph<'a>(
    content: &'a str,
    out: &mut Vec<Block<'a>>,
    start: &mut Option<usize>,
    end: usize,
) {
    if let Some(from) = start.take() {
        let text = &content[from..end];
        if !text.trim().is_empty() {
            out.push(Block::Paragraph(text));
        }
    }
}

/// The text of an ATX heading line, or `None` when the line is not one.
///
/// One to six `#` followed by whitespace — CommonMark's rule, minus the
/// closing-sequence trim, which is presentation rather than structure.
fn atx_heading(line: &str) -> Option<&str> {
    let text = line.trim_start();
    let hashes = text.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &text[hashes..];
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(rest.trim())
}

/// Rule 3: a paragraph that fits is itself; one that does not is hard-split
/// at the last whitespace before the budget.
fn split_paragraph(text: &str) -> Vec<&str> {
    if text.chars().count() <= CHUNK_CHARS {
        return vec![text];
    }
    let mut pieces: Vec<&str> = Vec::new();
    let mut rest = text;
    while rest.chars().count() > CHUNK_CHARS {
        // The byte offset of the CHUNK_CHARS'th character — a character
        // boundary by construction, so the slice below cannot panic.
        let limit = rest
            .char_indices()
            .nth(CHUNK_CHARS)
            .map_or(rest.len(), |(index, _)| index);
        let head = &rest[..limit];
        // The last whitespace inside the budget, so a cut lands between
        // words wherever the text has any. A blob with none is cut at the
        // limit: deterministic, and the alternative is an unbounded chunk.
        let cut = head
            .rfind(char::is_whitespace)
            .filter(|at| *at > 0)
            .unwrap_or(limit);
        pieces.push(rest[..cut].trim_end());
        rest = rest[cut..].trim_start();
    }
    if !rest.is_empty() {
        pieces.push(rest);
    }
    pieces
}

/// The shared name grammar: lower-case path segments, bounded, with `..`
/// refused outright.
///
/// [`crate::PromptName`]'s rules, applied to the two names this module
/// owns. A name is a key a person types, never a filesystem path to walk,
/// and two names differing only in case would be two packs that look like
/// one.
fn validate_name(name: &str, what: &str, max_segments: usize, max_chars: usize) -> Result<()> {
    let invalid = |detail: &str| {
        Err(Error::Invalid {
            message: format!("{what} {name:?} {detail}"),
        })
    };
    if name.is_empty() {
        return invalid("is empty");
    }
    if name.chars().count() > max_chars {
        return invalid(&format!("is longer than {max_chars} characters"));
    }
    let segments: Vec<&str> = name.split('/').collect();
    if segments.len() > max_segments {
        return invalid(&format!("has more than {max_segments} segments"));
    }
    for segment in &segments {
        if segment.is_empty() {
            return invalid("has an empty segment");
        }
        if segment.chars().count() > MAX_PACK_SEGMENT_CHARS {
            return invalid(&format!(
                "has a segment longer than {MAX_PACK_SEGMENT_CHARS} characters"
            ));
        }
        let mut chars = segment.chars();
        let first = chars.next().expect("a non-empty segment has a first char");
        if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
            return invalid("has a segment not starting with a lower-case letter or digit");
        }
        if let Some(bad) = chars.find(|c| !matches!(c, 'a'..='z' | '0'..='9' | '-' | '_' | '.')) {
            return invalid(&format!(
                "contains {bad:?}; segments are lower-case letters, digits, '-', '_' and '.'"
            ));
        }
        if segment.contains("..") {
            return invalid("contains '..'");
        }
    }
    Ok(())
}

/// Which version of a pack a caller is asking for (ADR-0050 decision 1,
/// on ADR-0049 decision 2's shape).
///
/// Deliberately **not** [`crate::Channel`], for [`crate::PromptChannel`]'s
/// reason: `published` is a channel; `draft` is a row, because a set
/// channel cannot express withdrawal (ADR-0032 decision 2) and an author
/// replacing a draft is exactly that withdrawal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextPackChannel {
    /// The authored working copy at one named scope.
    Draft,
    /// The reviewed version its scope stands behind — and, for a pack, the
    /// only one whose chunks compose into anybody's session.
    Published,
}

impl ContextPackChannel {
    /// Both values.
    pub const ALL: [ContextPackChannel; 2] =
        [ContextPackChannel::Draft, ContextPackChannel::Published];

    /// Stable wire name, identical to the serde form.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            ContextPackChannel::Draft => "draft",
            ContextPackChannel::Published => "published",
        }
    }
}

impl fmt::Display for ContextPackChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ContextPackChannel {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        ContextPackChannel::ALL
            .into_iter()
            .find(|channel| channel.as_str() == s)
            .ok_or_else(|| Error::Invalid {
                message: format!("unknown context pack channel: {s:?}"),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_and_document_names_round_trip() {
        for name in ["payments", "house-style", "p0", "api.v2"] {
            let parsed: ContextPackName = name.parse().unwrap();
            assert_eq!(parsed.as_str(), name);
            assert_eq!(parsed.to_string(), name);
        }
        for name in ["glossary.md", "runbooks/payments.md", "a/b/c"] {
            let parsed: DocumentName = name.parse().unwrap();
            assert_eq!(parsed.to_string(), name);
        }
        assert_eq!(
            "runbooks/payments.md"
                .parse::<DocumentName>()
                .unwrap()
                .segments()
                .collect::<Vec<_>>(),
            vec!["runbooks", "payments.md"]
        );
    }

    /// A pack name is one segment: the nesting belongs to the document, and
    /// allowing both would make `pack/document` ambiguous to parse back.
    #[test]
    fn a_pack_name_is_one_segment_and_a_document_name_is_a_path() {
        assert!("payments/runbooks".parse::<ContextPackName>().is_err());
        assert!("a/b/c/d".parse::<DocumentName>().is_err());
        for name in [
            "",
            "/leading",
            "trailing/",
            "double//slash",
            "House-Style",
            "with space",
            "../escape",
            "-leading-dash",
            "unicode-é",
        ] {
            assert!(name.parse::<DocumentName>().is_err(), "{name:?}");
        }
    }

    /// The split is at the first `/`, which is unambiguous exactly because
    /// the pack name cannot contain one.
    #[test]
    fn a_document_path_round_trips_through_its_rendered_form() {
        let path: DocumentPath = "payments/runbooks/oncall.md".parse().unwrap();
        assert_eq!(path.pack.as_str(), "payments");
        assert_eq!(path.document.as_str(), "runbooks/oncall.md");
        assert_eq!(path.to_string(), "payments/runbooks/oncall.md");
        assert_eq!(
            path.to_string().parse::<DocumentPath>().unwrap(),
            path,
            "the tree entry name parses back to the pair that wrote it"
        );
        assert!("payments".parse::<DocumentPath>().is_err());
    }

    /// Every path that parses fits the columns it is stored in — the tree
    /// entry (255) and a curator glob's ref name (200).
    #[test]
    fn every_parseable_path_fits_the_schema() {
        // The segment bound binds before the whole-name one, so the widest
        // parseable document name is built out of segments rather than
        // being one long run (PRMT-1's test found the same thing).
        assert!(
            "y".repeat(MAX_DOCUMENT_NAME_CHARS)
                .parse::<DocumentName>()
                .is_err()
        );
        let document = format!(
            "{}/{}",
            "y".repeat(MAX_PACK_SEGMENT_CHARS),
            "z".repeat(MAX_DOCUMENT_NAME_CHARS - MAX_PACK_SEGMENT_CHARS - 1)
        );
        let widest = DocumentPath::new(
            "x".repeat(MAX_PACK_NAME_CHARS).parse().unwrap(),
            document.parse().expect("the longest document name parses"),
        )
        .to_string();
        assert_eq!(
            widest.chars().count(),
            MAX_PACK_NAME_CHARS + 1 + MAX_DOCUMENT_NAME_CHARS
        );
        assert!(widest.len() <= 200, "fits vedaflow_refs.name");
        assert!(widest.len() <= 255, "fits a tree entry name");
        assert_eq!(
            widest.parse::<DocumentPath>().unwrap().to_string(),
            widest,
            "and it still parses back at that width"
        );
    }

    #[test]
    fn a_document_with_no_headings_is_one_chunk_when_it_fits() {
        let chunks = chunk("Refunds settle in three days.\n\nEscalate over £500.");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].ordinal, 0);
        assert_eq!(chunks[0].heading, None);
        assert_eq!(
            chunks[0].content,
            "Refunds settle in three days.\n\nEscalate over £500."
        );
    }

    /// Rule 1. A chunk that spanned two headings would make the index
    /// line's `§ heading` a guess about which one it came from.
    #[test]
    fn a_heading_always_starts_a_chunk_and_labels_what_follows() {
        let chunks = chunk("# Payments\n\nSettles in three days.\n\n## Refunds\n\nEscalate.\n");
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].heading.as_deref(), Some("Payments"));
        assert_eq!(chunks[0].content, "Settles in three days.");
        assert_eq!(chunks[1].heading.as_deref(), Some("Refunds"));
        assert_eq!(chunks[1].content, "Escalate.");
        assert_eq!(chunks[1].ordinal, 1);
    }

    /// Text before the first heading is a chunk of its own with no section,
    /// rather than being absorbed into a heading it appears above.
    #[test]
    fn a_preamble_above_the_first_heading_keeps_no_heading() {
        let chunks = chunk("Read this first.\n\n# Payments\n\nSettles.\n");
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].heading, None);
        assert_eq!(chunks[0].content, "Read this first.");
        assert_eq!(chunks[1].heading.as_deref(), Some("Payments"));
    }

    /// A heading with nothing under it contributes no chunk: a record
    /// holding a section title and no text would rank against real
    /// material and say nothing.
    #[test]
    fn an_empty_section_produces_no_chunk() {
        let chunks = chunk("# Empty\n\n## Also empty\n\n# Real\n\nText.\n");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].heading.as_deref(), Some("Real"));
    }

    #[test]
    fn hashes_that_are_not_headings_stay_prose() {
        // No space after the hashes, and seven of them: neither is an ATX
        // heading, so both are text.
        let chunks = chunk("#hashtag stays prose\n\n####### seven hashes\n");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].heading, None);
        assert!(chunks[0].content.contains("#hashtag"));
        assert!(chunks[0].content.contains("####### seven"));
    }

    /// Rule 2: paragraphs pack until the budget, and a chunk never carries
    /// half a paragraph while the paragraph fits in one.
    #[test]
    fn paragraphs_pack_up_to_the_budget_and_are_not_split_while_they_fit() {
        let para = "x".repeat(500);
        let document = format!("{para}\n\n{para}\n\n{para}");
        let chunks = chunk(&document);
        assert_eq!(chunks.len(), 2, "500+2+500 fits; a third does not");
        assert_eq!(chunks[0].content, format!("{para}\n\n{para}"));
        assert_eq!(chunks[1].content, para);
        for piece in &chunks {
            assert!(piece.content.chars().count() <= CHUNK_CHARS);
        }
    }

    /// Rule 3, and the property the whole feature's addressing rests on.
    #[test]
    fn an_oversized_paragraph_splits_on_whitespace_and_stays_bounded() {
        let word = "lorem ";
        let document = word.repeat(1_000);
        let chunks = chunk(&document);
        assert!(chunks.len() > 1);
        for piece in &chunks {
            assert!(
                piece.content.chars().count() <= CHUNK_CHARS,
                "every chunk is bounded"
            );
            assert!(!piece.content.starts_with(' '));
            assert!(!piece.content.ends_with(' '));
        }
        // Nothing is lost: the words come back in order.
        let rejoined: String = chunks
            .iter()
            .map(|piece| piece.content.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(rejoined, document.trim());
    }

    /// A blob with no whitespace is cut at the limit rather than becoming
    /// one unbounded chunk — deterministic, which is the whole point.
    #[test]
    fn a_paragraph_with_no_whitespace_is_cut_at_the_limit() {
        let blob = "a".repeat(CHUNK_CHARS * 2 + 7);
        let chunks = chunk(&blob);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].content.chars().count(), CHUNK_CHARS);
        assert_eq!(chunks[1].content.chars().count(), CHUNK_CHARS);
        assert_eq!(chunks[2].content.chars().count(), 7);
    }

    /// The property ADR-0050 decision 4 rests on: identical bytes chunk
    /// identically, so re-authoring an unchanged document re-embeds
    /// nothing. Option 9 rejected a model-driven splitter for failing it.
    #[test]
    fn chunking_is_a_pure_function_of_the_text() {
        let document = "# A\n\nalpha\n\n## B\n\nbeta gamma\n\n".repeat(20);
        assert_eq!(chunk(&document), chunk(&document));
        // And it is sensitive to the text: one changed character re-cuts.
        let edited = document.replacen("alpha", "alphas", 1);
        assert_ne!(chunk(&document), chunk(&edited));
    }

    #[test]
    fn ordinals_are_dense_and_start_at_zero() {
        let chunks = chunk("# A\n\none\n\n# B\n\ntwo\n\n# C\n\nthree\n");
        assert_eq!(
            chunks.iter().map(|piece| piece.ordinal).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn a_document_is_bounded_and_says_which_bound_it_broke() {
        let document = |content: String| PackDocument {
            name: "glossary.md".parse().unwrap(),
            title: "Glossary".to_owned(),
            content,
        };
        document("Real text.".to_owned()).validate().unwrap();

        let err = document(String::new()).validate().unwrap_err();
        assert!(format!("{err}").contains("characters"), "{err}");

        let err = document("x".repeat(MAX_DOCUMENT_CHARS + 1))
            .validate()
            .unwrap_err();
        assert!(format!("{err}").contains("characters"), "{err}");

        // Chunk-count: MAX_DOCUMENT_CHUNKS + 1 headed sections.
        let many: String = (0..=MAX_DOCUMENT_CHUNKS)
            .map(|n| format!("# s{n}\n\ntext\n\n"))
            .collect();
        let err = document(many).validate().unwrap_err();
        assert!(format!("{err}").contains("chunks"), "{err}");
    }

    #[test]
    fn a_title_is_one_bounded_line() {
        let document = |title: &str| PackDocument {
            name: "glossary.md".parse().unwrap(),
            title: title.to_owned(),
            content: "Real text.".to_owned(),
        };
        let err = document("two\nlines").validate().unwrap_err();
        assert!(format!("{err}").contains("spans lines"), "{err}");
        let err = document(&"t".repeat(MAX_DOCUMENT_TITLE_CHARS + 1))
            .validate()
            .unwrap_err();
        assert!(format!("{err}").contains("title is longer"), "{err}");
    }

    #[test]
    fn pack_channels_round_trip_and_are_not_channels() {
        for channel in ContextPackChannel::ALL {
            assert_eq!(
                channel.to_string().parse::<ContextPackChannel>().unwrap(),
                channel
            );
            assert_eq!(
                serde_json::to_string(&channel).unwrap(),
                format!("\"{}\"", channel.as_str())
            );
        }
        assert!("derived".parse::<ContextPackChannel>().is_err());
        assert!("draft".parse::<crate::Channel>().is_err());
    }
}
