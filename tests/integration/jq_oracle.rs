//! Every test here spawns the `1q` CLI (and, where relevant, real `jq`)
//! and compares the result: this file is where "run it and diff the
//! output" checks live. A regression that's just a filter + input that
//! must behave like real `jq` belongs in `tests/fixtures/jq-oracle/` as
//! a `<case>.jq` + `<case>.in.jsonl` pair (or, for cases that need a
//! fixed non-default set of CLI flags, as a new listed-cases test
//! function below) instead of a new `#[test]`.
use crate::common::{Report, jq_variants, list_fixtures, run_jq, run_oneq, stderr_matches};
use rayon::prelude::*;
use std::{path::PathBuf, sync::Mutex};

// This fixture emits scalar numbers, including sin/log. Different platform
// libm implementations can differ by a couple of ULPs. Keep integer results
// and all other fixtures byte-exact, including their formatting.
fn math_stdout_matches(expected: &[u8], actual: &[u8]) -> bool {
    let (Ok(expected), Ok(actual)) = (std::str::from_utf8(expected), std::str::from_utf8(actual))
    else {
        return false;
    };
    let expected: Vec<_> = expected.lines().collect();
    let actual: Vec<_> = actual.lines().collect();
    expected.len() == actual.len()
        && expected.iter().zip(&actual).all(|(a, b)| {
            if a == b {
                return true;
            }
            if !a.contains(['.', 'e', 'E']) || !b.contains(['.', 'e', 'E']) {
                return false;
            }
            match (a.parse::<f64>(), b.parse::<f64>()) {
                (Ok(a), Ok(b)) if a.is_finite() && b.is_finite() => {
                    a.to_bits().abs_diff(b.to_bits()) <= 2
                }
                _ => false,
            }
        })
}

/// `1q` (no subcommand) is meant to be a drop-in replacement for
/// `jq`: for every script x input fixture x invocation style (plain,
/// `-s`, `--stream` for JSON; `-R`, `-R -s` for text), running the
/// two must give identical exit code, stdout and stderr (apart from the
/// platform libm tolerance for fibo.jq described above). Side-effect
/// free I/O on both sides, so straight equality is the whole
/// contract.
///
/// The default CLI now uses the native compiler and evaluator. This strict
/// matrix tracks remaining compatibility gaps, including unsupported builtins
/// and error text; focused native integration tests cover the supported pipeline.
#[test]
fn test_jq_matches_reference() {
    let report = Mutex::new(Report::new("jq_parity"));
    let jsons = list_fixtures("tests/fixtures/jsons", "jsons");
    let texts = list_fixtures("tests/fixtures/texts", "txt");

    list_fixtures("tests/fixtures/scripts", "jq")
        .par_iter()
        .for_each(|script| {
        let script_src = std::fs::read_to_string(script).unwrap();
        let script_name = script.display().to_string();

        for input in jsons.iter().chain(texts.iter()) {
            let is_text = input.extension().and_then(|e| e.to_str()) == Some("txt");
            let data = std::fs::read(input).unwrap();

            for (variant_name, base_args) in jq_variants(is_text) {
                let mut args = base_args;
                args.extend(["-L", "tests/fixtures/modules"]);
                args.push(&script_src);

                let expected = run_jq(&args, &data);
                let actual = run_oneq(&args, &data);

                let case = format!("{script_name} <{}> [{variant_name}]", input.display());
                let same_stderr = stderr_matches(&expected.stderr, &actual.stderr);
                let same_stdout = expected.stdout == actual.stdout
                    || (script.file_name().unwrap() == "fibo.jq"
                        && math_stdout_matches(&expected.stdout, &actual.stdout));

                let result = if expected.status.code() == actual.status.code()
                    && same_stdout
                    && same_stderr
                {
                    Ok(())
                } else {
                    Err(format!(
                        "jq: exit={:?} stdout={:?} stderr={:?} | 1q: exit={:?} stdout={:?} stderr={:?}",
                        expected.status.code(),
                        String::from_utf8_lossy(&expected.stdout),
                        String::from_utf8_lossy(&expected.stderr),
                        actual.status.code(),
                        String::from_utf8_lossy(&actual.stdout),
                        String::from_utf8_lossy(&actual.stderr),
                    ))
                };
                report.lock().unwrap().record(case, result);
            }
        }
    });
    report.into_inner().unwrap().finish();
}

const JQ_ORACLE_CASES_DIR: &str = "tests/fixtures/jq-oracle";

fn jq_oracle_cases() -> Vec<PathBuf> {
    let filter = std::env::var("JQ_ORACLE_CASE").ok();
    let mut cases: Vec<PathBuf> = std::fs::read_dir(JQ_ORACLE_CASES_DIR)
        .unwrap_or_else(|e| panic!("read {JQ_ORACLE_CASES_DIR}: {e}"))
        .map(|e| e.unwrap().path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("jq"))
        // A `_`-prefixed file is a shared `include`/`import` helper, not a case.
        .filter(|path| !path.file_stem().unwrap().to_str().unwrap().starts_with('_'))
        .filter(|path| {
            filter
                .as_deref()
                .is_none_or(|f| path.file_stem().unwrap().to_str().unwrap().contains(f))
        })
        .collect();
    cases.sort();
    cases
}

/// A case's own leading `#! -flags...` line (shell-word split, quotes
/// respected) picks its CLI flags, e.g. `#! -n` or `#! -R --arg x y` —
/// so what a case needs to run is visible right in the file, no
/// separate directory-per-invocation-style needed.
fn shebang_args(source: &str) -> Vec<String> {
    let Some(rest) = source.lines().next().and_then(|l| l.strip_prefix("#!")) else {
        return Vec::new();
    };
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    for c in rest.trim().chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => current.push(c),
            None if c == '\'' || c == '"' => quote = Some(c),
            None if c.is_whitespace() => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            None => current.push(c),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// `check_stderr: false` mirrors the historical looser checks (status +
/// stdout only) for tests where `1q`'s error *text* isn't expected to
/// match `jq`'s wording; `true` also requires [`stderr_matches`].
fn record_jq_parity(
    report: &Mutex<Report>,
    case: String,
    args: &[&str],
    input: &[u8],
    check_stderr: bool,
) {
    let expected = run_jq(args, input);
    let actual = run_oneq(args, input);
    let ok = expected.status.code() == actual.status.code()
        && expected.stdout == actual.stdout
        && (!check_stderr || stderr_matches(&expected.stderr, &actual.stderr));
    let result = if ok {
        Ok(())
    } else {
        Err(format!(
            "jq: exit={:?} stdout={:?} stderr={:?} | 1q: exit={:?} stdout={:?} stderr={:?}",
            expected.status.code(),
            String::from_utf8_lossy(&expected.stdout),
            String::from_utf8_lossy(&expected.stderr),
            actual.status.code(),
            String::from_utf8_lossy(&actual.stdout),
            String::from_utf8_lossy(&actual.stderr),
        ))
    };
    report.lock().unwrap().record(case, result);
}

/// Paired-fixture jq-parity checks: every `<case>.jq` directly under
/// `tests/fixtures/jq-oracle` (except `_`-prefixed shared helpers) must
/// match real `jq`, run with `-c -L tests/fixtures/jq-oracle` (so a case
/// can `include`/`import` a local `_mod_*`/`_data_*` helper) plus
/// whatever flags its own leading `#! ...` line adds (e.g. `#! -n` or
/// `#! -R --arg x y`; see [`shebang_args`]). Input comes from the
/// sibling `<case>.in.jsonl`, or an empty stdin if that file doesn't
/// exist (the usual case for a `#! -n` filter, which ignores stdin
/// anyway).
///
/// Add a regression by dropping in a new `<case>.jq` (+ `<case>.in.jsonl`
/// if it reads input), not a new `#[test]` function. To run only some
/// cases locally: `JQ_ORACLE_CASE=name cargo test --test integration
/// jq_oracle_cases_match_jq` (substring match against the file stem).
#[test]
fn jq_oracle_cases_match_jq() {
    let cases = jq_oracle_cases();
    assert!(!cases.is_empty(), "no jq-oracle cases matched");

    let report = Mutex::new(Report::new("jq_oracle_cases"));
    cases.par_iter().for_each(|script| {
        let filter = std::fs::read_to_string(script).unwrap();
        let input = std::fs::read(script.with_extension("in.jsonl")).unwrap_or_default();
        let extra = shebang_args(&filter);

        let mut args = vec!["-c", "-L", JQ_ORACLE_CASES_DIR];
        args.extend(extra.iter().map(String::as_str));
        args.push(&filter);
        record_jq_parity(&report, script.display().to_string(), &args, &input, true);
    });
    report.into_inner().unwrap().finish();
}

/// `1q -Rc`/`-Rsc` must preserve raw bytes (including `\r`) exactly like
/// `jq`, across a fixed matrix of edge-case inputs and both flag forms.
/// This can't become a `jq-oracle` fixture pair: raw mode reads bytes
/// verbatim rather than parsing JSON, so there's no `.in.jsonl` to store.
#[test]
fn raw_input_preserves_carriage_returns() {
    let report = Mutex::new(Report::new("raw_input_preserves_carriage_returns"));
    for input in [b"a\r\n\r\nlast\r".as_slice(), b"\n\n", b"", b"a\r\r\n"] {
        for flags in ["-Rc", "-Rsc"] {
            record_jq_parity(
                &report,
                format!("{flags} {input:?}"),
                &[flags, "."],
                input,
                false,
            );
        }
    }
    report.into_inner().unwrap().finish();
}

/// CLI flag/mode combinations (slurp, raw, `--stream`, `-n` with
/// `input`/`inputs`, format auto-detection) that must behave like `jq`.
/// Each case needs its own fixed argv, so it's listed here rather than
/// as a `jq-oracle` fixture pair.
#[test]
fn input_modes_and_shared_input_builtin() {
    let report = Mutex::new(Report::new("input_modes_and_shared_input_builtin"));
    for (args, input) in [
        (vec!["-sc", "map(.+1)"], "1\n2\n3"),
        (vec!["-Rsc", "length"], "a\nb\n"),
        (vec!["-Rc", "length"], "ab\nc\n"),
        (vec!["--stream", "-c", "."], "{\"a\":[1,2]}"),
        (vec!["-nc", "[inputs]"], "1\n2\n3"),
        (vec!["-c", "{first:., rest:[inputs]}"], "1\n2\n3"),
        (vec!["-nc", "input,input"], "1\n2\n3"),
        (vec!["-nsc", "."], "1\n2\n3"),
    ] {
        record_jq_parity(&report, format!("{args:?}"), &args, input.as_bytes(), false);
    }
    report.into_inner().unwrap().finish();

    let out = run_oneq(&["-F", "json5", "-c", "map(.+1)"], b"[1,2,]");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"[2,3]\n");
}

/// `--arg`/`--argjson`/`--args`/`--jsonargs` and exit-status rules
/// (`-e`, `halt_error`) that must match `jq`. Each needs its own fixed
/// argv, so it's listed here rather than as a `jq-oracle` fixture pair.
#[test]
fn globals_and_exit_status() {
    let report = Mutex::new(Report::new("globals_and_exit_status"));
    for args in [
        vec![
            "-nc",
            "--arg",
            "x",
            "hello",
            "--argjson",
            "n",
            "2",
            "[$x,$n,$ARGS.named]",
        ],
        vec!["-nc", "--args", "$ARGS.positional", "a", "b"],
        vec!["-nc", "--jsonargs", "$ARGS.positional", "1", "true"],
        vec!["-nec", "empty"],
        vec!["-nec", "false"],
        vec!["-nec", "1,false"],
        vec!["-nec", "false,1"],
        vec!["-nc", "halt_error(7)"],
    ] {
        record_jq_parity(&report, format!("{args:?}"), &args, b"", false);
    }
    report.into_inner().unwrap().finish();
}

/// The final input read (not the first error) determines exit status,
/// even across `-e`/`empty`/error-vs-value combinations. Each case needs
/// its own fixed argv, so it's listed here rather than as a `jq-oracle`
/// fixture pair.
#[test]
fn final_input_determines_status_even_if_it_emits_nothing() {
    let report = Mutex::new(Report::new(
        "final_input_determines_status_even_if_it_emits_nothing",
    ));
    for args in [
        vec!["-c", ".a"],
        vec![
            "-c",
            "if type==\"number\" then error(\"bad\") else empty end",
        ],
        vec![
            "-ec",
            "if type==\"number\" then error(\"bad\") else empty end",
        ],
        vec!["-ec", "select(type==\"number\")"],
    ] {
        record_jq_parity(&report, format!("{args:?}"), &args, b"1\n{}", false);
    }
    report.into_inner().unwrap().finish();
}

/// `1q` doesn't need a `jq` executable on `$PATH`: the native backend
/// spawns nothing. Not a jq-parity check (no reference `jq` involved),
/// so it stays a bespoke CLI test.
#[test]
fn native_backend_needs_no_jq_executable() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_1q"))
        .args(["-c", "map(.+1)"])
        .env("PATH", "")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"[1,2]\n[3]")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"[2,3]\n[4]\n");
}

/// `1q`-specific compile/runtime error reporting (exit codes, partial
/// output before a runtime error, "compile error" prefix) with no `jq`
/// reference involved, so it stays a bespoke CLI test.
#[test]
fn compile_errors_precede_opening_input_and_runtime_keeps_partial_output() {
    for source in [
        "missing_filter",
        "$missing",
        "map",
        "if",
        "def f: missing_filter; .",
    ] {
        let out = run_oneq(&["-c", source, "/definitely/missing/input"], b"");
        assert_eq!(
            out.status.code(),
            Some(3),
            "{source}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.stdout.is_empty());
        assert!(String::from_utf8_lossy(&out.stderr).contains("compile error"));
    }
    let out = run_oneq(&["-nc", "1,error(\"bad\"),2"], b"");
    assert_eq!(out.status.code(), Some(5));
    assert_eq!(out.stdout, b"1\n");
    let out = run_oneq(&["-c", "."], b"1\n[broken");
    assert_eq!(out.status.code(), Some(4));
    assert_eq!(out.stdout, b"1\n");
}

/// `-f <file>` plus a relative `import` next to it. No `jq` reference
/// involved (this is about `1q`'s own file/module resolution), so it
/// stays a bespoke CLI test.
#[test]
fn filter_file_and_relative_module_are_compiled() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main.jq");
    std::fs::write(dir.path().join("helper.jq"), "def twice(f): f+f;").unwrap();
    std::fs::write(&main, "import \"helper\" as h; h::twice(.)").unwrap();
    let out = run_oneq(&["-c", "-f", main.to_str().unwrap()], b"2\n3");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"4\n6\n");
}
