use oneq::jq::parser::{
    pair::{FileSet, Pair, PairTag, pairs_to_value},
    parse_pairs,
};

fn assert_content_contract(pair: &Pair, files: &oneq::jq::parser::pair::FileSet) {
    pair.validate(files).unwrap();
}

#[test]
fn test_parser_pairs_match_fixtures_and_are_idempotent() {
    for source_path in std::fs::read_dir("tests/fixtures/parser").unwrap() {
        let source_path = source_path.unwrap().path();
        if source_path.extension().and_then(|e| e.to_str()) != Some("jq") {
            continue;
        }
        let source = std::fs::read_to_string(&source_path).unwrap();
        let expected_path = source_path.with_extension("pairs.json");
        let expected = std::fs::read_to_string(&expected_path)
            .unwrap_or_else(|_| panic!("missing {}", expected_path.display()));
        let (files, root) = parse_pairs(source_path.display().to_string(), &source).unwrap();
        assert_content_contract(&root, &files);
        let actual = pairs_to_value(std::slice::from_ref(&root), &files);
        // JSON object field order is not part of the Pair contract.
        let expected = oneq::data::parse_json_str(&expected).unwrap();
        assert_eq!(
            actual,
            expected,
            "{}\nactual canonical JSON:\n{}",
            source_path.display(),
            actual
        );

        let (files_again, root_again) =
            parse_pairs(source_path.display().to_string(), &source).unwrap();
        assert_eq!(
            actual,
            pairs_to_value(&[root_again], &files_again),
            "{} is not idempotent",
            source_path.display()
        );
    }
}

#[test]
fn invoke_normalization_preserves_real_applications_and_comments() {
    let files = FileSet::new();
    let leaf = || Pair::new(PairTag::Invoke, "f", vec![]);
    for op in ["|", ","] {
        let p = Pair::new(PairTag::Invoke, op, vec![leaf()]);
        assert_eq!(p.normalize(&files), leaf());
    }
    for source in ["f(.)", "-.a", ".a?", "a | (b | c)", "a # keep\n | b"] {
        let (files, root) = parse_pairs("test", source).unwrap();
        assert_eq!(root, root.clone().normalize(&files));
        if source.contains("keep") {
            assert!(root.to_value(&files).to_string().contains(" keep"));
        }
    }
    let (_, root) = parse_pairs("test", "a | (b | c)").unwrap();
    assert_eq!(root.children[0].children.len(), 3);
}
