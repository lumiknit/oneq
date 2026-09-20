//! Fixture-driven checks for `tests/unit/data/formats_valid_fixtures/*`.
//!
//! Every file is named `<case>.<style>.<ext>`. All files sharing a `case`
//! must parse to the same value, regardless of format or style. Files with
//! `style` `sorted`/`unsorted` additionally must serialize (with
//! `sort_keys` matching the style) to exactly the case's own `.jsonl`
//! fixture of that style. Other styles (`1`, `2`, `3`, ...) exist to cover
//! uglier/alternative syntax (yaml anchors, toml inline tables, json5
//! quirks, ...) and are only checked for parsing, not serialization.

use oneq::data::builder::StreamOption;
use oneq::data::{AnyParser, AnySerializer, DataFormat, Value, ValueBuilder};
use oneq::io::{Input, Output};
use oneq::render::{self, CompactLevel};
use std::{cell::RefCell, collections::BTreeMap, fs, path::PathBuf, rc::Rc};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/data/formats_valid_fixtures"
    ))
}

fn ext_to_format(ext: &str) -> DataFormat {
    match ext {
        "jsonl" => DataFormat::Json,
        "json5" => DataFormat::Json5,
        "yaml" => DataFormat::Yaml,
        "toml" => DataFormat::Toml,
        "raw" => DataFormat::Raw,
        other => panic!("fixtures: unknown extension {other:?}"),
    }
}

struct Fixture {
    case: String,
    style: String,
    ext: String,
    path: PathBuf,
}

fn all_fixtures() -> Vec<Fixture> {
    let mut out = Vec::new();
    for entry in fs::read_dir(fixtures_dir()).unwrap() {
        let path = entry.unwrap().path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name().unwrap().to_str().unwrap();
        let parts: Vec<&str> = name.split('.').collect();
        assert_eq!(
            parts.len(),
            3,
            "fixture name must be <case>.<style>.<ext>, got {name:?}"
        );
        out.push(Fixture {
            case: parts[0].to_owned(),
            style: parts[1].to_owned(),
            ext: parts[2].to_owned(),
            path,
        });
    }
    out
}

fn group_by_case(fixtures: &[Fixture]) -> BTreeMap<&str, Vec<&Fixture>> {
    let mut map: BTreeMap<&str, Vec<&Fixture>> = BTreeMap::new();
    for f in fixtures {
        map.entry(f.case.as_str()).or_default().push(f);
    }
    map
}

fn parse_fixture(f: &Fixture) -> Vec<Value> {
    let content =
        fs::read_to_string(&f.path).unwrap_or_else(|e| panic!("read {}: {e}", f.path.display()));
    let format = ext_to_format(&f.ext);
    ValueBuilder::new(
        AnyParser::new(format, Input::new_string(content)).unwrap(),
        StreamOption::default(),
    )
    .collect::<Result<Vec<_>, _>>()
    .unwrap_or_else(|e| panic!("parse {} ({}): {e:?}", f.path.display(), format))
}

/// Order-independent (object keys are always sorted by `Display`) textual
/// form of a parsed document stream, used to check semantic equality.
fn canonical(values: &[Value]) -> String {
    values
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

fn serialize_jsonl(values: Vec<Value>, sorted: bool) -> String {
    let buf = Rc::new(RefCell::new(Vec::new()));
    let options = render::Options::new(render::FormatOptions {
        compact_level: CompactLevel::Compact,
        sort_keys: sorted,
        indent: "  ",
        ..Default::default()
    });
    let mut serializer = AnySerializer::from_format(
        DataFormat::Json,
        Output::new_string_buffer(buf.clone()),
        options,
    )
    .unwrap();
    for value in values {
        serializer.put(value).unwrap();
    }
    let bytes = buf.borrow().clone();
    String::from_utf8(bytes).unwrap()
}

#[test]
fn all_variants_of_a_case_parse_to_the_same_value() {
    let fixtures = all_fixtures();
    for (case, group) in group_by_case(&fixtures) {
        let mut reference: Option<(&str, PathBuf)> = None;
        for f in &group {
            let values = parse_fixture(f);
            let text = canonical(&values);
            match &reference {
                None => {
                    // Leak is fine: test binary, tiny fixed set of strings.
                    reference = Some((Box::leak(text.into_boxed_str()), f.path.clone()));
                }
                Some((expected, source)) => {
                    assert_eq!(
                        &text,
                        expected,
                        "case {case:?}: {} parses differently than {}",
                        f.path.display(),
                        source.display()
                    );
                }
            }
        }
    }
}

#[test]
fn sorted_and_unsorted_variants_serialize_to_matching_jsonl() {
    let fixtures = all_fixtures();
    for (case, group) in group_by_case(&fixtures) {
        for (style, sorted) in [("sorted", true), ("unsorted", false)] {
            let Some(jsonl) = group.iter().find(|f| f.style == style && f.ext == "jsonl") else {
                continue;
            };
            let expected = fs::read_to_string(&jsonl.path).unwrap();
            for f in group.iter().filter(|f| f.style == style) {
                let values = parse_fixture(f);
                let actual = serialize_jsonl(values, sorted);
                assert_eq!(
                    actual,
                    expected,
                    "case {case:?} ({style}): {} does not serialize to match {}",
                    f.path.display(),
                    jsonl.path.display()
                );
            }
        }
    }
}
