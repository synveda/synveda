//! Agent Skills bundles read from one commit of a Git work tree (ADR-0085
//! amendment 1). Bytes come from the object database, never the working
//! tree, so a version's revision describes exactly what was stored. Only
//! plumbing that runs no hooks, filters or external diff drivers is used.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use synveda_types::{
    MAX_SKILL_BUNDLE_CHARS, MAX_SKILL_FILE_CHARS, MAX_SKILL_FILES, SKILL_MANIFEST, SkillFile,
    SkillFilePath, SkillName,
};

/// Skills one run may import.
const MAX_SKILLS: usize = 256;
/// Tree entries one listing may hold; a larger folder must be narrowed.
const MAX_TREE_ENTRIES: usize = 200_000;
const MAX_LISTING_BYTES: usize = 64 * 1024 * 1024;
const MAX_SMALL_OUTPUT: usize = 64 * 1024;
const GIT_TIMEOUT: Duration = Duration::from_secs(120);
// Rust's character limit counts Unicode scalars; each occupies at most four
// UTF-8 bytes.
const MAX_FILE_BYTES: u64 = (MAX_SKILL_FILE_CHARS * 4) as u64;
const MAX_BUNDLE_BYTES: u64 = (MAX_SKILL_BUNDLE_CHARS * 4) as u64;

/// The commit a run imports from.
#[derive(Debug)]
pub(crate) struct Source {
    pub(crate) commit: String,
    /// Credential-free `origin` remote, or the work tree's directory name.
    pub(crate) repository: String,
}

/// One skill directory read from that commit.
#[derive(Debug)]
pub(crate) struct GitSkill {
    pub(crate) name: SkillName,
    /// Path from the work tree root; `.` for the root itself.
    pub(crate) path: String,
    pub(crate) tree: String,
    pub(crate) files: Vec<SkillFile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Blob,
    Tree,
    Submodule,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    mode: String,
    kind: Kind,
    oid: String,
    size: Option<u64>,
    path: String,
}

#[derive(Debug)]
struct Planned {
    name: SkillName,
    dir: String,
    tree: String,
    files: Vec<(SkillFilePath, String)>,
    bytes: u64,
}

/// Read every skill under `folder` as committed at `rev`.
pub(crate) fn read(folder: &Path, rev: &str) -> Result<(Source, Vec<GitSkill>), String> {
    if rev.is_empty() || rev.starts_with('-') || rev.chars().any(char::is_control) {
        return Err(format!("--rev {rev:?} is not a revision"));
    }
    let program = crate::harness::which("git").ok_or_else(|| "git is not on PATH".to_owned())?;
    let located = locate(&program, folder, rev)?;
    let listing = git(
        &program,
        &located.top,
        &["ls-tree", "-r", "-t", "-l", "-z", &located.tree],
        None,
        MAX_LISTING_BYTES,
    )?;
    let entries = parse_listing(&listing)?;
    let folder_name = match located.prefix.rsplit('/').next() {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => base_name(&located.top)?,
    };
    let planned = discover(&entries, &located.tree, &folder_name)?;
    let contents = read_blobs(&program, &located.top, &entries, &planned)?;
    let skills = planned
        .into_iter()
        .map(|skill| assemble(skill, &located.prefix, &contents))
        .collect::<Result<Vec<_>, _>>()?;
    let repository = remote(&program, &located.top).map_or_else(|| base_name(&located.top), Ok)?;
    Ok((
        Source {
            commit: located.commit,
            repository,
        },
        skills,
    ))
}

/// Where `folder` sits in its work tree, and the commit and tree to read.
struct Located {
    top: PathBuf,
    prefix: String,
    commit: String,
    tree: String,
}

fn locate(program: &Path, folder: &Path, rev: &str) -> Result<Located, String> {
    let query = |dir: &Path, args: &[&str]| -> Result<String, String> {
        line(&git(program, dir, args, None, MAX_SMALL_OUTPUT)?)
    };
    let top = PathBuf::from(query(folder, &["rev-parse", "--show-toplevel"])?);
    let prefix = query(folder, &["rev-parse", "--show-prefix"])?
        .trim_end_matches('/')
        .to_owned();
    let commit = query(
        &top,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{rev}^{{commit}}"),
        ],
    )
    .map_err(|_| format!("{rev} is not a commit in {}", top.display()))?;
    if !is_object_id(&commit) {
        return Err(format!("git returned {commit:?} as a commit id"));
    }
    let folder_tree = if prefix.is_empty() {
        format!("{commit}^{{tree}}")
    } else {
        format!("{commit}:{prefix}")
    };
    let tree = query(&top, &["rev-parse", "--verify", "--quiet", &folder_tree]).map_err(|_| {
        format!(
            "{} does not exist in commit {commit}",
            display_path(&prefix)
        )
    })?;
    if !is_object_id(&tree) {
        return Err(format!("git returned {tree:?} as a tree id"));
    }
    Ok(Located {
        top,
        prefix,
        commit,
        tree,
    })
}

/// `reference` for one skill: the repository, then `#` and the skill's path.
pub(crate) fn reference(source: &Source, skill: &GitSkill) -> String {
    format!("{}#{}", source.repository, skill.path)
}

fn assemble(
    skill: Planned,
    prefix: &str,
    contents: &BTreeMap<String, String>,
) -> Result<GitSkill, String> {
    let path = match (prefix.is_empty(), skill.dir.is_empty()) {
        (true, true) => ".".to_owned(),
        (true, false) => skill.dir.clone(),
        (false, true) => prefix.to_owned(),
        (false, false) => format!("{prefix}/{}", skill.dir),
    };
    let mut files = Vec::with_capacity(skill.files.len());
    for (bundled, oid) in skill.files {
        let content = contents
            .get(&oid)
            .ok_or_else(|| format!("{path}/{bundled} was not read from the commit"))?
            .clone();
        if content.chars().count() > MAX_SKILL_FILE_CHARS {
            return Err(format!(
                "{path}/{bundled} exceeds the {MAX_SKILL_FILE_CHARS}-character file limit"
            ));
        }
        files.push(SkillFile {
            path: bundled,
            content,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(GitSkill {
        name: skill.name,
        path,
        tree: skill.tree,
        files,
    })
}

/// Skill directories in one listing, refused as a whole when any is ambiguous.
fn discover(entries: &[Entry], root_tree: &str, folder_name: &str) -> Result<Vec<Planned>, String> {
    let dirs = entries
        .iter()
        .filter(|entry| entry.kind == Kind::Blob && file_name(&entry.path) == SKILL_MANIFEST)
        .map(|entry| parent(&entry.path).to_owned())
        .collect::<BTreeSet<_>>();
    if dirs.is_empty() {
        return Err(format!(
            "no {SKILL_MANIFEST} under {folder_name} in this commit"
        ));
    }
    if dirs.len() > MAX_SKILLS {
        return Err(format!(
            "{} skills found; one run imports at most {MAX_SKILLS}, so import narrower folders",
            dirs.len()
        ));
    }
    for dir in &dirs {
        if let Some(outer) = owner(dir, &dirs, true) {
            return Err(format!(
                "skill {} lies inside skill {}; a skill's files cannot hold another skill",
                display_path(dir),
                display_path(outer)
            ));
        }
    }
    let mut planned = plan_dirs(&dirs, entries, root_tree, folder_name)?;
    for entry in entries {
        let Some(dir) = owner(&entry.path, &dirs, false) else {
            continue;
        };
        let skill = planned
            .get_mut(dir.as_str())
            .ok_or_else(|| format!("no plan for {dir}"))?;
        admit(skill, entry)?;
    }
    Ok(planned.into_values().collect())
}

/// Name each skill directory and find its tree, refusing duplicate names.
fn plan_dirs<'a>(
    dirs: &'a BTreeSet<String>,
    entries: &[Entry],
    root_tree: &str,
    folder_name: &str,
) -> Result<BTreeMap<&'a str, Planned>, String> {
    let trees = entries
        .iter()
        .filter(|entry| entry.kind == Kind::Tree)
        .map(|entry| (entry.path.as_str(), entry.oid.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut planned = BTreeMap::new();
    let mut names = BTreeMap::<String, &str>::new();
    for dir in dirs {
        let label = if dir.is_empty() {
            folder_name
        } else {
            file_name(dir)
        };
        let name = label
            .parse::<SkillName>()
            .map_err(|err| format!("{}: {err}", display_path(dir)))?;
        if let Some(other) = names.insert(name.to_string(), dir) {
            return Err(format!(
                "skills {} and {} are both named {name}",
                display_path(other),
                display_path(dir)
            ));
        }
        let tree = if dir.is_empty() {
            root_tree
        } else {
            trees
                .get(dir.as_str())
                .copied()
                .ok_or_else(|| format!("no tree entry for {dir}"))?
        };
        planned.insert(
            dir.as_str(),
            Planned {
                name,
                dir: dir.clone(),
                tree: tree.to_owned(),
                files: Vec::new(),
                bytes: 0,
            },
        );
    }
    Ok(planned)
}

/// Add one listed entry to the skill that holds it.
fn admit(skill: &mut Planned, entry: &Entry) -> Result<(), String> {
    match (entry.kind, entry.mode.as_str()) {
        (Kind::Tree, _) => return Ok(()),
        (Kind::Submodule, _) => {
            return Err(format!(
                "{} is a submodule; bundles carry bytes",
                entry.path
            ));
        }
        (Kind::Blob, "120000") => {
            return Err(format!(
                "{} is a symbolic link; bundles carry bytes",
                entry.path
            ));
        }
        (Kind::Blob, "100644" | "100755") => {}
        (Kind::Blob, mode) => return Err(format!("{} has unsupported mode {mode}", entry.path)),
    }
    let relative = if skill.dir.is_empty() {
        entry.path.as_str()
    } else {
        &entry.path[skill.dir.len() + 1..]
    };
    let bundled = relative
        .parse::<SkillFilePath>()
        .map_err(|err| format!("{}: {err}", entry.path))?;
    let size = entry.size.unwrap_or(u64::MAX);
    if size > MAX_FILE_BYTES {
        return Err(format!(
            "{} exceeds the {MAX_FILE_BYTES}-byte file limit",
            entry.path
        ));
    }
    skill.bytes = skill.bytes.saturating_add(size);
    if skill.bytes > MAX_BUNDLE_BYTES {
        return Err(format!(
            "skill {} exceeds the {MAX_BUNDLE_BYTES}-byte bundle limit",
            display_path(&skill.dir)
        ));
    }
    if skill.files.len() >= MAX_SKILL_FILES {
        return Err(format!(
            "skill {} has more than {MAX_SKILL_FILES} files",
            display_path(&skill.dir)
        ));
    }
    skill.files.push((bundled, entry.oid.clone()));
    Ok(())
}

/// The skill directory holding `path`: its nearest proper ancestor in
/// `dirs`, or `path` itself when `strict` is false and it is one.
fn owner<'a>(path: &str, dirs: &'a BTreeSet<String>, strict: bool) -> Option<&'a String> {
    if !strict && let Some(found) = dirs.get(path) {
        return Some(found);
    }
    let mut current = path;
    while !current.is_empty() {
        current = parent(current);
        if let Some(found) = dirs.get(current) {
            return Some(found);
        }
    }
    None
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

fn file_name(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}

fn display_path(path: &str) -> &str {
    if path.is_empty() { "." } else { path }
}

/// Parse `ls-tree -r -t -l -z` output.
fn parse_listing(bytes: &[u8]) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for record in bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        if entries.len() == MAX_TREE_ENTRIES {
            return Err(format!(
                "the folder holds more than {MAX_TREE_ENTRIES} entries; import a narrower folder"
            ));
        }
        let text = std::str::from_utf8(record)
            .map_err(|_| format!("{} is not a UTF-8 path", String::from_utf8_lossy(record)))?;
        let (meta, path) = text
            .split_once('\t')
            .ok_or_else(|| format!("unexpected ls-tree record {text:?}"))?;
        let fields = meta.split_ascii_whitespace().collect::<Vec<_>>();
        let [mode, kind, oid, size] = fields.as_slice() else {
            return Err(format!("unexpected ls-tree record {text:?}"));
        };
        let kind = match *kind {
            "blob" => Kind::Blob,
            "tree" => Kind::Tree,
            "commit" => Kind::Submodule,
            other => return Err(format!("{path} has unexpected object type {other}")),
        };
        if !is_object_id(oid) || path.is_empty() {
            return Err(format!("unexpected ls-tree record {text:?}"));
        }
        let size = match *size {
            "-" => None,
            digits => Some(
                digits
                    .parse::<u64>()
                    .map_err(|_| format!("unexpected ls-tree size in {text:?}"))?,
            ),
        };
        entries.push(Entry {
            mode: (*mode).to_owned(),
            kind,
            oid: (*oid).to_owned(),
            size,
            path: path.to_owned(),
        });
    }
    Ok(entries)
}

/// Read every planned blob through one `cat-file --batch`.
fn read_blobs(
    program: &Path,
    top: &Path,
    entries: &[Entry],
    planned: &[Planned],
) -> Result<BTreeMap<String, String>, String> {
    let sizes = entries
        .iter()
        .filter(|entry| entry.kind == Kind::Blob)
        .filter_map(|entry| entry.size.map(|size| (entry.oid.as_str(), size)))
        .collect::<BTreeMap<_, _>>();
    let wanted = planned
        .iter()
        .flat_map(|skill| skill.files.iter().map(|(_, oid)| oid.as_str()))
        .collect::<BTreeSet<_>>();
    let mut input = Vec::new();
    let mut limit = 0usize;
    for oid in &wanted {
        input.extend_from_slice(oid.as_bytes());
        input.push(b'\n');
        let size = sizes.get(oid).copied().unwrap_or(MAX_FILE_BYTES);
        limit = limit
            .saturating_add(usize::try_from(size).unwrap_or(usize::MAX))
            .saturating_add(128);
    }
    let output = git(program, top, &["cat-file", "--batch"], Some(input), limit)?;
    parse_batch(&output, &wanted)
}

/// Parse `cat-file --batch` output for exactly the requested blobs.
fn parse_batch(bytes: &[u8], wanted: &BTreeSet<&str>) -> Result<BTreeMap<String, String>, String> {
    let mut contents = BTreeMap::new();
    let mut rest = bytes;
    for _ in 0..wanted.len() {
        let end = rest
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or_else(|| "truncated cat-file output".to_owned())?;
        let header = std::str::from_utf8(&rest[..end])
            .map_err(|_| "unexpected cat-file header".to_owned())?;
        let fields = header.split(' ').collect::<Vec<_>>();
        let [oid, "blob", size] = fields.as_slice() else {
            return Err(format!("git could not read object {header:?}"));
        };
        if !wanted.contains(oid) {
            return Err(format!("git returned unrequested object {oid}"));
        }
        let size = size
            .parse::<usize>()
            .map_err(|_| format!("unexpected cat-file header {header:?}"))?;
        let body = rest
            .get(end + 1..end + 1 + size)
            .ok_or_else(|| format!("object {oid} is truncated"))?;
        if rest.get(end + 1 + size) != Some(&b'\n') {
            return Err(format!("object {oid} is not terminated"));
        }
        let text = String::from_utf8(body.to_vec())
            .map_err(|_| format!("object {oid} is not UTF-8 text"))?;
        contents.insert((*oid).to_owned(), text);
        rest = &rest[end + 2 + size..];
    }
    if !rest.is_empty() || contents.len() != wanted.len() {
        return Err("unexpected trailing cat-file output".to_owned());
    }
    Ok(contents)
}

/// The `origin` remote without credentials, or `None` when it has no
/// shareable form.
fn remote(program: &Path, top: &Path) -> Option<String> {
    let output = git(
        program,
        top,
        &["remote", "get-url", "origin"],
        None,
        MAX_SMALL_OUTPUT,
    )
    .ok()?;
    sanitize_remote(&line(&output).ok()?)
}

/// A remote reduced to a shareable locator: URL user names, passwords,
/// queries and fragments and scp-style user names are removed; local paths
/// and `file:` URLs yield `None`.
fn sanitize_remote(remote: &str) -> Option<String> {
    let remote = remote.trim();
    if remote.is_empty() || remote.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    if remote.contains("://") {
        let mut url = url::Url::parse(remote).ok()?;
        if url.scheme() == "file" || url.host_str().is_none_or(str::is_empty) {
            return None;
        }
        url.set_username("").ok()?;
        url.set_password(None).ok()?;
        url.set_query(None);
        url.set_fragment(None);
        return Some(url.to_string());
    }
    let (host, path) = remote.split_once(':')?;
    let host = host.rsplit_once('@').map_or(host, |(_, host)| host);
    // A drive letter or a slash before the colon is a local path.
    if host.len() <= 1 || host.contains(['/', '\\']) || path.is_empty() {
        return None;
    }
    let path = path.split(['?', '#']).next().unwrap_or_default();
    Some(format!("{host}:{path}"))
}

fn base_name(top: &Path) -> Result<String, String> {
    top.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or_else(|| format!("{} has no directory name", top.display()))
}

fn is_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn line(output: &[u8]) -> Result<String, String> {
    let text =
        std::str::from_utf8(output).map_err(|_| "git printed non-UTF-8 output".to_owned())?;
    Ok(text.trim_end_matches(['\n', '\r']).to_owned())
}

/// Run `git -C <dir> <args>`, bounded in time and output. Exceeding the
/// output bound closes the pipe, so git stops rather than waiting.
fn git(
    program: &Path,
    dir: &Path,
    args: &[&str],
    input: Option<Vec<u8>>,
    max_output: usize,
) -> Result<Vec<u8>, String> {
    let shown = format!("git {}", args.join(" "));
    let mut child = Command::new(program)
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("start {}: {err}", program.display()))?;
    let writer = child.stdin.take().zip(input).map(|(mut stdin, input)| {
        std::thread::spawn(move || {
            // A refused write means git already exited; its status reports why.
            let _ = stdin.write_all(&input);
        })
    });
    let stdout = child.stdout.take().ok_or("git stdout was not captured")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout.take(max_output as u64 + 1).read_to_end(&mut bytes);
        result.map(|_| bytes)
    });
    let stderr = child.stderr.take().ok_or("git stderr was not captured")?;
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        // Diagnostics are best effort; the exit status is authoritative.
        let _ = stderr.take(4096).read_to_string(&mut text);
        text
    });
    let status = wait(&mut child, &shown)?;
    if let Some(writer) = writer {
        writer
            .join()
            .map_err(|_| format!("{shown}: input writer panicked"))?;
    }
    let bytes = reader
        .join()
        .map_err(|_| format!("{shown}: output reader panicked"))?
        .map_err(|err| format!("{shown}: {err}"))?;
    let detail = errors.join().unwrap_or_default();
    if bytes.len() > max_output {
        return Err(format!("{shown} printed more than {max_output} bytes"));
    }
    if !status.success() {
        return Err(format!("{shown} failed ({status}): {}", detail.trim()));
    }
    Ok(bytes)
}

fn wait(child: &mut std::process::Child, shown: &str) -> Result<std::process::ExitStatus, String> {
    let deadline = Instant::now() + GIT_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                // Killing and reaping a timed-out child; the timeout is the error.
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{shown} did not finish within {} seconds",
                    GIT_TIMEOUT.as_secs()
                ));
            }
            Err(err) => return Err(format!("wait for {shown}: {err}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";
    const T: &str = "3333333333333333333333333333333333333333";

    fn blob(path: &str, oid: &str) -> Entry {
        Entry {
            mode: "100644".to_owned(),
            kind: Kind::Blob,
            oid: oid.to_owned(),
            size: Some(10),
            path: path.to_owned(),
        }
    }

    fn tree(path: &str) -> Entry {
        Entry {
            mode: "040000".to_owned(),
            kind: Kind::Tree,
            oid: T.to_owned(),
            size: None,
            path: path.to_owned(),
        }
    }

    #[test]
    fn listing_parses_blobs_trees_and_submodules() {
        let listing = format!(
            "040000 tree {T}       -\tskills\0100644 blob {A}     812\tskills/a/SKILL.md\0160000 commit {B}       -\tvendor\0"
        );
        let entries = parse_listing(listing.as_bytes()).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0], tree("skills"));
        assert_eq!(entries[1].size, Some(812));
        assert_eq!(entries[1].path, "skills/a/SKILL.md");
        assert_eq!(entries[2].kind, Kind::Submodule);
        for bad in [
            format!("100644 blob {A} 1 skills/a"),
            format!("100644 blob {A}\tskills/a"),
            format!("100644 tag {A} 1\tskills/a"),
            "100644 blob nothex 1\tskills/a".to_owned(),
            format!("100644 blob {A} x\tskills/a"),
        ] {
            assert!(parse_listing(bad.as_bytes()).is_err(), "{bad}");
        }
        assert!(
            parse_listing(b"100644 blob 1111111111111111111111111111111111111111 1\t\xff").is_err()
        );
    }

    #[test]
    fn discovery_names_skills_after_directories_and_ignores_other_files() {
        let entries = vec![
            blob("README.md", A),
            tree("a"),
            blob("a/SKILL.md", A),
            tree("a/scripts"),
            blob("a/scripts/run.sh", B),
            tree("b"),
            blob("b/SKILL.md", B),
        ];
        let planned = discover(&entries, T, "skills").unwrap();
        assert_eq!(planned.len(), 2);
        assert_eq!(planned[0].name.as_str(), "a");
        assert_eq!(
            planned[0]
                .files
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>(),
            ["SKILL.md", "scripts/run.sh"]
        );
        assert_eq!(planned[1].name.as_str(), "b");
        assert_eq!(planned[1].files.len(), 1);

        let root = discover(&[blob("SKILL.md", A), blob("notes.md", B)], T, "my-skill").unwrap();
        assert_eq!(root[0].name.as_str(), "my-skill");
        assert_eq!(root[0].tree, T);
        assert_eq!(root[0].files.len(), 2);
    }

    #[test]
    fn discovery_refuses_ambiguous_or_unsafe_folders() {
        let nested = vec![
            tree("a"),
            blob("a/SKILL.md", A),
            tree("a/b"),
            blob("a/b/SKILL.md", B),
        ];
        assert!(
            discover(&nested, T, "x")
                .unwrap_err()
                .contains("inside skill a")
        );
        let root_and_child = vec![blob("SKILL.md", A), tree("b"), blob("b/SKILL.md", B)];
        assert!(discover(&root_and_child, T, "x").is_err());
        let duplicate = vec![
            tree("one"),
            tree("one/review"),
            blob("one/review/SKILL.md", A),
            tree("two"),
            tree("two/review"),
            blob("two/review/SKILL.md", B),
        ];
        let message = discover(&duplicate, T, "x").unwrap_err();
        assert!(
            message.contains("one/review") && message.contains("two/review"),
            "{message}"
        );
        let mut link = blob("a/link", B);
        link.mode = "120000".to_owned();
        assert!(
            discover(&[tree("a"), blob("a/SKILL.md", A), link], T, "x")
                .unwrap_err()
                .contains("symbolic link")
        );
        let module = Entry {
            kind: Kind::Submodule,
            mode: "160000".to_owned(),
            ..blob("a/vendor", B)
        };
        assert!(
            discover(&[tree("a"), blob("a/SKILL.md", A), module], T, "x")
                .unwrap_err()
                .contains("submodule")
        );
        let mut large = blob("a/big.txt", B);
        large.size = Some(MAX_FILE_BYTES + 1);
        assert!(
            discover(&[tree("a"), blob("a/SKILL.md", A), large], T, "x")
                .unwrap_err()
                .contains("limit")
        );
        assert!(
            discover(&[blob("README.md", A)], T, "x")
                .unwrap_err()
                .contains("no SKILL.md")
        );
        assert!(discover(&[tree("Bad Name"), blob("Bad Name/SKILL.md", A)], T, "x").is_err());
        let mut many = vec![tree("a"), blob("a/SKILL.md", A)];
        many.extend((0..MAX_SKILL_FILES).map(|n| blob(&format!("a/f{n}.md"), B)));
        assert!(discover(&many, T, "x").unwrap_err().contains("more than"));
    }

    #[test]
    fn batch_output_is_bound_to_the_requested_blobs() {
        let wanted = BTreeSet::from([A, B]);
        let output = format!("{A} blob 5\nhello\n{B} blob 0\n\n");
        let contents = parse_batch(output.as_bytes(), &wanted).unwrap();
        assert_eq!(contents[A], "hello");
        assert_eq!(contents[B], "");
        for bad in [
            format!("{A} blob 5\nhello\n"),
            format!("{A} missing\n{B} blob 0\n\n"),
            format!("{A} blob 9\nhello\n{B} blob 0\n\n"),
            format!("{A} blob 5\nhello\n{B} blob 0\n\nextra"),
            format!("{A} tree 5\nhello\n{B} blob 0\n\n"),
        ] {
            assert!(parse_batch(bad.as_bytes(), &wanted).is_err(), "{bad}");
        }
        let binary = [
            format!("{A} blob 1\n").into_bytes(),
            vec![0xff, b'\n'],
            format!("{B} blob 0\n\n").into_bytes(),
        ]
        .concat();
        assert!(parse_batch(&binary, &wanted).unwrap_err().contains("UTF-8"));
    }

    #[test]
    fn remotes_lose_credentials_and_local_paths() {
        for (remote, expected) in [
            (
                "https://github.com/org/repo.git",
                Some("https://github.com/org/repo.git"),
            ),
            (
                "https://user:token@gitlab.example.com/org/repo.git",
                Some("https://gitlab.example.com/org/repo.git"),
            ),
            (
                "https://x-token-auth:abc@bitbucket.org/org/repo.git?private_token=abc#frag",
                Some("https://bitbucket.org/org/repo.git"),
            ),
            (
                "ssh://git@host.example:7999/org/repo.git",
                Some("ssh://host.example:7999/org/repo.git"),
            ),
            (
                "git@github.com:org/repo.git",
                Some("github.com:org/repo.git"),
            ),
            (
                "host.example:org/repo.git?x=1",
                Some("host.example:org/repo.git"),
            ),
            ("file:///home/me/repo", None),
            ("/home/me/repo", None),
            ("../repo", None),
            ("C:\\Users\\me\\repo", None),
            ("C:/Users/me/repo", None),
            ("", None),
            ("https://host/a b", None),
        ] {
            assert_eq!(sanitize_remote(remote).as_deref(), expected, "{remote}");
        }
    }

    #[test]
    fn owners_are_nearest_skill_ancestors() {
        let dirs = BTreeSet::from(["a".to_owned(), "a-b".to_owned()]);
        assert_eq!(
            owner("a/x/y.md", &dirs, false).map(String::as_str),
            Some("a")
        );
        assert_eq!(
            owner("a-b/SKILL.md", &dirs, false).map(String::as_str),
            Some("a-b")
        );
        assert_eq!(owner("a", &dirs, false).map(String::as_str), Some("a"));
        assert_eq!(owner("a", &dirs, true), None);
        assert_eq!(owner("c/SKILL.md", &dirs, false), None);
        let root = BTreeSet::from([String::new()]);
        assert_eq!(owner("x/y", &root, false).map(String::as_str), Some(""));
    }

    fn run(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=Synveda Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdout(Stdio::null())
            .status()
            .expect("git is required for the repository import test");
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn reads_committed_bytes_not_the_working_tree() {
        let root = std::env::temp_dir().join(format!(
            "synveda-skill-git-{}",
            synveda_types::ScopeId::new()
        ));
        let skills = root.join("repo").join(".claude").join("skills");
        std::fs::create_dir_all(skills.join("review").join("scripts")).unwrap();
        std::fs::create_dir_all(skills.join("deploy")).unwrap();
        std::fs::write(
            skills.join("review").join(SKILL_MANIFEST),
            "---\nname: review\ndescription: Review code.\n---\nCommitted.\n",
        )
        .unwrap();
        std::fs::write(
            skills.join("review").join("scripts").join("check.sh"),
            "echo ok\n",
        )
        .unwrap();
        std::fs::write(
            skills.join("deploy").join(SKILL_MANIFEST),
            "---\nname: deploy\ndescription: Deploy.\n---\n",
        )
        .unwrap();
        let repo = root.join("repo");
        run(&repo, &["init", "-q"]);
        run(
            &repo,
            &[
                "remote",
                "add",
                "origin",
                "https://user:secret@git.example.com/team/skills.git",
            ],
        );
        run(&repo, &["add", "."]);
        run(&repo, &["commit", "-q", "-m", "skills"]);
        std::fs::write(skills.join("review").join(SKILL_MANIFEST), "uncommitted\n").unwrap();
        std::fs::write(skills.join("review").join("untracked.md"), "untracked\n").unwrap();

        let (source, found) = read(&skills, "HEAD").unwrap();
        assert!(is_object_id(&source.commit));
        assert_eq!(source.repository, "https://git.example.com/team/skills.git");
        assert_eq!(
            found
                .iter()
                .map(|skill| skill.name.as_str())
                .collect::<Vec<_>>(),
            ["deploy", "review"]
        );
        let review = &found[1];
        assert_eq!(review.path, ".claude/skills/review");
        assert!(is_object_id(&review.tree));
        assert_eq!(
            reference(&source, review),
            "https://git.example.com/team/skills.git#.claude/skills/review"
        );
        assert_eq!(
            review
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            ["SKILL.md", "scripts/check.sh"]
        );
        assert!(review.files[0].content.ends_with("Committed.\n"));
        assert!(read(&skills, "--output=x").is_err());
        assert!(
            read(&skills, "no-such-revision")
                .unwrap_err()
                .contains("is not a commit")
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
