//! Closed function templates. Names resolve through Symbols; templates use stable
//! `FunctionIds` so aliases, shadowing and REPL rollback share the same resolution.
use super::rewrite::{self, Item, Mapper};
use crate::jq::{
    ir::{
        BindingId, CallTarget, Expr, ExprId, FunctionDef, FunctionId, Ir, LabelId, Pattern,
        PatternId,
    },
    parser::pairs::SpanPos,
    symbols::Symbols,
};
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

pub type TemplateStore = HashMap<FunctionId, Rc<ExprTemplate>>;

#[derive(Debug)]
pub struct ExprTemplate {
    fragment: Fragment,
    params: Vec<BindingId>,
}

#[derive(Debug)]
struct Fragment {
    ir: Ir,
    root: ExprId,
}

impl Fragment {
    fn capture(ir: &Ir, root: ExprId) -> Self {
        let mut output = Ir {
            files: ir.files.clone(),
            ..Ir::default()
        };
        let root = copy(ir, root, &mut output, None, &HashMap::new());
        Self { ir: output, root }
    }

    fn cost(&self) -> usize {
        // Count edges too: a flat Pipe can be much larger than its node count.
        struct Cost(usize);
        impl Mapper for Cost {
            fn expr(&mut self, id: ExprId) -> ExprId {
                self.0 += 1;
                id
            }
            fn pattern(&mut self, id: PatternId) -> PatternId {
                self.0 += 1;
                id
            }
        }
        let mut cost = Cost(self.ir.nodes.len() + self.ir.patterns.len());
        for node in &self.ir.nodes {
            rewrite::map_expr(&mut node.expr.clone(), &mut cost);
        }
        for pattern in &self.ir.patterns {
            rewrite::map_pattern(&mut pattern.clone(), &mut cost);
        }
        cost.0
    }
}

#[derive(Default)]
struct Remap {
    exprs: HashMap<ExprId, ExprId>,
    patterns: HashMap<PatternId, PatternId>,
    bindings: HashMap<BindingId, BindingId>,
    functions: HashMap<FunctionId, FunctionId>,
    labels: HashMap<LabelId, LabelId>,
}
impl Mapper for Remap {
    fn expr(&mut self, id: ExprId) -> ExprId {
        self.exprs[&id]
    }
    fn pattern(&mut self, id: PatternId) -> PatternId {
        self.patterns[&id]
    }
    fn binding(&mut self, id: BindingId) -> BindingId {
        self.bindings.get(&id).copied().unwrap_or(id)
    }
    fn function(&mut self, id: FunctionId) -> FunctionId {
        self.functions.get(&id).copied().unwrap_or(id)
    }
    fn label(&mut self, id: LabelId) -> LabelId {
        self.labels.get(&id).copied().unwrap_or(id)
    }
}

/// Copy a structural fragment, optionally freshening its declarations. Only
/// declarations inside the fragment are renamed; free references retain their IDs.
/// Filter substitutions are copied independently and retain the caller's bindings.
fn copy(
    source: &Ir,
    root: ExprId,
    output: &mut Ir,
    mut symbols: Option<&mut Symbols>,
    substitutions: &HashMap<BindingId, Fragment>,
) -> ExprId {
    let order = rewrite::postorder(source, root);
    let mut map = Remap::default();
    let mut definitions = Vec::new();
    let mut bindings = Vec::new();
    let mut labels = Vec::new();
    for item in &order {
        match *item {
            Item::Pattern(id) => {
                if let Pattern::Binding(id) = source.patterns[id.0] {
                    bindings.push(id);
                }
            }
            Item::Expr(id) => match &source.nodes[id.0].expr {
                Expr::Label { label, .. } => labels.push(*label),
                Expr::Scope { functions, .. } => {
                    for id in functions {
                        if definitions.iter().any(|f: &FunctionDef| f.id == *id) {
                            continue;
                        }
                        let def = source
                            .functions
                            .iter()
                            .find(|f| f.id == *id)
                            .expect("local definition");
                        bindings.extend_from_slice(&def.params);
                        definitions.push(def.clone());
                    }
                }
                _ => {}
            },
        }
    }
    if let Some(symbols) = symbols.as_deref_mut() {
        for id in bindings {
            map.bindings.entry(id).or_insert_with(|| {
                let fresh = BindingId(symbols.bindings.len());
                symbols.bindings.push(symbols.bindings[id.0].clone());
                fresh
            });
        }
        for id in labels {
            map.labels.entry(id).or_insert_with(|| {
                let fresh = LabelId(symbols.next_label);
                symbols.next_label += 1;
                fresh
            });
        }
        for def in &definitions {
            map.functions
                .insert(def.id, FunctionId(symbols.next_function));
            symbols.next_function += 1;
        }
    }
    let mut files = HashMap::new();
    for item in order {
        match item {
            Item::Expr(id) => {
                let node = &source.nodes[id.0];
                if let Expr::Call {
                    target: CallTarget::FilterParameter(param),
                    args,
                } = &node.expr
                    && let Some(argument) = substitutions.get(param)
                {
                    debug_assert!(args.is_empty());
                    let copied = copy(
                        &argument.ir,
                        argument.root,
                        output,
                        symbols.as_deref_mut(),
                        &HashMap::new(),
                    );
                    map.exprs.insert(id, copied);
                    continue;
                }
                let mut expr = node.expr.clone();
                rewrite::map_expr(&mut expr, &mut map);
                let span = node
                    .span
                    .as_ref()
                    .map(|span| relocate(span, source, output, &mut files));
                let copied = output.push(expr, span);
                map.exprs.insert(id, copied);
            }
            Item::Pattern(id) => {
                let mut pattern = source.patterns[id.0].clone();
                rewrite::map_pattern(&mut pattern, &mut map);
                let copied = PatternId(output.patterns.len());
                output.patterns.push(pattern);
                map.patterns.insert(id, copied);
            }
        }
    }
    for def in definitions {
        output.functions.push(FunctionDef {
            id: map.function(def.id),
            params: def.params.into_iter().map(|id| map.binding(id)).collect(),
            body: map.expr(def.body),
        });
    }
    map.expr(root)
}

fn relocate(
    span: &SpanPos,
    source: &Ir,
    output: &mut Ir,
    files: &mut HashMap<usize, usize>,
) -> SpanPos {
    if Rc::ptr_eq(&source.files, &output.files) {
        return span.clone();
    }
    let file = *files.entry(span.file).or_insert_with(|| {
        let original = &source.files.files[span.file];
        output
            .files
            .files
            .iter()
            .position(|f| f.path == original.path && f.source == original.source)
            .unwrap_or_else(|| {
                Rc::make_mut(&mut output.files).add(original.path.clone(), original.source.clone())
            })
    });
    let origin = source.files.files[span.file].start;
    let start = output.files.files[file].start;
    SpanPos {
        file,
        start: start + span.start - origin,
        end: start + span.end - origin,
    }
}

fn template(ir: &Ir, def: &FunctionDef) -> Option<ExprTemplate> {
    let order = rewrite::postorder(ir, def.body);
    let mut declared = HashSet::new();
    let mut labels = HashSet::new();
    for item in &order {
        match *item {
            Item::Pattern(id) => {
                if let Pattern::Binding(id) = ir.patterns[id.0] {
                    declared.insert(id);
                }
            }
            Item::Expr(id) => {
                if let Expr::Label { label, .. } = ir.nodes[id.0].expr {
                    labels.insert(label);
                }
            }
        }
    }
    for item in order {
        if let Item::Expr(id) = item {
            match &ir.nodes[id.0].expr {
                Expr::Read(id) if !declared.contains(id) => return None,
                Expr::Break(id) if !labels.contains(id) => return None,
                Expr::Call {
                    target: CallTarget::FilterParameter(id),
                    ..
                } if !def.params.contains(id) => return None,
                // Remaining function calls include recursive and captured callees.
                // Nested definitions also stay on the ordinary closure path for now.
                Expr::Call {
                    target: CallTarget::Function(_),
                    ..
                }
                | Expr::Scope { .. } => return None,
                _ => {}
            }
        }
    }
    let fragment = Fragment::capture(ir, def.body);
    (fragment.cost() <= 512).then(|| ExprTemplate {
        fragment,
        params: def.params.clone(),
    })
}

/// Optimize private compilation state. Limits apply to expansion, never to the
/// accepted source program: exceeding them simply preserves a normal call.
pub(super) fn optimize(
    ir: &mut Ir,
    entry: &mut ExprId,
    symbols: &mut Symbols,
    inherited: &TemplateStore,
) -> TemplateStore {
    let mut templates = inherited.clone();
    let mut visited = HashSet::new();
    let mut budget = 16_384usize;
    // Lowering records lexical dependencies before their users. Self-recursion
    // has no template, so a single pass expands acyclic chains without a fixpoint.
    for definition in ir.functions.clone() {
        optimize_tree(
            ir,
            definition.body,
            symbols,
            &templates,
            &mut visited,
            &mut budget,
            true,
        );
        if let Some(template) = template(ir, &definition) {
            templates.insert(definition.id, Rc::new(template));
        }
    }
    optimize_tree(
        ir,
        *entry,
        symbols,
        &templates,
        &mut visited,
        &mut budget,
        false,
    );
    prune_functions(ir, *entry);
    compact(ir, entry);
    templates.retain(|id, _| ir.export_functions.contains(id) && !inherited.contains_key(id));
    templates
}

pub(super) fn compact(ir: &mut Ir, entry: &mut ExprId) {
    let compact = Fragment::capture(ir, *entry);
    let mut output = compact.ir;
    output.export_bindings = std::mem::take(&mut ir.export_bindings);
    output.export_functions = std::mem::take(&mut ir.export_functions);
    *entry = compact.root;
    *ir = output;
}

fn prune_functions(ir: &mut Ir, entry: ExprId) {
    let live = live_functions(ir, entry);
    for node in &mut ir.nodes {
        if let Expr::Scope { functions, .. } = &mut node.expr {
            functions.retain(|id| live.contains(id));
        }
    }
    ir.functions.retain(|def| live.contains(&def.id));
}

fn live_functions(ir: &Ir, entry: ExprId) -> HashSet<FunctionId> {
    let bodies: HashMap<_, _> = ir.functions.iter().map(|def| (def.id, def.body)).collect();
    let mut live: HashSet<_> = ir.export_functions.iter().copied().collect();
    let mut pending = vec![entry];
    pending.extend(live.iter().filter_map(|id| bodies.get(id)).copied());
    let mut seen = HashSet::new();
    while let Some(root) = pending.pop() {
        for item in rewrite::body_order(ir, root) {
            let Item::Expr(id) = item else {
                continue;
            };
            if !seen.insert(id) {
                continue;
            }
            if let Expr::Call {
                target: CallTarget::Function(id),
                ..
            } = ir.nodes[id.0].expr
                && live.insert(id)
                && let Some(body) = bodies.get(&id)
            {
                pending.push(*body);
            }
        }
    }
    live
}

fn optimize_tree(
    ir: &mut Ir,
    root: ExprId,
    symbols: &mut Symbols,
    templates: &TemplateStore,
    visited: &mut HashSet<ExprId>,
    budget: &mut usize,
    local_scopes: bool,
) {
    for item in rewrite::body_order(ir, root) {
        let Item::Expr(id) = item else {
            continue;
        };
        if !visited.insert(id) {
            continue;
        }
        let mut node = ir.nodes[id.0].clone();
        // Module entry scopes publish declarations to later entries in the
        // linker pipe. Only function-local scopes can be pruned in isolation.
        if let Expr::Scope { functions, body } = &mut node.expr
            && local_scopes
        {
            let live = live_functions(ir, *body);
            functions.retain(|id| live.contains(id));
        }
        if let Expr::Call {
            target: CallTarget::Function(function),
            args,
        } = &node.expr
            && let Some(template) = templates.get(function)
        {
            let substitutions: HashMap<_, _> = template
                .params
                .iter()
                .copied()
                .zip(args.iter().map(|arg| Fragment::capture(ir, *arg)))
                .collect();
            let mut cost = template.fragment.cost();
            for node in &template.fragment.ir.nodes {
                if let Expr::Call {
                    target: CallTarget::FilterParameter(param),
                    ..
                } = &node.expr
                {
                    cost = cost.saturating_add(substitutions[param].cost());
                }
            }
            if cost <= *budget {
                *budget -= cost;
                let expanded = copy(
                    &template.fragment.ir,
                    template.fragment.root,
                    ir,
                    Some(symbols),
                    &substitutions,
                );
                ir.nodes[id.0] = ir.nodes[expanded.0].clone();
                continue;
            }
        }
        let canonical = ir.push(node.expr, node.span);
        ir.nodes[id.0] = ir.nodes[canonical.0].clone();
    }
}
