use super::{CompileError, CompileOptions};
use crate::{
    data::{self, Value},
    jq::{
        builtins,
        ir::{
            CallTarget, Expr, ExprId, FunctionDef, FunctionId, Ir, Literal, PathStep, Pattern,
            PatternId,
        },
        parser::pairs::{Pair, PairTag},
        symbols::Symbols,
    },
    strs,
};

pub(super) fn lower(
    ir: &mut Ir,
    root: &Pair,
    options: &CompileOptions,
    symbols: &mut Symbols,
) -> Result<ExprId, CompileError> {
    lower_expr(ir, root, symbols, options, options.repl)
}
fn lower_expr(
    ir: &mut Ir,
    pair: &Pair,
    symbols: &mut Symbols,
    options: &CompileOptions,
    export: bool,
) -> Result<ExprId, CompileError> {
    // Only declarations change lexical names. Restore at the scope boundary,
    // including early returns and errors, rather than snapshotting every node.
    let previous = (!export
        && matches!(
            pair.tag,
            PairTag::Root
                | PairTag::Def
                | PairTag::Bind
                | PairTag::Label
                | PairTag::Reduce
                | PairTag::ForEach
        ))
    .then(|| symbols.names.clone());
    let result = lower_expr_inner(ir, pair, symbols, options, export);
    if let Some(previous) = previous {
        symbols.names = previous;
    }
    result
}
fn lower_expr_inner(
    ir: &mut Ir,
    pair: &Pair,
    symbols: &mut Symbols,
    options: &CompileOptions,
    export: bool,
) -> Result<ExprId, CompileError> {
    let children: Vec<_> = pair.semantic_children().collect();
    let text = pair.text(&ir.files).unwrap_or("").to_string();
    let expr = match pair.tag {
        PairTag::Root => {
            let mut body = children.iter().copied().filter(|child| {
                !matches!(
                    child.tag,
                    PairTag::Module | PairTag::Import | PairTag::Include
                )
            });
            let Some(expression) = body.next() else {
                if options.repl {
                    return Ok(ir.push(Expr::Input, pair.span.clone()));
                }
                return Err(CompileError(format!(
                    "{}: missing program body",
                    options.path
                )));
            };
            let rest: Vec<_> = body.collect();
            return lower_definitions(ir, expression, &rest, symbols, options, export);
        }
        PairTag::Path if children.is_empty() => Expr::Input,
        PairTag::Path => {
            let base = ir.push(Expr::Input, pair.span.clone());
            let steps = children
                .iter()
                .map(|component| lower_path_step(ir, component, symbols, options))
                .collect::<Result<_, _>>()?;
            Expr::Path { base, steps }
        }
        PairTag::Empty if options.repl => Expr::Input,
        PairTag::Bind => {
            let source = lower_expr(ir, children[0], symbols, options, false)?;
            let minimum = symbols.bindings.len();
            let mut patterns = Vec::new();
            for child in &children[2..] {
                patterns.push(lower_pattern(ir, child, symbols, options, export, minimum)?);
            }
            let pattern = if patterns.len() == 1 {
                patterns[0]
            } else {
                let id = PatternId(ir.patterns.len());
                ir.patterns.push(Pattern::Alternatives(patterns));
                id
            };
            let body = lower_expr(ir, children[1], symbols, options, export)?;
            Expr::Bind {
                source,
                pattern,
                body,
            }
        }
        PairTag::Assign => {
            let path = lower_expr(ir, children[0], symbols, options, false)?;
            let rhs = lower_expr(ir, children[1], symbols, options, false)?;
            if text == "=" || text == "|=" {
                let name = if text == "=" { "_assign" } else { "_modify" };
                return lower_call(ir, symbols, name, vec![path, rhs], pair.span.clone());
            }
            let operator = text
                .strip_suffix('=')
                .ok_or_else(|| CompileError("invalid assignment operator".into()))?;
            let binding = symbols.fresh_binding(pair.span.clone());
            let pattern = PatternId(ir.patterns.len());
            ir.patterns.push(Pattern::Binding(binding));
            let lhs = ir.push(Expr::Input, pair.span.clone());
            let value = ir.push(Expr::Read(binding), pair.span.clone());
            let update = if operator == "//" {
                ir.push(Expr::Alternative { lhs, rhs: value }, pair.span.clone())
            } else {
                lower_call(ir, symbols, operator, vec![lhs, value], pair.span.clone())?
            };
            let body = lower_call(
                ir,
                symbols,
                "_modify",
                vec![path, update],
                pair.span.clone(),
            )?;
            Expr::Bind {
                source: rhs,
                pattern,
                body,
            }
        }
        PairTag::Label => {
            let label = crate::jq::ir::LabelId(symbols.next_label);
            symbols.next_label += 1;
            symbols.names.labels.insert(strs::intern(&text), label);
            let body = lower_expr(ir, children[0], symbols, options, false)?;
            Expr::Label { label, body }
        }
        PairTag::Break => Expr::Break(
            *symbols
                .names
                .labels
                .get(&strs::intern(&text))
                .ok_or_else(|| CompileError(format!("undefined label ${text}")))?,
        ),
        PairTag::Reduce | PairTag::ForEach => {
            let foreach = pair.tag == PairTag::ForEach;
            let pattern_index = if foreach { 4 } else { 3 };
            if children.len() != pattern_index + 1 {
                return Err(CompileError(
                    "fold pattern alternatives are not implemented yet".into(),
                ));
            }
            let source = lower_expr(ir, children[0], symbols, options, false)?;
            let init = lower_expr(ir, children[1], symbols, options, false)?;
            let minimum = symbols.bindings.len();
            let pattern = lower_pattern(
                ir,
                children[pattern_index],
                symbols,
                options,
                false,
                minimum,
            )?;
            let update = lower_expr(ir, children[2], symbols, options, false)?;
            if foreach {
                let extract = lower_expr(ir, children[3], symbols, options, false)?;
                Expr::Foreach {
                    source,
                    pattern,
                    init,
                    update,
                    extract,
                }
            } else {
                Expr::Reduce {
                    source,
                    pattern,
                    init,
                    update,
                }
            }
        }
        PairTag::Def => return lower_definitions(ir, pair, &[], symbols, options, export),
        PairTag::If => {
            let mut rest = children.as_slice();
            let mut branches = Vec::new();
            while rest.len() >= 2 {
                branches.push((
                    lower_expr(ir, rest[0], symbols, options, false)?,
                    lower_expr(ir, rest[1], symbols, options, false)?,
                ));
                rest = &rest[2..];
            }
            let mut no = if let Some(other) = rest.first() {
                lower_expr(ir, other, symbols, options, false)?
            } else {
                ir.push(Expr::Input, pair.span.clone())
            };
            for (condition, yes) in branches.into_iter().rev() {
                no = ir.push(Expr::If { condition, yes, no }, pair.span.clone());
            }
            return Ok(no);
        }
        PairTag::Int | PairTag::Float => Expr::Literal(Literal::Number(text)),
        PairTag::String if children.iter().all(|p| p.tag == PairTag::StringChunk) => {
            let raw: String = children
                .iter()
                .map(|p| p.text(&ir.files).unwrap_or(""))
                .collect();
            let value = data::parse_json_str(&format!("\"{raw}\"")).map_err(CompileError)?;
            Expr::Literal(Literal::Value(value))
        }
        PairTag::String => return lower_string(ir, pair, symbols, options, None),
        PairTag::Var if text == "__loc__" => {
            let location = pair
                .span
                .as_ref()
                .and_then(|span| ir.files.locate(span.start));
            let file = location.as_ref().map_or_else(
                || options.path.clone(),
                |loc| ir.files.files[loc.file].path.clone(),
            );
            let line = location.map_or(1, |loc| loc.line);
            Expr::Literal(Literal::Value(Value::Object(std::rc::Rc::new(
                [
                    (strs::keyword_file(), Value::String(file.into())),
                    (strs::keyword_line(), Value::int(line as i64)),
                ]
                .into_iter()
                .collect(),
            ))))
        }
        PairTag::Var if text == "ENV" && symbols.lookup(strs::intern("ENV")).is_none() => {
            Expr::BuiltinCall {
                builtin: builtins::lookup_symbol(strs::keyword_env(), 0).unwrap(),
                args: vec![],
            }
        }
        PairTag::Var => Expr::Read(
            symbols
                .lookup(strs::intern(&text))
                .ok_or_else(|| CompileError(format!("undefined variable ${text}")))?,
        ),
        PairTag::Invoke
            if text.starts_with('@')
                && children.len() == 1
                && children[0].tag == PairTag::String =>
        {
            return lower_string(ir, children[0], symbols, options, Some(&text));
        }
        PairTag::Invoke if text == "try" || text == "?" => {
            let body = lower_expr(ir, children[0], symbols, options, false)?;
            let handler = if let Some(handler) = children.get(1) {
                lower_expr(ir, handler, symbols, options, false)?
            } else {
                ir.push(Expr::Concat(vec![]), pair.span.clone())
            };
            Expr::Try { body, handler }
        }
        PairTag::Invoke if text == "//" => {
            let lhs = lower_expr(ir, children[0], symbols, options, false)?;
            let rhs = lower_expr(ir, children[1], symbols, options, false)?;
            Expr::Alternative { lhs, rhs }
        }
        PairTag::Invoke if (text == "and" || text == "or") && children.len() == 2 => {
            let condition = lower_expr(ir, children[0], symbols, options, false)?;
            let rhs = lower_expr(ir, children[1], symbols, options, false)?;
            let yes = ir.push(
                Expr::Literal(Literal::Value(Value::Bool(true))),
                pair.span.clone(),
            );
            let no = ir.push(
                Expr::Literal(Literal::Value(Value::Bool(false))),
                pair.span.clone(),
            );
            let rhs = ir.push(
                Expr::If {
                    condition: rhs,
                    yes,
                    no,
                },
                pair.span.clone(),
            );
            if text == "and" {
                Expr::If {
                    condition,
                    yes: rhs,
                    no,
                }
            } else {
                Expr::If {
                    condition,
                    yes,
                    no: rhs,
                }
            }
        }
        PairTag::Invoke if text == "." && children.len() == 2 => {
            let base = lower_expr(ir, children[0], symbols, options, false)?;
            let suffix = children[1];
            if suffix.tag == PairTag::Invoke && suffix.text(&ir.files) == Some("?") {
                lower_optional_path_suffix(ir, pair, base, suffix, symbols, options)?
            } else if suffix.tag != PairTag::Path {
                return Err(CompileError(
                    "optional path suffix is not implemented yet".into(),
                ));
            } else {
                let steps = suffix
                    .semantic_children()
                    .map(|p| lower_path_step(ir, p, symbols, options))
                    .collect::<Result<_, _>>()?;
                Expr::Path { base, steps }
            }
        }
        PairTag::Invoke if text == "|" || text == "," => {
            let expressions = children
                .iter()
                .map(|p| lower_expr(ir, p, symbols, options, false))
                .collect::<Result<_, _>>()?;
            if text == "|" {
                Expr::Pipe(expressions)
            } else {
                Expr::Concat(expressions)
            }
        }
        PairTag::Invoke
            if children.is_empty() && matches!(text.as_str(), "null" | "true" | "false") =>
        {
            Expr::Literal(Literal::Value(match text.as_str() {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => Value::Null,
            }))
        }
        PairTag::Array => {
            let inner = match children.len() {
                0 => ir.push(Expr::Concat(vec![]), pair.span.clone()),
                1 => lower_expr(ir, children[0], symbols, options, false)?,
                _ => {
                    let items = children
                        .iter()
                        .map(|p| lower_expr(ir, p, symbols, options, false))
                        .collect::<Result<_, _>>()?;
                    ir.push(Expr::Concat(items), pair.span.clone())
                }
            };
            Expr::Array(inner)
        }
        PairTag::Object => {
            let mut pairs = Vec::with_capacity(children.len() / 2);
            let mut rest = children.into_iter();
            while let (Some(key), Some(value)) = (rest.next(), rest.next()) {
                let key = lower_expr(ir, key, symbols, options, false)?;
                let value = lower_expr(ir, value, symbols, options, false)?;
                pairs.push((key, value));
            }
            Expr::Object(pairs)
        }
        // `$__loc__`: file/line are known at parse time, so this compiles to a constant.
        PairTag::Loc => {
            let file = if options.entry {
                "<top-level>".to_string()
            } else {
                children[0].text(&ir.files).unwrap_or("").to_string()
            };
            let line: i64 = children[1]
                .text(&ir.files)
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let mut fields = indexmap::IndexMap::new();
            fields.insert(strs::keyword_file(), Value::String(file.into()));
            fields.insert(strs::keyword_line(), Value::int(line));
            Expr::Literal(Literal::Value(Value::Object(std::rc::Rc::new(fields))))
        }
        PairTag::Invoke => {
            let args = children
                .iter()
                .map(|p| lower_expr(ir, p, symbols, options, false))
                .collect::<Result<_, _>>()?;
            let name = if text == ".." { "recurse" } else { &text };
            return lower_call(ir, symbols, name, args, pair.span.clone());
        }
        _ => {
            return Err(CompileError(format!(
                "{}: unsupported syntax {}",
                options.path, pair.tag
            )));
        }
    };
    Ok(ir.push(expr, pair.span.clone()))
}
/// `.a.b?` only guards the final path step: Invoke(".")[`.a`, Invoke("?")[`.b`]].
/// `?` must not shadow errors from `base` (e.g. `.a`), so `base` runs unprotected
/// and only the trailing steps run inside a try/catch, matching `.a | .b?`.
/// Kept out of `lower_expr` (never inlined) so this rarer path doesn't grow the
/// stack frame of that hot, deeply recursive function.
#[inline(never)]
fn lower_optional_path_suffix(
    ir: &mut Ir,
    pair: &Pair,
    base: ExprId,
    suffix: &Pair,
    symbols: &mut Symbols,
    options: &CompileOptions,
) -> Result<Expr, CompileError> {
    let Some(path) = suffix
        .semantic_children()
        .next()
        .filter(|p| p.tag == PairTag::Path)
    else {
        return Err(CompileError(
            "optional path suffix is not implemented yet".into(),
        ));
    };
    let steps = path
        .semantic_children()
        .map(|p| lower_path_step(ir, p, symbols, options))
        .collect::<Result<_, _>>()?;
    let input = ir.push(Expr::Input, pair.span.clone());
    let body = ir.push(Expr::Path { base: input, steps }, pair.span.clone());
    let handler = ir.push(Expr::Concat(vec![]), pair.span.clone());
    let guarded = ir.push(Expr::Try { body, handler }, pair.span.clone());
    Ok(Expr::Pipe(vec![base, guarded]))
}
// Consecutive definitions share a lexical scope. Lower their continuations in a
// loop so a prelude's length does not multiply lower_expr's debug stack frame.
fn lower_definitions<'a>(
    ir: &mut Ir,
    mut pair: &'a Pair,
    mut continuations: &[&'a Pair],
    symbols: &mut Symbols,
    options: &CompileOptions,
    export: bool,
) -> Result<ExprId, CompileError> {
    let mut functions = Vec::new();
    let span = pair.span.clone();
    loop {
        if pair.tag == PairTag::Empty && !continuations.is_empty() {
            pair = continuations[0];
            continuations = &continuations[1..];
        }
        if pair.tag != PairTag::Def {
            break;
        }
        let children: Vec<_> = pair.semantic_children().collect();
        let text = pair.text(&ir.files).unwrap_or("").to_owned();
        let count = children.len() - 2;
        let id = FunctionId(symbols.next_function);
        symbols.next_function += 1;
        symbols
            .names
            .functions
            .insert((strs::intern(&text), count), CallTarget::Function(id));
        if export {
            ir.export_functions.push(id);
        }
        let definition_names = symbols.names.clone();
        let mut params = Vec::new();
        let mut values = Vec::new();
        for param in &children[..count] {
            let name = strs::intern(param.text(&ir.files).unwrap_or(""));
            let binding = symbols.declare(name, param.span.clone());
            params.push(binding);
            if param.tag == PairTag::Var {
                values.push(binding);
            } else {
                symbols
                    .names
                    .functions
                    .insert((name, 0), CallTarget::FilterParameter(binding));
            }
        }
        let mut body = lower_expr(ir, children[count], symbols, options, false)?;
        for binding in values.into_iter().rev() {
            let source = ir.push(
                Expr::Call {
                    target: CallTarget::FilterParameter(binding),
                    args: vec![],
                },
                pair.span.clone(),
            );
            let pattern = PatternId(ir.patterns.len());
            ir.patterns.push(Pattern::Binding(binding));
            body = ir.push(
                Expr::Bind {
                    source,
                    pattern,
                    body,
                },
                pair.span.clone(),
            );
        }
        ir.functions.push(FunctionDef { id, params, body });
        symbols.names = definition_names;
        functions.push(id);
        pair = children[count + 1];
    }
    if !continuations.is_empty() {
        return Err(CompileError(format!(
            "{}: unsupported program structure",
            options.path
        )));
    }
    let body = lower_expr(ir, pair, symbols, options, export)?;
    Ok(ir.push(Expr::Scope { functions, body }, span))
}

fn lower_path_step(
    ir: &mut Ir,
    pair: &Pair,
    symbols: &mut Symbols,
    options: &CompileOptions,
) -> Result<PathStep, CompileError> {
    match pair.tag {
        PairTag::Spread => Ok(PathStep::Iterate),
        PairTag::Slice => {
            let bounds: Vec<_> = pair.semantic_children().collect();
            let start = Some(lower_expr(ir, bounds[0], symbols, options, false)?);
            let end = bounds
                .get(1)
                .map(|p| lower_expr(ir, p, symbols, options, false))
                .transpose()?;
            Ok(PathStep::Slice { start, end })
        }
        _ => Ok(PathStep::Index(lower_expr(
            ir, pair, symbols, options, false,
        )?)),
    }
}

fn lower_pattern(
    ir: &mut Ir,
    pair: &Pair,
    symbols: &mut Symbols,
    options: &CompileOptions,
    export: bool,
    minimum: usize,
) -> Result<PatternId, CompileError> {
    let pattern = match pair.tag {
        PairTag::Var => {
            let name = strs::intern(pair.text(&ir.files).unwrap_or(""));
            let id = symbols
                .lookup(name)
                .filter(|id| id.0 >= minimum)
                .unwrap_or_else(|| symbols.declare(name, pair.span.clone()));
            if export {
                ir.export_bindings.push(id);
            }
            Pattern::Binding(id)
        }
        PairTag::Array => Pattern::Array(
            pair.semantic_children()
                .map(|p| lower_pattern(ir, p, symbols, options, export, minimum))
                .collect::<Result<_, _>>()?,
        ),
        PairTag::Object => {
            let children: Vec<_> = pair.semantic_children().collect();
            let mut fields = Vec::new();
            for field in children.as_chunks::<2>().0 {
                let key = lower_expr(ir, field[0], symbols, options, false)?;
                let value = lower_pattern(ir, field[1], symbols, options, export, minimum)?;
                fields.push((key, value));
            }
            Pattern::Object(fields)
        }
        _ => return Err(CompileError("invalid binding pattern".into())),
    };
    let id = PatternId(ir.patterns.len());
    ir.patterns.push(pattern);
    Ok(id)
}

fn lower_string(
    ir: &mut Ir,
    pair: &Pair,
    symbols: &mut Symbols,
    options: &CompileOptions,
    formatter: Option<&str>,
) -> Result<ExprId, CompileError> {
    let children = pair.semantic_children();
    let add = builtins::lookup_symbol(strs::keyword_plus(), 2).unwrap();
    let stringify = builtins::lookup_symbol(strs::keyword_tostring(), 0).unwrap();
    let mut result = ir.push(
        Expr::Literal(Literal::Value(Value::String(String::new().into()))),
        pair.span.clone(),
    );
    for child in children {
        let piece = if child.tag == PairTag::StringChunk {
            let raw = child.text(&ir.files).unwrap_or("");
            let value = data::parse_json_str(&format!("\"{raw}\"")).map_err(CompileError)?;
            ir.push(Expr::Literal(Literal::Value(value)), child.span.clone())
        } else {
            let mut expression = lower_expr(ir, child, symbols, options, false)?;
            if let Some(formatter) = formatter {
                let format = lower_call(ir, symbols, formatter, vec![], child.span.clone())?;
                expression = ir.push(Expr::Pipe(vec![expression, format]), child.span.clone());
            }
            let call = ir.push(
                Expr::BuiltinCall {
                    builtin: stringify,
                    args: vec![],
                },
                child.span.clone(),
            );
            ir.push(Expr::Pipe(vec![expression, call]), child.span.clone())
        };
        result = ir.push(
            Expr::BuiltinCall {
                builtin: add,
                args: vec![result, piece],
            },
            pair.span.clone(),
        );
    }
    Ok(result)
}

fn lower_call(
    ir: &mut Ir,
    symbols: &Symbols,
    name: &str,
    args: Vec<ExprId>,
    span: Option<crate::jq::parser::pairs::SpanPos>,
) -> Result<ExprId, CompileError> {
    let symbol = strs::intern(name);
    let expr = if let Some(target) = symbols.names.functions.get(&(symbol, args.len())) {
        Expr::Call {
            target: target.clone(),
            args,
        }
    } else {
        let builtin = builtins::lookup_symbol(symbol, args.len())
            .ok_or_else(|| CompileError(format!("unsupported filter {name}/{}", args.len())))?;
        match builtins::spec(builtin).instr {
            builtins::BuiltinInstr::Path => Expr::Paths(args[0]),
            builtins::BuiltinInstr::Empty => Expr::Concat(vec![]),
            _ => Expr::BuiltinCall { builtin, args },
        }
    };
    Ok(ir.push(expr, span))
}
