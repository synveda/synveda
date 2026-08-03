//! Skill bundle quality scoring — the automated half (SKIL-3, ADR-0053).
//!
//! The scanner next door asks *does this bundle do something nobody could
//! want*; this asks *is this bundle any good*. The difference is not
//! degree, it is what the answer is allowed to do: a scanner's answer
//! refuses a write, so ADR-0052 decision 10 kept its blocking band to what
//! a lexical rule decides with certainty. A rubric's answer is **a number
//! a person reads beside the file**, so it may guess — "has examples" is a
//! heuristic about a document, not a fact about it — and a wrong guess
//! costs a reviewer thirty seconds rather than costing an author their
//! publication.
//!
//! That licence runs out at exactly the point where the number decides
//! something, which is why the publish gate it feeds has an override and
//! the security gate does not (ADR-0053 force 3).
//!
//! # The weights are confidence × consequence, not importance
//!
//! Decision 5. `no-placeholders` carries 20 because a `TODO` in a file is
//! a *fact* — fully decidable, no judgement — and the consequence is an
//! unfinished bundle on a fleet of laptops. `files-referenced` carries 5
//! because a helper called from a script rather than named in the manifest
//! is a perfectly good bundle this check marks down, and five points is
//! what that mistake is allowed to cost.
//!
//! Two weights were moved after the corpus test below was pointed at real
//! bundles, and the movement is the honest part of this table: a check
//! that turns out to be less certain than it looked gets cheaper, in the
//! same commit, rather than keeping a weight its accuracy does not earn.
//!
//! # What is not here
//!
//! The reviewer's half. Whether the instructions are *correct*, whether
//! the skill belongs at this scope, whether anybody ran it — none of that
//! is a property of the bytes, all of it is in
//! [`synveda_types::Checklist`], and the two are never averaged into one
//! number (decision 1).

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;
use synveda_types::{Frontmatter, SKILL_MANIFEST, SkillFile};

/// The rubric's version, carried on every score and every audit payload.
///
/// ADR-0052 force 4, one plane over: the table moves and the bytes do not,
/// so a score that did not name the rubric that produced it could not be
/// compared with one taken at review time. It is also what makes the
/// registry cache honest — a cached score whose version is not this one is
/// rendered stale rather than current (ADR-0053 decision 3). Bump it in
/// the same commit as any check or weight change.
pub const RUBRIC_VERSION: u32 = 1;

/// The most a bundle can score.
pub const MAX_SCORE: u8 = 100;

/// `SKILL.md`'s budget, in characters.
///
/// The open spec's guidance is to keep the manifest short and push detail
/// into bundled files a model opens on demand — the whole point of
/// progressive disclosure. A manifest that carries everything spends the
/// context the design exists to save.
///
/// **The number is calibrated against a real corpus rather than chosen.**
/// Across the 37 installed bundles `tests/skill_corpus_rubric.rs` reads,
/// manifests run 1.2K–33K characters with a median of 8.4K, so the first
/// draft of this constant — 8,000 — failed half the ecosystem for being
/// typical. A quality signal that fires on the median measures the number
/// somebody picked, not the bundle. This sits near the corpus's 75th
/// percentile, so it flags a manifest that is long *by the standard of
/// real skills*, and the corpus test is the standing instrument that
/// re-checks that as the ecosystem moves.
pub const MANIFEST_BUDGET_CHARS: usize = 16_000;

/// The band a `description` is useful in.
///
/// The upper bound is the open spec's own cap (`MAX_SKILL_DESCRIPTION_CHARS`
/// refuses past it, so this check only ever sees the lower half); the lower
/// is where a description stops carrying enough for a client to choose the
/// skill by.
pub const MIN_DESCRIPTION_CHARS: usize = 40;

/// One rubric check's outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckResult {
    /// The check that ran, e.g. `references-resolve`.
    pub check: &'static str,
    /// Whether it passed.
    pub passed: bool,
    /// What it is worth. Awarded in full or not at all — a partial credit
    /// scheme would be a second set of weights nobody could argue with.
    pub weight: u8,
    /// What to tell a reviewer or an author in one phrase. Written for
    /// the failing reading, because the passing one needs no explanation.
    pub title: &'static str,
    /// What specifically was wrong, when a check can say. Never file
    /// content — a path or a count, on AUD-1's discipline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// A whole bundle's automated score.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RubricScore {
    /// The rubric that produced it.
    pub rubric_version: u32,
    /// The sum of the passing checks' weights, 0..=100.
    pub score: u8,
    /// Every check, in table order — passing ones included, because "the
    /// rubric ran and this passed" and "the rubric does not check this"
    /// must not look the same to somebody deciding whether to trust the
    /// number.
    pub checks: Vec<CheckResult>,
}

impl RubricScore {
    /// The checks that failed, in table order.
    #[must_use]
    pub fn failed(&self) -> Vec<&CheckResult> {
        self.checks.iter().filter(|check| !check.passed).collect()
    }

    /// Nothing failed.
    #[must_use]
    pub fn is_perfect(&self) -> bool {
        self.score == MAX_SCORE
    }
}

/// Scores a bundle (ADR-0053 decision 5).
///
/// Synchronous CPU work, O(bundle bytes) over an input ADR-0051 already
/// bounds at 64 files / 256KB — so callers on an async runtime wrap it in
/// `spawn_blocking`, exactly as they wrap its two siblings.
///
/// A bundle with no `SKILL.md` cannot reach here through any authoring
/// path (`SkillBundle::validate` refuses it, and the store's own CHECK
/// refuses it again), but this function is total anyway: every check that
/// needs a manifest fails without one rather than panicking, so a score of
/// 0 is the answer to a bundle that should not exist.
#[must_use]
#[tracing::instrument(name = "ingest.skillrubric.score", skip_all, fields(
    rubric.files = files.len(),
    rubric.score,
))]
pub fn score_bundle(files: &[SkillFile]) -> RubricScore {
    let manifest = files.iter().find(|file| file.path.is_manifest());
    let body = manifest.map_or("", |file| file.content.as_str());
    let frontmatter = manifest.and_then(|file| Frontmatter::parse(&file.content).ok());
    let description = frontmatter
        .as_ref()
        .map_or("", |fm| fm.description.as_str());

    let checks = vec![
        references_resolve(body, files),
        description_states_when(description),
        manifest_concise(manifest.map(|_| body)),
        no_placeholders(files),
        has_examples(body),
        has_structure(body),
        description_length(description),
        files_referenced(body, files),
    ];

    let score = checks
        .iter()
        .filter(|check| check.passed)
        .map(|check| u16::from(check.weight))
        .sum::<u16>()
        .min(u16::from(MAX_SCORE)) as u8;

    tracing::Span::current().record("rubric.score", score);
    RubricScore {
        rubric_version: RUBRIC_VERSION,
        score,
        checks,
    }
}

// ── The checks ──────────────────────────────────────────────────────────

/// Every bundled path the manifest names exists in the bundle (weight 10).
///
/// A failure here is a *runtime* failure in front of a user: a model told
/// to read `scripts/check.py` that was never bundled has nothing to fall
/// back on. That is why the check exists — and the reason it carries 10
/// rather than the 20 it was first drafted with is the most useful thing
/// this rubric learned from a real corpus.
///
/// # Why it only looks inside the bundle's own directories
///
/// **A manifest mentioning a path is not a claim that the bundle contains
/// it.** Run naively over 37 installed bundles, this check fired on 29 of
/// them, and almost none were broken. What real manifests name are files
/// in the *user's* project the skill will read or write (`CLAUDE.md`,
/// `package.json`, `.mcp.json`), illustrative paths inside examples
/// (`src/api/users.ts`, `path/to/file.rs`), and files the skill instructs
/// the agent to *create* (`hooks.json`, `validate.sh`). A check that
/// cannot tell those from a dangling reference is measuring the English
/// language, not the bundle.
///
/// So the claim is narrowed to where it is decidable: a path is treated as
/// a reference to the bundle only if it is multi-segment **and its first
/// segment is a directory the bundle actually has**. `scripts/check.py` in
/// a bundle that ships a `scripts/` directory is a reference; `package.json`
/// is not a reference to anything, and neither is `src/api/users.ts` in a
/// bundle with no `src/`. This is ADR-0052 decision 10's line — what a
/// lexical rule can decide with certainty — arriving one plane over, and
/// the weight moved with the certainty.
fn references_resolve(body: &str, files: &[SkillFile]) -> CheckResult {
    let present: BTreeSet<&str> = files.iter().map(|file| file.path.as_str()).collect();
    let directories: BTreeSet<&str> = files
        .iter()
        .filter_map(|file| file.path.as_str().split_once('/'))
        .map(|(head, _)| head)
        .collect();
    let missing: Vec<String> = referenced_paths(body)
        .into_iter()
        .filter(|path| {
            path.split_once('/')
                .is_some_and(|(head, _)| directories.contains(head))
        })
        .filter(|path| !present.contains(path.as_str()))
        .collect();
    CheckResult {
        check: "references-resolve",
        passed: missing.is_empty(),
        weight: 10,
        title: "every file SKILL.md points at is in the bundle",
        detail: (!missing.is_empty()).then(|| format!("not bundled: {}", missing.join(", "))),
    }
}

/// The description says *when* to reach for the skill (weight 20).
///
/// The other heavy check, and the one that decides whether the skill is
/// ever loaded at all: a client reads descriptions at ~80 tokens to choose
/// among them, and SKIL-4 will advertise this same line. "Formats
/// changelogs" tells a model what the skill is; "use when preparing a
/// release or writing release notes" tells it when to reach for it, and
/// only the second gets it selected.
fn description_states_when(description: &str) -> CheckResult {
    CheckResult {
        check: "description-states-when",
        passed: !description.is_empty() && TRIGGER_LANGUAGE.is_match(description),
        weight: 20,
        title: "the description says when to use the skill, not only what it is",
        detail: (!description.is_empty() && !TRIGGER_LANGUAGE.is_match(description)).then(|| {
            "no trigger phrasing found (\"use when …\", \"when you …\", \"for …ing\")".to_owned()
        }),
    }
}

/// `SKILL.md` is within the progressive-disclosure budget (weight 15).
fn manifest_concise(body: Option<&str>) -> CheckResult {
    let chars = body.map_or(usize::MAX, |body| body.chars().count());
    CheckResult {
        check: "manifest-concise",
        passed: chars <= MANIFEST_BUDGET_CHARS,
        weight: 15,
        title: "SKILL.md is short enough to load every session",
        detail: (chars > MANIFEST_BUDGET_CHARS).then(|| {
            body.map_or_else(
                || format!("no {SKILL_MANIFEST}"),
                |_| format!("{chars} characters against a budget of {MANIFEST_BUDGET_CHARS}"),
            )
        }),
    }
}

/// No unfinished markers anywhere in the bundle (weight 20).
///
/// The one check that reads every file rather than the manifest, because
/// a `TODO` in a script is exactly as unfinished as one in the prose and
/// this is a bundle about to go onto a fleet of laptops. It carries the
/// joint-heaviest weight for the reason the table's preamble gives: it is
/// the most nearly *decidable* check here — a marker is present or it is
/// not — and 6 of 37 real bundles trip it, so it discriminates without
/// judging.
fn no_placeholders(files: &[SkillFile]) -> CheckResult {
    let found: Vec<String> = files
        .iter()
        .filter(|file| PLACEHOLDER.is_match(&file.content))
        .map(|file| file.path.as_str().to_owned())
        .collect();
    CheckResult {
        check: "no-placeholders",
        passed: found.is_empty(),
        weight: 20,
        title: "nothing is left marked unfinished",
        detail: (!found.is_empty()).then(|| format!("markers in: {}", found.join(", "))),
    }
}

/// At least one fenced code block (weight 15).
fn has_examples(body: &str) -> CheckResult {
    let passed = FENCED_BLOCK.is_match(body);
    CheckResult {
        check: "has-examples",
        passed,
        weight: 15,
        title: "SKILL.md shows at least one concrete example",
        detail: (!passed).then(|| "no fenced code block in SKILL.md".to_owned()),
    }
}

/// At least one `##` section (weight 10).
fn has_structure(body: &str) -> CheckResult {
    let passed = SECTION_HEADING.is_match(body);
    CheckResult {
        check: "has-structure",
        passed,
        weight: 10,
        title: "SKILL.md is sectioned so a model can skim it",
        detail: (!passed).then(|| "no `##` headings in SKILL.md".to_owned()),
    }
}

/// The description is long enough to be informative (weight 5).
fn description_length(description: &str) -> CheckResult {
    let chars = description.chars().count();
    CheckResult {
        check: "description-length",
        passed: chars >= MIN_DESCRIPTION_CHARS,
        weight: 5,
        title: "the description carries enough to choose the skill by",
        detail: (chars < MIN_DESCRIPTION_CHARS)
            .then(|| format!("{chars} characters, under a floor of {MIN_DESCRIPTION_CHARS}")),
    }
}

/// Every bundled file is named somewhere in `SKILL.md` (weight 5).
///
/// **The check most likely to be wrong**, and weighted accordingly
/// (ADR-0053 decision 5): a helper module imported by another script is a
/// legitimate bundle this marks down. It stays in because the common case
/// it catches is real — a file nothing names is a file no model will ever
/// open, which is dead weight in a bundle every session pays for.
fn files_referenced(body: &str, files: &[SkillFile]) -> CheckResult {
    let orphans: Vec<String> = files
        .iter()
        .filter(|file| !file.path.is_manifest())
        .filter(|file| !body.contains(file.path.as_str()))
        .map(|file| file.path.as_str().to_owned())
        .collect();
    CheckResult {
        check: "files-referenced",
        passed: orphans.is_empty(),
        weight: 5,
        title: "no bundled file goes unmentioned by SKILL.md",
        detail: (!orphans.is_empty()).then(|| format!("never mentioned: {}", orphans.join(", "))),
    }
}

// ── The patterns ────────────────────────────────────────────────────────

/// Multi-segment, bundle-relative paths as a manifest writes them: in
/// backticks, in a fenced command, or bare. Anchored on a known file
/// extension so ordinary prose containing a slash is not mistaken for one.
///
/// **At least one `/` is required**, and that is load-bearing twice over.
/// It is what `references_resolve` needs (a reference is only checkable
/// against a directory the bundle has), and it is what stops `Node.js` and
/// `Next.js` — real matches from the first run over a real corpus — being
/// read as filenames because `.js` is an extension.
static PATH_REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        (?:^|[\s`'\x22(\[])
        (?P<path>
            (?:[A-Za-z0-9._-]+/)+
            [A-Za-z0-9._-]+
            \.(?:py|sh|js|ts|rb|pl|md|txt|json|ya?ml|toml|csv|sql|r|go|rs)
        )
        (?:$|[\s`'\x22)\].,;:!?])
        ",
    )
    .expect("path reference pattern compiles")
});

/// Phrasing that tells a model *when* to reach for the skill.
static TRIGGER_LANGUAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?ix)
        \b(?:
              use \s+ (?:this \s+)? (?:skill \s+)? when
            | use \s+ when
            | when \s+ (?:you|the \s+ user|a \s+ user|asked|working|writing|reviewing|building|debugging|creating)
            | whenever
            | if \s+ (?:you|the \s+ user|a \s+ user)
            | for \s+ \w+ing\b
            | triggers? \s+ on
            | invoke \s+ (?:this|it) \s+ (?:when|for)
            | reach \s+ for \s+ (?:this|it)
        )",
    )
    .expect("trigger language pattern compiles")
});

/// Markers that say the author was not finished.
static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:TODO|FIXME|XXX|TBD|lorem ipsum)\b|<(?:placeholder|your[- _][a-z_-]+)>")
        .expect("placeholder pattern compiles")
});

/// A fenced code block: three backticks or three tildes at a line start,
/// twice.
static FENCED_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?ms)^\s*(?:```|~~~)[\s\S]*?^\s*(?:```|~~~)")
        .expect("fenced block pattern compiles")
});

/// A markdown section heading, `##` or deeper. `#` alone is the title.
static SECTION_HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s{0,3}#{2,6}\s+\S").expect("heading pattern compiles"));

/// Every bundle-relative path the manifest mentions, deduplicated.
///
/// Deliberately ignores anything that looks like a URL or an absolute
/// path: `https://example.com/setup.sh` is not a claim about the bundle,
/// and neither is `/etc/hosts`.
fn referenced_paths(body: &str) -> Vec<String> {
    let mut found = BTreeSet::new();
    for capture in PATH_REFERENCE.captures_iter(body) {
        let path = &capture["path"];
        let start = capture.name("path").expect("named group").start();
        // A path preceded by `:` `/` or `.` is part of a URL or a longer
        // path expression rather than a bundle-relative reference.
        if body[..start].ends_with([':', '/', '.', '~']) {
            continue;
        }
        found.insert(path.to_owned());
    }
    found.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use synveda_types::SkillFilePath;

    fn file(path: &str, content: &str) -> SkillFile {
        SkillFile {
            path: path.parse::<SkillFilePath>().expect("test path is valid"),
            content: content.to_owned(),
        }
    }

    /// A bundle that scores full marks — the shape every other test in
    /// this module perturbs by exactly one thing.
    fn good_manifest() -> String {
        "---\n\
         name: release-notes\n\
         description: Drafts release notes from a changelog. Use when preparing a release \
         or when the user asks for release notes.\n\
         ---\n\
         \n\
         # Release notes\n\
         \n\
         ## When to use\n\
         \n\
         Reach for this when cutting a release.\n\
         \n\
         ## How\n\
         \n\
         Run the collector, then edit what it produced:\n\
         \n\
         ```sh\n\
         python scripts/collect.py --since v1.2.0\n\
         ```\n\
         \n\
         See `reference/style.md` for the house voice.\n"
            .to_owned()
    }

    fn good_bundle() -> Vec<SkillFile> {
        vec![
            file(SKILL_MANIFEST, &good_manifest()),
            file("scripts/collect.py", "print('notes')\n"),
            file("reference/style.md", "Sentence case.\n"),
        ]
    }

    fn score_of(files: &[SkillFile]) -> u8 {
        score_bundle(files).score
    }

    fn check<'a>(score: &'a RubricScore, name: &str) -> &'a CheckResult {
        score
            .checks
            .iter()
            .find(|check| check.check == name)
            .unwrap_or_else(|| panic!("no check named {name}"))
    }

    #[test]
    fn a_well_made_bundle_scores_full_marks() {
        let scored = score_bundle(&good_bundle());
        assert_eq!(scored.score, MAX_SCORE, "{:#?}", scored.failed());
        assert!(scored.is_perfect());
        assert_eq!(scored.rubric_version, RUBRIC_VERSION);
    }

    #[test]
    fn the_weights_sum_to_the_maximum() {
        // The property that makes `score` a percentage rather than a
        // number out of whatever the table happens to add up to.
        let scored = score_bundle(&good_bundle());
        let total: u16 = scored.checks.iter().map(|c| u16::from(c.weight)).sum();
        assert_eq!(total, u16::from(MAX_SCORE));
        // And every check reports, passing ones included: "this passed"
        // and "this is not checked" must not look the same.
        assert_eq!(scored.checks.len(), 8);
        assert!(scored.checks.iter().all(|check| check.passed));
    }

    #[test]
    fn a_reference_into_a_directory_the_bundle_has_must_resolve() {
        // The case the check is narrowed *to*: the bundle ships a
        // `scripts/` directory, so `scripts/` is a namespace it owns, and
        // a path under it that is not there is a model sent to read
        // nothing.
        let manifest = good_manifest().replace(
            "python scripts/collect.py --since v1.2.0",
            "python scripts/collect.py --since v1.2.0\npython scripts/publish.py",
        );
        let mut files = good_bundle();
        files[0] = file(SKILL_MANIFEST, &manifest);
        let scored = score_bundle(&files);
        let result = check(&scored, "references-resolve");
        assert!(!result.passed, "{scored:#?}");
        assert_eq!(result.weight, 10);
        assert!(
            result
                .detail
                .as_deref()
                .unwrap()
                .contains("scripts/publish.py"),
            "{result:?}"
        );
        assert_eq!(scored.score, 90);
    }

    #[test]
    fn a_path_the_bundle_owns_no_directory_for_is_not_a_reference_to_it() {
        // **The finding that reset this check's weight.** Run naively over
        // 37 installed bundles it fired on 29, and almost none were
        // broken: real manifests name files in the *user's* project, paths
        // inside illustrative examples, and files the skill tells the
        // agent to create. None of those are claims about the bundle, and
        // a check that cannot tell them apart is measuring English.
        let manifest = good_manifest().replace(
            "See `reference/style.md` for the house voice.",
            "See `reference/style.md`. Read the project's `package.json` and `CLAUDE.md` \
             first, then update `src/api/users.ts` — for example `path/to/file.rs`. \
             Built with Node.js against https://example.com/docs/style.md.",
        );
        let mut files = good_bundle();
        files[0] = file(SKILL_MANIFEST, &manifest);
        let scored = score_bundle(&files);
        assert!(
            check(&scored, "references-resolve").passed,
            "{:#?}",
            check(&scored, "references-resolve")
        );
    }

    #[test]
    fn node_js_is_a_word_and_not_a_filename() {
        // A real match from the first corpus run: `.js` is an extension,
        // so `Node.js` and `Next.js` were read as bundled files. Requiring
        // a separator is what fixed it.
        for prose in [
            "Built with Node.js.",
            "A Next.js project.",
            "See jQuery.js.",
        ] {
            assert!(
                !PATH_REFERENCE.is_match(prose),
                "wrongly read as a path: {prose}"
            );
        }
        assert!(PATH_REFERENCE.is_match("run `scripts/collect.py` first"));
    }

    #[test]
    fn a_description_that_says_only_what_it_is_loses_twenty() {
        let manifest = good_manifest().replace(
            "Drafts release notes from a changelog. Use when preparing a release \
             or when the user asks for release notes.",
            "A tool that produces release notes from the repository changelog file.",
        );
        let mut files = good_bundle();
        files[0] = file(SKILL_MANIFEST, &manifest);
        let scored = score_bundle(&files);
        let result = check(&scored, "description-states-when");
        assert!(!result.passed, "{scored:#?}");
        assert_eq!(result.weight, 20);
        assert_eq!(scored.score, 80);
    }

    #[test]
    fn several_spellings_of_when_all_count() {
        // The check is a heuristic and its job is to catch the ways real
        // skills phrase a trigger, not one blessed form.
        for description in [
            "Formats changelogs. Use when preparing a release.",
            "Formats changelogs — use this skill when the user asks for notes.",
            "Formats changelogs, for drafting release announcements.",
            "Whenever a release is cut, this drafts the notes for it.",
            "Reach for this if you need release notes from a changelog file.",
        ] {
            assert!(
                TRIGGER_LANGUAGE.is_match(description),
                "not recognised: {description}"
            );
        }
        for description in [
            "A tool that produces release notes from a repository changelog.",
            "Release notes generator.",
        ] {
            assert!(
                !TRIGGER_LANGUAGE.is_match(description),
                "wrongly recognised: {description}"
            );
        }
    }

    #[test]
    fn an_unfinished_marker_anywhere_in_the_bundle_counts() {
        // Not only in the prose: a TODO in a script is exactly as
        // unfinished, and this bundle is about to reach a fleet of
        // laptops.
        let mut files = good_bundle();
        files[1] = file(
            "scripts/collect.py",
            "# TODO: handle tags\nprint('notes')\n",
        );
        let scored = score_bundle(&files);
        let result = check(&scored, "no-placeholders");
        assert!(!result.passed);
        assert_eq!(
            result.weight, 20,
            "the joint-heaviest, because it is a fact"
        );
        assert!(
            result
                .detail
                .as_deref()
                .unwrap()
                .contains("scripts/collect.py"),
            "{result:?}"
        );
        assert_eq!(scored.score, 80);
    }

    #[test]
    fn an_orphan_file_costs_the_least_because_the_check_is_the_least_certain() {
        // A helper imported by another script rather than named in the
        // manifest is a legitimate bundle this marks down. Five points is
        // what that is allowed to cost (ADR-0053 decision 5).
        let mut files = good_bundle();
        files.push(file("scripts/_util.py", "def helper():\n    pass\n"));
        let scored = score_bundle(&files);
        let result = check(&scored, "files-referenced");
        assert!(!result.passed);
        assert_eq!(result.weight, 5);
        assert_eq!(scored.score, 95);
    }

    #[test]
    fn a_manifest_over_budget_loses_fifteen() {
        let mut files = good_bundle();
        let bloated = format!(
            "{}\n{}",
            good_manifest(),
            "Every detail, inline, forever. ".repeat(600)
        );
        files[0] = file(SKILL_MANIFEST, &bloated);
        let scored = score_bundle(&files);
        let result = check(&scored, "manifest-concise");
        assert!(!result.passed, "{scored:#?}");
        assert!(
            result
                .detail
                .as_deref()
                .unwrap()
                .contains(&MANIFEST_BUDGET_CHARS.to_string()),
            "{result:?}"
        );
        assert_eq!(scored.score, 85);
    }

    #[test]
    fn a_manifest_with_no_examples_and_no_sections_loses_both() {
        let plain = "---\n\
                     name: thin\n\
                     description: Does a thing. Use when you need that thing done properly.\n\
                     ---\n\
                     \n\
                     Just a paragraph of prose with no sections and no examples at all.\n";
        let scored = score_bundle(&[file(SKILL_MANIFEST, plain)]);
        assert!(!check(&scored, "has-examples").passed);
        assert_eq!(check(&scored, "has-examples").weight, 15);
        assert!(!check(&scored, "has-structure").passed);
        assert_eq!(check(&scored, "has-structure").weight, 10);
        assert_eq!(scored.score, 75);
    }

    #[test]
    fn a_short_description_loses_the_smallest_weight_it_can() {
        let manifest = "---\n\
                        name: thin\n\
                        description: Use when tidying.\n\
                        ---\n\
                        \n\
                        ## How\n\
                        \n\
                        ```sh\n\
                        tidy\n\
                        ```\n";
        let scored = score_bundle(&[file(SKILL_MANIFEST, manifest)]);
        let result = check(&scored, "description-length");
        assert!(!result.passed, "{scored:#?}");
        assert_eq!(result.weight, 5);
        assert_eq!(scored.score, 95);
    }

    #[test]
    fn a_bundle_with_no_manifest_scores_low_rather_than_panicking() {
        // Unreachable through any authoring path — `SkillBundle::validate`
        // refuses it and the store's CHECK refuses it again — but this
        // function is total, because a scoring pass that can panic is one
        // that takes the gateway down for a bundle nobody could publish.
        let scored = score_bundle(&[file("scripts/only.py", "print('hi')\n")]);
        assert!(scored.score < 50, "{scored:#?}");
        assert!(!check(&scored, "manifest-concise").passed);
        assert!(!check(&scored, "description-states-when").passed);
        // The one check that still passes on its own terms: nothing was
        // referenced, so nothing dangles.
        assert!(check(&scored, "references-resolve").passed);
    }

    #[test]
    fn the_three_checks_that_pass_vacuously_pass_vacuously_on_purpose() {
        // An empty bundle scores 40 rather than 0, because
        // `references-resolve`, `no-placeholders` and `files-referenced`
        // have nothing to object to. That reads like a bug and is not
        // one: those three are the checks a **single-file skill**
        // genuinely satisfies — a SKILL.md that bundles nothing has no
        // dangling reference and no orphan, and refusing it the points
        // would be marking a legitimate shape down for being small.
        //
        // What stops that being a loophole is that the vacuous points do
        // not add up to anything: 40 is under every product pack's bar,
        // and the four checks that carry a bundle over one all read the
        // manifest.
        let scored = score_bundle(&[]);
        assert_eq!(scored.score, 35, "{scored:#?}");
        assert_eq!(scored.checks.len(), 8);
        for name in ["references-resolve", "no-placeholders", "files-referenced"] {
            assert!(check(&scored, name).passed, "{name} should pass vacuously");
        }
        assert!(
            scored.score < synveda_types::SkillQualityConfig::MODERATE.min_score,
            "vacuous passes must not clear a pack's bar"
        );
    }

    #[test]
    fn the_same_bytes_score_the_same_twice() {
        // The property the registry cache and the publish gate both rely
        // on: the score is a pure function of the bundle, so a cached one
        // and a recomputed one agree or the rubric version differs.
        let files = good_bundle();
        assert_eq!(score_bundle(&files), score_bundle(&files));
        // And file order does not change it — the store returns path
        // order, a proposal returns member order, and the two must agree.
        let mut shuffled = files.clone();
        shuffled.reverse();
        assert_eq!(score_of(&files), score_of(&shuffled));
    }

    #[test]
    fn no_detail_carries_file_content() {
        // AUD-1's discipline: a score renders in a review and rides an
        // audit payload, and neither may carry the bytes.
        let mut files = good_bundle();
        files[1] = file(
            "scripts/collect.py",
            "# TODO: the secret is hunter2\nprint('notes')\n",
        );
        let scored = score_bundle(&files);
        let rendered = serde_json::to_string(&scored).unwrap();
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(!rendered.contains("print("), "{rendered}");
        // The path is named, because that is what a reviewer needs to
        // open the file beside the report.
        assert!(rendered.contains("scripts/collect.py"), "{rendered}");
    }
}
