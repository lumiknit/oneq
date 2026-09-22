//! Shared ID mapping for IR copying, substitution and traversal.
use crate::jq::ir::{
    BindingId, CallTarget, Expr, ExprId, FunctionId, Ir, LabelId, PathStep, Pattern, PatternId,
};

pub(super) trait Mapper {
    fn expr(&mut self, id: ExprId) -> ExprId {
        id
    }
    fn pattern(&mut self, id: PatternId) -> PatternId {
        id
    }
    fn binding(&mut self, id: BindingId) -> BindingId {
        id
    }
    fn function(&mut self, id: FunctionId) -> FunctionId {
        id
    }
    fn label(&mut self, id: LabelId) -> LabelId {
        id
    }
}

pub(super) fn map_expr(expr: &mut Expr, map: &mut impl Mapper) {
    match expr {
        Expr::Input | Expr::Literal(_) => {}
        Expr::Read(id) => *id = map.binding(*id),
        Expr::Break(id) => *id = map.label(*id),
        Expr::Paths(id) | Expr::Array(id) | Expr::Last(id) | Expr::ConstPath { base: id, .. } => {
            *id = map.expr(*id);
        }
        Expr::Pipe(ids) | Expr::Concat(ids) => {
            for id in ids {
                *id = map.expr(*id);
            }
        }
        Expr::Bind {
            source,
            pattern,
            body,
        } => {
            *source = map.expr(*source);
            *pattern = map.pattern(*pattern);
            *body = map.expr(*body);
        }
        Expr::Call { target, args } => {
            match target {
                CallTarget::Function(id) => *id = map.function(*id),
                CallTarget::FilterParameter(id) => *id = map.binding(*id),
            }
            for id in args {
                *id = map.expr(*id);
            }
        }
        Expr::BuiltinCall { args, .. } => {
            for id in args {
                *id = map.expr(*id);
            }
        }
        Expr::Scope { functions, body } => {
            for id in functions {
                *id = map.function(*id);
            }
            *body = map.expr(*body);
        }
        Expr::Object(fields) => {
            for (key, value) in fields {
                *key = map.expr(*key);
                *value = map.expr(*value);
            }
        }
        Expr::Path { base, steps } => {
            *base = map.expr(*base);
            for step in steps {
                match step {
                    PathStep::Index(id) => *id = map.expr(*id),
                    PathStep::Iterate | PathStep::Key(_) => {}
                    PathStep::Slice { start, end } => {
                        for id in start.iter_mut().chain(end.iter_mut()) {
                            *id = map.expr(*id);
                        }
                    }
                }
            }
        }
        Expr::If { condition, yes, no } => {
            *condition = map.expr(*condition);
            *yes = map.expr(*yes);
            *no = map.expr(*no);
        }
        Expr::Alternative { lhs, rhs } => {
            *lhs = map.expr(*lhs);
            *rhs = map.expr(*rhs);
        }
        Expr::Try { body, handler } => {
            *body = map.expr(*body);
            *handler = map.expr(*handler);
        }
        Expr::Reduce {
            source,
            pattern,
            init,
            update,
        }
        | Expr::Foreach {
            source,
            pattern,
            init,
            update,
            ..
        } => {
            *source = map.expr(*source);
            *pattern = map.pattern(*pattern);
            *init = map.expr(*init);
            *update = map.expr(*update);
            if let Expr::Foreach { extract, .. } = expr {
                *extract = map.expr(*extract);
            }
        }
        Expr::Label { label, body } => {
            *label = map.label(*label);
            *body = map.expr(*body);
        }
    }
}

pub(super) fn map_pattern(pattern: &mut Pattern, map: &mut impl Mapper) {
    match pattern {
        Pattern::Binding(id) => *id = map.binding(*id),
        Pattern::Array(ids) | Pattern::Alternatives(ids) => {
            for id in ids {
                *id = map.pattern(*id);
            }
        }
        Pattern::Object(fields) => {
            for (key, value) in fields {
                *key = map.expr(*key);
                *value = map.pattern(*value);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Item {
    Expr(ExprId),
    Pattern(PatternId),
}

#[derive(Default)]
struct Children(Vec<Item>);
impl Mapper for Children {
    fn expr(&mut self, id: ExprId) -> ExprId {
        self.0.push(Item::Expr(id));
        id
    }
    fn pattern(&mut self, id: PatternId) -> PatternId {
        self.0.push(Item::Pattern(id));
        id
    }
}

/// Structural postorder, including computed pattern keys and nested definitions.
/// Calls are references, not traversal edges (recursive calls therefore terminate).
pub fn postorder(ir: &Ir, root: ExprId) -> Vec<Item> {
    walk(ir, root, true)
}

pub(super) fn body_order(ir: &Ir, root: ExprId) -> Vec<Item> {
    walk(ir, root, false)
}

fn walk(ir: &Ir, root: ExprId, definitions: bool) -> Vec<Item> {
    let mut seen = std::collections::HashSet::new();
    let mut pending = vec![(Item::Expr(root), false)];
    let mut order = Vec::new();
    while let Some((item, expanded)) = pending.pop() {
        if expanded {
            order.push(item);
            continue;
        }
        if !seen.insert(item) {
            continue;
        }
        pending.push((item, true));
        let mut children = Children::default();
        match item {
            Item::Expr(id) => {
                let mut expr = ir.nodes[id.0].expr.clone();
                if let Expr::Scope { functions, .. } = &expr
                    && definitions
                {
                    for id in functions {
                        let def = ir
                            .functions
                            .iter()
                            .find(|f| f.id == *id)
                            .expect("local definition");
                        children.0.push(Item::Expr(def.body));
                    }
                }
                map_expr(&mut expr, &mut children);
            }
            Item::Pattern(id) => map_pattern(&mut ir.patterns[id.0].clone(), &mut children),
        }
        pending.extend(children.0.into_iter().rev().map(|id| (id, false)));
    }
    order
}
