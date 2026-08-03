//! The frontmatter subset against real bundles (SKIL-1, ADR-0051 decision 4).
//!
//! ADR-0051 option 4 rejected a general YAML parser and recorded its own
//! reversal trigger: "a real `anthropics/skills` bundle the subset refuses
//! and every client accepts is a bug in the subset, and the fix is to widen
//! it deliberately". This is that trigger's instrument.
//!
//! It is `#[ignore]`d because it reads a corpus that is not in the
//! repository — the skills a developer's own clients have installed. Point
//! it at one and run it:
//!
//! ```text
//! SYNVEDA_SKILL_CORPUS=~/.codex/skills:~/.claude/plugins \
//!   cargo test -p synveda-types --test skill_corpus -- --ignored --nocapture
//! ```
//!
//! Every `SKILL.md` under those roots must parse. A failure is a finding
//! about this product, never about the bundle.

use std::path::{Path, PathBuf};

use synveda_types::{Frontmatter, SKILL_MANIFEST};

#[test]
#[ignore = "reads client-installed skills outside the repository; see the module docs"]
fn the_subset_parses_every_skill_md_in_a_real_corpus() {
    let Ok(roots) = std::env::var("SYNVEDA_SKILL_CORPUS") else {
        panic!("set SYNVEDA_SKILL_CORPUS to one or more ':'-separated directories");
    };
    let mut manifests = Vec::new();
    for root in roots.split(':').filter(|root| !root.is_empty()) {
        collect(&expand(root), &mut manifests);
    }
    assert!(
        !manifests.is_empty(),
        "no {SKILL_MANIFEST} found under {roots}; the corpus is empty, which measures nothing"
    );

    let mut refused = Vec::new();
    for path in &manifests {
        let content = std::fs::read_to_string(path).expect("read a manifest");
        match Frontmatter::parse(&content) {
            Ok(front) => assert!(
                !front.name.is_empty() && !front.description.is_empty(),
                "{}: parsed with an empty required field",
                path.display()
            ),
            Err(err) => refused.push(format!("  {}\n    {err}", path.display())),
        }
    }
    println!(
        "the subset read {}/{} manifests under {roots}",
        manifests.len() - refused.len(),
        manifests.len()
    );
    assert!(
        refused.is_empty(),
        "the subset refused {} real bundle(s), which fires ADR-0051 reversal trigger (a):\n{}",
        refused.len(),
        refused.join("\n")
    );
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

/// Every `SKILL.md` under `dir`, symlinks not followed — a bundle reached
/// through one is not a bundle this product would have stored (decision 15).
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
        } else if path.file_name().is_some_and(|name| name == SKILL_MANIFEST) {
            out.push(path);
        }
    }
}
