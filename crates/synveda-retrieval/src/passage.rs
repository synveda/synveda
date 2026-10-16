//! Pure, bounded source-span selection for conservative delivery (CTX-8).
//!
//! A span is an exact byte slice of one authorised immutable revision. It is
//! never presented as a generated summary. Empty lines outside fenced code
//! delimit units; explicit qualifier paragraphs stay attached to their claim,
//! applicability headings bind their sections, and fenced blocks remain whole.

use std::collections::HashSet;

/// Maximum semantic units considered from one already bounded Knowledge body.
pub const MAX_PASSAGE_UNITS: usize = 256;

/// Exact half-open UTF-8 byte range in an immutable source body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSpan {
    /// First included byte.
    pub start: usize,
    /// First byte after the span.
    pub end: usize,
}

impl SourceSpan {
    /// Returns the unmodified source bytes as UTF-8 text.
    #[must_use]
    pub fn text(self, source: &str) -> Option<&str> {
        source.get(self.start..self.end)
    }
}

/// Ranks complete source units by overlap with the caller's task, breaking
/// ties by source order. A missing or empty task yields source order alone.
/// This function consumes no model service and never infers authority from
/// the source text.
#[must_use]
pub fn ranked_source_units(source: &str, task: Option<&str>) -> Vec<SourceSpan> {
    let terms = task.map_or_else(HashSet::new, words);
    let mut units = Vec::new();
    let mut start = 0_usize;
    let mut offset = 0_usize;
    let mut fence: Option<(char, usize)> = None;
    let mut section_level = None;
    let mut last_retained = false;
    let mut saw_content = false;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim();
        if let Some((marker, length)) = fence_marker(trimmed) {
            match fence {
                None => fence = Some((marker, length)),
                Some((open, minimum))
                    if open == marker
                        && length >= minimum
                        && trimmed[length..].trim().is_empty() =>
                {
                    fence = None;
                }
                _ => {}
            }
        }
        if !trimmed.is_empty() {
            saw_content = true;
        }
        offset += line.len();
        if trimmed.is_empty() && fence.is_none() {
            if saw_content {
                append_unit(
                    source,
                    SourceSpan { start, end: offset },
                    &mut units,
                    &mut section_level,
                    &mut last_retained,
                );
            }
            start = offset;
            saw_content = false;
        }
    }
    if saw_content {
        append_unit(
            source,
            SourceSpan {
                start,
                end: source.len(),
            },
            &mut units,
            &mut section_level,
            &mut last_retained,
        );
    }
    units.sort_by(|left, right| {
        let left_score = score(left.text(source).unwrap_or_default(), &terms);
        let right_score = score(right.text(source).unwrap_or_default(), &terms);
        right_score
            .cmp(&left_score)
            .then_with(|| left.start.cmp(&right.start))
    });
    units
}

fn fence_marker(line: &str) -> Option<(char, usize)> {
    let marker = line.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let length = line
        .chars()
        .take_while(|character| *character == marker)
        .count();
    (length >= 3).then_some((marker, length))
}

fn heading_level(unit: &str) -> Option<usize> {
    let first = unit.trim_start().lines().next()?;
    let level = first.bytes().take_while(|byte| *byte == b'#').count();
    (matches!(level, 1..=6) && first.as_bytes().get(level) == Some(&b' ')).then_some(level)
}

fn explicit_qualifier(unit: &str) -> bool {
    let first = unit
        .trim_start()
        .trim_start_matches(['>', '-', '*', ' '])
        .to_ascii_lowercase();
    [
        "exception:",
        "exceptions:",
        "except ",
        "unless ",
        "only ",
        "provided ",
        "prerequisite:",
        "prerequisites:",
        "precondition:",
        "preconditions:",
        "however",
        "otherwise",
        "but ",
        "note:",
        "warning:",
        "caution:",
        "if ",
        "when ",
        "before ",
    ]
    .iter()
    .any(|prefix| first.starts_with(prefix))
}

fn append_unit(
    source: &str,
    span: SourceSpan,
    units: &mut Vec<SourceSpan>,
    section: &mut Option<usize>,
    last_retained: &mut bool,
) {
    let text = span.text(source).unwrap_or_default();
    let level = heading_level(text);
    let continues_section = section.is_some_and(|parent| level.is_none_or(|level| level > parent));
    let attaches = continues_section || (level.is_none() && explicit_qualifier(text));
    if attaches
        && *last_retained
        && let Some(last) = units.last_mut()
    {
        last.end = span.end;
        return;
    }
    if continues_section {
        return;
    }
    *section = level;
    *last_retained = units.len() < MAX_PASSAGE_UNITS;
    if *last_retained {
        units.push(span);
    }
}

/// Number of distinct task terms present in one exact source unit.
#[must_use]
pub fn task_overlap(unit: &str, task: &str) -> usize {
    let terms = words(task);
    score(unit, &terms)
}

fn words(text: &str) -> HashSet<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn score(unit: &str, task: &HashSet<String>) -> usize {
    if task.is_empty() {
        return 0;
    }
    let unit_words = words(unit);
    let folded = unit.to_lowercase();
    task.iter()
        .filter(|term| {
            unit_words.contains(*term) || (!term.is_ascii() && folded.contains(term.as_str()))
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exception_remains_in_the_ranked_retry_proposition() {
        let body = "Retry failed requests at most 3 times.\n\nException: never retry a payment after an unknown commit outcome.\n\nThe dashboard color is blue.\n";
        let units = ranked_source_units(body, Some("retry 3"));
        assert_eq!(
            units[0].text(body),
            Some(
                "Retry failed requests at most 3 times.\n\nException: never retry a payment after an unknown commit outcome.\n\n"
            )
        );
        assert_eq!(task_overlap("Retry cap 3", "3"), 1);
    }

    #[test]
    fn exhausting_the_group_limit_never_detaches_a_retained_exception() {
        let mut body = "Independent historical note.\n\n".repeat(MAX_PASSAGE_UNITS - 1);
        body.push_str("Retry at most 3 times.\n\nException: refuse an unknown payment outcome.\n\nUnrelated note beyond the limit.\n\nException: applies only to the omitted note.\n");
        let units = ranked_source_units(&body, Some("retry"));
        assert_eq!(units.len(), MAX_PASSAGE_UNITS);
        assert_eq!(
            units[0].text(&body),
            Some("Retry at most 3 times.\n\nException: refuse an unknown payment outcome.\n\n")
        );
        assert!(
            !units
                .iter()
                .any(|unit| unit.text(&body).unwrap().contains("omitted note"))
        );
    }

    #[test]
    fn applicability_headings_and_nested_fences_are_atomic() {
        let body = "## Production\n\nRetry at most 3 times.\n\nOnly retry idempotent writes.\n\n### Verification\n\n````text\n```\n\nKeep this identifier: X.\n````\n\n## Development\n\nRetry at most 5 times.\n";
        let units = ranked_source_units(body, Some("Production X"));
        assert_eq!(units.len(), 2);
        let expected_end = body.find("## Development").unwrap();
        assert_eq!(units[0].text(body), Some(&body[..expected_end]));
        assert!(units.iter().all(|span| span.text(body).is_some()));
    }

    #[test]
    fn preserves_exact_code_and_configuration_whitespace() {
        let body =
            "Overview.\n\n```toml\n[server]\n\nname = \"v1\"\n  # keep indent\n```\n\nEnd.\n";
        let units = ranked_source_units(body, Some("server name"));
        assert_eq!(
            units[0].text(body),
            Some("```toml\n[server]\n\nname = \"v1\"\n  # keep indent\n```\n\n")
        );
        assert!(units.iter().all(|span| span.text(body).is_some()));
    }

    #[test]
    fn ranking_is_relevant_and_stable_without_a_task() {
        let body = "The cache is optional.\n\nNever disable Cedar for tenant reads.\n\nThe cache is warm.\n";
        let relevant = ranked_source_units(body, Some("tenant Cedar"));
        assert!(relevant[0].text(body).unwrap().contains("Never disable"));
        assert_eq!(
            task_overlap(relevant[0].text(body).unwrap(), "tenant Cedar"),
            2
        );
        let fallback = ranked_source_units(body, None);
        assert_eq!(fallback[0].start, 0);
        assert!(
            fallback
                .windows(2)
                .all(|pair| pair[0].start < pair[1].start)
        );
    }

    #[test]
    fn unicode_spans_remain_exact_and_work_is_bounded() {
        let body = format!(
            "unrelated.\n\n前提: 設定を保持。\n\n{}",
            "a\n\n".repeat(500)
        );
        let units = ranked_source_units(&body, Some("設定"));
        assert_eq!(units.len(), MAX_PASSAGE_UNITS);
        assert_eq!(units[0].text(&body), Some("前提: 設定を保持。\n\n"));
        assert_eq!(task_overlap(units[0].text(&body).unwrap(), "設定"), 1);
    }
}
