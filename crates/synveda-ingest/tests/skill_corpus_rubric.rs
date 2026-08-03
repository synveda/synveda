//! The rubric against real bundles (SKIL-3, ADR-0053 force 4).
//!
//! ADR-0051 option 4 set the discipline and ADR-0052 reversal trigger (a)
//! extended it to the scanner: a real bundle the product judges wrongly is
//! a finding about the product, and the fix is a deliberate change with the
//! bundle named. This is that instrument for the rubric, and it matters
//! more here than for either sibling, because a rubric **guesses by
//! construction** — the only way to know whether it guesses well is to
//! point it at bundles nobody wrote to please it.
//!
//! It is `#[ignore]`d because it reads a corpus that is not in the
//! repository — the skills a developer's own clients have installed:
//!
//! ```text
//! SYNVEDA_SKILL_CORPUS=~/.codex/skills:~/.claude/plugins \
//!   cargo test -p synveda-ingest --test skill_corpus_rubric -- --ignored --nocapture
//! ```
//!
//! What it asserts is deliberately weak and what it prints is the point.
//! There is no score a real bundle *must* reach — a rubric that failed the
//! build over somebody else's skill would be this product asserting taste
//! — so the assertion is only that the rubric is **discriminating**: that
//! it does not award everything full marks (in which case it measures
//! nothing) and does not fail everything (in which case it measures the
//! wrong thing). The distribution beneath it is what a person reads.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use synveda_ingest::{MAX_SCORE, score_bundle};
use synveda_types::{SKILL_MANIFEST, SkillFile, SkillFilePath};

/// The largest file the rubric will read out of a real bundle. ADR-0051
/// bounds an *authored* bundle at 64KB a file; a corpus is not governed by
/// this product and may hold anything, so the walk skips rather than
/// refuses.
const MAX_CORPUS_FILE_BYTES: u64 = 64 * 1024;

#[test]
#[ignore = "reads client-installed skills outside the repository; see the module docs"]
fn the_rubric_discriminates_across_a_real_corpus() {
    let Ok(roots) = std::env::var("SYNVEDA_SKILL_CORPUS") else {
        panic!("set SYNVEDA_SKILL_CORPUS to one or more ':'-separated directories");
    };
    let mut bundles = Vec::new();
    for root in roots.split(':').filter(|root| !root.is_empty()) {
        collect(&expand(root), &mut bundles);
    }
    assert!(
        !bundles.is_empty(),
        "no {SKILL_MANIFEST} found under {roots}; the corpus is empty, which measures nothing"
    );

    let mut scored: Vec<(String, u8, Vec<String>)> = Vec::new();
    let mut failures: BTreeMap<&'static str, usize> = BTreeMap::new();
    for dir in &bundles {
        let files = read_bundle(dir);
        if files.is_empty() {
            continue;
        }
        let result = score_bundle(&files);
        for check in result.failed() {
            *failures.entry(check.check).or_insert(0) += 1;
        }
        scored.push((
            dir.display().to_string(),
            result.score,
            result
                .failed()
                .iter()
                .map(|check| check.check.to_owned())
                .collect(),
        ));
    }
    scored.sort_by_key(|(_, score, _)| *score);

    let total = scored.len();
    let sum: usize = scored.iter().map(|(_, score, _)| usize::from(*score)).sum();
    println!("\n{total} bundles under {roots}\n");
    for (path, score, failed) in &scored {
        println!(
            "  {score:>3}  {}{}",
            short(path),
            if failed.is_empty() {
                String::new()
            } else {
                format!("   ({})", failed.join(", "))
            }
        );
    }
    println!("\n  mean {:.1}", sum as f64 / total as f64);
    println!("  checks by how often they fired against a real bundle:");
    let mut ranked: Vec<(&&str, &usize)> = failures.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1));
    for (check, count) in ranked {
        println!("    {count:>3}/{total}  {check}");
    }

    let perfect = scored
        .iter()
        .filter(|(_, score, _)| *score == MAX_SCORE)
        .count();
    let floored = scored.iter().filter(|(_, score, _)| *score < 40).count();
    println!("  {perfect} at {MAX_SCORE}, {floored} under 40\n");

    // The rubric must separate bundles rather than rank them all the same.
    // Both halves of this are real failure modes: a rubric everything
    // passes is a decoration, and one everything fails is a rubric
    // measuring this product's taste instead of the ecosystem's practice.
    assert!(
        perfect < total,
        "every bundle in the corpus scored {MAX_SCORE}; the rubric is not measuring anything"
    );
    assert!(
        perfect * 100 / total.max(1) < 90,
        "{perfect}/{total} bundles scored full marks; the rubric is too easy to be worth rendering"
    );
    assert!(
        floored * 100 / total.max(1) < 50,
        "{floored}/{total} bundles scored under 40; the rubric is judging the ecosystem rather \
         than measuring it, which fires ADR-0053 reversal trigger (c)"
    );
}

/// The last two path segments — enough to tell two bundles apart without
/// printing a developer's home directory forty times.
fn short(path: &str) -> String {
    let parts: Vec<&str> = path.rsplit('/').take(2).collect();
    parts.into_iter().rev().collect::<Vec<_>>().join("/")
}

/// `~` is the only shell expansion a test harness owes anyone.
fn expand(root: &str) -> PathBuf {
    match root.strip_prefix("~/") {
        Some(rest) => match std::env::var_os("HOME") {
            Some(home) => Path::new(&home).join(rest),
            None => PathBuf::from(root),
        },
        None => PathBuf::from(root),
    }
}

/// Every directory under `dir` holding a `SKILL.md`, symlinks not followed.
fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_symlink() {
            continue;
        }
        if meta.is_dir() {
            collect(&path, out);
        } else if path.file_name().is_some_and(|name| name == SKILL_MANIFEST)
            && let Some(parent) = path.parent()
        {
            out.push(parent.to_path_buf());
        }
    }
}

/// One bundle's files as the rubric would see them, relative to its root.
///
/// Anything this product could not have stored is skipped rather than
/// failed — non-UTF-8, oversized, or nested past the path grammar — because
/// the corpus is somebody else's and the question is how the rubric reads
/// what it *can* read.
fn read_bundle(root: &Path) -> Vec<SkillFile> {
    let mut files = Vec::new();
    walk(root, root, &mut files);
    files.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
    files
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<SkillFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_symlink() {
            continue;
        }
        if meta.is_dir() {
            walk(root, &path, out);
            continue;
        }
        if meta.len() > MAX_CORPUS_FILE_BYTES {
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let Some(as_str) = relative.to_str() else {
            continue;
        };
        let Ok(bundled) = as_str.parse::<SkillFilePath>() else {
            continue;
        };
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        out.push(SkillFile {
            path: bundled,
            content,
        });
    }
}
