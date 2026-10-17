//! Builds the scripted fake vendor CLI (`fake_vendor.rs`) once per test
//! binary and places copies where a fixture's PATH will find them.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// Copies the fake vendor into `bin` as `name` plus the platform suffix.
pub fn install(bin: &Path, name: &str) -> PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    let built = BUILT.get_or_init(|| {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/fake_vendor.rs");
        let output = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "synveda-fake-vendor-{}{}",
            std::process::id(),
            std::env::consts::EXE_SUFFIX
        ));
        let status = Command::new("rustc")
            .args(["--edition", "2024", "--crate-name", "fake_vendor", "-o"])
            .arg(&output)
            .arg(&source)
            .status()
            .expect("rustc for the fake vendor");
        assert!(status.success(), "fake vendor did not compile");
        output
    });
    let target = bin.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(built, &target).expect("place fake vendor");
    target
}

/// Writes the vendor's rules: one `(argument line, actions)` row each.
pub fn rules(fixture: &Path, rules: &[(&str, &[&str])]) {
    let text: String = rules
        .iter()
        .map(|(command, actions)| format!("{command}\t{}\n", actions.join("\t")))
        .collect();
    std::fs::write(fixture.join("rules.txt"), text).expect("fake vendor rules");
}

/// The spelling native programs report for a canonicalized path: Rust's
/// verbatim `\\?\C:\` drive prefix is dropped, matching the CLI.
pub fn conventional(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|text| text.strip_prefix(r"\\?\")) {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}
