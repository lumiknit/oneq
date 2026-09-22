#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

pub const FORMAT_MODES: [&str; 3] = ["", "--inline-output", "--compact-output"];

/// Runs an already-spawned-style command: writes `stdin` then waits
/// for exit, capturing stdout/stderr. Shared by both the `1q` binary
/// and the system `jq` used as an oracle.
fn run(program: impl AsRef<Path>, args: &[&str], stdin: &[u8]) -> Output {
    let mut child = Command::new(program.as_ref())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {}: {e}", program.as_ref().display()));
    if let Err(error) = child.stdin.take().unwrap().write_all(stdin) {
        // jq may exit before consuming stdin (e.g. a compile error or break).
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::BrokenPipe,
            "failed to write stdin: {error}"
        );
    }
    child.wait_with_output().expect("failed to wait on child")
}

/// Runs the `oneq` binary under test.
pub fn run_oneq(args: &[&str], stdin: &[u8]) -> Output {
    run(env!("CARGO_BIN_EXE_1q"), args, stdin)
}

/// Runs `1q -fmt <mode> [path-args]` on `stdin` and returns stdout.
/// Panics if formatting fails, since these tests assume every fixture
/// is valid jq.
/// mode is one of "" (pretty), "--inline-output", or "--compact-output".
pub fn fmt(mode: &str, stdin: &[u8]) -> Vec<u8> {
    let mut args = vec!["--fmt"];
    if !mode.is_empty() {
        args.push(mode);
    }
    let out = run_oneq(args.as_slice(), stdin);
    assert!(
        out.status.success(),
        "1q-fmt '{mode}' failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

/// Runs the system `jq` (the reference implementation) with `args`
/// and `stdin`.
pub fn run_jq(args: &[&str], stdin: &[u8]) -> Output {
    let local = Path::new(env!("CARGO_MANIFEST_DIR")).join(".manual/jq.exe");
    let program = std::env::var_os("JQ").map_or_else(|| if local.is_file() { local } else { "jq".into() }, PathBuf::from);
    // Native Windows jq otherwise translates output newlines to CRLF.
    let mut flags = Vec::new();
    if cfg!(windows) {
        flags.push("--binary");
    }
    flags.extend_from_slice(args);
    run(program, &flags, stdin)
}

/// The base `jq` flags to layer a fixture's script argument onto, one
/// entry per way that fixture's input can legitimately be fed to
/// `jq`. JSON fixtures get exercised as a stream of values, slurped
/// into one array, and re-parsed with `--stream`'s event encoding;
/// raw-text fixtures only make sense under `-R`, with an optional
/// `-s` to slurp all lines into one string first.
pub fn jq_variants(is_text: bool) -> Vec<(&'static str, Vec<&'static str>)> {
    if is_text {
        vec![("jq -R", vec!["-R"]), ("jq -R -s", vec!["-R", "-s"])]
    } else {
        vec![
            ("jq", vec![]),
            ("jq -s", vec!["-s"]),
            ("jq --stream", vec!["--stream"]),
        ]
    }
}

/// Lists files directly under `dir` (relative to the crate root)
/// whose extension matches `ext`, sorted for deterministic test
/// ordering.
pub fn list_fixtures(dir: &str, ext: &str) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("failed to read {dir}: {e}"))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some(ext))
        .collect();
    paths.sort();
    paths
}

/// Two stderr blobs are equivalent for cross-implementation comparison if
/// they have the same line count and each line is either byte-identical or,
/// when both sides are a `jq: ...`/`1q: ...` line, agree on the first three
/// words after that line's last `:` - which is where each tool's own prefix,
/// location annotation, and value-quoting differences all end up, so
/// matching just that much of the tail is enough to call the lines
/// equivalent.
pub fn stderr_matches(expected: &[u8], actual: &[u8]) -> bool {
    let normalize_at = |s: &str| {
        regex::Regex::new(r"\(at[^)]+\)")
            .unwrap()
            .replace_all(s, "(at ...)")
            .into_owned()
    };
    let expected = normalize_at(&String::from_utf8_lossy(expected));
    let actual = normalize_at(&String::from_utf8_lossy(actual));
    if expected == actual {
        return true;
    }
    let expected_lines: Vec<&str> = expected.lines().collect();
    let actual_lines: Vec<&str> = actual.lines().collect();
    expected_lines.len() == actual_lines.len()
        && expected_lines
            .iter()
            .zip(actual_lines.iter())
            .all(|(e, a)| {
                e == a || matches!((error_key(e), error_key(a)), (Some(ek), Some(ak)) if ek == ak)
            })
}

/// For a line starting with `jq:` or `1q:`, the first three
/// whitespace-separated words after that line's last `:`.
fn error_key(line: &str) -> Option<String> {
    if !(line.starts_with("jq:") || line.starts_with("1q:")) {
        return None;
    }
    let rest = &line[line.rfind(':')? + 1..];
    Some(
        rest.split_whitespace()
            .take(3)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// A single named check's outcome, so callers can print a pass/fail
/// report instead of stopping at the first failure.
pub struct Report {
    pub label: String,
    pub cases: Vec<(String, Result<(), String>)>,
}

impl Report {
    pub fn new(label: &str) -> Self {
        Report {
            label: label.to_string(),
            cases: Vec::new(),
        }
    }

    pub fn record(&mut self, case: impl Into<String>, result: Result<(), String>) {
        self.cases.push((case.into(), result));
    }

    /// Prints a `passed/total (pct%)` summary plus every failing case,
    /// then panics if anything failed. Run `cargo test -- --nocapture`
    /// to see the summary even when everything passes.
    pub fn finish(self) {
        let total = self.cases.len();
        let failures: Vec<&(String, Result<(), String>)> =
            self.cases.iter().filter(|(_, r)| r.is_err()).collect();
        let passed = total - failures.len();
        let pct = if total == 0 {
            100.0
        } else {
            100.0 * passed as f64 / total as f64
        };
        println!("[{}] {}/{} passed ({:.1}%)", self.label, passed, total, pct);
        for (case, result) in &self.cases {
            if let Err(e) = result {
                println!("  FAIL {case}: {e}");
            }
        }
        assert!(
            failures.is_empty(),
            "[{}] {} of {} cases failed",
            self.label,
            failures.len(),
            total
        );
    }
}
