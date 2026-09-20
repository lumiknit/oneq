pub mod pairs;
pub use pairs as pair;
use pest::Parser;
use pest_derive::Parser;
pub mod printer;

#[derive(Parser)]
#[grammar = "jq/parser/jq.pest"]
pub struct JqParser;

/// Whether `src` is a syntactically incomplete jq program - i.e. it
/// fails to parse only because it ends too early (an unclosed
/// paren/bracket/brace, a trailing "|", an unterminated string, ...).
///
/// pest reports a parse failure's location as the input length itself
/// whenever the parser ran out of input while still expecting more
/// tokens, and strictly before it for every other kind of error (an
/// unexpected token appearing somewhere in the middle). REPLs use this
/// to decide whether to keep reading more lines instead of reporting
/// a syntax error.
pub fn is_incomplete(src: &str) -> bool {
    match JqParser::parse(Rule::program, src) {
        Ok(_) => false,
        Err(e) => {
            let end = match e.location {
                pest::error::InputLocation::Pos(p) => p,
                pest::error::InputLocation::Span((_, end)) => end,
            };
            end >= src.len()
        }
    }
}

/// Parse a source file while retaining its original text and offset-based locations.
pub fn parse_pairs(
    path: impl Into<String>,
    source: &str,
) -> Result<(pair::FileSet, pair::Pair), String> {
    let mut fs = pair::FileSet::new();
    let own = parse_pairs_in(&mut fs, path, source)?;
    Ok((fs, own))
}

/// Parse into a shared source map. Every returned span belongs to `files`.
pub fn parse_pairs_in(
    files: &mut pair::FileSet,
    path: impl Into<String>,
    source: &str,
) -> Result<pair::Pair, String> {
    let root = JqParser::parse(Rule::program, source)
        .map_err(|e| e.to_string())?
        .next()
        .unwrap();
    let file = files.add(path, source.to_owned());
    let own = pair::Pair::from_pest_in(root, files, file).normalize(files);
    own.validate(files)?;
    Ok(own)
}
