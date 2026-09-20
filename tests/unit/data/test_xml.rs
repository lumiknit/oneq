use oneq::data::builder::StreamOption;
use oneq::data::{AnyParser, AnySerializer, DataFormat, Value, ValueBuilder};
use oneq::io::{Input, Output};
use oneq::render;
use std::{cell::RefCell, rc::Rc};

fn read(format: DataFormat, text: &str) -> Result<Vec<Value>, oneq::data::DataError> {
    ValueBuilder::new(
        AnyParser::new(format, Input::new_string(text.into()))?,
        StreamOption::default(),
    )
    .collect()
}
fn json(text: &str) -> Value {
    read(DataFormat::Json, text).unwrap().remove(0)
}
fn write(value: Value) -> Result<String, oneq::data::DataError> {
    let buf = Rc::new(RefCell::new(Vec::new()));
    AnySerializer::from_format(
        DataFormat::Xml,
        Output::new_string_buffer(buf.clone()),
        render::Options::new(Default::default()),
    )?
    .put(value)?;
    let bytes = buf.borrow().clone();
    Ok(String::from_utf8(bytes).unwrap())
}

#[test]
fn mixed_content() {
    let expected = json(
        r#"{"name":"p","attributes":{},"children":[{"text":"Hello "},{"name":"b","attributes":{},"children":[{"text":"world"}]},{"text":"!"}]}"#,
    );
    let actual = read(DataFormat::Xml, "<p>Hello <b>world</b>!</p>")
        .unwrap()
        .remove(0);
    assert_eq!(actual.to_string(), expected.to_string());
    assert_eq!(write(expected).unwrap(), "<p>Hello <b>world</b>!</p>\n");
    assert_eq!("xml".parse::<DataFormat>().unwrap(), DataFormat::Xml);
}

#[test]
fn roundtrip_text_attributes_and_names() {
    let source = "<?xml version='1.0' encoding='UTF-8'?>\n<!--c--><한글 xmlns:x='urn:x' a='&quot;&apos;&lt;&gt;&amp;&#10;&#9;&#13;' ><x:b/> a<![CDATA[<b>]]><!--ignored--><?pi test?>z&#x1F600;\r\n</한글>";
    let first = read(DataFormat::Xml, source).unwrap().remove(0);
    let output = write(first.clone()).unwrap();
    let second = read(DataFormat::Xml, &output).unwrap().remove(0);
    assert_eq!(first.to_string(), second.to_string());
    assert!(output.contains(" a&lt;b&gt;z😀\n"));
}

#[test]
fn rejects_malformed_xml() {
    for source in [
        "",
        "<a>",
        "<a></b>",
        "<a/><b/>",
        "x<a/>",
        "<a/>x",
        "<a a='1' a='2'/>",
        "<a b='1'c='2'/>",
        "<a b=x/>",
        "<a>&unknown;</a>",
        "<a>&#0;</a>",
        "<a>&#+32;</a>",
        "<a>]]></a>",
        "<a b='<x'/>",
        "<a><!--a--b--></a>",
        "<a><![CDATA[x</a>",
        "<a><?xml version='1.0'?></a>",
        "<!DOCTYPE a><a/>",
        "<a>\0</a>",
        "<?xml version='1.1'?><a/>",
        "<?xml version='1.0' encoding='latin1'?><a/>",
    ] {
        assert!(
            read(DataFormat::Xml, source).is_err(),
            "accepted {source:?}"
        );
    }
}

#[test]
fn rejects_invalid_dom_without_writing() {
    for source in [
        "null",
        "[]",
        r#"{"text":"root"}"#,
        r#"{"name":"a"}"#,
        r#"{"name":"a","attributes":{},"children":[],"extra":1}"#,
        r#"{"name":"a","attributes":{"x":1},"children":[]}"#,
        r#"{"name":"a","attributes":{},"children":["text"]}"#,
        r#"{"name":"a","attributes":{},"children":[{"text":"x","extra":0}]}"#,
        r#"{"name":"a/>","attributes":{},"children":[]}"#,
        r#"{"name":"a","attributes":{},"children":[{"text":"\u0000"}]}"#,
    ] {
        let buf = Rc::new(RefCell::new(Vec::new()));
        let mut serializer = AnySerializer::from_format(
            DataFormat::Xml,
            Output::new_string_buffer(buf.clone()),
            render::Options::new(Default::default()),
        )
        .unwrap();
        assert!(serializer.put(json(source)).is_err(), "accepted {source}");
        assert!(buf.borrow().is_empty());
    }
}

#[test]
fn nesting_limit() {
    assert!(
        read(
            DataFormat::Xml,
            &format!("{}{}", "<a>".repeat(129), "</a>".repeat(129))
        )
        .is_err()
    );
    let values = read(
        DataFormat::Xml,
        &format!("{}x{}", "<a>".repeat(128), "</a>".repeat(128)),
    )
    .unwrap();
    assert!(write(values[0].clone()).is_ok());
}
