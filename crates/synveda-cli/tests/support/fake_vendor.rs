//! Test-only vendor CLI. Tests compile this file with `rustc` and copy
//! it into a fixture `bin` directory as `claude[.exe]`, so the same scripted
//! vendor runs on Unix and native Windows without a shell.
//!
//! `$SYNVEDA_FAKE_VENDOR/rules.txt` maps an exact argument line to actions,
//! separated by tabs: `exit-if FILE CODE`, `cat FILE`, `print TEXT`, `log`
//! (append `CWD: ARGS` to `mutations`) and `copy FROM TO`. File names are
//! fixture-relative leaves. An unknown argument line exits 99.

#![forbid(unsafe_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const MAX_RULES_BYTES: u64 = 64 * 1024;
const MAX_RULES: usize = 64;

fn main() -> ExitCode {
    let line = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let Some(fixture) = std::env::var_os("SYNVEDA_FAKE_VENDOR").map(PathBuf::from) else {
        eprintln!("SYNVEDA_FAKE_VENDOR is not set");
        return ExitCode::from(98);
    };
    let rules = match read_rules(&fixture) {
        Ok(rules) => rules,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(98);
        }
    };
    let Some(actions) = rules
        .iter()
        .find(|(command, _)| *command == line)
        .map(|(_, actions)| actions)
    else {
        eprintln!("unexpected native command: {line}");
        return ExitCode::from(99);
    };
    for action in actions {
        match run(&fixture, &line, action) {
            Ok(None) => {}
            Ok(Some(code)) => return ExitCode::from(code),
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::from(97);
            }
        }
    }
    ExitCode::SUCCESS
}

fn read_rules(fixture: &Path) -> Result<Vec<(String, Vec<String>)>, String> {
    let path = fixture.join("rules.txt");
    let size = std::fs::metadata(&path).map_err(|e| format!("rules: {e}"))?.len();
    if size > MAX_RULES_BYTES {
        return Err("rules exceed their bound".to_owned());
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("rules: {e}"))?;
    let rules: Vec<_> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut fields = line.split('\t').map(str::to_owned);
            let command = fields.next().unwrap_or_default();
            (command, fields.collect())
        })
        .collect();
    if rules.len() > MAX_RULES {
        return Err("too many rules".to_owned());
    }
    Ok(rules)
}

fn leaf(fixture: &Path, name: &str) -> Result<PathBuf, String> {
    let ordinary = !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        && name != "."
        && name != "..";
    if ordinary {
        Ok(fixture.join(name))
    } else {
        Err(format!("fixture file name is not a leaf: {name}"))
    }
}

/// One action; `Some(code)` ends the command with that exit status.
fn run(fixture: &Path, line: &str, action: &str) -> Result<Option<u8>, String> {
    let words: Vec<&str> = action.split(' ').collect();
    match words.as_slice() {
        ["exit-if", file, code] => {
            let code = code.parse().map_err(|_| "invalid exit code".to_owned())?;
            Ok(leaf(fixture, file)?.exists().then_some(code))
        }
        ["cat", file] => {
            let bytes = std::fs::read(leaf(fixture, file)?).map_err(|e| format!("cat: {e}"))?;
            std::io::stdout()
                .write_all(&bytes)
                .map_err(|e| format!("cat: {e}"))?;
            Ok(None)
        }
        ["print", ..] => {
            println!("{}", &action["print ".len()..]);
            Ok(None)
        }
        ["log"] => {
            let cwd = std::env::current_dir().map_err(|e| format!("cwd: {e}"))?;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(fixture.join("mutations"))
                .map_err(|e| format!("log: {e}"))?;
            writeln!(file, "{}: {line}", cwd.display()).map_err(|e| format!("log: {e}"))?;
            Ok(None)
        }
        ["copy", from, to] => {
            std::fs::copy(leaf(fixture, from)?, leaf(fixture, to)?)
                .map_err(|e| format!("copy: {e}"))?;
            Ok(None)
        }
        _ => Err(format!("unsupported fixture action: {action}")),
    }
}
