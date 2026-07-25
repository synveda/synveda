//! Rendering the two sides of a proposed change (FLOW-6, ADR-0035
//! decision 7).
//!
//! A memory object is canonical JSON with sorted keys — the form FLOW-1
//! chose because "FLOW-6 renders diffs of it and FLOW-8 exports it into a
//! real git repository" (`synveda-vedaflow`, `MemoryAsset::
//! canonical_bytes`). Diffing those bytes as text would be correct and
//! unreadable exactly where it matters: a multi-line content edit is one
//! enormous escaped line inside a JSON string.
//!
//! So the object is read as the record of governed fields it is, and each
//! field renders as `old → new`, except the text — which renders as a
//! line-level unified diff. A proposal that only closes `valid_to` or
//! raises `sensitivity` is a real change to what crosses the trust
//! boundary, and a content-only diff would render it as nothing at all.
//!
//! Rendering is presentation and lives here alone; the gateway ships bytes.

use std::fmt::Write as _;

/// The field holding the text of a memory asset — the one that gets a line
/// diff rather than an `old → new` line.
const TEXT_FIELD: &str = "content";

/// Lines of unchanged text kept either side of a change, as in `diff -u`.
const CONTEXT_LINES: usize = 3;

/// How a rendered line should be marked up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// Added: `+`.
    Added,
    /// Removed: `-`.
    Removed,
    /// Context, a header, or a field that did not change.
    Plain,
    /// A hunk header or a field name.
    Meta,
}

/// One rendered line, with what it means. Colour is applied by the caller,
/// so this module stays testable as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub mark: Mark,
    pub text: String,
}

impl Line {
    fn new(mark: Mark, text: impl Into<String>) -> Self {
        Self {
            mark,
            text: text.into(),
        }
    }
}

/// Renders the change from `before` to `after`, both canonical object
/// bytes as text. `before` is `None` for material the channel does not
/// hold yet, which renders as an addition of every field.
///
/// Bytes that do not parse as a JSON object are diffed as plain text —
/// the honest fallback for an asset kind this renderer does not know, and
/// the one that will keep working when PRMT-1 and SKIL-1 bring objects
/// whose shape is not this one.
#[must_use]
pub fn render(before: Option<&str>, after: &str) -> Vec<Line> {
    match (before.map(parse), parse(after)) {
        (Some(Some(before)), Some(after)) => fields(Some(&before), &after),
        (None, Some(after)) => fields(None, &after),
        // One side is not an object: fall back to a text diff of the raw
        // bytes rather than render nothing.
        _ => unified(before.unwrap_or(""), after),
    }
}

/// A JSON object as `(key, value)` pairs. Canonical bytes are already
/// key-sorted, so the order is the object's own.
fn parse(text: &str) -> Option<Vec<(String, serde_json::Value)>> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let object = value.as_object()?;
    Some(
        object
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    )
}

/// Field-wise rendering: every key either side names, in one sorted order.
///
/// `before` is `None` for an addition — material the channel does not hold
/// at all — which renders as an addition of every field rather than as a
/// removal of nothing: a reviewer admitting new content should not have to
/// read a column of `(absent)` to learn there was no old version.
fn fields(
    before: Option<&[(String, serde_json::Value)]>,
    after: &[(String, serde_json::Value)],
) -> Vec<Line> {
    let existing = before.unwrap_or(&[]);
    let mut keys: Vec<&str> = existing
        .iter()
        .chain(after.iter())
        .map(|(key, _)| key.as_str())
        .collect();
    keys.sort_unstable();
    keys.dedup();

    let find = |set: &[(String, serde_json::Value)], key: &str| {
        set.iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    };

    let mut lines = Vec::new();
    for key in keys {
        let new = find(after, key);
        if before.is_none() {
            if key == TEXT_FIELD {
                lines.push(Line::new(Mark::Meta, format!("  {key}:")));
                if let Some(text) = new.as_ref().and_then(|value| value.as_str()) {
                    lines.extend(indent(all(Mark::Added, text)));
                }
            } else {
                lines.push(Line::new(
                    Mark::Added,
                    format!("+ {key}: {}", scalar(new.as_ref())),
                ));
            }
            continue;
        }
        let old = find(existing, key);
        if old == new {
            // Unchanged fields are still shown: a reviewer reading a
            // promotion needs to see the sensitivity it carries, not only
            // the fields that moved.
            lines.push(Line::new(
                Mark::Plain,
                format!("  {key}: {}", scalar(new.as_ref())),
            ));
            continue;
        }
        if key == TEXT_FIELD {
            lines.push(Line::new(Mark::Meta, format!("  {key}:")));
            let old_text = old.as_ref().and_then(|value| value.as_str());
            let new_text = new.as_ref().and_then(|value| value.as_str());
            match (old_text, new_text) {
                (Some(old_text), Some(new_text)) => {
                    lines.extend(indent(unified(old_text, new_text)));
                }
                // Absent on one side (an addition, or a shape this
                // renderer did not expect): show the whole text.
                (None, Some(new_text)) => lines.extend(indent(all(Mark::Added, new_text))),
                (Some(old_text), None) => lines.extend(indent(all(Mark::Removed, old_text))),
                (None, None) => {}
            }
            continue;
        }
        lines.push(Line::new(
            Mark::Removed,
            format!("- {key}: {}", scalar(old.as_ref())),
        ));
        lines.push(Line::new(
            Mark::Added,
            format!("+ {key}: {}", scalar(new.as_ref())),
        ));
    }
    lines
}

/// A field value as one line. Strings lose their quotes (a reviewer is
/// reading a value, not JSON); everything else keeps its JSON rendering.
fn scalar(value: Option<&serde_json::Value>) -> String {
    match value {
        None => "(absent)".to_owned(),
        Some(serde_json::Value::Null) => "(none)".to_owned(),
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    }
}

fn indent(lines: Vec<Line>) -> Vec<Line> {
    lines
        .into_iter()
        .map(|line| Line {
            text: format!("    {}", line.text),
            mark: line.mark,
        })
        .collect()
}

fn all(mark: Mark, text: &str) -> Vec<Line> {
    let sign = if mark == Mark::Added { '+' } else { '-' };
    text.lines()
        .map(|line| Line::new(mark, format!("{sign} {line}")))
        .collect()
}

/// A `diff -u`-shaped line diff of two texts.
#[must_use]
pub fn unified(before: &str, after: &str) -> Vec<Line> {
    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();
    let script = edits(&old, &new);
    if script.iter().all(|edit| matches!(edit, Edit::Keep(_, _))) {
        return Vec::new();
    }
    hunks(&script, &old, &new)
}

/// One step of an edit script over lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edit {
    /// Present in both, at these indices.
    Keep(usize, usize),
    /// Only in the old text, at this index.
    Remove(usize),
    /// Only in the new text, at this index.
    Insert(usize),
}

/// The longest-common-subsequence edit script.
///
/// O(n·m) in time and space over *lines*, which is the right trade for
/// what this diffs: memory records, prompt bodies, curator files — texts
/// measured in lines, not megabytes. Hand-written rather than taken as a
/// dependency because the core path's licence rule makes even a small one
/// a reviewed diff (ADR-0035), and because this is the boring half of the
/// algorithm.
fn edits(old: &[&str], new: &[&str]) -> Vec<Edit> {
    // lengths[i][j] = LCS length of old[i..] and new[j..].
    let mut lengths = vec![vec![0usize; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            lengths[i][j] = if old[i] == new[j] {
                lengths[i + 1][j + 1] + 1
            } else {
                lengths[i + 1][j].max(lengths[i][j + 1])
            };
        }
    }

    let mut script = Vec::with_capacity(old.len() + new.len());
    let (mut i, mut j) = (0, 0);
    while i < old.len() && j < new.len() {
        if old[i] == new[j] {
            script.push(Edit::Keep(i, j));
            i += 1;
            j += 1;
        } else if lengths[i + 1][j] >= lengths[i][j + 1] {
            script.push(Edit::Remove(i));
            i += 1;
        } else {
            script.push(Edit::Insert(j));
            j += 1;
        }
    }
    while i < old.len() {
        script.push(Edit::Remove(i));
        i += 1;
    }
    while j < new.len() {
        script.push(Edit::Insert(j));
        j += 1;
    }
    script
}

/// Groups an edit script into `@@` hunks with [`CONTEXT_LINES`] of context.
fn hunks(script: &[Edit], old: &[&str], new: &[&str]) -> Vec<Line> {
    let changed: Vec<usize> = script
        .iter()
        .enumerate()
        .filter(|(_, edit)| !matches!(edit, Edit::Keep(_, _)))
        .map(|(position, _)| position)
        .collect();
    if changed.is_empty() {
        return Vec::new();
    }

    // Merge changes whose context windows touch into one hunk.
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for position in changed {
        let start = position.saturating_sub(CONTEXT_LINES);
        let end = (position + CONTEXT_LINES + 1).min(script.len());
        match spans.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => spans.push((start, end)),
        }
    }

    let mut lines = Vec::new();
    for (start, end) in spans {
        let span = &script[start..end];
        let (old_start, old_count) = range(span, |edit| match edit {
            Edit::Keep(i, _) | Edit::Remove(i) => Some(*i),
            Edit::Insert(_) => None,
        });
        let (new_start, new_count) = range(span, |edit| match edit {
            Edit::Keep(_, j) | Edit::Insert(j) => Some(*j),
            Edit::Remove(_) => None,
        });
        let mut header = String::new();
        // 1-based, as every diff reader expects; a zero-length side keeps
        // the 0 start `diff -u` uses for it.
        let _ = write!(
            header,
            "@@ -{},{} +{},{} @@",
            if old_count == 0 { 0 } else { old_start + 1 },
            old_count,
            if new_count == 0 { 0 } else { new_start + 1 },
            new_count
        );
        lines.push(Line::new(Mark::Meta, header));
        for edit in span {
            match edit {
                Edit::Keep(i, _) => lines.push(Line::new(Mark::Plain, format!("  {}", old[*i]))),
                Edit::Remove(i) => lines.push(Line::new(Mark::Removed, format!("- {}", old[*i]))),
                Edit::Insert(j) => lines.push(Line::new(Mark::Added, format!("+ {}", new[*j]))),
            }
        }
    }
    lines
}

/// The `start, count` of one side of a hunk.
fn range(span: &[Edit], index: impl Fn(&Edit) -> Option<usize>) -> (usize, usize) {
    let indices: Vec<usize> = span.iter().filter_map(index).collect();
    match indices.first() {
        Some(first) => (*first, indices.len()),
        None => (0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn asset(content: &str, sensitivity: &str) -> String {
        serde_json::json!({
            "class": "procedure",
            "content": content,
            "id": "0198f000-0000-7000-8000-000000000001",
            "kind": "derived",
            "sensitivity": sensitivity,
            "valid_to": serde_json::Value::Null,
        })
        .to_string()
    }

    #[test]
    fn identical_texts_produce_no_diff() {
        assert!(unified("a\nb\nc", "a\nb\nc").is_empty());
        assert!(unified("", "").is_empty());
    }

    #[test]
    fn a_changed_line_is_one_removal_and_one_addition_with_context() {
        let lines = unified(
            "alpha\nbravo\ncharlie\ndelta",
            "alpha\nbravo\nCHARLIE\ndelta",
        );
        let rendered = text(&lines);
        assert!(rendered.contains("- charlie"), "{rendered}");
        assert!(rendered.contains("+ CHARLIE"), "{rendered}");
        assert!(rendered.contains("  alpha"), "context must survive");
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.mark == Mark::Removed)
                .count(),
            1
        );
        assert_eq!(
            lines.iter().filter(|line| line.mark == Mark::Added).count(),
            1
        );
    }

    #[test]
    fn distant_changes_become_two_hunks_and_near_ones_become_one() {
        let old: Vec<String> = (0..30).map(|n| format!("line {n}")).collect();
        let mut new = old.clone();
        new[1] = "line one changed".to_owned();
        new[25] = "line twenty-five changed".to_owned();
        let far = unified(&old.join("\n"), &new.join("\n"));
        assert_eq!(
            far.iter().filter(|line| line.mark == Mark::Meta).count(),
            2,
            "changes 24 lines apart are two hunks:\n{}",
            text(&far)
        );

        let mut near = old.clone();
        near[10] = "ten changed".to_owned();
        near[12] = "twelve changed".to_owned();
        let close = unified(&old.join("\n"), &near.join("\n"));
        assert_eq!(
            close.iter().filter(|line| line.mark == Mark::Meta).count(),
            1,
            "changes 2 lines apart share a hunk:\n{}",
            text(&close)
        );
    }

    #[test]
    fn hunk_headers_count_the_lines_they_cover() {
        // One line replaced in a four-line file: the whole file is inside
        // the context window, so one hunk covering 4 old and 4 new lines.
        let lines = unified("a\nb\nc\nd", "a\nB\nc\nd");
        let header = &lines[0];
        assert_eq!(header.mark, Mark::Meta);
        assert_eq!(header.text, "@@ -1,4 +1,4 @@", "{}", text(&lines));

        // A pure addition to an empty text has no old side at all.
        let added = unified("", "first\nsecond");
        assert_eq!(added[0].text, "@@ -0,0 +1,2 @@", "{}", text(&added));
    }

    #[test]
    fn an_addition_renders_every_field_of_the_new_object() {
        let lines = render(None, &asset("rotate the key", "internal"));
        let rendered = text(&lines);
        // No baseline: nothing is a removal.
        assert!(
            lines.iter().all(|line| line.mark != Mark::Removed),
            "an addition removes nothing:\n{rendered}"
        );
        assert!(rendered.contains("sensitivity: internal"), "{rendered}");
        assert!(rendered.contains("+ rotate the key"), "{rendered}");
    }

    #[test]
    fn a_governed_field_change_is_never_rendered_as_no_change() {
        // The case a content-only diff would show as empty (ADR-0035
        // decision 7): same text, raised classification.
        let lines = render(
            Some(&asset("rotate the key", "internal")),
            &asset("rotate the key", "restricted"),
        );
        let rendered = text(&lines);
        assert!(rendered.contains("- sensitivity: internal"), "{rendered}");
        assert!(rendered.contains("+ sensitivity: restricted"), "{rendered}");
        // ...and the text, being unchanged, contributes no ± lines.
        assert!(!rendered.contains("rotate the key\n+"), "{rendered}");
    }

    #[test]
    fn a_multi_line_edit_is_a_line_diff_and_not_one_escaped_string() {
        let before = asset("step one\nstep two\nstep three", "internal");
        let after = asset("step one\nstep two, revised\nstep three", "internal");
        let rendered = text(&render(Some(&before), &after));
        assert!(rendered.contains("- step two"), "{rendered}");
        assert!(rendered.contains("+ step two, revised"), "{rendered}");
        // The whole point: the unchanged lines are not part of the change.
        assert!(
            !rendered.contains("- step one"),
            "an unchanged line must not be rendered as removed:\n{rendered}"
        );
        assert!(
            !rendered.contains("\\n"),
            "the text must be lines, not an escaped JSON string:\n{rendered}"
        );
    }

    #[test]
    fn a_field_that_only_one_side_has_is_shown_on_both() {
        let before = serde_json::json!({"content": "x"}).to_string();
        let after =
            serde_json::json!({"content": "x", "valid_to": "2026-01-01T00:00:00Z"}).to_string();
        let rendered = text(&render(Some(&before), &after));
        assert!(rendered.contains("- valid_to: (absent)"), "{rendered}");
        assert!(rendered.contains("+ valid_to: 2026-01-01"), "{rendered}");
    }

    #[test]
    fn bytes_that_are_not_an_object_fall_back_to_a_text_diff() {
        // What a future asset kind, or a corrupted object, looks like.
        let rendered = text(&render(Some("one\ntwo"), "one\nthree"));
        assert!(rendered.contains("- two"), "{rendered}");
        assert!(rendered.contains("+ three"), "{rendered}");
    }
}
