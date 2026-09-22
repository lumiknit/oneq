//! JSON -> <format> -> JSON roundtrip checks: for each structured format,
//! parsing a JSON value, serializing it to that format, then parsing that
//! back should reproduce the original value.

use oneq::data::builder::StreamOption;
use oneq::data::{AnyParser, AnySerializer, DataFormat, Value, ValueBuilder};
use oneq::io::{Input, Output};
use oneq::render;
use std::{cell::RefCell, rc::Rc};

fn parse_json(content: &str) -> Value {
    let mut values: Vec<Value> = ValueBuilder::new(
        AnyParser::new(DataFormat::Json, Input::new_string(content.to_owned())).unwrap(),
        StreamOption::default(),
    )
    .collect::<Result<_, _>>()
    .unwrap_or_else(|e| panic!("parse json {content:?}: {e:?}"));
    assert_eq!(values.len(), 1, "expected exactly one JSON value");
    values.pop().unwrap()
}

fn serialize(format: DataFormat, value: Value) -> String {
    serialize_with(format, value, render::FormatOptions::default())
}

fn serialize_with(format: DataFormat, value: Value, out: render::FormatOptions) -> String {
    let buf = Rc::new(RefCell::new(Vec::new()));
    let options = render::Options::new(out);
    let mut serializer =
        AnySerializer::from_format(format, Output::new_string_buffer(buf.clone()), options)
            .unwrap_or_else(|e| panic!("build {format} serializer: {e:?}"));
    serializer
        .put(value)
        .unwrap_or_else(|e| panic!("serialize to {format}: {e:?}"));
    let bytes = buf.borrow().clone();
    String::from_utf8(bytes).unwrap()
}

fn parse(format: DataFormat, content: &str) -> Value {
    let mut values: Vec<Value> = ValueBuilder::new(
        AnyParser::new(format, Input::new_string(content.to_owned())).unwrap(),
        StreamOption::default(),
    )
    .collect::<Result<_, _>>()
    .unwrap_or_else(|e| panic!("parse {format} {content:?}: {e:?}"));
    assert_eq!(
        values.len(),
        1,
        "expected exactly one value from {format} content {content:?}"
    );
    values.pop().unwrap()
}

/// Round-trips `json` through `format` and checks the value survives,
/// using each value's canonical `Display` form (order-independent for
/// objects) for comparison.
fn assert_roundtrip(format: DataFormat, json: &str) {
    let original = parse_json(json);
    let encoded = serialize(format, original.clone());
    let decoded = parse(format, &encoded);
    assert_eq!(
        decoded.to_string(),
        original.to_string(),
        "{format} roundtrip mismatch: {json} -> {encoded:?} -> {decoded}"
    );
}

const OBJECT_SAMPLE: &str =
    r#"{"a": 1, "b": "two", "c": true, "d": null, "e": [1, 2, 3], "f": {"g": 1.5}}"#;
const NESTED_SAMPLE: &str =
    r#"{"users": [{"name": "Alice", "age": 30}, {"name": "Bob", "age": 25}]}"#;

#[test]
fn yaml_roundtrip() {
    assert_roundtrip(DataFormat::Yaml, OBJECT_SAMPLE);
    assert_roundtrip(DataFormat::Yaml, NESTED_SAMPLE);
    assert_roundtrip(DataFormat::Yaml, "[1, 2, 3]");
    assert_roundtrip(DataFormat::Yaml, "\"just a string\"");
}

#[test]
fn toml_roundtrip() {
    // TOML documents are always tables, and TOML has no `null` - use an
    // object without null members for a fair roundtrip.
    assert_roundtrip(
        DataFormat::Toml,
        r#"{"a": 1, "b": "two", "c": true, "e": [1, 2, 3], "f": {"g": 1.5}}"#,
    );
    assert_roundtrip(DataFormat::Toml, NESTED_SAMPLE);
}

#[test]
fn xml_roundtrip() {
    // XML has no generic "object -> element" mapping: a serializable value
    // must already be the `{"name", "attributes", "children"}` element tree
    // (with `{"text": ...}` leaves) that `XmlParser` itself produces.
    let xml_tree = r#"{
        "name": "root",
        "attributes": {"id": "1"},
        "children": [
            {"name": "a", "attributes": {}, "children": [{"text": "1"}]},
            {"name": "b", "attributes": {}, "children": [{"text": "2"}]}
        ]
    }"#;
    assert_roundtrip(DataFormat::Xml, xml_tree);
}

#[test]
fn csv_roundtrip() {
    // CSV only round-trips faithfully for its "array of flat objects" shape
    // - everything comes back out as strings, so compare against the
    // stringified original rather than the typed original.
    let original = parse_json(r#"[{"a": "1", "b": "two"}, {"a": "3", "b": "four"}]"#);
    let encoded = serialize(DataFormat::Csvh, original.clone());
    let decoded = parse(DataFormat::Csvh, &encoded);
    assert_eq!(decoded.to_string(), original.to_string());
}

#[test]
fn env_roundtrip() {
    let original = parse_json(r#"{"A": "1", "B": "two words", "C": "with\"quote"}"#);
    let encoded = serialize(DataFormat::Env, original.clone());
    let decoded = parse(DataFormat::Env, &encoded);
    assert_eq!(decoded.to_string(), original.to_string());
}

/// A handful of formats describing the same document must all parse to the
/// same `Value`.
#[test]
fn equivalent_documents_across_formats_parse_equal() {
    let json = r#"{"name": "oneq", "version": 1, "tags": ["cli", "jq"], "nested": {"ok": true}}"#;
    let yaml = "name: oneq\nversion: 1\ntags:\n  - cli\n  - jq\nnested:\n  ok: true\n";
    let toml = "name = \"oneq\"\nversion = 1\ntags = [\"cli\", \"jq\"]\n\n[nested]\nok = true\n";

    let from_json = parse(DataFormat::Json, json).to_string();
    let from_yaml = parse(DataFormat::Yaml, yaml).to_string();
    let from_toml = parse(DataFormat::Toml, toml).to_string();

    assert_eq!(from_json, from_yaml);
    assert_eq!(from_json, from_toml);
}

#[test]
fn xml_streams_partial_events_before_truncation_error() {
    // A document missing its closing tags: the inner `<b>` element and the
    // outer `<a>` never close - `pump()` should still have emitted events
    // for `id`/`x` (the fully-closed grandchild) before hitting EOF.
    let truncated = r#"<root><a><b><x>1</x>"#;
    let mut parser =
        oneq::data::AnyParser::new(DataFormat::Xml, Input::new_string(truncated.to_owned()))
            .unwrap();
    let mut items = Vec::new();
    let mut saw_error = false;
    for item in &mut parser {
        match item {
            Ok(stream_item) => items.push(stream_item),
            Err(_) => {
                saw_error = true;
                break;
            }
        }
    }
    assert!(
        saw_error,
        "expected a parse error for the truncated document"
    );
    assert!(
        !items.is_empty(),
        "expected at least the fully-closed <x>1</x> events before the error"
    );
    // The deepest fully-closed element's tag name/text must have streamed.
    let has_x_name = items.iter().any(
        |i| matches!(i, oneq::data::StreamItem::Value(oneq::data::Value::String(s)) if &**s == "x"),
    );
    assert!(
        has_x_name,
        "expected the <x> element's name to have streamed before the truncation error"
    );
}

#[test]
fn yaml_streams_partial_events_before_truncation_error() {
    // `a` and `b` are valid top-level siblings; `c`'s nested mapping then
    // has an indentation error on its second key - `a`/`b` (already fully
    // resolved before the error) must still have streamed.
    let truncated = "a: 1\nb: 2\nc:\n  d: 1\n   e: 2\n";
    let parser =
        oneq::data::AnyParser::new(DataFormat::Yaml, Input::new_string(truncated.to_owned()))
            .unwrap();
    let mut parser = ValueBuilder::new(parser, StreamOption::Stream);
    let mut items = Vec::new();
    let mut saw_error = false;
    for item in &mut parser {
        match item {
            Ok(stream_item) => items.push(stream_item),
            Err(_) => {
                saw_error = true;
                break;
            }
        }
    }
    assert!(saw_error, "expected an indentation error for `c`");
    let has_a = items.iter().any(|i| i.to_string() == r#"[["a"],1]"#);
    let has_b = items.iter().any(|i| i.to_string() == r#"[["b"],2]"#);
    assert!(
        has_a && has_b,
        "expected a's and b's leaf events to have streamed before the error in c"
    );
}

#[test]
fn yaml_multiline_string_uses_block_scalar_in_default_mode() {
    // Strings with an embedded newline should use a literal block scalar
    // (`|`/`|-`) in the default (pretty) style, and round-trip exactly.
    assert_roundtrip(DataFormat::Yaml, r#"{"a": "line1\nline2\nline3\n"}"#);
    // No trailing newline -> `|-` (strip chomping).
    assert_roundtrip(DataFormat::Yaml, r#"{"a": "line1\nline2"}"#);
    // A single embedded newline still counts.
    assert_roundtrip(DataFormat::Yaml, r#"["x\ny\n", "p\nq"]"#);
    // Nested under an object/array key.
    assert_roundtrip(
        DataFormat::Yaml,
        r#"{"nested": {"b": "m1\nm2\n"}, "arr": ["l1\nl2\n"]}"#,
    );

    let encoded = serialize(DataFormat::Yaml, parse_json(r#"{"a": "line1\nline2\n"}"#));
    assert!(
        encoded.contains("a: |\n"),
        "expected a literal block scalar header, got: {encoded:?}"
    );
}

#[test]
fn yaml_multiline_string_falls_back_to_quoted_when_unsafe() {
    // A string whose lines carry their own leading/trailing whitespace
    // can't round-trip through this reader's block scalar (it folds
    // per-line indentation into each line's own indent), so it must stay
    // double-quoted instead - never opening `|`.
    let value = parse_json(r#"{"a": "line1\n  line2\n"}"#);
    let encoded = serialize(DataFormat::Yaml, value);
    assert!(
        !encoded.contains('|'),
        "expected quoted fallback (no block scalar), got: {encoded:?}"
    );
}

#[test]
fn yaml_multiline_string_stays_quoted_outside_pretty_mode() {
    let value = parse_json(r#"{"a": "line1\nline2\n"}"#);
    let mut out = render::FormatOptions::default();
    out.with_compact_level(render::CompactLevel::Compact);
    let encoded = serialize_with(DataFormat::Yaml, value, out);
    assert!(
        !encoded.contains('|'),
        "compact YAML must keep using quoted strings, got: {encoded:?}"
    );
}

#[test]
fn toml_multiline_string_uses_triple_quote_in_default_mode() {
    assert_roundtrip(DataFormat::Toml, r#"{"a": "line1\nline2\nline3"}"#);
    assert_roundtrip(
        DataFormat::Toml,
        r#"{"a": "with \"quotes\"\nand a\nnewline"}"#,
    );
    assert_roundtrip(
        DataFormat::Toml,
        r#"{"nested": {"b": "m1\nm2"}, "e": [1, 2, 3]}"#,
    );

    let encoded = serialize(DataFormat::Toml, parse_json(r#"{"a": "line1\nline2"}"#));
    assert!(
        encoded.contains("a = \"\"\"\n"),
        "expected a triple-quoted multi-line string, got: {encoded:?}"
    );
}

#[test]
fn toml_multiline_string_stays_single_quote_inside_arrays_and_inline_tables() {
    // The triple-quote form is only used for a top-level `key = value`
    // assignment, never for a string nested inside an array or an inline
    // table. A mixed-type array (not all-object) keeps its object element
    // as an inline `{ ... }` table rather than an array-of-tables section.
    let value = parse_json(r#"{"a": ["x\ny"], "b": [{"c": "x\ny"}, 1]}"#);
    let encoded = serialize(DataFormat::Toml, value.clone());
    assert!(
        !encoded.contains("\"\"\""),
        "nested multi-line strings must not use triple quotes, got: {encoded:?}"
    );
    let decoded = parse(DataFormat::Toml, &encoded);
    assert_eq!(decoded.to_string(), value.to_string());
}

#[test]
fn toml_multiline_string_stays_single_quote_outside_pretty_mode() {
    let value = parse_json(r#"{"a": "line1\nline2"}"#);
    let mut out = render::FormatOptions::default();
    out.with_compact_level(render::CompactLevel::Compact);
    let encoded = serialize_with(DataFormat::Toml, value, out);
    assert!(
        !encoded.contains("\"\"\""),
        "compact TOML must keep using single-quoted strings, got: {encoded:?}"
    );
}

#[test]
fn yaml_alias_streams_as_a_single_event_not_leaf_by_leaf() {
    // A value produced in one shot by `text_value` - a flow mapping/array,
    // scalar, block scalar, or alias - is already a complete `Value`, so it
    // streams as one `[path, value]` event carrying the whole thing, rather
    // than being re-decomposed into one event per leaf. This holds both for
    // `a` (a literal flow mapping) and `c` (an alias to it).
    let yaml = "a: &t {\"a\": 20}\nc: *t\n";
    let events: Vec<Value> = ValueBuilder::new(
        AnyParser::new(DataFormat::Yaml, Input::new_string(yaml.to_owned())).unwrap(),
        StreamOption::Stream,
    )
    .collect::<Result<_, _>>()
    .unwrap();
    let rendered: Vec<String> = events.iter().map(|v| v.to_string()).collect();
    assert_eq!(
        rendered,
        vec![r#"[["a"],{"a":20}]"#, r#"[["c"],{"a":20}]"#, r#"[["c"]]"#,]
    );
}

#[test]
fn yaml_block_style_mapping_still_streams_leaf_by_leaf() {
    // Unlike a flow mapping, a block-style nested mapping is built up
    // incrementally line-by-line (via `map`/`map_first`), so it must still
    // stream one event per leaf.
    let yaml = "a:\n  x: 1\n  y: 2\n";
    let events: Vec<Value> = ValueBuilder::new(
        AnyParser::new(DataFormat::Yaml, Input::new_string(yaml.to_owned())).unwrap(),
        StreamOption::Stream,
    )
    .collect::<Result<_, _>>()
    .unwrap();
    let rendered: Vec<String> = events.iter().map(|v| v.to_string()).collect();
    assert_eq!(
        rendered,
        vec![
            r#"[["a","x"],1]"#,
            r#"[["a","y"],2]"#,
            r#"[["a","y"]]"#,
            r#"[["a"]]"#,
        ]
    );
}

#[test]
fn yaml_merge_key_overridden_by_later_explicit_key_wins() {
    // Regression test for the streaming map_first rewrite: an explicit key
    // occurring anywhere in the mapping must win over a same-named merged
    // key, and the merge itself must never leak a literal "<<" key into
    // the result.
    let yaml = "base: &b\n  x: 1\n  y: 2\nchild:\n  <<: *b\n  y: 99\n";
    let value = parse(DataFormat::Yaml, yaml);
    assert_eq!(
        value.to_string(),
        r#"{"base":{"x":1,"y":2},"child":{"x":1,"y":99}}"#
    );
}

#[test]
fn yaml_merge_key_stream_events_rely_on_last_write_wins() {
    // The parser doesn't un-stream or re-derive events to hide a merged-in
    // leaf that a later explicit key overrides - both events are emitted,
    // in order, and it's `set_path`'s last-write-wins semantics (see
    // `Default`-mode building in `builder.rs`) that makes the final value
    // correct despite the stale intermediate leaf.
    let yaml = "base: &b\n  x: 1\n  y: 2\nchild:\n  <<: *b\n  y: 99\n";
    let events: Vec<Value> = ValueBuilder::new(
        AnyParser::new(DataFormat::Yaml, Input::new_string(yaml.to_owned())).unwrap(),
        StreamOption::Stream,
    )
    .collect::<Result<_, _>>()
    .unwrap();
    let rendered: Vec<String> = events.iter().map(|v| v.to_string()).collect();
    assert_eq!(
        rendered,
        vec![
            r#"[["base"],{"x":1,"y":2}]"#,
            r#"[["child","x"],1]"#,
            r#"[["child","y"],2]"#,
            r#"[["child","y"],99]"#,
            r#"[["child","y"]]"#,
            r#"[["child"]]"#,
        ]
    );
}
