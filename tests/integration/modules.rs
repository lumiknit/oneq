//! Module/import tests that actually execute a program via `Session`
//! and compare its output. Structural `ModuleGraph` checks that never
//! run a program live in `tests/unit/compiler.rs`.
use oneq::{
    data::Value,
    jq::{CompileOptions, Session},
};
use std::{fs, path::Path};

fn options(path: &Path) -> CompileOptions {
    CompileOptions {
        path: path.join("main.jq").to_string_lossy().into_owned(),
        module_dirs: vec![path.to_path_buf()],
        ..CompileOptions::default()
    }
}
fn write(path: impl AsRef<Path>, source: &str) {
    let path = path.as_ref();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
}
fn execute(session: &mut Session, entry: oneq::jq::EntryId) -> Vec<Value> {
    use oneq::jq::vm::{InputMode, host::InputHost};
    let mut host = InputHost::new(std::iter::empty());
    session
        .run(entry, &mut host, InputMode::Null)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn append_loads_dependencies_before_attempting_ir_and_keeps_completed_cache() {
    let dir = tempfile::tempdir().unwrap();
    let opts = options(dir.path());
    let mut session = Session::new();
    let identity = session.append(".", CompileOptions::default()).unwrap();
    let error = session
        .append("include \"library\"; .", opts.clone())
        .unwrap_err()
        .to_string();
    assert!(error.contains("library"), "{error}");
    assert!(error.contains("not found"), "{error}");
    write(
        dir.path().join("library.jq"),
        "import \"data\" as $d; def f: $d::d;",
    );
    write(dir.path().join("data.json"), "1\n[");
    let error = session
        .append("include \"library\"; .", opts.clone())
        .unwrap_err()
        .to_string();
    assert!(error.contains("data.json"), "{error}");
    write(dir.path().join("data.json"), "9");
    let entry = session
        .append("include \"library\"; f", opts.clone())
        .unwrap();
    assert_eq!(
        execute(&mut session, entry),
        vec![oneq::data::parse_json_str("[9]").unwrap()]
    );
    fs::remove_file(dir.path().join("library.jq")).unwrap();
    fs::remove_file(dir.path().join("data.json")).unwrap();
    let graph = session.prepare("include \"library\"; .", &opts).unwrap();
    let oneq::jq::compiler::modules::DependencyTarget::Data(data) =
        &graph.modules[1].dependencies[0].target
    else {
        panic!("not a data import")
    };
    assert_eq!(data.value.to_string(), "[9]");
    assert!(session.dump(identity).is_ok());
}

#[test]
fn json_imports_execute_with_both_names_and_cached_stream_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path().join("values.json"), "1\n{\"name\":\"λ\"}\n");
    write(dir.path().join("empty.json"), "");
    let mut session = Session::new();
    let opts = options(dir.path());
    let entry = session.append(
        "module {}; import \"values\" as $x; import \"empty\" as $e; [$x, $x::x, $e, ($x | length)]",
        opts.clone(),
    ).unwrap();
    let expected =
        oneq::data::parse_json_str("[[1,{\"name\":\"λ\"}],[1,{\"name\":\"λ\"}],[],2]").unwrap();
    assert_eq!(execute(&mut session, entry), vec![expected.clone()]);
    fs::remove_file(dir.path().join("values.json")).unwrap();
    let alias = session
        .append("import \"values\" as $other; $other::other", opts.clone())
        .unwrap();
    assert_eq!(
        execute(&mut session, alias),
        vec![oneq::data::parse_json_str("[1,{\"name\":\"λ\"}]").unwrap()]
    );
    assert_eq!(execute(&mut session, entry), vec![expected]);
    // Ordinary programs do not export their import aliases to future appends.
    assert!(session.append("$x", opts).is_err());
}

#[test]
fn repl_data_import_redefinition_failure_and_rollback_preserve_binding_identity() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path().join("one.json"), "1");
    write(dir.path().join("two.json"), "2");
    let mut session = Session::new();
    let mut opts = options(dir.path());
    opts.repl = true;
    let first = session
        .append("import \"one\" as $x; $x", opts.clone())
        .unwrap();
    let checkpoint = session.checkpoint();
    let second = session
        .append("import \"two\" as $x; $x::x", opts.clone())
        .unwrap();
    let one = oneq::data::parse_json_str("[1]").unwrap();
    let two = oneq::data::parse_json_str("[2]").unwrap();
    assert_eq!(execute(&mut session, first), vec![one.clone()]);
    assert_eq!(execute(&mut session, second), vec![two.clone()]);
    assert!(
        session
            .append("import \"one\" as $x; $missing", opts.clone())
            .is_err()
    );
    let current = session.append("$x, $x::x", opts.clone()).unwrap();
    assert_eq!(execute(&mut session, current), vec![two.clone(), two]);
    session.rollback(checkpoint).unwrap();
    assert!(session.dump(second).is_err());
    let restored = session.append("$x, $x::x", opts).unwrap();
    assert_eq!(execute(&mut session, restored), vec![one.clone(), one]);
}

#[test]
fn source_modules_link_hygienically_through_aliases_includes_and_data() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path().join("base.jq"),
        "def hidden: 3; def value(f): hidden + f;",
    );
    write(
        dir.path().join("left.jq"),
        "import \"base\" as b; def hidden: 100; def left: b::value(2);",
    );
    write(
        dir.path().join("right.jq"),
        "include \"base\"; import \"data\" as $d; def right: value($d[0]);",
    );
    write(dir.path().join("data.json"), "7");
    let mut session = Session::new();
    let mut opts = options(dir.path());
    opts.repl = true;
    let entry = session.append(
        "import \"left\" as l; import \"right\" as r; import \"base\" as a; import \"base\" as b; def hidden: 999; [l::left,r::right,a::value(1),b::value(2),hidden]",
        opts.clone(),
    ).unwrap();
    assert_eq!(
        execute(&mut session, entry),
        vec![oneq::data::parse_json_str("[5,10,4,5,999]").unwrap()]
    );
    let next = session.append("[l::left, r::right]", opts).unwrap();
    assert_eq!(
        execute(&mut session, next),
        vec![oneq::data::parse_json_str("[5,10]").unwrap()]
    );
}
