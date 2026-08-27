//! The MCP protocol corpus (ADPT-2, ADR-0057 decision 11).
//!
//! ADPT-2's acceptance criterion is "works in Claude Desktop + one
//! non-Anthropic client", and CNSL-1 established both the pattern and the
//! reason: a criterion phrased *works in X* is unfalsifiable until what X
//! exchanges is on disk and replayed. These cases are that, for the
//! protocol — each one a sequence of frames sent to the real `synveda mcp`
//! binary over a real pipe, with the real answers settled beside them.
//!
//! ```text
//! cargo test -p synveda-cli --test mcp_corpus     # verify
//! SYNVEDA_RECORD_MCP=1 cargo test -p synveda-cli --test mcp_corpus   # re-record
//! ```
//!
//! No gateway is needed and none is used: every case here is decided by
//! the server alone — the handshake of both eras, what each launch mode
//! advertises, the refusals, and the sentence a caller gets when no
//! credential resolves. CPR-20's gateway-backed seam is exercised by
//! `demos/cpr-20-context-planning.sh`; this corpus remains the deterministic
//! protocol boundary.
//!
//! # What is real here, and what is not
//!
//! **The responses are real.** Every `expect` in every fixture was
//! produced by running the shipped binary, not written by hand. That half
//! is the regression guard, and it is the half that fails when the
//! protocol drifts.
//!
//! **Both AC clients' requests are real too**, recorded on 2026-08-05 from
//! Claude Desktop 1.25927.0 and Zed 1.13.2 with `fixtures/mcp/capture.sh`.
//! Each fixture says where its frames came from in its own `provenance`
//! block, and [`every_case_declares_where_its_frames_came_from`] fails if a
//! case is missing one — or if either AC client stops being represented by
//! a real recording.
//!
//! Recording them was not a formality. The authored cases they replaced
//! were wrong in ways nothing else would have caught:
//!
//! | | authored | recorded |
//! |---|---|---|
//! | first request id | `1` | **`0`**, from both clients |
//! | `tools/list` params | `"params": {}` | **absent** — and *present* on Claude Desktop's other launch |
//! | Claude Desktop launches | one | **two**, with different `clientInfo` and different capabilities |
//! | era both open at | Desktop `2025-06-18`, Zed `2025-11-25` | **`2025-11-25` from both** |
//!
//! That last row is the one that matters beyond this file. **No shipping
//! client opens in the modern era**, so `modern-era` — the `2026-07-28`
//! frames decision 3 requires the server to serve — stays authored, is
//! attributed to the specification rather than to a vendor, and is the only
//! thing exercising that path. It becomes `captured` the day a client
//! arrives that opens there. `unsupported-version` is synthetic by
//! construction and never will be: no client sends a version on purpose to
//! be refused.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

/// Set to re-record every case's answers from the shipped binary.
const RECORD: &str = "SYNVEDA_RECORD_MCP";

/// Every case in the corpus. Named in one place so a fixture added to the
/// directory and not to this list is a fixture nothing replays.
const CASES: &[&str] = &[
    "claude-desktop-probe",
    "claude-desktop-agent",
    "zed",
    "modern-era",
    "host-owned-write-mode",
    "unsupported-version",
];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/mcp")
}

fn read_case(name: &str) -> Value {
    let path = fixtures().join(format!("{name}.json"));
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|err| panic!("parse {}: {err}", path.display()))
}

/// Runs one case's frames through the shipped binary and returns the
/// answers it gave, in the order they arrived.
///
/// A real pipe and a real process on purpose: an in-process handler would
/// prove the handlers and skip the transport, and the transport is where a
/// protocol era is decided.
fn exchange(case: &Value) -> Vec<Value> {
    let launch: Vec<String> = case["launch"]
        .as_array()
        .expect("launch is an array")
        .iter()
        .map(|arg| arg.as_str().expect("launch args are strings").to_owned())
        .collect();

    let mut child = Command::new(env!("CARGO_BIN_EXE_synveda"))
        .args(&launch)
        // The corpus must not depend on whoever runs it being logged in:
        // point the credential seam at a directory with nothing in it, so
        // the no-credential answers are the ones recorded and a developer
        // with a live session records the same bytes as CI. `XDG_CONFIG_HOME`
        // as well as `HOME`, because it wins when it is set (credentials.rs).
        .env(
            "HOME",
            std::env::temp_dir().join("synveda-mcp-corpus-nohome"),
        )
        .env(
            "XDG_CONFIG_HOME",
            std::env::temp_dir().join("synveda-mcp-corpus-nohome/.config"),
        )
        .env_remove("SYNVEDA_TOKEN")
        .env_remove("SYNVEDA_PROFILE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn synveda mcp");

    let frames = case["exchange"].as_array().expect("exchange is an array");
    let expected: usize = frames
        .iter()
        .filter(|frame| !frame["send"]["id"].is_null())
        .count();

    {
        let stdin = child.stdin.as_mut().expect("stdin");
        for frame in frames {
            writeln!(stdin, "{}", frame["send"]).expect("write a frame");
        }
        stdin.flush().expect("flush");
    }

    let mut answers = Vec::new();
    let stdout = BufReader::new(child.stdout.take().expect("stdout"));
    for line in stdout.lines() {
        let line = line.expect("read a line");
        if line.trim().is_empty() {
            continue;
        }
        answers.push(serde_json::from_str(&line).expect("an answer is JSON"));
        if answers.len() == expected {
            break;
        }
    }

    // Closing stdin is what ends the server: the client going away is the
    // only shutdown a stdio transport has.
    drop(child.stdin.take());
    let _ = child.wait();
    assert_eq!(
        answers.len(),
        expected,
        "the server answered {} of {expected} requests — a notification must draw no reply \
         and a request must draw exactly one",
        answers.len(),
    );
    answers
}

/// Replaces every `serverInfo.version` in a tree with a placeholder, and
/// returns what was there.
///
/// `serverInfo.version` is `CARGO_PKG_VERSION`, so it changes on every
/// release and on nothing else. Left in the byte comparison it makes a
/// version bump fail this corpus with a one-line diff — and a golden corpus
/// whose only routine diff is uninteresting is one people re-record without
/// reading, which is the whole of what it is for. The caller asserts the
/// live value against this binary's own version instead, which is a
/// stronger claim than a recorded string can make.
fn normalise_version(value: &mut Value) -> Vec<String> {
    let mut seen = Vec::new();
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                // `serverInfo` on the classic handshake, and
                // `io.modelcontextprotocol/serverInfo` under `_meta` on the
                // 2026-07-28 one — matched by suffix so the next namespace
                // does not need a third arm here.
                let is_server_info = key == "serverInfo" || key.ends_with("/serverInfo");
                if is_server_info
                    && let Some(info) = child.as_object_mut()
                    && let Some(version) = info.get_mut("version")
                    && let Some(text) = version.as_str()
                {
                    seen.push(text.to_owned());
                    *version = Value::String("<CARGO_PKG_VERSION>".to_owned());
                    continue;
                }
                seen.extend(normalise_version(child));
            }
        }
        Value::Array(items) => {
            for item in items {
                seen.extend(normalise_version(item));
            }
        }
        _ => {}
    }
    seen
}

/// Settles one case's answers against the fixture, or re-records them.
fn settle(name: &str) {
    let mut case = read_case(name);
    let answers = exchange(&case);

    let frames = case["exchange"].as_array().expect("exchange").clone();
    let mut answered = answers.into_iter();
    let mut settled = Vec::with_capacity(frames.len());
    for mut frame in frames {
        if frame["send"]["id"].is_null() {
            // A notification. The spec forbids replying to one, and a
            // client that receives an id-less response may disconnect, so
            // "no answer" is the recorded fact.
            frame["expect"] = Value::Null;
        } else {
            frame["expect"] = answered.next().expect("an answer per request");
        }
        settled.push(frame);
    }
    let mut recorded = Value::Array(settled);

    // `serverInfo.version` is `CARGO_PKG_VERSION`, so it changes on every
    // release and on nothing else. Left in the byte comparison it made a
    // version bump fail this test with a one-line diff — and a golden
    // corpus whose only routine diff is uninteresting is one people
    // re-record without reading, which is the whole of what it is for.
    //
    // So it is asserted *harder* here than a recorded string could: the
    // server must report exactly the version this binary was built as. Then
    // both sides are normalised and everything else stays byte-exact.
    // Wherever it appears, not at a fixed path: `initialize` carries it at
    // `result.serverInfo` and `server/discover` somewhere else again, and a
    // normaliser that knows one handshake fails the other.
    let live = normalise_version(&mut recorded);
    normalise_version(&mut case["exchange"]);
    for reported in live {
        assert_eq!(
            reported.as_str(),
            env!("CARGO_PKG_VERSION"),
            "\n{name}: the server reports version {reported} but this binary is {}. \
             `serverInfo.version` is what an MCP client shows a user and what a bug \
             report quotes.\n",
            env!("CARGO_PKG_VERSION"),
        );
    }

    let path = fixtures().join(format!("{name}.json"));
    if std::env::var(RECORD).is_ok() {
        case["exchange"] = recorded;
        let body = format!(
            "{}\n",
            serde_json::to_string_pretty(&case).expect("serialise")
        );
        std::fs::write(&path, body).unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
        return;
    }

    assert_eq!(
        case["exchange"],
        recorded,
        "\n{name}: the server no longer answers this exchange the way {} records.\n\
         If the change is intended, re-record with `{RECORD}=1 cargo test -p synveda-cli \
         --test mcp_corpus` and read the diff — a protocol corpus is only worth what its \
         last review was.\n",
        path.display(),
    );
}

#[test]
fn every_case_replays_against_the_shipped_binary() {
    for name in CASES {
        settle(name);
    }
}

/// The corpus must not quietly become a set of cases nobody replays.
#[test]
fn every_fixture_on_disk_is_a_case_this_suite_runs() {
    let mut found: Vec<String> = std::fs::read_dir(fixtures())
        .expect("fixtures dir")
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().to_str()?.to_owned();
            name.strip_suffix(".json").map(str::to_owned)
        })
        .collect();
    found.sort();
    let mut expected: Vec<String> = CASES.iter().map(|name| (*name).to_owned()).collect();
    expected.sort();
    assert_eq!(found, expected);
}

/// ADR-0057 decision 11 asks for *each client's real frames*, and the
/// corpus is honest about which it has. A case whose `send` frames were
/// authored rather than captured says so, so nobody reads this suite as
/// evidence the AC is met.
#[test]
fn every_case_declares_where_its_frames_came_from() {
    let mut vendor_recorded = Vec::new();
    for name in CASES {
        let case = read_case(name);
        let provenance = &case["provenance"];
        let kind = provenance["kind"].as_str().unwrap_or_default();
        assert!(
            matches!(kind, "authored" | "captured"),
            "{name}: provenance.kind must be `authored` or `captured`, got {kind:?}",
        );
        assert!(
            provenance["note"]
                .as_str()
                .is_some_and(|note| !note.is_empty()),
            "{name}: provenance needs a note saying where the frames came from",
        );
        assert!(
            case["client"].as_str().is_some(),
            "{name}: a case names the client it stands for",
        );
        assert!(
            matches!(case["era"].as_str(), Some("legacy" | "modern")),
            "{name}: era is `legacy` or `modern` — the two ADR-0057 decision 3 serves",
        );
        if kind == "captured" {
            vendor_recorded.push(case["client"].as_str().unwrap_or_default().to_owned());
        }
    }

    // The acceptance criterion is about *clients*, not cases: "works in
    // Claude Desktop + one non-Anthropic client". Counting cases would set
    // a target the corpus can never reach — `modern-era` is the spec's own
    // frames and no shipping client opens there, and `unsupported-version`
    // is synthetic by construction, so neither will ever be captured from
    // anything.
    for client in ["claude-desktop", "zed"] {
        assert!(
            vendor_recorded.iter().any(|seen| seen == client),
            "no case carries real frames from {client}; ADR-0057 decision 11 makes the AC a \
             recorded corpus, and an authored case is this suite testing my assumptions \
             about {client} rather than {client}. Record with fixtures/mcp/capture.sh.",
        );
        // Decision 11 enumerates the frames it wants — `server/discover` or
        // `initialize`, `tools/list`, **`tools/call`** — and the first two
        // arrive on their own when a client starts the server. Only the
        // third needs a model to decide to call a tool, which makes it the
        // one a capture session quietly ends without. A client recorded to
        // the handshake and no further has demonstrated that it launches
        // us, not that it can use us.
        assert!(
            CASES
                .iter()
                .map(|name| read_case(name))
                .filter(|case| case["provenance"]["kind"] == "captured" && case["client"] == client)
                .any(
                    |case| case["exchange"].as_array().is_some_and(|frames| frames
                        .iter()
                        .any(|frame| frame["send"]["method"] == "tools/call"))
                ),
            "{client} is recorded, but no captured case from it carries a `tools/call` — \
             so the corpus shows it starting the server and never using it.",
        );
    }

    // Not an assertion, because a case going from authored to captured is
    // always the good outcome and never an obligation this suite can name
    // in advance.
    eprintln!(
        "mcp corpus: {} of {} cases carry a real client's frames ({})",
        vendor_recorded.len(),
        CASES.len(),
        vendor_recorded.join(", "),
    );
}

/// The one way diagnostics can break this server, checked at the level it
/// would break: a whole process, at the noisiest filter there is.
///
/// `tracing_subscriber`'s fmt layer writes to **stdout** by default, and
/// stdout here is the JSON-RPC stream — so one log line on the wrong
/// descriptor is a parse error at the client and a server that "won't
/// connect" for reasons nothing in the code reads as a logging bug. The
/// unit tests cannot see this; only running the binary can.
///
/// `RUST_LOG=trace` rather than `debug`, and `rmcp` in the filter as well
/// as `synveda`, because the frames the SDK traces are the ones most
/// likely to be written next to the frames it sends.
#[test]
fn a_talkative_server_writes_nothing_but_protocol_to_stdout() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_synveda"))
        .args(["mcp", "--writes", "tool"])
        .env("RUST_LOG", "trace")
        .env(
            "HOME",
            std::env::temp_dir().join("synveda-mcp-corpus-nohome"),
        )
        .env(
            "XDG_CONFIG_HOME",
            std::env::temp_dir().join("synveda-mcp-corpus-nohome/.config"),
        )
        .env_remove("SYNVEDA_TOKEN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn synveda mcp");

    let meta = serde_json::json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": { "name": "probe", "version": "0" },
    });
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        for frame in [
            serde_json::json!({"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":meta}}),
            serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"_meta":meta}}),
            // One of each outcome the span records, so every logging path
            // this call can take runs while stdout is being watched.
            serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
                "_meta":meta,"name":"recall","arguments":{"query":"anything"}}}),
            serde_json::json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{
                "_meta":meta,"name":"not-a-tool","arguments":{}}}),
        ] {
            writeln!(stdin, "{frame}").expect("write a frame");
        }
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().expect("wait");

    let mut frames = 0;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if line.trim().is_empty() {
            continue;
        }
        serde_json::from_str::<Value>(line).unwrap_or_else(|err| {
            panic!(
                "a non-protocol line reached stdout ({err}): {line:?}\n\
                 stdout is the JSON-RPC stream — diagnostics belong on stderr, and \
                 tracing_subscriber's fmt layer defaults to stdout, so check \
                 `.with_writer(std::io::stderr)` in mcp::subscribe.",
            )
        });
        frames += 1;
    }
    assert_eq!(frames, 4, "one answer per request, and nothing else");

    // And the diagnostics did happen — otherwise this test passes on a
    // server that logs nothing at all, which is not the property wanted.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("mcp server starting"),
        "nothing was traced at RUST_LOG=trace, so stdout being clean proves nothing:\n{stderr}",
    );
    assert!(
        stderr.contains("mcp.tools/call"),
        "the tool-call span did not reach stderr:\n{stderr}",
    );
}

/// The two eras are both in the corpus, and so are both launch modes.
/// Without this a corpus can drift into covering only the path its author
/// happened to exercise — which for a dual-era server is the whole risk.
#[test]
fn the_corpus_covers_both_eras_and_both_launch_modes() {
    let cases: Vec<Value> = CASES.iter().map(|name| read_case(name)).collect();
    for era in ["legacy", "modern"] {
        assert!(
            cases.iter().any(|case| case["era"] == era),
            "no case opens in the {era} era",
        );
    }
    for writes in ["tool", "host"] {
        assert!(
            cases.iter().any(|case| {
                case["launch"]
                    .as_array()
                    .is_some_and(|args| args.iter().any(|arg| arg == writes))
            }),
            "no case launches --writes {writes}",
        );
    }
}
