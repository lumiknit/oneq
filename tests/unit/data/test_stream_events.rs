use oneq::data::{AnyParser, DataFormat, StreamItem, Value, ValueBuilder, builder::StreamOption};
use oneq::io::{Input, InputTracker};

fn values(format: DataFormat, source: &str, option: StreamOption) -> Vec<String> {
    ValueBuilder::new(
        AnyParser::new(format, Input::new_str(source)).unwrap(),
        option,
    )
    .map(|v| v.unwrap().to_string())
    .collect()
}

#[test]
fn relative_events_pop_values_and_nested_containers() {
    let parser = AnyParser::new(DataFormat::Json, Input::new_str(r#"{"a":[1]} 42 [3]"#)).unwrap();
    let events: Vec<String> = parser
        .map(|event| match event.unwrap() {
            StreamItem::Push(item) => {
                let (value, key) = item.unpack();
                if key {
                    format!("push {}", oneq::strs::resolve(value).unwrap())
                } else {
                    format!("push {value}")
                }
            }
            StreamItem::Value(value) => format!("value {value}"),
            StreamItem::Close => "close".into(),
        })
        .collect();
    assert_eq!(
        events,
        [
            "push a", "push 0", "value 1", "close", "close", "value 42", "push 0", "value 3",
            "close"
        ]
    );
    assert_eq!(
        values(
            DataFormat::Json,
            r#"{"a":[1]} 42 [3]"#,
            StreamOption::Stream
        ),
        [
            r#"[["a",0],1]"#,
            r#"[["a",0]]"#,
            r#"[["a"]]"#,
            "[[],42]",
            "[[0],3]",
            "[[0]]"
        ]
    );
    assert_eq!(
        values(
            DataFormat::Json,
            r#"{"a":[1]} 42 [3]"#,
            StreamOption::Default
        ),
        [r#"{"a":[1]}"#, "42", "[3]"]
    );
}

#[test]
fn empty_containers_and_siblings_do_not_close_the_document_early() {
    let source = r#"{"a":[[],{},[1,2]],"b":{"c":3},"d":4} [] {} null"#;
    assert_eq!(
        values(DataFormat::Json, source, StreamOption::Stream),
        [
            r#"[["a",0],[]]"#,
            r#"[["a",1],{}]"#,
            r#"[["a",2,0],1]"#,
            r#"[["a",2,1],2]"#,
            r#"[["a",2,1]]"#,
            r#"[["a",2]]"#,
            r#"[["b","c"],3]"#,
            r#"[["b","c"]]"#,
            r#"[["d"],4]"#,
            r#"[["d"]]"#,
            "[[],[]]",
            "[[],{}]",
            "[[],null]",
        ]
    );
    assert_eq!(
        values(DataFormat::Json, source, StreamOption::Default),
        [
            r#"{"a":[[],{},[1,2]],"b":{"c":3},"d":4}"#,
            "[]",
            "{}",
            "null"
        ]
    );
}

#[test]
fn undefined_does_not_push_or_consume_an_array_index() {
    let source = "undefined {a: undefined, b: [undefined, 1, undefined, {}, undefined], c: undefined} [undefined] {x:undefined} 42";
    assert_eq!(
        values(DataFormat::Json5, source, StreamOption::Stream),
        [
            r#"[["b",0],1]"#,
            r#"[["b",1],{}]"#,
            r#"[["b",1]]"#,
            r#"[["b"]]"#,
            "[[],[]]",
            "[[],{}]",
            "[[],42]",
        ]
    );
    assert_eq!(
        values(DataFormat::Json5, source, StreamOption::Default),
        [r#"{"b":[1,{}]}"#, "[]", "{}", "42"]
    );
}

#[test]
fn json_yields_before_reading_the_whole_document() {
    let tracker = InputTracker::shared();
    let source = format!("[0,{}1]", "\n".repeat(8192));
    let input = Input::new_string_with_tracker(source, tracker.clone());
    let parser = AnyParser::new(DataFormat::Json, input).unwrap();
    let mut builder = ValueBuilder::new(parser, StreamOption::Stream);
    assert_eq!(builder.next().unwrap().unwrap().to_string(), "[[0],0]");
    assert!(
        tracker.borrow().line_number < 8192,
        "parser read the whole document before yielding its first value"
    );
    assert_eq!(
        builder.map(|v| v.unwrap().to_string()).collect::<Vec<_>>(),
        ["[[1],1]", "[[1]]"]
    );
}

#[test]
fn json_partial_events_precede_one_terminal_error() {
    for source in ["[1,", "[1,]", "[1/*", "[1, {\"x\":"] {
        let parser = AnyParser::new(DataFormat::Json, Input::new_str(source)).unwrap();
        let mut builder = ValueBuilder::new(parser, StreamOption::Stream);
        assert_eq!(builder.next().unwrap().unwrap().to_string(), "[[0],1]");
        assert!(builder.next().unwrap().is_err(), "{source}");
        assert!(
            builder.next().is_none(),
            "error should terminate the parser"
        );

        let parser = AnyParser::new(DataFormat::Json, Input::new_str(source)).unwrap();
        let mut builder = ValueBuilder::new(parser, StreamOption::Default);
        assert!(
            builder.next().unwrap().is_err(),
            "partial default-mode document must not be yielded"
        );
        assert!(builder.next().is_none());

        let parser = AnyParser::new(DataFormat::Json, Input::new_str(source)).unwrap();
        let mut builder = ValueBuilder::new(parser, StreamOption::StreamError);
        assert_eq!(builder.next().unwrap().unwrap().to_string(), "[[0],1]");
        let Value::Array(error) = builder.next().unwrap().unwrap() else {
            panic!("expected error array")
        };
        assert!(matches!(&error[0], Value::String(_)));
        assert!(builder.next().is_none());
    }
}

#[test]
fn all_formats_use_the_last_completed_child_for_close() {
    for (format, source, expected) in [
        (
            DataFormat::Yaml,
            "a:\n  - 1\nb: 2\n",
            vec![
                r#"[["a",0],1]"#,
                r#"[["a",0]]"#,
                r#"[["b"],2]"#,
                r#"[["b"]]"#,
            ],
        ),
        (
            DataFormat::Toml,
            "[a]\nb = 1\n[c]\nd = 2\n",
            vec![
                r#"[["a","b"],1]"#,
                r#"[["a","b"]]"#,
                r#"[["c","d"],2]"#,
                r#"[["c","d"]]"#,
                r#"[["c"]]"#,
            ],
        ),
        (
            DataFormat::Env,
            "A=1\nB=2\n",
            vec![r#"[["A"],"1"]"#, r#"[["B"],"2"]"#, r#"[["B"]]"#],
        ),
        (
            DataFormat::Csv,
            "a,b\nc,d\n",
            vec![
                r#"[[0,0],"a"]"#,
                r#"[[0,1],"b"]"#,
                "[[0,1]]",
                r#"[[1,0],"c"]"#,
                r#"[[1,1],"d"]"#,
                "[[1,1]]",
                "[[1]]",
            ],
        ),
    ] {
        assert_eq!(
            values(format, source, StreamOption::Stream),
            expected,
            "{format}"
        );
    }
}

#[test]
fn empty_csv_has_no_child_to_close() {
    for (format, source) in [(DataFormat::Csv, ""), (DataFormat::Csvh, "a,b\n")] {
        assert_eq!(values(format, source, StreamOption::Default), ["null"]);
        assert_eq!(values(format, source, StreamOption::Stream), ["[[],null]"]);
    }
}

#[test]
fn yaml_close_uses_last_emitted_key_even_when_it_is_a_duplicate() {
    assert_eq!(
        values(DataFormat::Yaml, "a: 1\nb: 2\na: 3\n", StreamOption::Stream),
        [r#"[["a"],1]"#, r#"[["b"],2]"#, r#"[["a"],3]"#, r#"[["a"]]"#]
    );
}

#[test]
fn default_reopens_containers_without_losing_siblings_or_key_order() {
    // Preserve the builder's existing leaf-update behavior for duplicate
    // container keys, including untouched array elements and object fields.
    assert_eq!(
        values(
            DataFormat::Json,
            r#"{"a":{"x":1,"keep":2},"b":3,"a":{"x":4}} {"a":[1,2],"a":[3]}"#,
            StreamOption::Default,
        ),
        [r#"{"a":{"keep":2,"x":4},"b":3}"#, r#"{"a":[3,2]}"#],
    );
    assert_eq!(
        values(
            DataFormat::Toml,
            "[a]\nx = 1\n[b]\ny = 2\n[a]\nz = 3\n",
            StreamOption::Default,
        ),
        [r#"{"a":{"x":1,"z":3},"b":{"y":2}}"#],
    );
}

#[test]
fn default_reopened_alias_does_not_mutate_shared_source() {
    assert_eq!(
        values(
            DataFormat::Yaml,
            "base: &base {x: 1}\na: *base\na:\n  y: 2\n",
            StreamOption::Default,
        ),
        [r#"{"a":{"x":1,"y":2},"base":{"x":1}}"#],
    );
}

#[test]
fn default_reopened_container_keeps_type_errors() {
    use oneq::data::DataError;
    for source in [r#"{"a":1,"a":{"b":2}}"#, r#"{"a":[],"a":{"b":2}}"#] {
        let parser = AnyParser::new(DataFormat::Json, Input::new_str(source)).unwrap();
        let mut builder = ValueBuilder::new(parser, StreamOption::Default);
        assert!(matches!(
            builder.next(),
            Some(Err(DataError::UnexpectedObjectKeyType { .. }))
        ));
    }
    let parser = AnyParser::new(DataFormat::Json, Input::new_str(r#"{"a":{},"a":[2]}"#)).unwrap();
    let mut builder = ValueBuilder::new(parser, StreamOption::Default);
    assert!(matches!(
        builder.next(),
        Some(Err(DataError::UnexpectedArrayIndexType { .. }))
    ));
}

#[test]
fn default_reopened_array_resolves_negative_indices() {
    use oneq::data::PathItem;
    let events = [
        StreamItem::Push(PathItem::new_key_str("a")),
        StreamItem::Value(oneq::data::parse_json_str("[10,20]").unwrap()),
        StreamItem::Push(PathItem::new_key_str("a")),
        StreamItem::Push(PathItem::new_idx(-1)),
        StreamItem::Value(Value::int(30)),
        StreamItem::Close,
        StreamItem::Close,
    ];
    let result: Vec<_> = ValueBuilder::new(events.into_iter().map(Ok), StreamOption::Default)
        .map(|v| v.unwrap().to_string())
        .collect();
    assert_eq!(result, [r#"{"a":[10,30]}"#]);
}

#[test]
fn default_detaching_subtree_keeps_object_insertion_order() {
    let parser = AnyParser::new(
        DataFormat::Json,
        Input::new_str(r#"{"z":{"x":1,"keep":2},"b":3,"z":{"x":4}}"#),
    )
    .unwrap();
    let Value::Object(root) = ValueBuilder::new(parser, StreamOption::Default)
        .next()
        .unwrap()
        .unwrap()
    else {
        panic!("expected object")
    };
    let keys: Vec<_> = root
        .keys()
        .map(|key| oneq::strs::resolve(*key).unwrap())
        .collect();
    assert_eq!(keys, ["z", "b"]);
    let Value::Object(child) = root.values().next().unwrap() else {
        panic!("expected child object")
    };
    let keys: Vec<_> = child
        .keys()
        .map(|key| oneq::strs::resolve(*key).unwrap())
        .collect();
    assert_eq!(keys, ["x", "keep"]);
}
