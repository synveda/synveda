//! Pure, bounded source-span selection for conservative delivery (CTX-8).
//!
//! A span is an exact byte slice of one authorised immutable revision. It is
//! never presented as a generated summary. Empty lines outside fenced code
//! delimit units; fenced blocks remain whole even when they contain blanks.

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
    let mut fence: Option<char> = None;
    let mut saw_content = false;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim();
        let marker = trimmed.chars().next().filter(|first| {
            (*first == '`' || *first == '~') && trimmed.starts_with(&first.to_string().repeat(3))
        });
        if let Some(marker) = marker {
            match fence {
                None => fence = Some(marker),
                Some(open) if open == marker => fence = None,
                _ => {}
            }
        }
        if !trimmed.is_empty() {
            saw_content = true;
        }
        offset += line.len();
        if trimmed.is_empty() && fence.is_none() {
            if saw_content && units.len() < MAX_PASSAGE_UNITS {
                units.push(SourceSpan { start, end: offset });
            }
            start = offset;
            saw_content = false;
        }
    }
    if saw_content && units.len() < MAX_PASSAGE_UNITS {
        units.push(SourceSpan {
            start,
            end: source.len(),
        });
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

/// Number of distinct task terms present in one exact source unit.
#[must_use]
pub fn task_overlap(unit: &str, task: &str) -> usize {
    let terms = words(task);
    score(unit, &terms)
}

fn words(text: &str) -> HashSet<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| word.chars().count() >= 2)
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
