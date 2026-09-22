//! Serializable syntax contracts, independent of Pest's precedence wrappers.
//!
//! A node's tag, content, and semantic children determine its meaning without
//! consulting its external source span. Content belongs to the node itself:
//! Invoke stores its callee/operator, and named declarations store their name.
//! Comments are leaf trivia interleaved with children, never semantic operands.
//! All slot counts and indexes below exclude Comment and `TrailingComment`.
//!
//! Normalization is operator-specific: singleton pipe/comma containers may be
//! collapsed and adjacent pipes/commas may be flattened where scope and evaluation
//! order are preserved. Never erase an Invoke merely because it has zero or one
//! arguments: calls, unary minus, optional, and try retain their operation.
//! Parentheses may be rebuilt from structure, precedence, and associativity.
//! Optional boundaries must survive normalization and path coalescing.
//!
//! Pest conversion, AST lowering, JSON serialization, and printing share these
//! contracts. JSON contains content text and never requires the original `FileSet`.
use super::Rule;
use crate::{data::Value, strs};
use pest::iterators::Pair as PestPair;
use strum::Display;

/*
 * PairTag notation
 * Tag(CONTENT)[CHILDREN...] allows content and children independently.
 * Uppercase names denote semantic slots, not literal source tokens.
 * SLOT: A | B lists permitted kinds; ? means an omitted optional slot.
 * Backticks abbreviate a parsed jq expression, e.g. Invoke("+")[`1`, `2`].
 * A bare field name is a string key, not a call: `.a` is Path[`"a"`].
 * Ident and Special are not separate tags. Bare filter names and operators
 * use Invoke content; identifier-shaped declarations use their owning node's
 * content. Filter parameters use Invoke(NAME)[] in parameter context.
 *
 * Pattern = Var(NAME) | Array[PATTERN...] | Object[(KEY: Expr, PATTERN)...].
 * In pattern context Array children are positional patterns, not generators.
 * Object keys remain expressions; shorthand {$x} expands to [`"x"`, Var("x")].
 * In Bind, patterns are stored as sequential children after TARGET and BODY;
 * a plain variable is an ordinary Var child just like a structured pattern.
 * Reduce and ForEach store accumulator slots first and append all patterns sequentially.
 *
 * Trivia policy: comments are childless nodes in source order, attached to the
 * smallest container that owns their syntactic gap. They never wrap expressions.
 * Leading/trailing and internal comments (including comments in empty containers)
 * remain children even when no semantic child exists. A trailing comment follows
 * source on the same line; the printer must terminate it before emitting code.
 * Printers consume trivia separately from semantic slots and may canonicalize
 * spacing or punctuation placement while preserving comment order and meaning.
 * AST lowering skips trivia; JSON preserves its content and child ordering.
 */

#[derive(Debug, Display, Clone, PartialEq, Eq)]
pub enum PairTag {
    /// `Comment(TEXT_WITHOUT_INITIAL_HASH)`[]; no wrapped expression.
    /// Preserve the remaining spelling, including backslash-newline continuation.
    /// The printer restores the initial '#' and a terminating newline as needed.
    Comment,

    /// `TrailingComment(TEXT_WITHOUT_INITIAL_HASH)`[]; code precedes it on its line.
    /// Same content and childless contract as Comment.
    TrailingComment,

    /// Var(NAME)[]; `$test` => Var("test"). Also a variable declaration in patterns
    /// and a value parameter in Def. The '$' prefix is reconstructed, not content.
    Var,

    /// Loc[FILE: `StringChunk`, LINE: Int]; `$__loc__` resolved at its original site.
    /// FILE contains the filename encoded as a jq string chunk; LINE is one-based.
    /// Both children survive JSON export even when all external spans are omitted.
    /// Lower to {"file": FILE, "line": LINE}. To preserve semantics after moving or
    /// formatting source, print this explicit object rather than a relocated $__loc__.
    Loc,

    /// `Int(NUM_REPR)`[]; preserve the original numeric spelling.
    Int,

    /// `Float(NUM_REPR)`[]; preserve the original numeric spelling.
    Float,

    /// StringChunk(CONTENT)[]; preserve raw string spelling, including escapes.
    /// Normally occurs inside String; Loc also uses it for its stored filename.
    StringChunk,

    /// String[ITEM...: `StringChunk` | Expr]; semantic non-StringChunk children are
    /// interpolation expressions. Trivia is never an interpolation by itself.
    /// `"hello \(.name)"` => String[StringChunk("hello "), Path[`"name"`]].
    /// Comments inside an interpolation belong to that expression's container;
    /// printers must not move interpolation comments into literal string text.
    String,

    /// Array[Expr...]; collect and concatenate every child's output stream.
    /// Each child may emit zero, one, or many elements; [empty] is an empty array.
    /// The printer joins expression children with commas, adding parentheses as
    /// needed to preserve their scope. `[1, 2, 3]` => Array[Int(1), Int(2), Int(3)].
    /// In pattern context children are positional Pattern slots instead.
    Array,

    /// Object[(KEY: Expr, VALUE: Expr)...]; even semantic indexes are keys.
    /// Parsing retains computed key expressions without requiring a string type;
    /// compilation/evaluation validates keys. Computed keys need parentheses in jq.
    /// `{(42): "hello"}` => Object[Int(42), `"hello"`] (invalid key type later).
    /// Expand shorthand into explicit key/value pairs: {foo} => [`"foo"`, `.foo`],
    /// {$foo} => [`"foo"`, Var("foo")]. The formatter may shorten equivalent pairs.
    /// In pattern context values are Patterns, and {$foo} declares Var("foo").
    Object,

    /// Slice[FROM: Expr, TO?: Expr]; only a Path component.
    /// An omitted right bound removes its child. An omitted left bound inserts
    /// a synthetic `null`: [:3] => Slice[Invoke("null"), Int(3)], [4:] =>
    /// Slice[Int(4)]. `null` is what jq itself substitutes, so [:3] and
    /// [null:3] canonicalize together - but *not* [0:3], which jq reports
    /// differently when the base is unsliceable ("start":0 vs "start":null).
    /// Bounds may be variables or arbitrary expressions, including unary minus.
    Slice,

    /// Spread[]; only a Path component, for iteration with '.[]'.
    Spread,

    /// Path[COMPONENT...: Slice | Spread | Expr]; empty means identity '.'.
    /// Expr components are index/key expressions, not sequential pipe stages.
    /// `.a[1:2].b` => Path[`"a"`, Slice[Int(1), Int(2)], `"b"`].
    /// Coalesce uninterrupted path components as far as possible. Do not cross
    /// optional boundaries or turn `.a[.b]` into `.a | .[.b]`: index expressions
    /// must retain jq's original-input evaluation semantics.
    /// Use Invoke(".")[BASE, SUFFIX] for separated access, e.g.
    /// `$a.b.c.d` => Invoke(".")[Var("a"), Path[`"b"`, `"c"`, `"d"`]].
    /// Optional scope is structural: Invoke("?")[`.a.b`] differs from
    /// Invoke(".")[`.a`, Invoke("?")[`.b`]]. Parenthesized/call bases may also
    /// require separated access. '..' is Invoke("..")[], not a Path component.
    Path,

    /// Label(NAME)[BODY: Expr]; `label $x | body` => Label("x")[BODY].
    Label,

    /// Break(NAME)[]; `break $x` => Break("x").
    /// `break $x | body` uses Invoke("|")[Break("x"), BODY].
    Break,

    /// Invoke(NAME)[ARG...: Expr]; NAME is the complete callee or operator spelling.
    /// No Ident, Special, Format, or `ModuleAccess` child is needed.
    /// `true` => Invoke("true")[]; `m::f(1)` => `Invoke("m::f`")[`1`].
    /// `@base64d` => Invoke("@base64d")[]; `@uri "hi \(.x)"` =>
    /// Invoke("@uri")[String[StringChunk("hi "), `.x`]]. With a String argument,
    /// format only interpolated values; this is not a call on the whole string.
    /// `..` => Invoke("..")[]; `fibo(3; 4)` => Invoke("fibo")[`3`, `4`].
    /// `42 | .a | map` => Invoke("|")[`42`, `.a`, Invoke("map")[]].
    /// `3, . + 4` => Invoke(",")[`3`, Invoke("+")[`.`, `4`]].
    /// `. + 2 * 4` => Invoke("+")[`.`, Invoke("*")[`2`, `4`]].
    /// `try (3, 4) catch .` => Invoke("try")[Invoke(",")[`3`, `4`], `.`].
    /// `(. | .a)?` => Invoke("?")[Invoke("|")[`.`, `.a`]].
    /// `-a` => Invoke("-")[Invoke("a")[]]. Unary/binary '-' differ by arity.
    /// Validate operator arity; singleton removal is permitted only for explicitly
    /// specified neutral containers such as pipe/comma, not arbitrary operators.
    Invoke,

    /// Assign(OP)[LHS: Expr, RHS: Expr]; OP is =, |=, +=, -=, *=, /=, %=, or //=.
    /// LHS must select paths in the input at evaluation time; it is not a variable
    /// destructuring Pattern and is not restricted to the `PairTag::Path` shape.
    /// `(.a, .b) = 0` => Assign("=")[Invoke(",")[`.a`, `.b`], `0`].
    /// '=' evaluates RHS on the original input; '|=' evaluates it on selected values.
    /// Reference: <https://jqlang.org/manual/v1.8/#assignment>
    Assign,

    /// Bind[TARGET: Expr, BODY: Expr, PATTERNS: Pattern...]; content is always None.
    /// A plain variable is an ordinary Var child; ?// alternatives follow sequentially. Never store `as`.
    Bind,

    /// Reduce and `ForEach` append alternative patterns directly; there is no `PatternAlt` node.

    /// Reduce[EXPR, INIT, UPDATE, PATTERN...]; content is always None.
    /// `ForEach`[EXPR, INIT, UPDATE, EXTRACT, PATTERN...]; EXTRACT is always present.
    /// An omitted source EXTRACT becomes identity Path[]. Patterns start at index 4.
    /// `?//` alternatives are appended in source order after the accumulator slots.
    Reduce,

    ForEach,
    /// If[(COND: Expr, THEN: Expr)..., ELSE?: Expr]; at least one branch.
    /// Semantic children are [COND0, THEN0, COND1, THEN1, ..., ELSE?].
    /// Odd semantic child count means an else branch; never inspect source text.
    If,

    /// Empty[]; only Def's NEXT slot, marking a definition-only continuation.
    /// At file scope this ends a module's definition chain, not identity execution.
    Empty,

    /// Def(NAME)[PARAMS...: Invoke | Var, BODY: Expr, NEXT: Expr | Def | Empty].
    /// Last two semantic children are always BODY and NEXT; preceding ones are params.
    /// Invoke("f")[] declares a filter parameter; Var("x") declares a value parameter.
    /// `def f: 1; f` => Def("f")[`1`, Invoke("f")[]].
    Def,

    /// Module[METADATA: Expr]; `module Metadata;`. No redundant keyword content.
    Module,

    /// Include[PATH: String, METADATA?: Expr]; `include "path" Metadata?;`.
    /// Keep metadata structural; the path must be a constant string.
    Include,

    /// Import(ALIAS)[PATH: String, METADATA?: Expr]; `import "path" as alias Metadata?;`.
    /// Content stores the declared alias, including '$' for a data import so that
    /// `as m` and `as $m` remain distinct. Path is a constant string; metadata a child.
    Import,

    /// Root[DECLARATIONS..., EXECUTION?: Expr]; execution can only occur last.
    /// A Def chain may own the final execution through NEXT; do not duplicate it.
    /// Formatting canonicalizes declaration placement before execution, preserving
    /// scope, relative definition order, and associated comments. Declarations emit
    /// their required semicolons; never join arbitrary expressions using ';'.
    Root,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanPos {
    pub file: usize,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentSpan {
    Pos(SpanPos),
    Str(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    /// The semantic tag of this pair.
    pub tag: PairTag,

    /// Children in source order, including leaf trivia. Semantic slot access
    /// excludes Comment and `TrailingComment`; JSON retains their ordering.
    pub children: Vec<Self>,

    /// The external span of this pair. The start and end are byte offsets
    /// which wraps the source text of this pair. This information may be omitted
    /// when passed to external (e.g. JSON) representations.
    pub span: Option<SpanPos>,

    /// The node's own spelling, when tag and children cannot reconstruct it:
    /// callee/operator, declared name, variable name, literal spelling, or comment.
    /// May coexist with children (Invoke, Def, named Bind, etc.). Presence and
    /// interpretation follow the tag contract, not whether the node is a leaf.
    /// Keywords implied by the tag need not be stored. JSON must materialize Pos
    /// into owned content text; content cannot be omitted with the external span.
    pub content_span: Option<ContentSpan>,
}
pub type Pairs = Vec<Pair>;
/// Precedence used when rendering or lowering operator pairs. Larger binds tighter.
#[must_use]
pub fn operator_precedence(op: &str) -> u8 {
    match op {
        "|" => 1,
        "," => 2,
        "//" => 3,
        "=" | "|=" | "+=" | "-=" | "*=" | "/=" | "%=" | "//=" => 4,
        "or" => 5,
        "and" => 6,
        "==" | "!=" | "<" | ">" | "<=" | ">=" => 7,
        "+" | "-" => 8,
        "*" | "/" | "%" => 9,
        _ => 10,
    }
}
impl Pair {
    pub fn new(tag: PairTag, content: impl Into<String>, children: Vec<Self>) -> Self {
        Self {
            tag,
            children,
            span: None,
            content_span: Some(ContentSpan::Str(content.into())),
        }
    }
    #[must_use]
    pub const fn node(tag: PairTag, children: Vec<Self>) -> Self {
        Self {
            tag,
            children,
            span: None,
            content_span: None,
        }
    }
    #[must_use]
    pub const fn is_comment(&self) -> bool {
        matches!(self.tag, PairTag::Comment | PairTag::TrailingComment)
    }
    /// Scopes a postfix `?` to just the last `n` path components of `base`,
    /// leaving any earlier coalesced components unprotected: `.a.b?` must
    /// still raise if `.a` errors, matching jq's per-step optional semantics.
    fn wrap_optional(mut base: Self, n: usize) -> Self {
        if n == 0 {
            return Self::new(PairTag::Invoke, "?", vec![base]);
        }
        if base.tag == PairTag::Path {
            let split_at = base.children.len().saturating_sub(n);
            let tail = base.children.split_off(split_at);
            let optional = Self::new(PairTag::Invoke, "?", vec![Self::node(PairTag::Path, tail)]);
            if base.children.is_empty() {
                optional
            } else {
                Self::new(PairTag::Invoke, ".", vec![base, optional])
            }
        } else {
            let tail_path = base.children.last_mut().expect("path suffix tail");
            let split_at = tail_path.children.len().saturating_sub(n);
            let tail = tail_path.children.split_off(split_at);
            let optional = Self::new(PairTag::Invoke, "?", vec![Self::node(PairTag::Path, tail)]);
            if tail_path.children.is_empty() {
                base.children.pop();
            }
            // Invoke(".") always takes exactly two operands; collapse a
            // single leftover prefix instead of nesting a degenerate one.
            let prefix = if base.children.len() == 1 {
                base.children.pop().unwrap()
            } else {
                base
            };
            Self::new(PairTag::Invoke, ".", vec![prefix, optional])
        }
    }
    #[must_use]
    pub fn semantic_children(&self) -> impl DoubleEndedIterator<Item = &Self> {
        self.children.iter().filter(|p| !p.is_comment())
    }
    #[must_use]
    pub fn text<'a>(&'a self, files: &'a FileSet) -> Option<&'a str> {
        match self.content_span.as_ref()? {
            ContentSpan::Str(s) => Some(s),
            ContentSpan::Pos(s) => files.text(s),
        }
    }
    #[must_use]
    pub fn source<'a>(&self, files: &'a FileSet) -> Option<&'a str> {
        files.text(self.span.as_ref()?)
    }
    #[must_use]
    pub fn string(s: &str) -> Self {
        let mut escaped = String::new();
        crate::data::escape::escape_string_json(s, '"', &mut escaped).unwrap();
        Self::node(
            PairTag::String,
            vec![Self::new(PairTag::StringChunk, escaped, vec![])],
        )
    }
    #[must_use]
    pub fn normalize(mut self, files: &FileSet) -> Self {
        self.children = self
            .children
            .into_iter()
            .map(|p| p.normalize(files))
            .collect();
        if self.tag == PairTag::Root {
            // Keep comments attached while ordering JSON-supplied declarations.
            let mut groups: Vec<Vec<Self>> = vec![];
            let mut leading = vec![];
            for child in self.children {
                if child.is_comment() {
                    if child.tag == PairTag::TrailingComment && !groups.is_empty() {
                        groups.last_mut().unwrap().push(child);
                    } else {
                        leading.push(child);
                    }
                } else {
                    leading.push(child);
                    groups.push(std::mem::take(&mut leading));
                }
            }
            groups.sort_by_key(
                |group| match group.iter().find(|p| !p.is_comment()).unwrap().tag {
                    PairTag::Module => 0,
                    PairTag::Include | PairTag::Import => 1,
                    PairTag::Def => 2,
                    _ => 3,
                },
            );
            self.children = groups.into_iter().flatten().chain(leading).collect();
        }
        if self.tag == PairTag::Invoke && matches!(self.text(files), Some("|" | ",")) {
            let op = self.text(files).unwrap().to_owned();
            let mut flat = vec![];
            for child in self.children {
                if child.tag == PairTag::Invoke && child.text(files) == Some(op.as_str()) {
                    flat.extend(child.children);
                } else {
                    flat.push(child);
                }
            }
            self.children = flat;
            if self.children.len() == 1 {
                return self.children.remove(0);
            }
        }
        self
    }
    pub fn from_pest_in(p: PestPair<'_, Rule>, files: &mut FileSet, file: usize) -> Self {
        // Lower children before their parent without recursively entering the
        // large rule-conversion function. In debug builds its frame multiplied
        // by Pest's precedence wrappers can exhaust the Windows main stack even
        // while parsing the built-in library for the identity filter.
        let mut pending = vec![(p, false)];
        let mut converted = Vec::new();
        while let Some((p, visited)) = pending.pop() {
            if visited {
                let count = p
                    .clone()
                    .into_inner()
                    .filter(|p| p.as_rule() != Rule::EOI)
                    .count();
                let parts = converted.split_off(converted.len() - count);
                let rule = p.as_rule();
                converted.push((rule, Self::from_pest_parts(p, parts, files, file)));
            } else {
                pending.push((p.clone(), true));
                for child in p.into_inner().rev().filter(|p| p.as_rule() != Rule::EOI) {
                    pending.push((child, false));
                }
            }
        }
        converted.pop().expect("Pest root").1
    }

    fn from_pest_parts(
        p: PestPair<'_, Rule>,
        mut parts: Vec<(Rule, Self)>,
        files: &FileSet,
        file: usize,
    ) -> Self {
        use PairTag as T;
        let rule = p.as_rule();
        let span = p.as_span();
        let pos = SpanPos {
            file,
            start: files.files[file].start + span.start(),
            end: files.files[file].start + span.end(),
        };
        let raw = span.as_str();
        // Keywords are syntax, not operands. Operator rules are consumed below.
        parts.retain(|(r, _)| {
            !matches!(
                r,
                Rule::as_kw
                    | Rule::def_kw
                    | Rule::if_kw
                    | Rule::then_kw
                    | Rule::elif_kw
                    | Rule::else_kw
                    | Rule::end_kw
                    | Rule::reduce_kw
                    | Rule::foreach_kw
                    | Rule::try_kw
                    | Rule::catch_kw
                    | Rule::break_kw
                    | Rule::label_kw
                    | Rule::module_kw
                    | Rule::include_kw
                    | Rule::import_kw
            )
        });
        let rules: Vec<Rule> = parts.iter().map(|(r, _)| *r).collect();
        let mut xs: Vec<Self> = parts.into_iter().map(|(_, p)| p).collect();
        let leaf = |tag, skip| Self {
            tag,
            children: vec![],
            span: Some(pos.clone()),
            content_span: Some(ContentSpan::Pos(SpanPos {
                start: pos.start + skip,
                ..pos.clone()
            })),
        };
        let take = |xs: &mut Vec<Self>| {
            let i = xs
                .iter()
                .position(|p| !p.is_comment())
                .expect("grammar semantic child");
            xs.remove(i)
        };
        let mut result = match rule {
            Rule::COMMENT => {
                let trailing = !span.get_input()[..span.start()]
                    .rsplit('\n')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .is_empty();
                leaf(
                    if trailing {
                        T::TrailingComment
                    } else {
                        T::Comment
                    },
                    1,
                )
            }
            Rule::number => leaf(
                if raw.contains(['.', 'e', 'E']) {
                    T::Float
                } else {
                    T::Int
                },
                0,
            ),
            Rule::ident
            | Rule::format
            | Rule::optional
            | Rule::recurse
            | Rule::assign_op
            | Rule::compare_op
            | Rule::add_op
            | Rule::mul_op
            | Rule::and_kw
            | Rule::or_kw
            | Rule::as_kw
            | Rule::def_kw
            | Rule::if_kw
            | Rule::then_kw
            | Rule::elif_kw
            | Rule::else_kw
            | Rule::end_kw
            | Rule::reduce_kw
            | Rule::foreach_kw
            | Rule::try_kw
            | Rule::catch_kw
            | Rule::break_kw
            | Rule::label_kw
            | Rule::module_kw
            | Rule::include_kw
            | Rule::import_kw => leaf(T::Invoke, 0),
            Rule::var if raw == "$__loc__" => {
                let filename = Self::string(&files.files[file].path).children.remove(0);
                let line = span.get_input()[..span.start()]
                    .bytes()
                    .filter(|b| *b == b'\n')
                    .count()
                    + 1;
                Self::node(
                    T::Loc,
                    vec![filename, Self::new(T::Int, line.to_string(), vec![])],
                )
            }
            // jq's own source (e.g. builtin.jq) sometimes stutters the sigil
            // (`$$$$v`); treat any run of leading `$`s as a single one.
            Rule::var => {
                let skip = raw.bytes().take_while(|&b| b == b'$').count().max(1);
                leaf(T::Var, skip)
            }
            Rule::string_char => leaf(T::StringChunk, 0),
            Rule::identity => Self::node(T::Path, vec![]),
            Rule::string => {
                let mut chunks: Vec<Self> = vec![];
                for x in xs {
                    if x.tag == T::StringChunk
                        && chunks.last().is_some_and(|p| p.tag == T::StringChunk)
                    {
                        let prev = chunks.last_mut().unwrap();
                        if let (Some(ContentSpan::Pos(a)), Some(ContentSpan::Pos(b))) =
                            (&mut prev.content_span, &x.content_span)
                        {
                            a.end = b.end;
                        }
                        if let (Some(a), Some(b)) = (&mut prev.span, x.span) {
                            a.end = b.end;
                        }
                    } else {
                        chunks.push(x);
                    }
                }
                Self::node(T::String, chunks)
            }
            Rule::leading_field | Rule::field => {
                let name = take(&mut xs);
                xs.insert(0, Self::string(name.text(files).unwrap()));
                Self::node(T::Path, xs)
            }
            Rule::leading_quoted_field | Rule::quoted_field => Self::node(T::Path, xs),
            Rule::bracket_op => {
                let semantic: Vec<_> = xs.iter().filter(|p| !p.is_comment()).collect();
                if semantic.is_empty() {
                    Self::node(T::Spread, xs)
                } else if semantic[0].tag == T::Slice {
                    unwrap(xs)
                } else {
                    Self::node(T::Path, xs)
                }
            }
            Rule::leading_bracket => {
                let mut x = take(&mut xs);
                if x.tag == T::Path {
                    x.children.extend(xs);
                    x
                } else {
                    xs.insert(0, x);
                    Self::node(T::Path, xs)
                }
            }
            Rule::slice => {
                if !rules.contains(&Rule::slice_from) {
                    xs.insert(0, Self::new(T::Invoke, "null", vec![]));
                }
                Self::node(T::Slice, xs)
            }
            Rule::postfix => {
                let mut base = take(&mut xs);
                // How many trailing Path components in `base` belong to the most
                // recently appended suffix, so a following `?` can scope itself to
                // just that suffix instead of every coalesced component before it.
                let mut last_suffix_len: usize = 0;
                for suffix in xs {
                    if suffix.is_comment() {
                        base.children.push(suffix);
                        continue;
                    }
                    if suffix.tag == T::Invoke && suffix.text(files) == Some("?") {
                        base = Self::wrap_optional(base, last_suffix_len);
                        last_suffix_len = 0;
                        continue;
                    }
                    let suffix = if matches!(suffix.tag, T::Slice | T::Spread) {
                        Self::node(T::Path, vec![suffix])
                    } else {
                        suffix
                    };
                    let suffix_len = suffix.children.len();
                    if base.tag == T::Path && suffix.tag == T::Path {
                        base.children.extend(suffix.children);
                        last_suffix_len = suffix_len;
                    } else if base.tag == T::Invoke
                        && base.text(files) == Some(".")
                        && base.children.last().is_some_and(|p| p.tag == T::Path)
                        && suffix.tag == T::Path
                    {
                        base.children
                            .last_mut()
                            .unwrap()
                            .children
                            .extend(suffix.children);
                        last_suffix_len = suffix_len;
                    } else {
                        base = Self::new(T::Invoke, ".", vec![base, suffix]);
                        last_suffix_len = suffix_len;
                    }
                }
                base
            }
            Rule::pipe | Rule::comma | Rule::alt | Rule::dict_expr => {
                let count = xs.iter().filter(|p| !p.is_comment()).count();
                if count == 1 {
                    unwrap(xs)
                } else {
                    let op = match rule {
                        Rule::comma => ",",
                        Rule::alt => "//",
                        _ => "|",
                    };
                    // // is right-associative; do not erase its grouping.
                    if rule == Rule::alt && count > 2 {
                        let mut rhs = xs.pop().unwrap();
                        while let Some(lhs) = xs.pop() {
                            if lhs.is_comment() {
                                rhs.children.insert(0, lhs);
                            } else {
                                rhs = Self::new(T::Invoke, op, vec![lhs, rhs]);
                            }
                        }
                        rhs
                    } else {
                        Self::new(T::Invoke, op, xs)
                    }
                }
            }
            Rule::assign
            | Rule::or_expr
            | Rule::and_expr
            | Rule::compare
            | Rule::additive
            | Rule::multiplicative => {
                let mut iter = rules.into_iter().zip(xs);
                let mut leading = vec![];
                let mut lhs = loop {
                    let (_, p) = iter.next().expect("operand");
                    if p.is_comment() {
                        leading.push(p);
                    } else {
                        break p;
                    }
                };
                lhs.children.splice(0..0, leading);
                let mut pending: Option<Self> = None;
                for (r, child) in iter {
                    if child.is_comment() {
                        lhs.children.push(child);
                    } else if matches!(
                        r,
                        Rule::assign_op
                            | Rule::compare_op
                            | Rule::add_op
                            | Rule::mul_op
                            | Rule::and_kw
                            | Rule::or_kw
                    ) {
                        pending = Some(child);
                    } else {
                        let op = pending.take().expect("binary operator");
                        lhs = Self::new(
                            if rule == Rule::assign {
                                T::Assign
                            } else {
                                T::Invoke
                            },
                            op.text(files).unwrap(),
                            vec![lhs, child],
                        );
                    }
                }
                lhs
            }
            Rule::unary_minus => Self::new(T::Invoke, "-", xs),
            Rule::try_expr => Self::new(T::Invoke, "try", xs),
            Rule::funccall | Rule::format_prefixed_string => {
                let head = take(&mut xs);
                Self::new(T::Invoke, head.text(files).unwrap(), xs)
            }
            Rule::array | Rule::array_pattern => {
                if rule == Rule::array
                    && xs.len() == 1
                    && xs[0].tag == T::Invoke
                    && xs[0].text(files) == Some(",")
                {
                    xs = xs.remove(0).children;
                }
                Self::node(T::Array, xs)
            }
            Rule::object_key => {
                if rules.contains(&Rule::ident) {
                    let x = take(&mut xs);
                    xs.insert(0, Self::string(x.text(files).unwrap()));
                }
                unwrap(xs)
            }
            Rule::object_pair | Rule::object_pattern_field => {
                let key_index = xs.iter().position(|p| !p.is_comment()).unwrap();
                let key = xs[key_index].clone();
                let count = xs.iter().filter(|p| !p.is_comment()).count();
                let is_var = key.tag == T::Var;
                // `$__loc__` is special-cased to a `T::Loc` node (not
                // `T::Var`) at the point the raw `var` token is converted,
                // so the bare-variable shorthand (`{$__loc__}` -> key
                // "__loc__", value the location literal) needs its own
                // check here too.
                let is_loc = key.tag == T::Loc;
                if is_var && (count == 1 || rule == Rule::object_pattern_field) {
                    xs[key_index] = Self::string(key.text(files).unwrap());
                } else if is_loc && count == 1 {
                    xs[key_index] = Self::string("__loc__");
                }
                if count == 1 {
                    let val = if is_var || is_loc {
                        key
                    } else {
                        Self::node(T::Path, vec![key])
                    };
                    xs.push(val);
                } else if is_var && rule == Rule::object_pattern_field {
                    // `$b:pattern` in a destructuring pattern (unlike the
                    // otherwise-identical `object_pair` syntax used for
                    // plain object construction) binds $b to the *whole*
                    // field value *and* destructures it with `pattern` -
                    // jq applies both, so this field expands into two
                    // (key, subpattern) entries against the same key.
                    let key_str = xs[key_index].clone();
                    xs.insert(key_index + 1, key);
                    xs.insert(key_index + 2, key_str);
                }
                Self::node(T::Root, xs) // temporary field list, consumed by Object
            }
            Rule::object | Rule::object_pattern => Self::node(
                T::Object,
                xs.into_iter()
                    .flat_map(|p| {
                        if p.tag == T::Root {
                            p.children
                        } else {
                            vec![p]
                        }
                    })
                    .collect(),
            ),
            Rule::patterns => {
                if xs.iter().filter(|p| !p.is_comment()).count() == 1 {
                    unwrap(xs)
                } else {
                    Self::node(T::Root, xs)
                }
            }
            Rule::bind_tail => Self::node(T::Bind, xs),
            Rule::bind_or_expr => {
                if let Some(tail) = rules.iter().position(|r| *r == Rule::bind_tail) {
                    let tail = xs.remove(tail);
                    let mut semantic = tail.semantic_children().cloned().collect::<Vec<_>>();
                    let body = semantic.pop().expect("bind body");
                    let patterns = semantic.pop().expect("bind patterns");
                    let mut children = vec![
                        xs.into_iter()
                            .find(|p| !p.is_comment())
                            .expect("bind target"),
                        body,
                    ];
                    if patterns.tag == T::Root {
                        children.extend(patterns.children);
                    } else {
                        children.push(patterns);
                    }
                    Self::node(T::Bind, children)
                } else {
                    unwrap(xs)
                }
            }
            Rule::reduce_expr => reorder_fold(T::Reduce, xs),
            Rule::foreach_expr => reorder_fold(T::ForEach, xs),
            Rule::label_expr | Rule::break_expr => {
                let name = take(&mut xs);
                Self::new(
                    if rule == Rule::label_expr {
                        T::Label
                    } else {
                        T::Break
                    },
                    name.text(files).unwrap(),
                    xs,
                )
            }
            Rule::if_expr => Self::node(T::If, xs),
            Rule::funcdef => {
                let name = take(&mut xs);
                xs.push(Self::node(T::Empty, vec![]));
                Self::new(T::Def, name.text(files).unwrap(), xs)
            }
            Rule::funcdef_prefix => {
                let mut def = take(&mut xs);
                def.children.pop();
                def.children.extend(xs);
                def
            }
            Rule::program => Self::node(T::Root, xs),
            Rule::module_decl => Self::node(T::Module, xs),
            Rule::include_decl => Self::node(T::Include, xs),
            Rule::import_decl_full => {
                let alias_idx = xs
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| !p.is_comment())
                    .nth(1)
                    .unwrap()
                    .0;
                let alias = xs.remove(alias_idx);
                let name = format!(
                    "{}{}",
                    if alias.tag == T::Var { "$" } else { "" },
                    alias.text(files).unwrap()
                );
                Self::new(T::Import, name, xs)
            }
            // Only these Pest productions are transparent. Unknown rules must not
            // silently turn into arbitrary invocations.
            Rule::query
            | Rule::comma_operand
            | Rule::expr
            | Rule::term
            | Rule::primary
            | Rule::postfix_op
            | Rule::bracket_inner
            | Rule::string_part
            | Rule::pattern
            | Rule::param
            | Rule::import_decl
            | Rule::slice_from
            | Rule::slice_to
            | Rule::interpolation
            | Rule::paren => unwrap(xs),
            _ => unreachable!("non-tree Pest rule: {rule:?}"),
        };
        result.span = Some(pos);
        result
    }
    #[must_use]
    pub fn to_value(&self, files: &FileSet) -> Value {
        let mut o = indexmap::IndexMap::new();
        o.insert(
            strs::keyword_tag(),
            Value::String(self.tag.to_string().into()),
        );
        if let Some(content) = self.text(files) {
            o.insert(
                strs::keyword_content(),
                Value::String(content.to_string().into()),
            );
        }
        o.insert(
            strs::keyword_children(),
            Value::Array(std::rc::Rc::new(
                self.children.iter().map(|p| p.to_value(files)).collect(),
            )),
        );
        Value::Object(std::rc::Rc::new(o))
    }
    pub fn from_value(value: &Value) -> Result<Self, String> {
        let Value::Object(o) = value else {
            return Err("pair must be an object".into());
        };
        let Some(Value::String(tag)) = o.get(&strs::keyword_tag()) else {
            return Err("pair tag must be a string".into());
        };
        let content_span = match o.get(&strs::keyword_content()) {
            Some(Value::String(s)) => Some(ContentSpan::Str(s.to_string())),
            Some(Value::Null) | None => None,
            _ => return Err("pair content must be a string or null".into()),
        };
        let Some(Value::Array(xs)) = o.get(&strs::keyword_children()) else {
            return Err("pair children must be an array".into());
        };
        Ok(Self {
            tag: PairTag::from_name(tag)?,
            children: xs.iter().map(Self::from_value).collect::<Result<_, _>>()?,
            span: None,
            content_span,
        })
    }
}
fn unwrap(mut xs: Vec<Pair>) -> Pair {
    let i = xs
        .iter()
        .position(|p| !p.is_comment())
        .expect("transparent rule operand");
    let mut result = xs.remove(i);
    let after = xs.split_off(i);
    result.children.splice(0..0, xs);
    result.children.extend(after);
    result
}
fn reorder_fold(tag: PairTag, xs: Vec<Pair>) -> Pair {
    let mut sem: Vec<Pair> = xs.into_iter().filter(|p| !p.is_comment()).collect();
    let patterns = sem.remove(1);
    let mut out = sem;
    if tag == PairTag::ForEach && out.len() == 3 {
        out.push(Pair::node(PairTag::Path, vec![]));
    }
    if patterns.tag == PairTag::Root {
        out.extend(patterns.children);
    } else {
        out.push(patterns);
    }
    Pair::node(tag, out)
}
impl PairTag {
    pub fn from_name(name: &str) -> Result<Self, String> {
        use PairTag::{
            Array, Assign, Bind, Break, Comment, Def, Empty, Float, ForEach, If, Import, Include,
            Int, Invoke, Label, Loc, Module, Object, Path, Reduce, Root, Slice, Spread, String,
            StringChunk, TrailingComment, Var,
        };
        Ok(match name {
            "Comment" => Comment,
            "TrailingComment" => TrailingComment,
            "Var" => Var,
            "Loc" => Loc,
            "Int" => Int,
            "Float" => Float,
            "StringChunk" => StringChunk,
            "String" => String,
            "Array" => Array,
            "Object" => Object,
            "Slice" => Slice,
            "Spread" => Spread,
            "Path" => Path,
            "Label" => Label,
            "Break" => Break,
            "Invoke" => Invoke,
            "Assign" => Assign,
            "Bind" => Bind,
            "Reduce" => Reduce,
            "ForEach" => ForEach,
            "If" => If,
            "Empty" => Empty,
            "Def" => Def,
            "Module" => Module,
            "Include" => Include,
            "Import" => Import,
            "Root" => Root,
            _ => return Err(format!("unknown pair tag {name:?}")),
        })
    }
}
#[must_use]
pub fn pairs_to_value(pairs: &[Pair], files: &FileSet) -> Value {
    Value::Array(std::rc::Rc::new(
        pairs.iter().map(|p| p.to_value(files)).collect(),
    ))
}
#[derive(Debug, Clone)]
pub struct FileItem {
    pub path: String,
    pub start: usize,
    pub end: usize,
    pub source: String,
    pub lines: Vec<usize>,
}
#[derive(Debug, Clone, Default)]
pub struct FileSet {
    pub files: Vec<FileItem>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub file: usize,
    pub line: usize,
    pub column: usize,
}
impl FileSet {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    #[must_use]
    pub fn text(&self, span: &SpanPos) -> Option<&str> {
        let f = self.files.get(span.file)?;
        f.source
            .get(span.start.checked_sub(f.start)?..span.end.checked_sub(f.start)?)
    }
    pub fn add(&mut self, path: impl Into<String>, source: impl Into<String>) -> usize {
        let source = source.into();
        let mut lines = vec![0];
        lines.extend(
            source
                .bytes()
                .enumerate()
                .filter_map(|(i, b)| (b == b'\n').then_some(i + 1)),
        );
        // Reserve an EOF position for each file, including empty files.
        let start = self.files.last().map_or(0, |f| f.end + 1);
        let end = start + source.len();
        self.files.push(FileItem {
            path: path.into(),
            start,
            end,
            source,
            lines,
        });
        self.files.len() - 1
    }
    #[must_use]
    pub fn locate(&self, offset: usize) -> Option<Location> {
        let file = self
            .files
            .partition_point(|f| f.start <= offset)
            .checked_sub(1)?;
        let f = &self.files[file];
        if offset > f.end {
            return None;
        }
        let n = offset - f.start;
        let line = f.lines.partition_point(|&x| x <= n);
        Some(Location {
            file,
            line,
            column: n - f.lines[line - 1] + 1,
        })
    }
    pub fn import_pairs(&mut self, source: &Self, roots: &[Pair]) -> Result<Pairs, String> {
        fn validate(p: &Pair, files: &FileSet) -> Result<(), String> {
            for s in p.span.iter().chain(match &p.content_span {
                Some(ContentSpan::Pos(s)) => Some(s),
                _ => None,
            }) {
                if files.text(s).is_none() {
                    return Err("Pair span lies outside its source file or UTF-8 boundary".into());
                }
            }
            for x in &p.children {
                validate(x, files)?;
            }
            Ok(())
        }
        for p in roots {
            validate(p, source)?;
        }
        let mapping: Vec<_> = source
            .files
            .iter()
            .map(|f| {
                let id = self.add(f.path.clone(), f.source.clone());
                (id, self.files[id].start)
            })
            .collect();
        fn rebase(p: &mut Pair, source: &FileSet, mapping: &[(usize, usize)]) {
            for s in p.span.iter_mut().chain(match &mut p.content_span {
                Some(ContentSpan::Pos(s)) => Some(s),
                _ => None,
            }) {
                let origin = source.files[s.file].start;
                let (id, base) = mapping[s.file];
                s.file = id;
                s.start = base + (s.start - origin);
                s.end = base + (s.end - origin);
            }
            for x in &mut p.children {
                rebase(x, source, mapping);
            }
        }
        let mut roots = roots.to_vec();
        for p in &mut roots {
            rebase(p, source, &mapping);
        }
        Ok(roots)
    }
}

impl Pair {
    /// Validate the public JSON boundary before indexing positional slots.
    pub fn validate(&self, files: &FileSet) -> Result<(), String> {
        use PairTag as T;
        let xs: Vec<_> = self.semantic_children().collect();
        let n = xs.len();
        let content = self.text(files);
        if self.content_span.is_some() && content.is_none() {
            return Err("invalid content span".into());
        }
        let named = content.is_some_and(|s| !s.is_empty());
        let valid = match self.tag {
            T::Comment | T::TrailingComment => content.is_some() && self.children.is_empty(),
            T::Int | T::Float | T::Var | T::Break => named && n == 0,
            T::StringChunk => content.is_some() && n == 0,
            T::Invoke => {
                named
                    && match content.unwrap() {
                        "." => n == 2,
                        "?" => n == 1,
                        "-" | "try" => n == 1 || n == 2,
                        ".." => n == 0,
                        "|" | "," => n >= 1,
                        op if super::printer::is_binary(op) => n == 2,
                        op if op.starts_with('@') => n == 0 || (n == 1 && xs[0].tag == T::String),
                        _ => true,
                    }
            }
            T::Assign => {
                n == 2
                    && matches!(
                        content,
                        Some("=" | "|=" | "+=" | "-=" | "*=" | "/=" | "%=" | "//=")
                    )
            }
            T::Label => named && n == 1,
            T::Def => {
                named
                    && n >= 2
                    && xs[..n - 2].iter().all(|p| {
                        matches!(p.tag, T::Var | T::Invoke) && p.semantic_children().count() == 0
                    })
            }
            T::Bind => {
                content.is_none()
                    && n >= 3
                    && is_expression(xs[0])
                    && is_expression(xs[1])
                    && xs[2..].iter().all(|p| is_pattern(p))
            }
            T::Reduce => content.is_none() && n >= 4,
            T::ForEach => content.is_none() && n >= 5,
            T::Import => named && matches!(n, 1 | 2) && xs[0].tag == T::String,
            T::Include => content.is_none() && matches!(n, 1 | 2) && xs[0].tag == T::String,
            T::Module => content.is_none() && n == 1,
            T::Loc => {
                content.is_none() && n == 2 && xs[0].tag == T::StringChunk && xs[1].tag == T::Int
            }
            T::Slice => content.is_none() && matches!(n, 1 | 2),
            T::Spread | T::Empty => content.is_none() && n == 0,
            T::If => content.is_none() && n >= 2,
            T::Object => content.is_none() && n % 2 == 0,
            T::Root | T::Path | T::Array | T::String => content.is_none(),
        };
        if !valid {
            return Err(format!(
                "malformed PairTag::{} (content={content:?}, children={n})",
                self.tag
            ));
        }
        let slots_valid = match self.tag {
            T::Bind => {
                xs.len() >= 3
                    && is_expression(xs[0])
                    && is_expression(xs[1])
                    && xs[2..].iter().all(|p| is_pattern(p))
            }
            T::Reduce => {
                xs.len() >= 4
                    && xs[0..3].iter().all(|p| is_expression(p))
                    && xs[3..].iter().all(|p| is_pattern(p))
            }
            T::ForEach => {
                xs.len() >= 5
                    && xs[0..4].iter().all(|p| is_expression(p))
                    && xs[4..].iter().all(|p| is_pattern(p))
            }
            T::Path => xs
                .iter()
                .all(|p| matches!(p.tag, T::Slice | T::Spread) || is_expression(p)),
            T::Array | T::Object | T::Slice | T::If | T::Label | T::Module => {
                xs.iter().all(|p| is_expression(p))
            }
            T::Def => {
                is_expression(xs[n - 2]) && (xs[n - 1].tag == T::Empty || is_expression(xs[n - 1]))
            }
            T::Root => {
                xs.iter().all(|p| {
                    matches!(p.tag, T::Module | T::Include | T::Import) || is_expression(p)
                }) && xs.iter().filter(|p| has_execution(p)).count() <= 1
            }
            _ => true,
        };
        if !slots_valid {
            return Err(format!("invalid semantic slots in PairTag::{}", self.tag));
        }
        for child in &self.children {
            child.validate(files)?;
        }
        Ok(())
    }
}
const fn is_expression(p: &Pair) -> bool {
    !matches!(
        p.tag,
        PairTag::Comment
            | PairTag::TrailingComment
            | PairTag::StringChunk
            | PairTag::Slice
            | PairTag::Spread
            | PairTag::Empty
            | PairTag::Module
            | PairTag::Include
            | PairTag::Import
            | PairTag::Root
    )
}
fn is_pattern(p: &Pair) -> bool {
    match p.tag {
        PairTag::Var => true,
        PairTag::Array => p.semantic_children().all(is_pattern),
        PairTag::Object => {
            let xs: Vec<_> = p.semantic_children().collect();
            xs.len() % 2 == 0
                && xs
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .all(|kv| is_expression(kv[0]) && is_pattern(kv[1]))
        }
        _ => false,
    }
}
fn has_execution(p: &Pair) -> bool {
    match p.tag {
        PairTag::Def => p.semantic_children().last().is_some_and(has_execution),
        PairTag::Empty | PairTag::Module | PairTag::Import | PairTag::Include => false,
        _ => is_expression(p),
    }
}
