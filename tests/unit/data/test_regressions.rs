use oneq::data::{PathItem, parse_json_str};

#[test]
fn raw_slurp_reports_read_errors_once() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), [0xff]).unwrap();
    let input = oneq::io::Input::new_files(&[file.path().to_str().unwrap()]).unwrap();
    let mut parser = oneq::data::formats::raw::RawSlurpParser::new(input);
    assert!(parser.next().unwrap().is_err());
    assert!(parser.next().is_none());
}

#[test]
fn negative_paths_are_checked_without_unsigned_overflow() {
    let value = parse_json_str("[10,20]").unwrap();
    for (index, expected) in [(-1, "20"), (-2, "10")] {
        assert_eq!(
            value.get_path(&[PathItem::new_idx(index)]).unwrap(),
            &parse_json_str(expected).unwrap()
        );
    }
    assert!(value.get_path(&[PathItem::new_idx(-3)]).is_err());
    assert!(
        parse_json_str("[]")
            .unwrap()
            .get_path(&[PathItem::new_idx(-1)])
            .is_err()
    );
}
#[test]
fn integer_literal_conversion_matches_float_parser_and_shares_digits() {
    use oneq::data::{Value, core::decimal::Decimal};
    use std::rc::Rc;

    for literal in [
        "0",
        "-0",
        "1",
        "-1",
        "9007199254740991",
        "9007199254740992",
        "9007199254740993",
        "9223372036854775807",
        "-9223372036854775808",
        "9999999999999999999",
        "10000000000000000000",
        "1.50",
        "-0.00",
        "1e300",
        "1e-300",
    ] {
        let decimal = Decimal::parse(literal).unwrap();
        assert_eq!(
            decimal.to_f64().to_bits(),
            literal.parse::<f64>().unwrap().to_bits(),
            "{literal}"
        );
        let value = Value::decimal(decimal);
        let copy = value.clone();
        let (Value::Decimal(a), Value::Decimal(b)) = (&value, &copy) else {
            unreachable!()
        };
        assert!(Rc::ptr_eq(a, b));
    }
    assert_ne!(
        Value::decimal(Decimal::parse("9007199254740992").unwrap()),
        Value::decimal(Decimal::parse("9007199254740993").unwrap())
    );
}
