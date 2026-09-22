use oneq::{
    data::{self, Value},
    jq::{
        CompileOptions, Session,
        vm::{InputMode, host::InputHost},
    },
};
use std::rc::Rc;

fn run(source: &str, inline: bool) -> (Vec<Result<Value, String>>, String) {
    let mut session = Session::new();
    let entry = session
        .append(
            source,
            CompileOptions {
                inline,
                ..CompileOptions::default()
            },
        )
        .unwrap();
    let dump = session.dump(entry).unwrap();
    let mut host = InputHost::new(std::iter::empty());
    let values = session
        .run(entry, &mut host, InputMode::Null)
        .unwrap()
        .map(|result| result.map_err(|error| error.to_string()))
        .collect();
    (values, dump)
}

#[test]
fn templates_inline_chains_and_builtin_functions() {
    for (source, expected) in [
        ("def twice(f): f | f; [1,2] | map(twice(.+1))", "[3,4]"),
        (
            "def a(f): f | f; def b(f): a(f); def c(f): b(f); 1 | c(.+1)",
            "3",
        ),
        ("def f($x): $x + .; 10 | [f((1,2))]", "[11,12]"),
        ("def f: 1; def g: f; def f: 2; [g,f]", "[1,2]"),
        ("def f(g): def h(a): a|a; h(g); 1|f(.+1)", "3"),
        ("def recurse: 9; [1] | [..]", "[9]"),
    ] {
        let (actual, dump) = run(source, true);
        assert_eq!(actual, run(source, false).0, "{source}");
        assert_eq!(
            actual,
            vec![Ok(data::parse_json_str(expected).unwrap())],
            "{source}"
        );
        assert!(!dump.contains("target: Function"), "{source}\n{dump}");
        assert!(!dump.contains("Scope"), "{source}\n{dump}");
    }
}

#[test]
fn substitution_preserves_bindings_labels_patterns_and_caller_closures() {
    for source in [
        "def twice(f): f | f; 1 | twice(. as $x | $x+1)",
        "def twice(f): f | f; 1 | twice(. as $x | def local: $x+1; local)",
        "def twice(f): f | f; 1 | twice(def local($x): $x+1; local(.))",
        "def twice(f): f | f; 2 | twice(def local: if .>0 then .-1|local else 3 end; local)",
        "def f(g): g as $x | [$x, g]; 3 as $x | f($x+1)",
        "def f($x): $x as [$a, {b:$b}] | $a+$b; [f([1,{b:2}]),f([3,{b:4}])]",
        "def f: . as {(.key): $v} | $v; {key:\"x\",x:7} | f",
        "def twice(f): f,f; [twice(label $out | 1, break $out)]",
        "label $out | def twice(f): f,f; [twice(1, break $out)]",
        "def f: reduce (1,2) as $x (0; .+$x); [f,f]",
        "def f: foreach (1,2) as $x (0; .+$x; .); [f,f]",
        "def f(g): g; {a:1} | path(f(.a))",
        "def f(g): g; {a:1} | f(.a) |= .+1",
        "def f(g): g; [.a.b?] | f(.)",
        "def f(g): g as [$a] ?// {a:$a} | $a; f({a:3})",
    ] {
        assert_eq!(run(source, true).0, run(source, false).0, "{source}");
    }
}

#[test]
fn effects_errors_and_collection_boundaries_survive_inlining() {
    for source in [
        "def twice(f): f,f; twice(1,error(\"e\"))",
        "def twice(f): f,f; try twice(error(\"e\")) catch .",
        "def f(g): [g][]; try f(1,error(\"e\")) catch .",
        "def f($x): $x; try f(1,error(\"e\")) catch .",
        "def f(g): g; first(f(1,error(\"e\")))",
        "def empty: 7; [empty]",
        "def f: empty; [f]",
        "def f(g): g; f(halt_error(3))",
    ] {
        assert_eq!(run(source, true).0, run(source, false).0, "{source}");
    }
    for (source, expected, remaining) in [
        ("[twice(input)]", "[1,2]", 3),
        ("first(twice(input))", "1", 2),
        ("first([twice(input)][])", "1", 3),
    ] {
        for inline in [false, true] {
            let mut session = Session::new();
            let entry = session
                .append(
                    &format!("def twice(f): f,f; {source}"),
                    CompileOptions {
                        inline,
                        ..CompileOptions::default()
                    },
                )
                .unwrap();
            let mut host = InputHost::new([1, 2, 3].into_iter().map(|n| Ok(Value::int(i64::from(n)))));
            let values = session
                .run(entry, &mut host, InputMode::Null)
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(values, vec![data::parse_json_str(expected).unwrap()]);
            assert_eq!(
                oneq::jq::vm::host::Host::next_input(&mut host)
                    .unwrap()
                    .unwrap(),
                Value::int(i64::from(remaining))
            );
        }
    }
}

#[test]
fn direct_lowering_preserves_scope_and_library_resolution() {
    let mut session = Session::new();
    for invalid in [
        "[(1 as $x | $x), $x]",
        "[\"\\(1 as $x | $x)\", $x]",
        "[(label $out | .), break $out]",
        "def unused: missing; 1",
    ] {
        assert!(
            session.append(invalid, CompileOptions::default()).is_err(),
            "{invalid}"
        );
    }
    for (source, expected) in [
        (". | (. | 1) | .", "1"),
        ("[empty, (1, empty), 2]", "[1,2]"),
        ("def empty: 7; [empty]", "[7]"),
        ("def recurse: 9; ..", "9"),
        ("def recurse(f): 9; [1] | [..]", "[[1],1]"),
        (
            "[last(1,2), (\"true\"|toboolean), ({a:1}|[leaf_paths])]",
            "[2,true,[[\"a\"]]]",
        ),
        ("[{a:1} | .a += (2,3)]", "[{\"a\":3},{\"a\":4}]"),
    ] {
        let values = run(source, true).0;
        assert_eq!(
            values,
            vec![Ok(data::parse_json_str(expected).unwrap())],
            "{source}"
        );
    }
}

#[test]
fn module_aliases_share_templates_and_keep_source_locations() {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library.jq");
    std::fs::write(&library, "def twice(f): f|f;\ndef location: $__loc__;\n").unwrap();
    for inline in [false, true] {
        let mut session = Session::new();
        let entry = session.append(
            "import \"library\" as a; import \"library\" as b; [a::twice(1+.), b::twice(2+.), a::location]",
            CompileOptions { inline, module_dirs: vec![dir.path().to_owned()], ..CompileOptions::default() },
        ).unwrap();
        if inline {
            assert!(!session.dump(entry).unwrap().contains("target: Function"));
        }
        let mut host = InputHost::new(std::iter::empty());
        let values = session
            .run(entry, &mut host, InputMode::Null)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let path = library
            .to_string_lossy()
            .replace('\\', "\\\\\\\\")
            .replace('"', "\\\"");
        let expected = format!("[2,4,{{\"file\":\"{path}\",\"line\":2}}]");
        let mut expected_value = data::parse_json_str(&expected).unwrap();
        if let Value::Array(items) = &expected_value {
            let mut normalized = items.as_ref().clone();
            normalized[0] = Value::Float(2.0);
            normalized[1] = Value::Float(4.0);
            expected_value = Value::Array(Rc::new(normalized));
        }
        assert_eq!(values, vec![expected_value]);
    }
}

#[test]
fn captured_and_recursive_functions_keep_calls() {
    for source in [
        "(1,2) as $x | def f: $x; [f,f]",
        "def f: if .>0 then .-1|f else 0 end; 3|f",
        "label $out | def f: break $out; 1, f",
    ] {
        let (actual, dump) = run(source, true);
        assert_eq!(actual, run(source, false).0, "{source}");
        assert!(dump.contains("target: Function"), "{source}\n{dump}");
    }
}

#[test]
fn session_templates_follow_execution_redefinition_and_rollback() {
    let mut session = Session::new();
    let repl = CompileOptions {
        repl: true,
        ..CompileOptions::default()
    };
    let definition = session.append("def f: 1;", repl.clone()).unwrap();
    let uninitialized = session.append("f", repl.clone()).unwrap();
    assert!(
        session
            .dump(uninitialized)
            .unwrap()
            .contains("target: Function")
    );
    let mut host = InputHost::new(std::iter::empty());
    assert!(
        session
            .run(uninitialized, &mut host, InputMode::Null)
            .unwrap()
            .next()
            .unwrap()
            .is_err()
    );
    session
        .run(definition, &mut host, InputMode::Null)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let old = session.append("f", repl.clone()).unwrap();
    assert!(!session.dump(old).unwrap().contains("target: Function"));
    let checkpoint = session.checkpoint();
    let new = session.append("def f: 2; f", repl.clone()).unwrap();
    let values = session
        .run(new, &mut host, InputMode::Null)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(values, vec![Value::int(2)]);
    assert!(session.append("def f: 3; missing", repl.clone()).is_err());
    session.rollback(checkpoint).unwrap();
    let restored = session.append("f", repl).unwrap();
    for entry in [old, restored] {
        let values = session
            .run(entry, &mut host, InputMode::Null)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(values, vec![Value::int(1)]);
    }
}

#[test]
fn expansion_budget_keeps_large_chains_compilable() {
    let mut source = "def f0(g): g; ".to_owned();
    for n in 1..30 {
        source.push_str(&format!("def f{n}(g): f{}(g)|f{}(g); ", n - 1, n - 1));
    }
    source.push_str("f29(empty)");
    let (actual, dump) = run(&source, true);
    assert!(actual.is_empty());
    assert!(dump.contains("target: Function"));
    assert!(dump.len() < 1_000_000);
}
