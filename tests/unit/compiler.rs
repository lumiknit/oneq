//! `Session::prepare`/`ModuleGraph` structure tests: no `run`/`append`
//! execution, no output to compare. Tests that actually execute a
//! program and check its output live in `tests/integration/modules.rs`.
use oneq::{
    data::Value,
    jq::{
        CompileOptions, Session,
        compiler::modules::{DependencyTarget, ImportKind, ModuleGraph},
    },
};
use std::{fs, path::Path, rc::Rc};

#[test]
fn jq_library_definitions_have_no_duplicate_native_specs() {
    for source in [
        include_str!("../../src/jq/builtins/builtin.jq"),
        include_str!("../../src/jq/builtins/compat.jq"),
    ] {
        let (files, root) = oneq::jq::parser::parse_pairs("<library>", source).unwrap();
        let mut pending: Vec<_> = root.semantic_children().collect();
        while let Some(pair) = pending.pop() {
            if pair.tag != oneq::jq::parser::pairs::PairTag::Def {
                continue;
            }
            let children: Vec<_> = pair.semantic_children().collect();
            let name = pair.text(&files).unwrap();
            let arity = children.len() - 2;
            assert!(
                oneq::jq::builtins::lookup_symbol(oneq::strs::intern(name), arity).is_none(),
                "duplicate native {name}/{arity}"
            );
            pending.push(children[children.len() - 1]);
        }
    }
}

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
fn imported_data(
    graph: &ModuleGraph,
    module: usize,
    dependency: usize,
) -> &Rc<oneq::jq::compiler::modules::ParsedData> {
    let DependencyTarget::Data(data) = &graph.modules[module].dependencies[dependency].target
    else {
        panic!("not a data import")
    };
    data
}

#[test]
fn preparation_parses_transitive_sources_and_json_with_metadata_and_spans() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path().join("lib/util/util.jq"),
        "module {homepage:\"example\", info:[true,null,2]};\nimport \"values\" as $items {search:\".\"};\ndef value: $items::items;",
    );
    write(dir.path().join("lib/util/values.json"), "1\n{\"n\":2}\n");
    let mut session = Session::new();
    let graph = session.prepare("# entry\nimport \"util\" as first {search:[\"missing\",\"lib\"]};\ninclude \"util\" {search:\"lib\"}; .", &options(dir.path())).unwrap();
    assert_eq!(graph.modules.len(), 2);
    assert!(
        matches!(&graph.entry().dependencies[0].import.kind, ImportKind::Module { alias } if alias == "first")
    );
    assert_eq!(
        graph.entry().dependencies[1].import.kind,
        ImportKind::Include
    );
    let dep = &graph.modules[1];
    assert!(
        dep.source
            .metadata
            .as_ref()
            .unwrap()
            .to_string()
            .contains("homepage")
    );
    assert_eq!(
        dep.source.imports[0].kind,
        ImportKind::Data {
            alias: "items".into()
        }
    );
    assert_eq!(
        imported_data(&graph, 1, 0).value.to_string(),
        "[1,{\"n\":2}]"
    );
    for module in &graph.modules {
        module.source.root.validate(&module.source.files).unwrap();
        for import in &module.source.imports {
            assert!(
                module
                    .source
                    .files
                    .text(import.span.as_ref().unwrap())
                    .unwrap()
                    .starts_with("import")
                    || module
                        .source
                        .files
                        .text(import.span.as_ref().unwrap())
                        .unwrap()
                        .starts_with("include")
            );
        }
    }
}

#[test]
fn diamond_imports_keep_distinct_aliases_but_share_one_source() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path().join("left.jq"),
        "import \"shared\" as local; def left: local::value;",
    );
    write(
        dir.path().join("right.jq"),
        "include \"shared\"; def right: value;",
    );
    write(dir.path().join("shared.jq"), "def value: 3;");
    let graph = Session::new()
        .prepare(
            "import \"left\" as a; import \"right\" as b; .",
            &options(dir.path()),
        )
        .unwrap();
    assert_eq!(graph.modules.len(), 4);
    let DependencyTarget::Module(left_target) = graph.modules[1].dependencies[0].target else {
        panic!()
    };
    let DependencyTarget::Module(right_target) = graph.modules[3].dependencies[0].target else {
        panic!()
    };
    assert_eq!(left_target, right_target);
    assert_eq!(
        graph
            .module(left_target)
            .unwrap()
            .source
            .path
            .file_name()
            .unwrap(),
        "shared.jq"
    );
}

#[test]
fn repl_reuses_source_and_json_snapshots_across_preparations_and_rollback() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path().join("shared.jq"),
        "import \"data\" as $d; def value: $d::d;",
    );
    write(dir.path().join("data.json"), "[1,2]\n3");
    let mut session = Session::new();
    let opts = options(dir.path());
    let checkpoint = session.checkpoint();
    let first = session.prepare("include \"shared\"; .", &opts).unwrap();
    write(dir.path().join("shared.jq"), "this is no longer valid jq (");
    fs::remove_file(dir.path().join("data.json")).unwrap();
    session.rollback(checkpoint).unwrap();
    let second = session
        .prepare("import \"shared\" as renamed; .", &opts)
        .unwrap();
    assert!(Rc::ptr_eq(
        &first.modules[1].source,
        &second.modules[1].source
    ));
    assert!(Rc::ptr_eq(
        imported_data(&first, 1, 0),
        imported_data(&second, 1, 0)
    ));
    assert_eq!(imported_data(&second, 1, 0).value.to_string(), "[[1,2],3]");
    assert!(
        Session::new()
            .prepare("include \"shared\"; .", &opts)
            .is_err()
    );
}

#[test]
fn failed_dependency_loading_is_atomic_and_retry_reads_repaired_files() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path().join("first.jq"),
        "include \"missing\"; def f: 1;",
    );
    let mut session = Session::new();
    let opts = options(dir.path());
    assert!(session.prepare("include \"first\"; .", &opts).is_err());
    write(dir.path().join("first.jq"), "def f: 2;");
    let graph = session.prepare("include \"first\"; .", &opts).unwrap();
    assert_eq!(graph.modules.len(), 2);
    assert!(graph.modules[1].dependencies.is_empty());
    write(dir.path().join("broken.jq"), "def (");
    assert!(session.prepare("include \"broken\"; .", &opts).is_err());
    write(dir.path().join("broken.jq"), "def fixed: 1;");
    assert!(session.prepare("include \"broken\"; .", &opts).is_ok());
}

#[test]
fn cycle_errors_include_chain_and_do_not_poison_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path().join("a.jq"), "include \"b\"; def a: 1;");
    write(dir.path().join("b.jq"), "include \"a\"; def b: 2;");
    let mut session = Session::new();
    let opts = options(dir.path());
    let error = session
        .prepare("include \"a\"; .", &opts)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("cyclic import") && error.contains("a.jq") && error.contains("b.jq"),
        "{error}"
    );
    write(dir.path().join("b.jq"), "def b: 2;");
    assert_eq!(
        session
            .prepare("include \"a\"; .", &opts)
            .unwrap()
            .modules
            .len(),
        3
    );
    // The caller-provided root is part of the active graph, even if it is on disk.
    write(dir.path().join("main.jq"), "include \"main\"; .");
    assert!(
        session
            .prepare("include \"main\"; .", &opts)
            .unwrap_err()
            .to_string()
            .contains("cyclic import")
    );
}

#[test]
fn search_changes_resolve_new_edges_without_reparsing_cached_sources() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path().join("parent.jq"),
        "include \"child\"; def f: child;",
    );
    write(dir.path().join("v1/child.jq"), "def child: 1;");
    write(dir.path().join("v2/child.jq"), "def child: 2;");
    let mut opts = options(dir.path());
    opts.module_dirs.push(dir.path().join("v1"));
    let mut session = Session::new();
    let first = session.prepare("include \"parent\"; .", &opts).unwrap();
    opts.module_dirs[1] = dir.path().join("v2");
    let second = session.prepare("include \"parent\"; .", &opts).unwrap();
    assert!(Rc::ptr_eq(
        &first.modules[1].source,
        &second.modules[1].source
    ));
    assert!(!Rc::ptr_eq(
        &first.modules[2].source,
        &second.modules[2].source
    ));
}

#[test]
fn metadata_search_precedes_options_and_terminators_stop_fallback() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path().join("local/dep.jq"), "def value: 1;");
    write(dir.path().join("dep.jq"), "def value: 2;");
    let opts = options(dir.path());
    let mut session = Session::new();
    let graph = session
        .prepare("include \"dep\" {search:\"local\"}; .", &opts)
        .unwrap();
    assert_eq!(
        graph.modules[1].source.canonical_path.as_ref().unwrap(),
        &fs::canonicalize(dir.path().join("local/dep.jq")).unwrap()
    );
    for metadata in [
        "[]",
        "[\"missing\"]",
        "null",
        "\"\"",
        "[\"missing\",null,\"local\"]",
        "[\"missing\",\"\",\"local\"]",
    ] {
        let source = format!("include \"dep\" {{search:{metadata}}}; .");
        assert!(
            session
                .prepare(&source, &opts)
                .unwrap_err()
                .to_string()
                .contains("not found")
        );
    }
    for name in ["../dep", "dep/dep", ""] {
        let source = format!("include \"{name}\"; .");
        assert!(session.prepare(&source, &opts).is_err());
    }
    assert!(
        session
            .prepare("include \"dep\" {search:.}; .", &opts)
            .is_err()
    );
}

#[test]
fn data_imports_share_values_across_aliases_and_collect_empty_streams() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path().join("data.json"), "");
    let graph = Session::new()
        .prepare(
            "import \"data\" as $a; import \"data\" as $b; $a::a",
            &options(dir.path()),
        )
        .unwrap();
    assert!(Rc::ptr_eq(
        imported_data(&graph, 0, 0),
        imported_data(&graph, 0, 1)
    ));
    assert_eq!(
        imported_data(&graph, 0, 0).value,
        Value::Array(Rc::new(vec![]))
    );
    assert_eq!(
        graph.entry().dependencies[1].import.kind,
        ImportKind::Data { alias: "b".into() }
    );
}

#[cfg(unix)]
#[test]
fn canonical_paths_share_symlinked_modules_and_data() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path().join("real.jq"), "def value: 1;");
    write(dir.path().join("real.json"), "1");
    std::os::unix::fs::symlink("real.jq", dir.path().join("alias.jq")).unwrap();
    std::os::unix::fs::symlink("real.json", dir.path().join("alias.json")).unwrap();
    let graph = Session::new().prepare("import \"real\" as x; import \"alias\" as y; import \"real\" as $a; import \"alias\" as $b; .", &options(dir.path())).unwrap();
    assert_eq!(graph.modules.len(), 2);
    assert!(Rc::ptr_eq(
        imported_data(&graph, 0, 2),
        imported_data(&graph, 0, 3)
    ));
}

#[test]
fn deeply_nested_modules_use_an_explicit_traversal_stack() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..200 {
        let source = if i == 199 {
            "def f: 1;".into()
        } else {
            format!("include \"m{}\"; def f: 1;", i + 1)
        };
        write(dir.path().join(format!("m{i}.jq")), &source);
    }
    let graph = Session::new()
        .prepare("include \"m0\"; .", &options(dir.path()))
        .unwrap();
    assert_eq!(graph.modules.len(), 201);
}

#[test]
fn origin_search_expands_relative_to_the_executable() {
    let executable = std::env::current_exe().unwrap();
    let fixture = tempfile::tempdir_in(executable.parent().unwrap()).unwrap();
    write(fixture.path().join("origin.jq"), "def value: 1;");
    let directory = fixture.path().file_name().unwrap().to_str().unwrap();
    let source = format!("include \"origin\" {{search:\"$ORIGIN/{directory}\"}}; .");
    let graph = Session::new()
        .prepare(&source, &CompileOptions::default())
        .unwrap();
    assert_eq!(
        graph.modules[1].source.canonical_path.as_ref().unwrap(),
        &fs::canonicalize(fixture.path().join("origin.jq")).unwrap()
    );
}

#[test]
fn top_level_source_is_parsed_again_even_when_its_path_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let opts = options(dir.path());
    let mut session = Session::new();
    let first = session.prepare("1", &opts).unwrap();
    let second = session.prepare("2", &opts).unwrap();
    assert!(!Rc::ptr_eq(&first.entry().source, &second.entry().source));
    assert_eq!(first.entry().source.files.files[0].source, "1");
    assert_eq!(second.entry().source.files.files[0].source, "2");
}
