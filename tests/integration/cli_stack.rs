use crate::common::run_oneq;

// Spawn the actual binary: a test worker has a larger default stack than the
// Windows main thread, which hid the debug parser overflow in unit tests.
// The specific filters that used to be hardcoded here (identity, map,
// join, a small recursive def) now live as jq-oracle fixtures, since
// they're just generic jq behavior with no dependence on the worker's
// stack size; this test only needs one filter deep enough to have once
// overflowed the Windows main thread's default stack.
#[test]
fn cli_compiles_builtins_on_default_main_stack() {
    let out = run_oneq(
        &[
            "-c",
            "def f($x): if $x > 0 then [$x, f($x - 1)] else [] end; f(3)",
        ],
        b"null",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"[3,[2,[1,[]]]]\n");
    assert!(out.stderr.is_empty());
}

#[test]
fn cli_compiles_long_definition_sequences() {
    let mut filter = String::new();
    for index in 0..128 {
        filter.push_str(&format!("def f{index}: {index}; "));
    }
    filter.push_str("[f0,f127]");
    let output = run_oneq(&["-nc", &filter], b"");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"[0,127]\n");
}

#[test]
fn cli_data_formats_match_expected_json_in_all_input_modes() {
    for (format, source, expected) in [
        (
            "yaml",
            include_bytes!("../fixtures/data/catalog.yaml").as_slice(),
            include_bytes!("../fixtures/data/catalog.expected.jsons").as_slice(),
        ),
        (
            "toml",
            include_bytes!("../fixtures/data/build.toml").as_slice(),
            include_bytes!("../fixtures/data/build.expected.jsons").as_slice(),
        ),
    ] {
        for mode in [
            vec![],
            vec!["--stream"],
            vec!["--slurp"],
            vec!["--stream", "--slurp"],
        ] {
            let mut args = vec!["-c", "-S"];
            args.extend(mode);
            args.push(".");
            // Object order in the source need not match the expected fixture.
            // Check assembled values against the fixture, and stream events
            // against JSON with the source's actual insertion order.
            let assembled = run_oneq(&["-F", format, "-c", "."], source);
            assert!(assembled.status.success());
            let reference_input = if args.contains(&"--stream") {
                assembled.stdout.as_slice()
            } else {
                expected
            };
            let reference = run_oneq(&args, reference_input);
            args.splice(0..0, ["-F", format]);
            let actual = run_oneq(&args, source);
            for out in [&reference, &actual] {
                assert!(
                    out.status.success(),
                    "{args:?}: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
            }
            assert_eq!(actual.stdout, reference.stdout, "{args:?}");
        }
    }
}
