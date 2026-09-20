//! Each format must reject genuinely malformed input (bad syntax), not just
//! "unusual" input (jq itself is lenient about things like duplicate keys,
//! so those aren't tested here).

use oneq::data::builder::StreamOption;
use oneq::data::{AnyParser, DataFormat, ValueBuilder};
use oneq::io::Input;

fn parse(
    format: DataFormat,
    content: &str,
) -> Result<Vec<oneq::data::Value>, oneq::data::DataError> {
    ValueBuilder::new(
        AnyParser::new(format, Input::new_string(content.to_owned())).unwrap(),
        StreamOption::default(),
    )
    .collect()
}

fn assert_invalid(format: DataFormat, content: &str) {
    let result = parse(format, content);
    assert!(
        result.is_err(),
        "expected {format} to reject {content:?}, but it parsed as {:?}",
        result.ok()
    );
}

#[test]
fn json_rejects_malformed_input() {
    for bad in [
        "{",
        "{\"a\": }",
        "{\"a\" 1}",
        "[1, 2,]",
        "{\"a\": 1,}",
        "{a: 1}",
        "\"unterminated",
        "nul",
        "{\"a\": 1} extra",
    ] {
        assert_invalid(DataFormat::Json, bad);
    }
}

#[test]
fn yaml_rejects_malformed_input() {
    for bad in [
        "a:\n    b: 1\n  c: 2\n", // inconsistent indentation
        "[1, 2",                  // unterminated flow sequence
        "{a: 1",                  // unterminated flow mapping
        "a: *undefined\n",        // unknown alias
        "<<: *undefined\na: 1\n",
    ] {
        assert_invalid(DataFormat::Yaml, bad);
    }
}

#[test]
fn toml_rejects_malformed_input() {
    for bad in [
        "a = \n",
        "a = \"unterminated\n",
        "[a\nb = 1\n",
        "a = 1 2\n",
        "= 1\n",
        "a = [1, 2\n",
        "a = {b = 1\n",
    ] {
        assert_invalid(DataFormat::Toml, bad);
    }
}

#[test]
fn xml_rejects_malformed_input() {
    for bad in [
        "<a><b></a></b>",
        "<a>",
        "<a></b>",
        "not xml at all <",
        "<a attr=unquoted>x</a>",
    ] {
        assert_invalid(DataFormat::Xml, bad);
    }
}

#[test]
fn sv_rejects_malformed_input() {
    // CSV/TSV are intentionally very permissive (no quoting errors reject
    // input, per sv.rs's documented relaxed dialect) - there is nothing
    // meaningfully "invalid" to test here beyond an unreadable input, so
    // this only documents that fact rather than asserting failures.
}
