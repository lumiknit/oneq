use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Module search path for the "module system" cases (`import`/`include`) -
/// mirrors jqlang/jq's own `tests/modules/` fixture directory (fetched from
/// upstream since this repo's `jq.test` copy didn't bring its sibling
/// modules along).
const MODULE_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/jq-test/modules");

fn run_oneq_timeout(args: &[&str], input: &[u8]) -> Option<Output> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_1q"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(input).ok()?;
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if child.try_wait().ok()?.is_some() {
            return child.wait_with_output().ok();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Whether two JSON texts represent the same *value*, per 1q's own `==`
/// (rather than a byte-for-byte string comparison of some canonical
/// rendering). This is what jq's own test suite actually means by "the
/// output matches" - `==` already treats objects as key-order-insensitive
/// (e.g. `.foo = .bar` on `{"bar":42}` is checked against
/// `{"foo":42,"bar":42}` even though real jq emits `{"bar":42,"foo":42}`)
/// and, now that number literals preserve their exact decimal spelling
/// (`Value::Decimal`), also treats e.g. `2` and the differently-spelled but
/// numerically-equal `20e-1` as equal - which a plain textual/canonical-form
/// comparison no longer would.
fn values_equal(actual: &str, expected: &str) -> bool {
    let combined = format!("[{actual},{expected}]");
    let Some(out) = run_oneq_timeout(&["-c", "--", ".[0] == .[1]"], combined.as_bytes()) else {
        return false;
    };
    out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true"
}

enum Case<'a> {
    Pass {
        program: &'a str,
        input: &'a str,
        expected: String,
    },
    Fail {
        program: &'a str,
        input: &'a str,
    },
}

#[test]
fn upstream_jq_test() {
    let lines: Vec<&str> = include_str!("jq.test").lines().map(str::trim_end).collect();
    let mut cases = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        while i < lines.len()
            && (lines[i].trim().is_empty() || lines[i].trim_start().starts_with('#'))
        {
            i += 1;
        }
        if i >= lines.len() {
            break;
        }
        let fail = lines[i].starts_with("%%FAIL");
        if fail {
            i += 1;
        }
        assert!(i + 1 < lines.len(), "truncated jq.test case near line {i}");
        let (program, input) = (lines[i], lines[i + 1]);
        i += 2;
        let start = i;
        while i < lines.len() && !lines[i].trim().is_empty() {
            i += 1;
        }
        if fail {
            cases.push(Case::Fail { program, input });
        } else {
            // Per the format's own header comment ("Blank lines and lines
            // starting with # are ignored"), a `#` line inside the expected
            // block is documentation, not an expected output value - e.g.
            // the "Destructuring DUP/POP issues" cases only have a comment
            // noting the program raises a runtime error, with zero actual
            // expected-output lines. Filtering these out (rather than
            // requiring success below) makes such cases compare real
            // (possibly empty, possibly partial-before-error) stdout
            // against the real expected list, matching upstream intent.
            let expected: Vec<&str> = lines[start..i]
                .iter()
                .filter(|line| !line.trim_start().starts_with('#'))
                .copied()
                .collect();
            cases.push(Case::Pass {
                program,
                input,
                expected: expected.join("\n"),
            });
        }
    }
    eprintln!("jq.test: collected {} cases", cases.len());
    // Each case is an independent subprocess spawn/wait, so this is exactly
    // the kind of embarrassingly-parallel workload rayon is for - running
    // the full suite serially spends almost all its wall-clock time
    // blocked on process I/O, not CPU, so several dozen cases can run
    // concurrently without contention.
    use rayon::prelude::*;
    let completed = std::sync::atomic::AtomicUsize::new(0);
    let failed_count = std::sync::atomic::AtomicUsize::new(0);
    let total = cases.len();
    let mut failed: Vec<(usize, &str)> = cases
        .par_iter()
        .enumerate()
        .filter_map(|(n, case)| {
            let (program, input, should_fail) = match case {
                Case::Pass { program, input, .. } => (*program, *input, false),
                Case::Fail { program, input } => (*program, *input, true),
            };
            let out = run_oneq_timeout(
                &["-L", MODULE_PATH, "-a", "-c", "--", program],
                format!("{input}\n").as_bytes(),
            );
            let ok = match (should_fail, out) {
                (_, None) => false,
                (true, Some(out)) => !out.status.success(),
                (false, Some(out)) => {
                    // Upstream's own test format doesn't require success for a
                    // non-%%FAIL case - it only compares whatever stdout was
                    // produced (which may be empty, or partial-before-error)
                    // against the expected output list. Some upstream cases
                    // (e.g. the "Destructuring DUP/POP issues" group) document a
                    // runtime error via a comment with zero expected lines,
                    // relying on exactly this.
                    let expected = match case {
                        Case::Pass { expected, .. } => expected,
                        _ => unreachable!(),
                    };
                    let actual = String::from_utf8_lossy(&out.stdout).trim_end().to_string();
                    if expected.is_empty() {
                        actual.is_empty()
                    } else {
                        let actual_lines: Vec<&str> = actual.lines().collect();
                        let expected_lines: Vec<&str> = expected.lines().collect();
                        actual_lines.len() == expected_lines.len()
                            && actual_lines
                                .iter()
                                .zip(expected_lines.iter())
                                .all(|(a, e)| values_equal(a, e))
                    }
                }
            };
            let done = completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            let fails = if ok {
                failed_count.load(std::sync::atomic::Ordering::Relaxed)
            } else {
                failed_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
            };
            if done % 30 == 0 || done == total {
                eprintln!("jq.test: completed {done}/{total} (failed {fails})");
            }
            (!ok).then_some((n + 1, program))
        })
        .collect();
    failed.sort_by_key(|&(n, _)| n);
    if !failed.is_empty() {
        eprintln!("jq.test: {} failures (showing up to 10):", failed.len());
        for (n, program) in failed.iter().take(10) {
            eprintln!("  case {n}: {program}");
        }
        panic!("jq.test failed");
    }
}
