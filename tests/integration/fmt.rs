use crate::common::{FORMAT_MODES, Report, fmt, jq_variants, list_fixtures, run_jq, run_oneq};
use rayon::prelude::*;
use std::{
    io::Write,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

/// `1q -fmt -m A | 1q -fmt -m B` must equal `1q -fmt -m B` applied
/// directly: reformatting doesn't depend on which mode produced the
/// source it's reformatting. `A == B` is the idempotency case, so
/// this covers idempotency for free.
///
/// Exception: `min`/`oneline` drop comments irreversibly, so chaining
/// one of them into `pretty` can never recover what direct `pretty`
/// would show - that combination is skipped rather than asserted.
#[test]
fn test_fmt_idempotent() {
    let mut report = Report::new("fmt_idempotent");
    for script in list_fixtures("tests/fixtures/scripts", "jq") {
        let src = std::fs::read(&script).unwrap();
        let name = script.display().to_string();

        let directs: Vec<Vec<u8>> = FORMAT_MODES.iter().map(|&mode| fmt(mode, &src)).collect();

        for (i, mode_b) in FORMAT_MODES.iter().enumerate() {
            for (j, mode_a) in FORMAT_MODES.iter().enumerate() {
                if mode_b.is_empty() && !mode_a.is_empty() {
                    continue;
                }

                let direct = &directs[i];
                let intermediate = &directs[j];
                let double = fmt(mode_b, intermediate);
                let case = format!("{name}: -m {mode_a} | -m {mode_b}");
                let result = if &double == direct {
                    Ok(())
                } else {
                    Err(format!(
                        "expected {:?}, got {:?}",
                        String::from_utf8_lossy(direct),
                        String::from_utf8_lossy(&double)
                    ))
                };
                report.record(case, result);
            }
        }
    }
    report.finish();
}

/// Output size must grow (or stay equal) from `min` to `oneline` to
/// `pretty`, for every fixture.
#[test]
fn test_fmt_size() {
    let report = Mutex::new(Report::new("fmt_size"));
    list_fixtures("tests/fixtures/scripts", "jq")
        .par_iter()
        .for_each(|script| {
            let src = std::fs::read(script).unwrap();
            let name = script.display().to_string();

            let min_len = fmt("--compact-output", &src).len();
            let oneline_len = fmt("--inline-output", &src).len();
            let pretty_len = fmt("", &src).len();

            let result = if min_len <= oneline_len && oneline_len <= pretty_len {
                Ok(())
            } else {
                Err(format!(
                    "min={min_len} oneline={oneline_len} pretty={pretty_len}"
                ))
            };
            report.lock().unwrap().record(name, result);
        });
    report.into_inner().unwrap().finish();
}

/// Every emitted line must be clean for use in version-controlled jq files.
#[test]
fn test_fmt_no_trailing_whitespace() {
    let mut report = Report::new("fmt_no_trailing_whitespace");
    for script in list_fixtures("tests/fixtures/scripts", "jq") {
        let source = std::fs::read(&script).unwrap();
        for &mode in &FORMAT_MODES {
            let formatted = fmt(mode, &source);
            let result = String::from_utf8_lossy(&formatted)
                .lines()
                .enumerate()
                .find(|(_, line)| line.ends_with([' ', '\t']))
                .map_or(Ok(()), |(line, _)| {
                    Err(format!("trailing whitespace at line {}", line + 1))
                });
            report.record(format!("{} -m {mode}", script.display()), result);
        }
    }
    report.finish();
}

#[test]
fn test_fmt_stdin_with_identity_filter() {
    let default = run_oneq(&["--fmt"], br#"{"a": 20}"#);
    assert!(
        default.status.success(),
        "{}",
        String::from_utf8_lossy(&default.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&default.stdout), r#"{a: 20}"#);
    let output = run_oneq(&["--fmt", "."], br#"{"a": 20}"#);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), r#"{a: 20}"#);
}

/// Focused regression probes for syntax that is easy to regroup while
/// printing Pairs.  Each variant is compared with jq itself, so this tests
/// both that the emitted source compiles and that it keeps the same meaning.
#[test]
fn test_fmt_edge_cases() {
    let cases: &[(&str, &str, &[u8])] = &[
        (
            "elif",
            "if .a then .x elif .b then .y else .z end",
            br#"{"a":false,"b":true,"x":1,"y":2,"z":3}"#,
        ),
        (
            "postfix-alt",
            "(.a | .b?) // .c? // \"fallback\"",
            br#"{"a":{"b":"left"},"c":"right"}"#,
        ),
        (
            "update",
            ".items |= map(if .score >= 10 then .score + 1 else .score end)",
            br#"{"items":[{"score":10},{"score":3}]}"#,
        ),
        (
            "quoted-key-slice",
            "{\"long key\": (.items | .[1:]), nested: {a: [.a, .b, .c]}}",
            br#"{"items":[0,1,2],"a":1,"b":2,"c":3}"#,
        ),
        (
            "try-catch",
            "try (.x | tonumber) catch (if .x == null then 0 else -1 end)",
            br#"{"x":"not-a-number"}"#,
        ),
    ];
    for (name, source, input) in cases {
        let before = run_jq(&[source], input);
        for &mode in &FORMAT_MODES {
            let formatted = fmt(mode, source.as_bytes());
            let formatted = String::from_utf8(formatted).unwrap();
            let after = run_jq(&[&formatted], input);
            assert_eq!(
                (after.status.code(), &after.stdout, &after.stderr),
                (before.status.code(), &before.stdout, &before.stderr),
                "{name} -m {mode}: formatted={formatted:?}",
            );
        }
    }
}

/// Formatting must never change a script's behavior: running the
/// original and the formatted source through the real `jq`, over
/// every JSON/text fixture and every way that fixture can legitimately
/// be fed to `jq` (plain stream, `-s` slurp, `--stream` for JSON;
/// `-R` and `-R -s` for text), must produce identical exit code,
/// stdout and stderr. `jq` here is only an oracle for jq-language
/// semantics, not a test of 1q's own evaluator.
#[test]
fn test_fmt_behave() {
    let report = Mutex::new(Report::new("fmt_behave"));

    let inputs: Vec<(std::path::PathBuf, Vec<u8>, bool)> =
        list_fixtures("tests/fixtures/jsons", "jsons")
            .into_iter()
            .chain(list_fixtures("tests/fixtures/texts", "txt"))
            .map(|path| {
                let is_text = path.extension().and_then(|e| e.to_str()) == Some("txt");
                let data = std::fs::read(&path).unwrap();
                (path, data, is_text)
            })
            .collect();

    let scripts = list_fixtures("tests/fixtures/scripts", "jq");

    let total_tasks = scripts.len() * inputs.len();
    let completed = AtomicUsize::new(0);

    scripts.par_iter().for_each(|script| {
        let src = std::fs::read(script).unwrap();
        let original = std::str::from_utf8(&src).unwrap();
        let script_name = script.display().to_string();

        // Formatting only depends on (script, mode), so compute it once.
        let formatted: Vec<_> = FORMAT_MODES
            .iter()
            .map(|&mode| {
                let output = fmt(mode, &src);
                let output = String::from_utf8(output).unwrap();
                (mode, output)
            })
            .collect();

        for (input_path, data, is_text) in &inputs {
            for (variant_name, base_args) in jq_variants(*is_text) {
                let mut before_args = base_args.clone();
                before_args.push(original);

                let before = run_jq(&before_args, data);

                for (mode, formatted) in &formatted {
                    let mut after_args = base_args.clone();
                    after_args.push(formatted.as_str());

                    let after = run_jq(&after_args, data);

                    let case = format!(
                        "{script_name} -m {mode} <{}> [{variant_name}]",
                        input_path.display()
                    );

                    let result = if before.status.code() == after.status.code()
                        && before.stdout == after.stdout
                        && before.stderr == after.stderr
                    {
                        Ok(())
                    } else {
                        Err(format!(
                            "before: exit={:?} stdout={:?} stderr={:?} | after: exit={:?} stdout={:?} stderr={:?}",
                            before.status.code(),
                            String::from_utf8_lossy(&before.stdout),
                            String::from_utf8_lossy(&before.stderr),
                            after.status.code(),
                            String::from_utf8_lossy(&after.stdout),
                            String::from_utf8_lossy(&after.stderr),
                        ))
                    };

                    report.lock().unwrap().record(case, result);
                }
            }

            let curr = completed.fetch_add(1, Ordering::Relaxed) + 1;
            let percentage = (curr as f64 / total_tasks as f64) * 100.0;

            print!("\rProgress: {}/{} ({:.1}%)", curr, total_tasks, percentage);
            let _ = std::io::stdout().flush();
        }

        println!("Script {} DONE\n", script_name);
        let _ = std::io::stdout().flush();
    });

    println!("\nDone!");
    report.into_inner().unwrap().finish();
}
