//! Structural jq printer. Only node content is read; external spans are optional.
use crate::{
    jq::parser::pair::{FileSet, Pair, PairTag, operator_precedence},
    render::{self, CompactLevel, FormatOptions, ThemeIdx},
};

pub struct Printer<'a> {
    files: &'a FileSet,
    options: &'a FormatOptions,
    theme: Option<&'a render::ColorTheme>,
    out: String,
    indent: usize,
}
struct Children<'a> {
    xs: &'a [Pair],
    index: usize,
}
impl<'a> Children<'a> {
    fn new(p: &'a Pair) -> Self {
        Self {
            xs: &p.children,
            index: 0,
        }
    }
    fn next(&mut self, printer: &mut Printer<'_>) -> Option<&'a Pair> {
        while let Some(x) = self.xs.get(self.index) {
            self.index += 1;
            if x.is_comment() {
                printer.comment(x);
            } else {
                return Some(x);
            }
        }
        None
    }
    fn finish(&mut self, printer: &mut Printer<'_>) {
        while self.next(printer).is_some() {}
    }
}
impl<'a> Printer<'a> {
    pub fn new(files: &'a FileSet, options: &'a FormatOptions) -> Self {
        Self {
            files,
            options,
            theme: None,
            out: String::new(),
            indent: 0,
        }
    }
    pub fn with_render(files: &'a FileSet, options: &'a render::Options) -> Self {
        Self {
            theme: options.theme.as_ref(),
            ..Self::new(files, &options.out)
        }
    }
    fn pretty(&self) -> bool {
        self.options.compact_level == CompactLevel::Pretty
    }
    fn compact(&self) -> bool {
        self.options.compact_level == CompactLevel::Compact
    }
    fn space(&mut self) {
        if !self.compact() && !self.out.ends_with([' ', '\n']) {
            self.out.push(' ');
        }
    }
    fn nl(&mut self) {
        while self.out.ends_with([' ', '\t']) {
            self.out.pop();
        }
        if !self.out.ends_with('\n') {
            self.out.push('\n');
        }
        for _ in 0..self.indent {
            self.out.push_str(self.options.indent);
        }
    }
    fn styled(&mut self, text: &str, kind: ThemeIdx) {
        if let Some(theme) = self.theme
            && let Some(style) = theme.styles[kind as usize].clone().into()
        {
            self.out.push_str(&style.to_string());
            self.out.push_str(text);
            self.out.push_str(render::RESET);
            return;
        }
        self.out.push_str(text);
    }
    fn keyword(&mut self, text: &str) {
        self.styled(text, ThemeIdx::JQKeyword);
    }
    fn content(&self, p: &Pair) -> String {
        p.text(self.files).unwrap_or("").to_owned()
    }
    /// If `key` is a plain `String[StringChunk(...)]` (no interpolation) whose
    /// content is a valid bare jq identifier, returns that raw text - so the
    /// key can be printed unquoted (`foo:` instead of `"foo":`).
    fn bare_key(&self, key: &Pair) -> Option<String> {
        if key.tag != PairTag::String {
            return None;
        }
        let mut children = key.semantic_children();
        let chunk = children.next()?;
        if children.next().is_some() || chunk.tag != PairTag::StringChunk {
            return None;
        }
        let text = chunk.text(self.files)?;
        is_bare_ident(text).then(|| text.to_owned())
    }
    fn shorthand_key(&self, key: &Pair, value: &Pair) -> Option<String> {
        let name = self.bare_key(key)?;
        if value.tag != PairTag::Path {
            return None;
        }
        let mut parts = value.semantic_children();
        let path = parts.next()?;
        if parts.next().is_some() || self.bare_key(path).as_deref() != Some(name.as_str()) {
            return None;
        }
        Some(name)
    }
    fn comment(&mut self, p: &Pair) {
        if !self.pretty() {
            return;
        }
        if p.tag == PairTag::Comment && !self.out.trim_end().is_empty() {
            self.nl();
        } else if !self.out.is_empty() && !self.out.ends_with([' ', '\n']) {
            self.out.push(' ');
        }
        self.styled(&format!("#{}", self.content(p)), ThemeIdx::JQComment);
        self.nl();
    }
    pub fn print(mut self, p: &Pair) -> Result<String, String> {
        p.validate(self.files)?;
        self.p(&p.clone().normalize(self.files));
        while self.out.ends_with([' ', '\t', '\n']) {
            self.out.pop();
        }
        Ok(self.out)
    }
    fn precedence(&self, p: &Pair) -> u8 {
        match p.tag {
            PairTag::Bind
            | PairTag::Label
            | PairTag::Def
            | PairTag::If
            | PairTag::Reduce
            | PairTag::ForEach => 0,
            PairTag::Assign => operator_precedence("="),
            PairTag::Invoke => match p.text(self.files).unwrap_or("") {
                "?" | "." => 12,
                "try" => 10,
                "-" if p.semantic_children().count() == 1 => 11,
                op if is_binary(op) => operator_precedence(op),
                _ => 13,
            },
            _ => 13,
        }
    }
    fn operand(&mut self, p: &Pair, minimum: u8) {
        let parens = self.precedence(p) < minimum;
        if parens {
            self.out.push('(');
        }
        self.p(p);
        if parens {
            self.out.push(')');
        }
    }
    fn multiline(&self, p: &Pair) -> bool {
        if !self.pretty() {
            return false;
        }
        if matches!(p.tag, PairTag::Array | PairTag::Object) && p.semantic_children().count() == 0 {
            return false;
        }
        if p.children.iter().any(|p| p.is_comment()) {
            return true;
        }
        let mut options = *self.options;
        options.compact_level = CompactLevel::Inline;
        let mut flat = Printer::new(self.files, &options);
        flat.p(p);
        visual_width(&self.out) + visual_width(&flat.out) > options.max_width()
    }
    fn p(&mut self, p: &Pair) {
        use PairTag as T;
        let n = p.semantic_children().count();
        let mut xs = Children::new(p);
        match p.tag {
            T::Comment | T::TrailingComment => self.comment(p),
            T::Root => {
                let mut wrote = false;
                while let Some(x) = xs.next(self) {
                    if wrote {
                        if self.pretty() {
                            self.nl();
                        } else {
                            self.space();
                        }
                    }
                    self.p(x);
                    wrote = true;
                }
            }
            T::Empty => {}
            T::Var => {
                self.styled(&format!("${}", self.content(p)), ThemeIdx::JQVariable);
            }
            T::Int | T::Float => self.styled(&self.content(p), ThemeIdx::Number),
            T::StringChunk => self.styled(&self.content(p), ThemeIdx::String),
            T::String => {
                self.styled("\"", ThemeIdx::String);
                // Comments are owned by interpolation expressions, never literal text.
                while let Some(x) = xs.next(self) {
                    if x.tag == T::StringChunk {
                        self.p(x);
                    } else {
                        self.styled("\\(", ThemeIdx::JQStringEscape);
                        self.p(x);
                        self.styled(")", ThemeIdx::JQStringEscape);
                    }
                }
                self.styled("\"", ThemeIdx::String);
            }
            T::Loc => {
                self.out.push_str("{\"file\":");
                self.space();
                self.out.push('"');
                let child = xs.next(self).unwrap();
                self.p(child);
                self.out.push_str("\",\"line\":");
                self.space();
                let child = xs.next(self).unwrap();
                self.p(child);
                self.out.push('}');
            }
            T::Array => {
                let multiline = self.multiline(p);
                self.styled("[", ThemeIdx::Array);
                if multiline {
                    self.indent += 1;
                }
                for i in 0..n {
                    if i > 0 {
                        self.out.push(',');
                        self.space();
                    }
                    if multiline {
                        self.nl();
                    }
                    let x = xs.next(self).unwrap();
                    self.operand(x, 0);
                }
                xs.finish(self);
                if multiline {
                    self.indent -= 1;
                    self.nl();
                }
                self.styled("]", ThemeIdx::Array);
            }
            T::Object => {
                let multiline = self.multiline(p);
                self.styled("{", ThemeIdx::Object);
                if multiline {
                    self.indent += 1;
                }
                for i in 0..n / 2 {
                    if i > 0 {
                        self.out.push(',');
                        self.space();
                    }
                    if multiline {
                        self.nl();
                    }
                    let key = xs.next(self).unwrap();
                    if let Some(bare) = self.bare_key(key) {
                        self.styled(&bare, ThemeIdx::String);
                    } else {
                        let computed = key.tag != T::String;
                        if computed {
                            self.out.push('(');
                        }
                        self.p(key);
                        if computed {
                            self.out.push(')');
                        }
                    }
                    self.out.push(':');
                    self.space();
                    let value = xs.next(self).unwrap();
                    if self.shorthand_key(key, value).is_some() {
                        self.out.truncate(self.out.len() - 2);
                    } else {
                        // Object values accept ordinary expressions directly;
                        // only low-precedence filters such as `if`/`|` need
                        // grouping to keep the pair boundary unambiguous.
                        self.operand(value, 3);
                    }
                }
                xs.finish(self);
                if multiline {
                    self.indent -= 1;
                    self.nl();
                }
                self.styled("}", ThemeIdx::Object);
            }
            T::Path => {
                self.path(p, true);
                xs.index = p.children.len();
            }
            T::Slice | T::Spread => {
                self.path_component(p);
                xs.index = p.children.len();
            }
            T::Invoke | T::Assign => {
                self.invoke(p);
                xs.index = p.children.len();
            }
            T::Label => {
                self.keyword("label");
                self.out.push(' ');
                self.styled(&format!("${}", self.content(p)), ThemeIdx::JQVariable);
                self.out.push_str(" | ");
                let body = xs.next(self).unwrap();
                self.p(body);
            }
            T::Break => {
                self.keyword("break");
                self.out.push(' ');
                self.styled(&format!("${}", self.content(p)), ThemeIdx::JQVariable);
            }
            T::Bind | T::Reduce | T::ForEach => {
                if p.tag != T::Bind {
                    self.keyword(if p.tag == T::Reduce {
                        "reduce"
                    } else {
                        "foreach"
                    });
                    self.out.push(' ');
                }
                let value = xs.next(self).unwrap();
                self.operand(value, 13);
                self.out.push(' ');
                self.keyword("as");
                self.out.push(' ');
                if p.tag == T::Bind {
                    let body = xs.next(self).unwrap();
                    let patterns: Vec<&Pair> = std::iter::from_fn(|| xs.next(self)).collect();
                    for (i, pattern) in patterns.iter().enumerate() {
                        if i > 0 {
                            self.out.push_str(" ?// ");
                        }
                        self.p(pattern);
                    }
                    self.out.push_str(" | ");
                    self.p(body);
                } else {
                    let rest: Vec<&Pair> = std::iter::from_fn(|| xs.next(self)).collect();
                    let pattern_start = if p.tag == T::ForEach { 3 } else { 2 };
                    for (i, pattern) in rest[pattern_start..].iter().enumerate() {
                        if i > 0 {
                            self.out.push_str(" ?// ");
                        }
                        self.p(pattern);
                    }
                    self.out.push_str(" (");
                    self.p(rest[0]);
                    self.out.push(';');
                    self.space();
                    self.p(rest[1]);
                    if pattern_start == 3 {
                        self.out.push(';');
                        self.space();
                        self.p(rest[2]);
                    }
                    self.out.push(')');
                }
            }
            T::If => {
                for i in 0..n / 2 {
                    if i > 0 {
                        if self.pretty() {
                            self.nl();
                        } else {
                            self.out.push(' ');
                        }
                    }
                    self.keyword(if i == 0 { "if" } else { "elif" });
                    self.out.push(' ');
                    let cond = xs.next(self).unwrap();
                    self.p(cond);
                    self.out.push(' ');
                    self.keyword("then");
                    if self.pretty() {
                        self.indent += 1;
                        self.nl();
                    } else {
                        self.out.push(' ');
                    }
                    let body = xs.next(self).unwrap();
                    self.p(body);
                    if self.pretty() {
                        self.indent -= 1;
                    }
                }
                if n % 2 == 1 {
                    if self.pretty() {
                        self.nl();
                    } else {
                        self.out.push(' ');
                    }
                    self.keyword("else");
                    if self.pretty() {
                        self.indent += 1;
                        self.nl();
                    } else {
                        self.out.push(' ');
                    }
                    let body = xs.next(self).unwrap();
                    self.p(body);
                    if self.pretty() {
                        self.indent -= 1;
                    }
                }
                xs.finish(self);
                if self.pretty() {
                    self.nl();
                } else {
                    self.out.push(' ');
                }
                self.keyword("end");
            }
            T::Def => {
                self.keyword("def");
                self.out.push(' ');
                self.styled(&self.content(p), ThemeIdx::JQFunction);
                if n > 2 {
                    self.out.push('(');
                    for i in 0..n - 2 {
                        if i > 0 {
                            self.out.push(';');
                            self.space();
                        }
                        let param = xs.next(self).unwrap();
                        self.p(param);
                    }
                    self.out.push(')');
                }
                self.out.push(':');
                if self.pretty() {
                    self.indent += 1;
                    self.nl();
                } else {
                    self.space();
                }
                let body = xs.next(self).unwrap();
                self.p(body);
                if self.pretty() {
                    self.indent -= 1;
                }
                self.out.push(';');
                let next = xs.next(self).unwrap();
                if next.tag != T::Empty {
                    if self.pretty() {
                        self.nl();
                    } else {
                        self.space();
                    }
                    self.p(next);
                }
            }
            T::Module | T::Include | T::Import => {
                self.keyword(match p.tag {
                    T::Module => "module",
                    T::Include => "include",
                    _ => "import",
                });
                self.out.push(' ');
                let first = xs.next(self).unwrap();
                self.p(first);
                if p.tag == T::Import {
                    self.out.push_str(" as ");
                    self.styled(&self.content(p), ThemeIdx::JQVariable);
                }
                if let Some(metadata) = xs.next(self) {
                    self.out.push(' ');
                    self.p(metadata);
                }
                self.out.push(';');
            }
        }
        xs.finish(self);
    }
    fn path(&mut self, p: &Pair, leading: bool) {
        let mut xs = Children::new(p);
        let mut first = leading;
        if p.semantic_children().count() == 0 && leading {
            self.out.push('.');
        }
        while let Some(x) = xs.next(self) {
            if first && x.tag != PairTag::String {
                self.out.push('.');
            }
            self.path_component(x);
            first = false;
        }
    }
    fn path_component(&mut self, p: &Pair) {
        use PairTag as T;
        match p.tag {
            T::String => {
                self.out.push('.');
                if let Some(bare) = self.bare_key(p) {
                    self.styled(&bare, ThemeIdx::String);
                } else {
                    self.p(p);
                }
            }
            T::Spread => {
                self.out.push('[');
                for x in &p.children {
                    self.comment(x);
                }
                self.out.push(']');
            }
            T::Slice => {
                let mut xs = Children::new(p);
                self.out.push('[');
                let from = xs.next(self).unwrap();
                if from.tag != T::Int || self.content(from) != "0" {
                    self.p(from);
                }
                self.out.push(':');
                if let Some(to) = xs.next(self) {
                    self.p(to);
                }
                self.out.push(']');
            }
            _ => {
                self.out.push('[');
                self.p(p);
                self.out.push(']');
            }
        }
    }
    fn suffix(&mut self, p: &Pair) {
        if p.tag == PairTag::Path {
            self.path(p, false);
        } else if p.tag == PairTag::Invoke && p.text(self.files) == Some("?") {
            let mut xs = Children::new(p);
            let inner = xs.next(self).unwrap();
            self.suffix(inner);
            self.out.push('?');
            xs.finish(self);
        } else {
            self.out.push_str(" | ");
            self.operand(p, 2);
        }
    }
    fn invoke(&mut self, p: &Pair) {
        let op = self.content(p);
        let n = p.semantic_children().count();
        let mut xs = Children::new(p);
        match op.as_str() {
            "." => {
                let base = xs.next(self).unwrap();
                self.operand(base, 12);
                let path = xs.next(self).unwrap();
                self.suffix(path);
            }
            "?" => {
                let x = xs.next(self).unwrap();
                self.operand(x, 12);
                self.out.push('?');
            }
            "-" if n == 1 => {
                self.styled("-", ThemeIdx::JQOperator);
                let x = xs.next(self).unwrap();
                self.operand(x, 11);
            }
            "try" => {
                self.keyword("try");
                self.out.push(' ');
                let body = xs.next(self).unwrap();
                self.operand(body, 12);
                if let Some(handler) = xs.next(self) {
                    self.out.push(' ');
                    self.keyword("catch");
                    self.out.push(' ');
                    self.operand(handler, 12);
                }
            }
            op if is_binary(op) || p.tag == PairTag::Assign => {
                let prec = operator_precedence(op);
                let break_pipe = op == "|" && self.pretty() && self.multiline(p);
                for i in 0..n {
                    if i > 0 {
                        let mandatory = matches!(op, "and" | "or" | "//");
                        if break_pipe {
                            self.nl();
                        } else if mandatory {
                            self.out.push(' ');
                        } else if op != "," {
                            self.space();
                        }
                        self.styled(op, ThemeIdx::JQOperator);
                        if mandatory {
                            self.out.push(' ');
                        } else {
                            self.space();
                        }
                    }
                    let arg = xs.next(self).unwrap();
                    // Parenthesize equal-precedence children unless the operation's
                    // own associativity proves that side can be printed ungrouped.
                    let strict = if op == "|" || op == "," {
                        false
                    } else if op == "//" {
                        i == 0
                    } else {
                        i > 0
                            || p.tag == PairTag::Assign
                            || matches!(op, "==" | "!=" | "<" | ">" | "<=" | ">=")
                    };
                    self.operand(arg, prec + u8::from(strict));
                }
            }
            op if op.starts_with('@') => {
                self.styled(op, ThemeIdx::JQFormat);
                if let Some(string) = xs.next(self) {
                    self.out.push(' ');
                    self.p(string);
                }
            }
            _ => {
                self.styled(
                    &op,
                    if op == ".." {
                        ThemeIdx::JQOperator
                    } else {
                        ThemeIdx::JQFunction
                    },
                );
                if n > 0 {
                    self.out.push('(');
                    for i in 0..n {
                        if i > 0 {
                            self.out.push(';');
                            self.space();
                        }
                        let arg = xs.next(self).unwrap();
                        self.p(arg);
                    }
                    self.out.push(')');
                }
            }
        }
        xs.finish(self);
    }
}
/// Matches this crate's `ident` grammar rule (`jq.pest`): `[A-Za-z_][A-Za-z0-9_]*`.
fn is_bare_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}
pub fn is_binary(op: &str) -> bool {
    matches!(
        op,
        "|" | ","
            | "//"
            | "and"
            | "or"
            | "=="
            | "!="
            | "<"
            | ">"
            | "<="
            | ">="
            | "+"
            | "-"
            | "*"
            | "/"
            | "%"
    )
}
pub fn format_source(source: &str, options: &FormatOptions) -> Result<String, String> {
    let (files, root) = super::parse_pairs("<jq>", source)?;
    Printer::new(&files, options).print(&root)
}
pub fn format_source_colored(source: &str, options: &render::Options) -> Result<String, String> {
    let (files, root) = super::parse_pairs("<jq>", source)?;
    Printer::with_render(&files, options).print(&root)
}

fn visual_width(text: &str) -> usize {
    let mut width = 0;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.next() == Some('[') {
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
            }
        } else if c == '\n' {
            width = 0;
        } else {
            width += 1;
        }
    }
    width
}
